use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use futures::TryStreamExt;
use futures::stream::FuturesUnordered;
use rustc_hash::FxHashSet;

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_configuration::Concurrency;
use uv_dispatch::{BuildDispatch, UniversalState};
use uv_distribution::DistributionDatabase;
use uv_distribution_types::{Dist, Node, Resolution, ResolvedDist, SourceDist};
use uv_environment_operations::install_target::InstallTarget;
use uv_environment_operations::store_credentials_from_target;
use uv_installer::Preparer;
use uv_lock::{Installable, Metadata};
use uv_metadata::read_flat_wheel_modules;
use uv_preview::Preview;
use uv_pypi_types::ModuleName;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_resolve_operations::{resolution_markers, resolution_tags};
use uv_resolver::FlatIndex;
use uv_settings::ResolverSettings;
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

/// Discover modules from locked artifacts without installing them into the project environment.
pub(super) async fn collect_modules(
    target: InstallTarget<'_>,
    metadata: &Metadata,
    interpreter: &Interpreter,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    state: &UniversalState,
    concurrency: &Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    preview: Preview,
) -> Result<BTreeMap<ModuleName, Vec<String>>> {
    let packages = metadata
        .packages_for_environment(target.lock(), interpreter.markers())
        .collect::<Vec<_>>();
    if packages.is_empty() {
        return Ok(BTreeMap::new());
    }

    let marker_env = resolution_markers(None, None, interpreter);
    let tags = resolution_tags(None, None, interpreter)?;
    let packages = packages
        .into_iter()
        .map(|(package_id, package)| {
            let Node::Dist { dist, .. } =
                target.installable_node(package, &tags, &marker_env, &settings.build_options)?
            else {
                bail!("Locked package `{package_id}` has no installable distribution");
            };
            Ok((package_id, dist))
        })
        .collect::<Result<Vec<_>>>()?;
    let hashes = target
        .lock()
        .hash_strategy(target.install_path(), &FxHashSet::default())?;
    let build_constraints = target.lock().build_constraints(target.install_path());
    let build_hashes = HashStrategy::from_constraints(
        &build_constraints,
        Some(&marker_env),
        settings.build_hash_checking,
    )?;
    let build_hashes = hashes.clone().with_constraint_hashes(&build_hashes)?;
    let extra_build_requires = target
        .extra_build_requires(
            &settings.extra_build_dependencies,
            &settings.index_locations,
            &settings.sources,
            cache,
            workspace_cache,
            client_builder,
        )
        .await?
        .match_runtime_distributions(|| packages.iter().map(|(_, dist)| dist))?;

    store_credentials_from_target(target, client_builder)?;
    let client = RegistryClientBuilder::new(
        client_builder.clone().keyring(settings.keyring_provider),
        cache.clone(),
    )
    .index_locations(settings.index_locations.clone())
    .index_strategy(settings.index_strategy)
    .markers(interpreter.markers())
    .platform(interpreter.platform())
    .build()?;
    let flat_index = FlatIndex::load(&client, cache, &settings.index_locations).await?;
    let build_environment = PythonEnvironment::from_interpreter(interpreter.clone());
    let build_isolation = match &settings.build_isolation {
        uv_configuration::BuildIsolation::Isolate => BuildIsolation::Isolated,
        uv_configuration::BuildIsolation::Shared => BuildIsolation::Shared(&build_environment),
        uv_configuration::BuildIsolation::SharedPackage(packages) => {
            BuildIsolation::SharedPackage(&build_environment, packages)
        }
    };
    let state = state.fork();
    let build_dispatch = BuildDispatch::new(
        &client,
        cache,
        &build_constraints,
        interpreter,
        &settings.index_locations,
        &flat_index,
        &settings.dependency_metadata,
        state.clone().into_inner(),
        settings.index_strategy,
        &settings.config_setting,
        &settings.config_settings_package,
        build_isolation,
        &extra_build_requires,
        &settings.extra_build_variables,
        settings.link_mode,
        &settings.build_options,
        &build_hashes,
        settings.exclude_newer.clone(),
        settings.sources.clone(),
        SourceTreeEditablePolicy::Project,
        workspace_cache.clone(),
        concurrency.clone(),
        preview,
    );
    let preparer = Preparer::new(
        cache,
        &tags,
        &hashes,
        &settings.build_options,
        DistributionDatabase::new(
            &client,
            &build_dispatch,
            concurrency.downloads_semaphore.clone(),
        ),
    );
    let resolution = Resolution::default();
    let prepared = packages
        .into_iter()
        .map(|(package_id, dist)| {
            let preparer = &preparer;
            let resolution = &resolution;
            let state = &state;
            async move {
                let ResolvedDist::Installable { dist, .. } = dist else {
                    bail!("Locked package `{package_id}` has no installable distribution");
                };
                let mut dist = Arc::unwrap_or_clone(dist);
                // Editable wheels can contain only an import hook; a regular wheel records the modules.
                if let Dist::Source(SourceDist::Directory(directory)) = &mut dist {
                    directory.editable = Some(false);
                }
                let wheel = preparer
                    .get_wheel(dist, state.in_flight(), resolution)
                    .await
                    .with_context(|| {
                        format!("Failed to prepare `{package_id}` for module discovery")
                    })?;
                let modules = read_flat_wheel_modules(
                    wheel.filename(),
                    wheel.path(),
                    interpreter.extension_suffixes(),
                )
                .with_context(|| format!("Failed to discover modules for `{package_id}`"))?;
                Ok::<_, anyhow::Error>((package_id, modules))
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<_>>()
        .await?;

    let mut owners = BTreeMap::<ModuleName, BTreeSet<String>>::new();
    for (package_id, modules) in prepared {
        for module in modules {
            owners.entry(module).or_default().insert(package_id.clone());
        }
    }
    Ok(owners
        .into_iter()
        .map(|(module, owners)| (module, owners.into_iter().collect()))
        .collect())
}
