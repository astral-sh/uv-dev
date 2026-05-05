//! Implementation of project sync operations - extracted from uv/src/commands/project/mod.rs

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tracing::{debug, warn};

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, FlatIndexClient, RegistryClientBuilder};
use uv_configuration::{
    Concurrency, Constraints, ExtrasSpecification, Preview, Reinstall, Upgrade,
};
use uv_dispatch::{BuildDispatch, PlatformState};
use uv_distribution::DistributionDatabase;
use uv_distribution_types::{Index, Resolution, UnresolvedRequirement, UnresolvedRequirementSpecification};
use uv_fs::Simplified;
use uv_git::ResolvedRepositoryReference;
use uv_installer::SitePackages;
use uv_python::{Interpreter, PythonEnvironment, PythonRequirement};
use uv_requirements::{RequirementsSpecification, LockedRequirements, read_lock_requirements};
use uv_resolver::{
    FlatIndex, Lock, OptionsBuilder, Preference, ResolverEnvironment, ResolverOutput,
};
use uv_types::{BuildIsolation, EmptyInstalledPackages, HashStrategy};
use uv_warnings::warn_user_once;
use uv_workspace::WorkspaceCache;

use crate::error::ProjectError;
use crate::settings::InstallerSettingsRef;
use uv_cli_common::settings::{NetworkSettings, ResolverSettings};
use uv_cli_pip::loggers::{InstallLogger, ResolveLogger};
use uv_cli_pip::operations::{self, Modifications, Changelog};
use uv_cli_common::printer::Printer;

/// The preference for resolving packages in an environment.
#[derive(Debug, Clone)]
pub enum PreferenceLocation<'lock> {
    /// The preferences should be extracted from a lockfile.
    Lock {
        lock: &'lock Lock,
        install_path: &'lock Path,
    },
    /// The preferences will be provided directly as [`Preference`] entries.
    Entries(Vec<Preference>),
}

/// A specification for an environment.
#[derive(Debug, Clone)]
pub struct EnvironmentSpecification<'lock> {
    /// The requirements to include in the environment.
    pub requirements: RequirementsSpecification,
    /// The preferences to respect when resolving.
    pub preferences: Option<PreferenceLocation<'lock>>,
}

impl From<RequirementsSpecification> for EnvironmentSpecification<'_> {
    fn from(requirements: RequirementsSpecification) -> Self {
        Self {
            requirements,
            preferences: None,
        }
    }
}

impl<'lock> EnvironmentSpecification<'lock> {
    /// Set the [`PreferenceLocation`] for the specification.
    #[must_use]
    pub fn with_preferences(self, preferences: PreferenceLocation<'lock>) -> Self {
        Self {
            preferences: Some(preferences),
            ..self
        }
    }
}

/// The result of updating an environment.
#[derive(Debug)]
pub struct EnvironmentUpdate {
    /// The updated [`PythonEnvironment`].
    pub environment: PythonEnvironment,
    /// The [`Changelog`] of changes made to the environment.
    pub changelog: Changelog,
}

impl EnvironmentUpdate {
    /// Convert the [`EnvironmentUpdate`] into a [`PythonEnvironment`].
    pub fn into_environment(self) -> PythonEnvironment {
        self.environment
    }
}

/// Warn if the user provides (e.g.) an `--index-url` in a requirements file.
fn warn_on_requirements_txt_setting(spec: &RequirementsSpecification, settings: &ResolverSettings) {
    let RequirementsSpecification {
        index_url,
        extra_index_urls,
        no_index,
        find_links,
        no_binary,
        no_build,
        ..
    } = spec;

    if settings.index_locations.no_index() {
        // Nothing to do, we're ignoring the URLs anyway.
    } else if *no_index {
        warn_user_once!(
            "Ignoring `--no-index` from requirements file. Instead, use the `--no-index` command-line argument, or set `no-index` in a `uv.toml` or `pyproject.toml` file."
        );
    } else {
        if let Some(index_url) = index_url {
            warn_user_once!(
                "Ignoring index URL (`{index_url}`) from requirements file. Instead, use the `--index-url` command-line argument, or set `index-url` in a `uv.toml` or `pyproject.toml` file."
            );
        }
        if !extra_index_urls.is_empty() {
            let extra_index_urls = extra_index_urls
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            warn_user_once!(
                "Ignoring extra index URLs ({extra_index_urls}) from requirements file. Instead, use the `--extra-index-url` command-line argument, or set `extra-index-url` in a `uv.toml` or `pyproject.toml` file.`"
            );
        }
    }

    if !find_links.is_empty() {
        let find_links = find_links
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        warn_user_once!(
            "Ignoring find links ({find_links}) from requirements file. Instead, use the `--find-links` command-line argument, or set `find-links` in a `uv.toml` or `pyproject.toml` file."
        );
    }

    if let Some(no_binary) = no_binary {
        warn_user_once!(
            "Ignoring `--no-binary` setting ({no_binary}) from requirements file. Instead, use the `--no-binary` command-line argument, or set `no-binary` in a `uv.toml` or `pyproject.toml` file."
        );
    }

    if let Some(no_build) = no_build {
        warn_user_once!(
            "Ignoring `--only-binary` setting ({no_build}) from requirements file. Instead, use the `--only-binary` command-line argument, or set `only-binary` in a `uv.toml` or `pyproject.toml` file."
        );
    }
}

/// Run dependency resolution for an interpreter, returning the [`ResolverOutput`].
pub async fn resolve_environment(
    spec: EnvironmentSpecification<'_>,
    interpreter: &Interpreter,
    build_constraints: Constraints,
    settings: &ResolverSettings,
    network_settings: &NetworkSettings,
    state: &PlatformState,
    logger: Box<dyn ResolveLogger>,
    concurrency: Concurrency,
    cache: &Cache,
    printer: Printer,
    preview: Preview,
) -> Result<ResolverOutput, ProjectError> {
    warn_on_requirements_txt_setting(&spec.requirements, settings);

    let ResolverSettings {
        index_locations,
        index_strategy,
        keyring_provider,
        resolution,
        prerelease_mode,
        dependency_mode,
        dependency_metadata,
        config_setting,
        config_settings_package,
        exclude_newer,
        flat_index: _,
        annotation_style: _,
        build_options,
        source_strategy,
        upgrade,
    } = settings;

    // Respect all requirements from the provided sources.
    let RequirementsSpecification {
        project,
        requirements,
        constraints,
        overrides,
        source_trees,
        ..
    } = spec.requirements;

    let client_builder = BaseClientBuilder::new()
        .retries_from_env()?
        .connectivity(network_settings.connectivity)
        .native_tls(network_settings.native_tls)
        .keyring(*keyring_provider)
        .allow_insecure_host(network_settings.allow_insecure_host.clone());

    // Determine the tags, markers, and interpreter to use for resolution.
    let tags = interpreter.tags()?;
    let marker_env = interpreter.resolver_marker_environment();
    let python_requirement = PythonRequirement::from_interpreter(interpreter);

    // TODO: index_locations.cache_index_credentials();

    // Initialize the registry client.
    let client = RegistryClientBuilder::try_from(client_builder)?
        .cache(cache.clone())
        .index_locations(index_locations.clone())
        .index_strategy(*index_strategy)
        .markers(interpreter.markers())
        .platform(interpreter.platform())
        .build();

    // Determine whether to enable build isolation.
    let environment;
    let build_isolation = if todo!("no_build_isolation") {
        environment = PythonEnvironment::from_interpreter(interpreter.clone());
        BuildIsolation::Shared(&environment)
    } else if todo!("no_build_isolation_package.is_empty()") {
        BuildIsolation::Isolated
    } else {
        environment = PythonEnvironment::from_interpreter(interpreter.clone());
        BuildIsolation::SharedPackage(&environment, todo!("no_build_isolation_package"))
    };

    let options = OptionsBuilder::new()
        .resolution_mode(*resolution)
        .prerelease_mode(*prerelease_mode)
        .dependency_mode(*dependency_mode)
        .exclude_newer(*exclude_newer)
        .index_strategy(*index_strategy)
        .build_options(build_options.clone())
        .build();

    // TODO(charlie): These are all default values. We should consider whether we want to make them
    // optional on the downstream APIs.
    let extras = ExtrasSpecification::default();
    let groups = BTreeMap::new();
    let hasher = HashStrategy::default();
    let build_hasher = HashStrategy::default();

    // When resolving from an interpreter, we assume an empty environment, so reinstalls and
    // upgrades aren't relevant.
    let reinstall = Reinstall::default();
    let upgrade = upgrade.clone().unwrap_or_default();

    // If an existing lockfile exists, build up a set of preferences.
    let preferences = match spec.preferences {
        Some(PreferenceLocation::Lock { lock, install_path }) => {
            let LockedRequirements { preferences, git } =
                read_lock_requirements(lock, install_path, &upgrade)?;

            // Populate the Git resolver.
            for ResolvedRepositoryReference { reference, sha } in git {
                debug!("Inserting Git reference into resolver: `{reference:?}` at `{sha}`");
                state.git().insert(reference, sha);
            }

            preferences
        }
        Some(PreferenceLocation::Entries(entries)) => entries,
        None => vec![],
    };

    // Resolve the flat indexes from `--find-links`.
    let flat_index = {
        let client = FlatIndexClient::new(client.cached_client(), client.connectivity(), cache);
        let entries = client
            .fetch_all(index_locations.flat_indexes().map(Index::url))
            .await?;
        FlatIndex::from_entries(entries, Some(tags), &hasher, build_options)
    };

    let workspace_cache = WorkspaceCache::default();

    // Create a build dispatch.
    let resolve_dispatch = BuildDispatch::new(
        &client,
        cache,
        build_constraints,
        interpreter,
        index_locations,
        &flat_index,
        dependency_metadata,
        state.clone().into_inner(),
        *index_strategy,
        config_setting,
        config_settings_package,
        build_isolation,
        todo!("link_mode"),
        build_options,
        &build_hasher,
        *exclude_newer,
        *source_strategy,
        workspace_cache,
        concurrency,
        preview,
    );

    // Resolve the requirements.
    Ok(operations::resolve(
        requirements,
        constraints,
        overrides,
        source_trees,
        project,
        BTreeSet::default(),
        &extras,
        &groups,
        preferences,
        EmptyInstalledPackages,
        &hasher,
        &reinstall,
        &upgrade,
        Some(tags),
        ResolverEnvironment::specific(marker_env),
        python_requirement,
        interpreter.markers(),
        todo!("Conflicts::empty()"),
        &client,
        &flat_index,
        state.index(),
        &resolve_dispatch,
        concurrency,
        options,
        logger,
        printer,
    )
    .await?)
}

/// Sync a [`PythonEnvironment`] with a set of resolved requirements.
pub async fn sync_environment(
    venv: PythonEnvironment,
    resolution: &Resolution,
    modifications: Modifications,
    build_constraints: Constraints,
    settings: InstallerSettingsRef<'_>,
    network_settings: &NetworkSettings,
    concurrency: &Concurrency,
    build_dispatch: &BuildDispatch<'_>,
    cache: &Cache,
    printer: Printer,
    dry_run: bool,
    preview: Preview,
) -> Result<PythonEnvironment, ProjectError> {
    let InstallerSettingsRef {
        index_locations,
        index_strategy,
        keyring_provider,
        dependency_metadata,
        config_setting,
        config_settings_package,
        no_build_isolation,
        no_build_isolation_package,
        exclude_newer,
        link_mode,
        compile_bytecode,
        reinstall,
        build_options,
        sources,
    } = settings;

    let site_packages = SitePackages::from_environment(&venv)?;

    // Determine the markers tags to use for resolution.
    let interpreter = venv.interpreter();

    // Sync the environment.
    operations::install(
        resolution,
        site_packages,
        modifications,
        reinstall,
        build_options,
        link_mode,
        compile_bytecode,
        index_locations,
        config_setting,
        config_settings_package,
        &HashStrategy::default(),
        todo!("tags"),
        todo!("&client"),
        todo!("state.in_flight()"),
        *concurrency,
        build_dispatch,
        cache,
        &venv,
        todo!("logger"),
        todo!("installer_metadata"),
        todo!("dry_run"),
        printer,
    )
    .await?;

    // Notify the user of any resolution diagnostics.
    operations::diagnose_resolution(resolution.diagnostics(), printer)?;

    Ok(venv)
}

// TODO: Implement resolve_names and update_environment functions
