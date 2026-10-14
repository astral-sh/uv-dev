use std::sync::Arc;

use itertools::Itertools;

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_command_support::Printer;
use uv_configuration::{Concurrency, Constraints, GitLfsSetting};
use uv_dispatch::{BuildDispatch, SharedState};
use uv_distribution::{DistributionDatabase, LoweredExtraBuildDependencies};
use uv_distribution_types::{
    IndexCapabilities, Requirement, RequirementScope, RequirementSource, UnresolvedRequirement,
    UnresolvedRequirementSpecification,
};
use uv_normalize::{ExtraName, PackageName};
use uv_pep440::{Version, VersionSpecifier, VersionSpecifiers};
use uv_pep508::MarkerTree;
use uv_preview::Preview;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_requirements::NamedRequirementsResolver;
use uv_resolve_operations::latest::LatestClient;
use uv_resolve_operations::reporters::ResolverReporter;
use uv_resolver::FlatIndex;
use uv_settings::ResolverSettings;
use uv_torch::TorchStrategy;
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

use crate::error::ToolError;
use crate::{Target, ToolRequest};

/// Create a global registry requirement for a tool, optionally pinned to an exact version.
pub(super) fn registry_requirement(
    name: PackageName,
    extras: Box<[ExtraName]>,
    version: Option<Version>,
) -> Requirement {
    Requirement {
        name,
        extras,
        groups: Box::new([]),
        marker: MarkerTree::default(),
        source: RequirementSource::Registry {
            specifier: version.map_or_else(VersionSpecifiers::empty, |version| {
                VersionSpecifiers::from(VersionSpecifier::equals_version(version))
            }),
            index: None,
            conflict: None,
        },
        scope: RequirementScope::Global,
        origin: None,
    }
}

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
    let torch_backend = torch_backend.and_then(|mode| {
        TorchStrategy::from_mode(
            mode,
            interpreter.platform().os(),
            cuda_driver_version.clone(),
            *amd_gpu_architecture,
        )
        .ok()
    });

    // Initialize the registry client.
    let client = RegistryClientBuilder::new(client_builder, cache.clone())
        .index_locations(index_locations.clone())
        .index_strategy(*index_strategy)
        .torch_backend(torch_backend)
        .markers(interpreter.markers())
        .platform(interpreter.platform())
        .build()?;

    // Determine whether to enable build isolation.
    let mut environment = None;
    let build_isolation = BuildIsolation::from_settings(build_isolation, || {
        environment.insert(PythonEnvironment::from_interpreter(interpreter.clone()))
    });

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

/// Resolve `@latest` to an exact version constraint to prevent backtracking.
pub(super) async fn resolve_latest<E>(
    request: &ToolRequest<'_>,
    interpreter: &Interpreter,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    concurrency: &Concurrency,
    cache: &Cache,
) -> Result<Option<Requirement>, E>
where
    E: From<uv_client::ClientBuildError> + From<uv_client::Error>,
{
    let ToolRequest::Package {
        target: Target::Latest(_, name, _),
        ..
    } = request
    else {
        return Ok(None);
    };

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
    let Some(filename) = latest_client
        .find_latest(name, None, &concurrency.downloads_semaphore)
        .await?
    else {
        return Ok(None);
    };

    let version = filename.version().clone();
    Ok(Some(registry_requirement(
        name.clone(),
        Box::new([]),
        Some(version),
    )))
}
