use std::collections::BTreeMap;
use std::sync::Arc;

use itertools::Itertools;
use tracing::debug;

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_command_support::Printer;
use uv_configuration::{
    Concurrency, Constraints, DependencyModifiers, Excludes, GitLfsSetting, Override, Overrides,
    TargetTriple,
};
use uv_dispatch::{BuildDispatch, SharedState};
use uv_distribution::{DistributionDatabase, LoweredExtraBuildDependencies};
use uv_distribution_types::{
    IndexCapabilities, NameRequirementSpecification, Requirement, RequirementScope,
    RequirementSource, UnresolvedRequirement, UnresolvedRequirementSpecification,
};
use uv_normalize::PackageName;
use uv_pep440::{VersionSpecifier, VersionSpecifiers};
use uv_pep508::MarkerTree;
use uv_preview::Preview;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_requirements::{NamedRequirementsResolver, RequirementsSpecification};
use uv_resolve_operations::latest::LatestClient;
use uv_resolve_operations::reporters::ResolverReporter;
use uv_resolve_operations::resolution_markers;
use uv_resolver::{Exclusions, FlatIndex, Manifest, Preferences, ResolverEnvironment};
use uv_settings::ResolverSettings;
use uv_torch::TorchStrategy;
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

use crate::error::ToolError;

/// Resolve any [`UnresolvedRequirementSpecification`] into a fully-qualified [`Requirement`].
pub(super) async fn resolve_names(
    requirements: Vec<UnresolvedRequirementSpecification>,
    interpreter: &Interpreter,
    settings: &ResolverSettings,
    build_constraints: &Constraints,
    client_builder: &BaseClientBuilder<'_>,
    state: &SharedState,
    concurrency: &Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
    lfs: GitLfsSetting,
) -> Result<Vec<Requirement>, ToolError> {
    // Partition the requirements into named and unnamed requirements.
    let (mut requirements, unnamed): (Vec<_>, Vec<_>) = requirements
        .into_iter()
        .map(|spec| {
            spec.requirement
                .augment_requirement(None, None, None, lfs.into(), None)
        })
        .partition_map(|requirement| match requirement {
            UnresolvedRequirement::Named(requirement) => itertools::Either::Left(requirement),
            UnresolvedRequirement::Unnamed(requirement) => itertools::Either::Right(requirement),
        });

    // Short-circuit if there are no unnamed requirements.
    if unnamed.is_empty() {
        return Ok(requirements);
    }

    // Extract the project settings.
    let ResolverSettings {
        build_options,
        config_setting,
        config_settings_package,
        dependency_metadata,
        exclude_newer,
        fork_strategy: _,
        index_locations,
        index_strategy,
        keyring_provider,
        link_mode,
        build_isolation,
        build_hash_checking,
        extra_build_dependencies,
        extra_build_variables,
        prerelease: _,
        resolution: _,
        sources,
        torch_backend,
        cuda_driver_version,
        amd_gpu_architecture,
        upgrade: _,
    } = settings;

    let client_builder = client_builder.clone().keyring(*keyring_provider);

    // Determine the PyTorch backend.
    let torch_backend = torch_backend
        .map(|mode| {
            TorchStrategy::from_mode(
                mode,
                interpreter.platform().os(),
                cuda_driver_version.clone(),
                *amd_gpu_architecture,
            )
        })
        .transpose()
        .ok()
        .flatten();

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

    // TODO(charlie): These are all default values. We should consider whether we want to make them
    // optional on the downstream APIs.
    let hasher = HashStrategy::default();
    let build_hasher = HashStrategy::from_constraints(
        build_constraints,
        Some(&interpreter.to_resolver_marker_environment()),
        *build_hash_checking,
    )
    .map_err(uv_requirements::Error::from)?;
    let flat_index = FlatIndex::load(&client, cache, index_locations)
        .await
        .map_err(|error| uv_requirements::Error::FlatIndex(Box::new(error)))?;

    // Lower the extra build dependencies, if any.
    let extra_build_requires =
        LoweredExtraBuildDependencies::from_non_lowered(extra_build_dependencies.clone())
            .into_inner();

    // Create a build dispatch.
    let build_dispatch = BuildDispatch::new(
        &client,
        cache,
        build_constraints,
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
        SourceTreeEditablePolicy::Project,
        workspace_cache.clone(),
        concurrency.clone(),
        preview,
    );

    // Resolve the unnamed requirements.
    requirements.extend(
        NamedRequirementsResolver::new(
            &hasher,
            state.index(),
            DistributionDatabase::new(
                &client,
                &build_dispatch,
                concurrency.downloads_semaphore.clone(),
            ),
        )
        .with_reporter(Arc::new(ResolverReporter::from(printer)))
        .resolve(unnamed.into_iter())
        .await?,
    );

    Ok(requirements)
}

/// Derive the latest-version constraint for one resolution attempt.
///
/// The supplied requirements remain unchanged so a different interpreter can select its own index
/// and latest version without retaining the constraint from an earlier attempt.
pub(super) async fn with_latest_constraint(
    specification: &RequirementsSpecification,
    package: Option<&PackageName>,
    interpreter: &Interpreter,
    python_platform: Option<&TargetTriple>,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    concurrency: &Concurrency,
    cache: &Cache,
) -> Result<RequirementsSpecification, ToolError> {
    let mut specification = specification.clone();
    let Some(package) = package else {
        return Ok(specification);
    };
    let modifiers = DependencyModifiers::new(
        Overrides::from_entries(
            specification
                .override_dependencies
                .iter()
                .cloned()
                .chain(
                    resolved_requirements(&specification.overrides)?
                        .into_iter()
                        .map(Override::Requirement),
                )
                .collect(),
        )
        .map_err(uv_resolve_operations::Error::from)?,
        Excludes::from_entries(specification.excludes.iter().cloned()),
    );
    let manifest = Manifest::new(
        resolved_requirements(&specification.requirements)?,
        Constraints::from_requirements(
            specification
                .constraints
                .iter()
                .map(|constraint| constraint.requirement.clone()),
        ),
        modifiers,
        Preferences::default(),
        None,
        BTreeMap::new(),
        Exclusions::default(),
        Vec::new(),
    );
    let markers = resolution_markers(None, python_platform, interpreter);
    let index = manifest
        .index_for(package, &ResolverEnvironment::specific(markers))
        .map_err(uv_resolve_operations::Error::from)?;
    let client = RegistryClientBuilder::new(
        client_builder.clone().keyring(settings.keyring_provider),
        cache.clone(),
    )
    .index_locations(settings.index_locations.clone())
    .index_strategy(settings.index_strategy)
    .markers(interpreter.markers())
    .platform(interpreter.platform())
    .build()?;
    let capabilities = IndexCapabilities::default();
    let latest_client = LatestClient {
        client: &client,
        capabilities: &capabilities,
        prerelease: &settings.prerelease,
        exclude_newer: &settings.exclude_newer,
        index_locations: &settings.index_locations,
        tags: None,
        requires_python: None,
    };
    if let Some(filename) = latest_client
        .find_latest(
            package,
            index.as_ref().map(Into::into),
            &concurrency.downloads_semaphore,
        )
        .await?
    {
        let version = filename.version().clone();
        debug!("Resolved `{package}@latest` to `{package}=={version}`");
        specification
            .constraints
            .push(NameRequirementSpecification::from(Requirement {
                name: package.clone(),
                extras: Box::new([]),
                groups: Box::new([]),
                marker: MarkerTree::default(),
                source: RequirementSource::Registry {
                    specifier: VersionSpecifiers::from(VersionSpecifier::equals_version(version)),
                    index: None,
                    conflict: None,
                },
                scope: RequirementScope::Global,
                origin: None,
            }));
    }
    Ok(specification)
}

fn resolved_requirements(
    requirements: &[UnresolvedRequirementSpecification],
) -> Result<Vec<Requirement>, ToolError> {
    requirements
        .iter()
        .map(|requirement| match &requirement.requirement {
            UnresolvedRequirement::Named(requirement) => Ok(requirement.clone()),
            UnresolvedRequirement::Unnamed(_) => Err(anyhow::anyhow!(
                "Expected resolved requirements before selecting the latest tool version"
            )
            .into()),
        })
        .collect()
}
