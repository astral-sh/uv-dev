//! Resolve requirements and synchronize their selected environment.

use std::collections::BTreeMap;
use std::path::Path;

use itertools::Itertools;
use tracing::debug;

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_command_support::Printer;
use uv_configuration::{
    Concurrency, Constraints, DryRun, ExtrasSpecification, Modifications, Reinstall, TargetTriple,
    Upgrade,
};
use uv_dispatch::{BuildDispatch, PlatformState, SharedState};
use uv_distribution::LoweredExtraBuildDependencies;
use uv_distribution_types::{
    ExtraBuildRequires, HashCollection, Index, RequiresPython, Resolution,
};
use uv_git::ResolvedRepositoryReference;
use uv_install_operations::Changelog;
use uv_install_operations::loggers::InstallLogger;
use uv_installer::{InstallationStrategy, SatisfiesResult, SitePackages};
use uv_lock::Lock;
use uv_preview::Preview;
use uv_pypi_types::Conflicts;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_requirements::RequirementsSpecification;
use uv_resolve_operations::locked_requirements::{LockedRequirements, read_lock_requirements};
use uv_resolve_operations::loggers::ResolveLogger;
use uv_resolver::{
    DependencyMode, FlatIndex, OptionsBuilder, Preference, PythonRequirement, ResolverEnvironment,
    ResolverOutput,
};
use uv_settings::{InstallerSettingsRef, ResolverInstallerSettings, ResolverSettings};
use uv_torch::TorchStrategy;
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy};
use uv_warnings::warn_user_once;
use uv_workspace::WorkspaceCache;

use crate::EnvironmentError;

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

#[derive(Debug, Clone)]
pub struct EnvironmentSpecification<'lock> {
    /// The requirements to include in the environment.
    requirements: RequirementsSpecification,
    /// The preferences to respect when resolving.
    preferences: Option<PreferenceLocation<'lock>>,
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

#[derive(Clone, Copy)]
pub enum EnvironmentResolution {
    Specific,
    Universal,
}

/// Run dependency resolution for an interpreter, returning the [`ResolverOutput`].
pub async fn resolve_environment(
    spec: EnvironmentSpecification<'_>,
    resolution_scope: EnvironmentResolution,
    interpreter: &Interpreter,
    python_platform: Option<&TargetTriple>,
    source_tree_editable_policy: SourceTreeEditablePolicy,
    build_constraints: Constraints,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    state: &PlatformState,
    logger: Box<dyn ResolveLogger>,
    concurrency: &Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> Result<ResolverOutput, EnvironmentError> {
    warn_on_requirements_txt_setting(&spec.requirements, settings);

    let ResolverSettings {
        index_locations,
        index_strategy,
        keyring_provider,
        resolution,
        prerelease,
        fork_strategy,
        dependency_metadata,
        config_setting,
        config_settings_package,
        build_isolation,
        build_hash_checking,
        extra_build_dependencies,
        extra_build_variables,
        exclude_newer,
        link_mode,
        upgrade,
        build_options,
        sources,
        torch_backend,
        cuda_driver_version,
        amd_gpu_architecture,
    } = settings;

    // Respect all requirements from the provided sources.
    let RequirementsSpecification {
        project,
        requirements,
        constraints,
        overrides,
        override_dependencies,
        excludes,
        source_trees,
        ..
    } = spec.requirements;

    let client_builder = client_builder.clone().keyring(*keyring_provider);

    // Determine the tags and marker environment to use for resolution.
    let (tags, resolver_environment) = match resolution_scope {
        EnvironmentResolution::Specific => {
            let tags = uv_resolve_operations::resolution_tags(None, python_platform, interpreter)?;
            let marker_environment =
                uv_resolve_operations::resolution_markers(None, python_platform, interpreter);
            (
                Some(tags),
                ResolverEnvironment::specific(marker_environment),
            )
        }
        EnvironmentResolution::Universal => (None, ResolverEnvironment::universal(Vec::new())),
    };
    let python_requirement = match resolution_scope {
        EnvironmentResolution::Specific => PythonRequirement::from_interpreter(interpreter),
        EnvironmentResolution::Universal => PythonRequirement::from_requires_python(
            interpreter,
            RequiresPython::greater_than_equal_version(&interpreter.python_minor_version()),
        ),
    };

    let python_platform = match resolution_scope {
        EnvironmentResolution::Specific => python_platform,
        EnvironmentResolution::Universal => None,
    };

    // Determine the PyTorch backend.
    let torch_backend = torch_backend
        .map(|mode| {
            TorchStrategy::from_mode(
                mode,
                python_platform
                    .map(|t| t.platform())
                    .as_ref()
                    .unwrap_or(interpreter.platform())
                    .os(),
                cuda_driver_version.clone(),
                *amd_gpu_architecture,
            )
        })
        .transpose()?;

    // Initialize the registry client.
    let client = RegistryClientBuilder::new(client_builder, cache.clone())
        .index_locations(index_locations.clone())
        .index_strategy(*index_strategy)
        .torch_backend(torch_backend.clone())
        .markers(interpreter.markers())
        .platform(interpreter.platform())
        .build()?;

    // Determine whether to enable build isolation.
    let environment;
    let build_isolation = match build_isolation {
        uv_configuration::BuildIsolation::Isolate => BuildIsolation::Isolated,
        uv_configuration::BuildIsolation::Shared => {
            environment = PythonEnvironment::from_interpreter(interpreter.clone());
            BuildIsolation::Shared(&environment)
        }
        uv_configuration::BuildIsolation::SharedPackage(packages) => {
            environment = PythonEnvironment::from_interpreter(interpreter.clone());
            BuildIsolation::SharedPackage(&environment, packages)
        }
    };

    let options = OptionsBuilder::new()
        .resolution_mode(*resolution)
        .prerelease(prerelease.clone())
        .fork_strategy(*fork_strategy)
        .exclude_newer(exclude_newer.clone())
        .index_strategy(*index_strategy)
        .build_options(build_options.clone())
        .build();

    // TODO(charlie): These are all default values. We should consider whether we want to make them
    // optional on the downstream APIs.
    let extras = ExtrasSpecification::default();
    let groups = BTreeMap::new();
    let hasher = match resolution_scope {
        EnvironmentResolution::Specific => HashStrategy::default(),
        EnvironmentResolution::Universal => HashStrategy::collect(HashCollection::Url),
    };
    let build_hasher = HashStrategy::from_constraints(
        &build_constraints,
        Some(&interpreter.to_resolver_marker_environment()),
        *build_hash_checking,
    )?;

    // When resolving from an interpreter, we assume an empty environment, so reinstalls aren't
    // relevant. Upgrades are only relevant for universal resolutions that use an existing lock as
    // a preference source.
    let reinstall = Reinstall::default();
    let upgrade = match resolution_scope {
        EnvironmentResolution::Specific => Upgrade::default(),
        EnvironmentResolution::Universal => upgrade.clone(),
    };

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
    let flat_index = FlatIndex::load(&client, cache, index_locations).await?;

    // Lower the extra build dependencies, if any.
    let extra_build_requires =
        LoweredExtraBuildDependencies::from_non_lowered(extra_build_dependencies.clone())
            .into_inner();

    // Create a build dispatch.
    let resolve_dispatch = BuildDispatch::new(
        &client,
        cache,
        &build_constraints,
        interpreter,
        index_locations,
        &flat_index,
        dependency_metadata,
        state.clone().into_inner(),
        *index_strategy,
        config_setting,
        config_settings_package,
        build_isolation,
        &extra_build_requires,
        extra_build_variables,
        *link_mode,
        build_options,
        &build_hasher,
        exclude_newer.clone(),
        sources.clone(),
        source_tree_editable_policy,
        workspace_cache.clone(),
        concurrency.clone(),
        preview,
    );

    // Resolve the requirements.
    Ok(uv_resolve_operations::resolve(
        requirements,
        constraints,
        overrides,
        override_dependencies,
        excludes,
        source_trees,
        project,
        BTreeMap::default(),
        &extras,
        &groups,
        preferences,
        None,
        &hasher,
        &reinstall,
        &upgrade,
        tags.as_deref(),
        resolver_environment,
        python_requirement,
        interpreter.markers(),
        Conflicts::empty(),
        &client,
        &flat_index,
        state.index(),
        &resolve_dispatch,
        concurrency,
        options,
        None,
        logger,
        printer,
    )
    .await?
    .0)
}

/// Sync a [`PythonEnvironment`] with a set of resolved requirements.
pub async fn sync_environment(
    venv: PythonEnvironment,
    resolution: &Resolution,
    hasher: HashStrategy,
    modifications: Modifications,
    build_constraints: Constraints,
    settings: InstallerSettingsRef<'_>,
    client_builder: &BaseClientBuilder<'_>,
    state: &PlatformState,
    logger: Box<dyn InstallLogger>,
    installer_metadata: bool,
    concurrency: &Concurrency,
    cache: &Cache,
    printer: Printer,
    preview: Preview,
) -> Result<PythonEnvironment, EnvironmentError> {
    let InstallerSettingsRef {
        index_locations,
        index_strategy,
        keyring_provider,
        dependency_metadata,
        config_setting,
        config_settings_package,
        build_isolation,
        build_hash_checking,
        extra_build_dependencies,
        extra_build_variables,
        exclude_newer,
        link_mode,
        compile_bytecode,
        reinstall,
        build_options,
        sources,
    } = settings;

    let client_builder = client_builder.clone().keyring(keyring_provider);

    let site_packages = SitePackages::from_environment(&venv)?;

    // Determine the markers tags to use for resolution.
    let interpreter = venv.interpreter();
    let tags = venv.interpreter().tags()?;

    // Initialize the registry client.
    let client = RegistryClientBuilder::new(client_builder, cache.clone())
        .index_locations(index_locations.clone())
        .index_strategy(index_strategy)
        .markers(interpreter.markers())
        .platform(interpreter.platform())
        .build()?;

    // Determine whether to enable build isolation.
    let build_isolation = match build_isolation {
        uv_configuration::BuildIsolation::Isolate => BuildIsolation::Isolated,
        uv_configuration::BuildIsolation::Shared => BuildIsolation::Shared(&venv),
        uv_configuration::BuildIsolation::SharedPackage(packages) => {
            BuildIsolation::SharedPackage(&venv, packages)
        }
    };

    let build_hasher = HashStrategy::from_constraints(
        &build_constraints,
        Some(&interpreter.to_resolver_marker_environment()),
        build_hash_checking,
    )?;
    // TODO(charlie): These are all default values. We should consider whether we want to make them
    // optional on the downstream APIs.
    let dry_run = DryRun::default();
    let workspace_cache = WorkspaceCache::default();

    // Resolve the flat indexes from `--find-links`.
    let flat_index = FlatIndex::load(&client, cache, index_locations).await?;

    // Lower the extra build dependencies, if any.
    let extra_build_requires =
        LoweredExtraBuildDependencies::from_non_lowered(extra_build_dependencies.clone())
            .into_inner();

    // Create a build dispatch.
    let build_dispatch = BuildDispatch::new(
        &client,
        cache,
        &build_constraints,
        interpreter,
        index_locations,
        &flat_index,
        dependency_metadata,
        state.clone().into_inner(),
        index_strategy,
        config_setting,
        config_settings_package,
        build_isolation,
        &extra_build_requires,
        extra_build_variables,
        link_mode,
        build_options,
        &build_hasher,
        exclude_newer.clone(),
        sources,
        SourceTreeEditablePolicy::Project,
        workspace_cache,
        concurrency.clone(),
        preview,
    );

    // Sync the environment.
    uv_install_operations::install(
        resolution,
        site_packages,
        InstallationStrategy::Permissive,
        modifications,
        reinstall,
        build_options,
        link_mode,
        compile_bytecode.then_some(uv_install_operations::BytecodeCompilation::All),
        &hasher,
        tags,
        &client,
        state.in_flight(),
        concurrency,
        &build_dispatch,
        cache,
        &venv,
        logger,
        installer_metadata,
        dry_run,
        printer,
        preview,
    )
    .await?;

    // Notify the user of any resolution diagnostics.
    uv_resolve_operations::diagnose_resolution(resolution.diagnostics(), printer)?;

    Ok(venv)
}

/// The result of updating a [`PythonEnvironment`] to satisfy a [`RequirementsSpecification`].
#[derive(Debug)]
pub struct EnvironmentUpdate {
    /// The updated [`PythonEnvironment`].
    pub environment: PythonEnvironment,
    /// The [`Changelog`] of changes made to the environment.
    pub changelog: Changelog,
}

/// Update a [`PythonEnvironment`] to satisfy a [`RequirementsSpecification`].
pub async fn update_environment(
    venv: PythonEnvironment,
    spec: RequirementsSpecification,
    modifications: Modifications,
    python_platform: Option<&TargetTriple>,
    source_tree_editable_policy: SourceTreeEditablePolicy,
    build_constraints: Constraints,
    extra_build_requires: ExtraBuildRequires,
    settings: &ResolverInstallerSettings,
    client_builder: &BaseClientBuilder<'_>,
    state: &SharedState,
    resolve: Box<dyn ResolveLogger>,
    install: Box<dyn InstallLogger>,
    installer_metadata: bool,
    concurrency: &Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    dry_run: DryRun,
    printer: Printer,
    preview: Preview,
) -> Result<EnvironmentUpdate, EnvironmentError> {
    warn_on_requirements_txt_setting(&spec, &settings.resolver);

    let ResolverInstallerSettings {
        resolver:
            ResolverSettings {
                build_options,
                config_setting,
                config_settings_package,
                dependency_metadata,
                exclude_newer,
                fork_strategy,
                index_locations,
                index_strategy,
                keyring_provider,
                link_mode,
                build_isolation,
                build_hash_checking,
                extra_build_dependencies: _,
                extra_build_variables,
                prerelease,
                resolution,
                sources,
                torch_backend,
                cuda_driver_version,
                amd_gpu_architecture,
                upgrade,
            },
        compile_bytecode,
        reinstall,
    } = settings;

    let client_builder = client_builder.clone().keyring(*keyring_provider);

    // Respect all requirements from the provided sources.
    let RequirementsSpecification {
        project,
        requirements,
        constraints,
        overrides,
        override_dependencies,
        excludes,
        source_trees,
        ..
    } = spec;

    // Determine markers and tags to use for resolution.
    let interpreter = venv.interpreter();
    let marker_env = uv_resolve_operations::resolution_markers(None, python_platform, interpreter);
    let tags = uv_resolve_operations::resolution_tags(None, python_platform, interpreter)?;

    // Check if the current environment satisfies the requirements
    let site_packages = SitePackages::from_environment(&venv)?;
    if reinstall.is_none()
        && upgrade.is_none()
        && source_trees.is_empty()
        && matches!(modifications, Modifications::Sufficient)
    {
        match site_packages.satisfies_spec(
            &requirements,
            &constraints,
            &overrides,
            &override_dependencies,
            &excludes,
            dependency_metadata,
            DependencyMode::Transitive,
            InstallationStrategy::Permissive,
            &marker_env,
            &tags,
            config_setting,
            config_settings_package,
            &extra_build_requires,
            extra_build_variables,
        )? {
            // If the requirements are already satisfied, we're done.
            SatisfiesResult::Fresh {
                recursive_requirements,
            } => {
                if recursive_requirements.is_empty() {
                    debug!("No requirements to install");
                } else {
                    debug!(
                        "All requirements satisfied: {}",
                        recursive_requirements
                            .iter()
                            .map(ToString::to_string)
                            .sorted()
                            .join(" | ")
                    );
                }
                return Ok(EnvironmentUpdate {
                    environment: venv,
                    changelog: Changelog::default(),
                });
            }
            SatisfiesResult::Unsatisfied(requirement) => {
                debug!("At least one requirement is not satisfied: {requirement}");
            }
        }
    }

    // Determine the PyTorch backend.
    let torch_backend = torch_backend
        .map(|mode| {
            TorchStrategy::from_mode(
                mode,
                python_platform
                    .map(|t| t.platform())
                    .as_ref()
                    .unwrap_or(interpreter.platform())
                    .os(),
                cuda_driver_version.clone(),
                *amd_gpu_architecture,
            )
        })
        .transpose()?;

    // Initialize the registry client.
    let client = RegistryClientBuilder::new(client_builder, cache.clone())
        .index_locations(index_locations.clone())
        .index_strategy(*index_strategy)
        .torch_backend(torch_backend.clone())
        .markers(interpreter.markers())
        .platform(interpreter.platform())
        .build()?;

    // Determine whether to enable build isolation.
    let build_isolation = match build_isolation {
        uv_configuration::BuildIsolation::Isolate => BuildIsolation::Isolated,
        uv_configuration::BuildIsolation::Shared => BuildIsolation::Shared(&venv),
        uv_configuration::BuildIsolation::SharedPackage(packages) => {
            BuildIsolation::SharedPackage(&venv, packages)
        }
    };

    let options = OptionsBuilder::new()
        .resolution_mode(*resolution)
        .prerelease(prerelease.clone())
        .fork_strategy(*fork_strategy)
        .exclude_newer(exclude_newer.clone())
        .index_strategy(*index_strategy)
        .build_options(build_options.clone())
        .build();

    let build_hasher = HashStrategy::from_constraints(
        &build_constraints,
        Some(&interpreter.to_resolver_marker_environment()),
        *build_hash_checking,
    )?;
    // TODO(charlie): These are all default values. We should consider whether we want to make them
    // optional on the downstream APIs.
    let extras = ExtrasSpecification::default();
    let groups = BTreeMap::new();
    let hasher = HashStrategy::default();
    let preferences = Vec::default();

    // Determine the tags to use for resolution.
    let python_requirement = PythonRequirement::from_interpreter(interpreter);

    // Resolve the flat indexes from `--find-links`.
    let flat_index = FlatIndex::load(&client, cache, index_locations).await?;

    // Create a build dispatch.
    let build_dispatch = BuildDispatch::new(
        &client,
        cache,
        &build_constraints,
        interpreter,
        index_locations,
        &flat_index,
        dependency_metadata,
        state.clone(),
        *index_strategy,
        config_setting,
        config_settings_package,
        build_isolation,
        &extra_build_requires,
        extra_build_variables,
        *link_mode,
        build_options,
        &build_hasher,
        exclude_newer.clone(),
        sources.clone(),
        source_tree_editable_policy,
        workspace_cache.clone(),
        concurrency.clone(),
        preview,
    );

    // Resolve the requirements.
    let (resolution, hasher) = match uv_resolve_operations::resolve(
        requirements,
        constraints,
        overrides,
        override_dependencies,
        excludes,
        source_trees,
        project,
        BTreeMap::default(),
        &extras,
        &groups,
        preferences,
        Some(site_packages.clone()),
        &hasher,
        reinstall,
        upgrade,
        Some(&tags),
        ResolverEnvironment::specific(marker_env.clone()),
        python_requirement,
        venv.interpreter().markers(),
        Conflicts::empty(),
        &client,
        &flat_index,
        state.index(),
        &build_dispatch,
        concurrency,
        options,
        None,
        resolve,
        printer,
    )
    .await
    {
        Ok((resolution, hasher)) => (Resolution::from(resolution), hasher),
        Err(err) => return Err(err.into()),
    };
    // Sync the environment.
    let changelog = uv_install_operations::install(
        &resolution,
        site_packages,
        InstallationStrategy::Permissive,
        modifications,
        reinstall,
        build_options,
        *link_mode,
        (*compile_bytecode).then_some(uv_install_operations::BytecodeCompilation::All),
        &hasher,
        &tags,
        &client,
        state.in_flight(),
        concurrency,
        &build_dispatch,
        cache,
        &venv,
        install,
        installer_metadata,
        dry_run,
        printer,
        preview,
    )
    .await?;

    // Notify the user of any resolution diagnostics.
    uv_resolve_operations::diagnose_resolution(resolution.diagnostics(), printer)?;

    Ok(EnvironmentUpdate {
        environment: venv,
        changelog,
    })
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
            if settings.index_locations.default_index().map(Index::url) != Some(index_url) {
                warn_user_once!(
                    "Ignoring `--index-url` value `{index_url}` from requirements file. Instead, use the `--index-url` command-line argument, or set `index-url` in a `uv.toml` or `pyproject.toml` file."
                );
            }
        }
        for extra_index_url in extra_index_urls {
            if !settings
                .index_locations
                .implicit_indexes()
                .any(|index| index.url() == extra_index_url)
            {
                warn_user_once!(
                    "Ignoring `--extra-index-url` value `{extra_index_url}` from requirements file. Instead, use the `--extra-index-url` command-line argument, or set `extra-index-url` in a `uv.toml` or `pyproject.toml` file."
                );
            }
        }
        for find_link in find_links {
            if !settings
                .index_locations
                .flat_indexes()
                .any(|index| index.url() == find_link)
            {
                warn_user_once!(
                    "Ignoring `--find-links` value `{find_link}` from requirements file. Instead, use the `--find-links` command-line argument, or set `find-links` in a `uv.toml` or `pyproject.toml` file."
                );
            }
        }
    }

    if !no_binary.is_none() && settings.build_options.no_binary() != no_binary {
        warn_user_once!(
            "Ignoring `--no-binary` setting from requirements file. Instead, use the `--no-binary` command-line argument, or set `no-binary` in a `uv.toml` or `pyproject.toml` file."
        );
    }

    if !no_build.is_none() && settings.build_options.no_build() != no_build {
        warn_user_once!(
            "Ignoring `--no-binary` setting from requirements file. Instead, use the `--no-build` command-line argument, or set `no-build` in a `uv.toml` or `pyproject.toml` file."
        );
    }
}
