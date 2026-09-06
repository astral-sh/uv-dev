use std::fmt::Write;
use std::path::Path;

use anstream::print;
use anyhow::{Context, Error, Result};
use futures::StreamExt;
use uv_cache::{Cache, Refresh};
use uv_cache_info::Timestamp;
use uv_cli::TreeFormat;
use uv_client::{BaseClientBuilder, FlatIndexClient, RegistryClientBuilder};
use uv_configuration::{ActiveEnvironment, Concurrency, DependencyGroups, TargetTriple};
use uv_dispatch::BuildDispatch;
use uv_distribution::{DistributionDatabase, LoweredExtraBuildDependencies, Metadata};
use uv_distribution_types::{Index, IndexCapabilities};
use uv_normalize::DefaultGroups;
use uv_normalize::PackageName;
use uv_pep508::MarkerEnvironment;
use uv_platform_tags::Tags;
use uv_preview::{Preview, PreviewFeature};
use uv_python::{
    ConfigDiscovery, Interpreter, PythonDownloads, PythonEnvironment, PythonPreference,
    PythonRequest, PythonVersion,
};
use uv_resolver::{FlatIndex, Lock, Package, PackageMap, TreeDisplay, TreeJsonTarget};
use uv_scripts::Pep723Script;
use uv_settings::PythonInstallMirrors;
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy};
use uv_warnings::warn_user;
use uv_workspace::{DiscoveryOptions, VirtualProject, WorkspaceCache};

use crate::commands::pip::latest::LatestClient;
use crate::commands::pip::loggers::DefaultResolveLogger;
use crate::commands::pip::{resolution_markers, resolution_tags};
use crate::commands::project::lock::{LockMode, LockOperation};
use crate::commands::project::lock_target::LockTarget;
use crate::commands::project::{
    ProjectEnvironmentPolicy, ProjectError, ProjectInterpreter, ScriptInterpreter, UniversalState,
    WorkspacePython, default_dependency_groups, script_extra_build_requires,
};
use crate::commands::reporters::LatestVersionReporter;
use crate::commands::{ExitStatus, diagnostics};
use crate::printer::Printer;
use crate::settings::FrozenSource;
use crate::settings::LockCheck;
use crate::settings::ResolverSettings;

/// Run a command.
#[expect(clippy::fn_params_excessive_bools)]
pub(crate) async fn tree(
    project_dir: &Path,
    show_version_specifiers: bool,
    groups: DependencyGroups,
    lock_check: LockCheck,
    frozen: Option<FrozenSource>,
    universal: bool,
    format: TreeFormat,
    depth: u8,
    prune: Vec<PackageName>,
    package: Vec<PackageName>,
    no_dedupe: bool,
    invert: bool,
    outdated: bool,
    show_sizes: bool,
    python_version: Option<PythonVersion>,
    python_platform: Option<TargetTriple>,
    python: Option<String>,
    install_mirrors: PythonInstallMirrors,
    settings: ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    script: Option<Pep723Script>,
    python_preference: PythonPreference,
    python_downloads: PythonDownloads,
    concurrency: Concurrency,
    config_discovery: ConfigDiscovery,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    if matches!(format, TreeFormat::Json) && !preview.is_enabled(PreviewFeature::JsonOutput) {
        warn_user!(
            "The `--format json` option is experimental and the schema may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::JsonOutput
        );
    }

    // Find the project requirements.
    let virtual_project;
    let target = if let Some(script) = script.as_ref() {
        LockTarget::Script(script)
    } else {
        virtual_project = VirtualProject::discover(
            project_dir,
            &DiscoveryOptions::default(),
            cache,
            workspace_cache,
        )
        .await?;
        LockTarget::Workspace(virtual_project.workspace())
    };

    // Determine the groups to include.
    let default_groups = match target {
        LockTarget::Workspace(workspace) => default_dependency_groups(workspace.pyproject_toml())?,
        LockTarget::Script(_) => DefaultGroups::default(),
    };
    let groups = groups.with_defaults(default_groups);

    // Find an interpreter only if needed for locking, filtering, or retrieving package metadata.
    let discover_interpreter = async || {
        Ok::<_, Error>(match target {
            LockTarget::Script(script) => ScriptInterpreter::discover(
                script.into(),
                python.as_deref().map(PythonRequest::parse),
                client_builder,
                python_preference,
                python_downloads,
                &install_mirrors,
                false,
                config_discovery,
                ActiveEnvironment::Ignore,
                cache,
                printer,
            )
            .await?
            .into_interpreter(),
            LockTarget::Workspace(workspace) => {
                let workspace_python = WorkspacePython::from_request(
                    python.as_deref().map(PythonRequest::parse),
                    Some(workspace),
                    &groups,
                    project_dir,
                    config_discovery,
                )
                .await?;
                ProjectInterpreter::discover(
                    workspace,
                    &groups,
                    workspace_python,
                    client_builder,
                    python_preference,
                    python_downloads,
                    &install_mirrors,
                    ProjectEnvironmentPolicy::Optional,
                    ActiveEnvironment::Ignore,
                    cache,
                    printer,
                )
                .await?
                .into_interpreter()
            }
        })
    };
    let mut interpreter = if frozen.is_some() && universal {
        None
    } else {
        Some(discover_interpreter().await?)
    };

    // Determine the lock mode.
    let mode = if let Some(frozen_source) = frozen {
        LockMode::Frozen(frozen_source.into())
    } else if let LockCheck::Enabled(lock_check) = lock_check {
        LockMode::Locked(interpreter.as_ref().unwrap(), lock_check)
    } else if matches!(target, LockTarget::Script(_)) && !target.lock_path().is_file() {
        // If we're locking a script, avoid creating a lockfile if it doesn't already exist.
        LockMode::DryRun(interpreter.as_ref().unwrap())
    } else {
        LockMode::Write(interpreter.as_ref().unwrap())
    };

    // Initialize any shared state.
    let state = UniversalState::default();

    // Update the lockfile, if necessary.
    let lock = match Box::pin(
        LockOperation::new(
            mode,
            &settings,
            client_builder,
            &state,
            Box::new(DefaultResolveLogger),
            &concurrency,
            cache,
            workspace_cache,
            printer,
            preview,
        )
        .execute(target),
    )
    .await
    {
        Ok(result) => result.into_lock(),
        Err(ProjectError::Operation(err)) => {
            return diagnostics::OperationDiagnostic::default()
                .report(err)
                .map_or(Ok(ExitStatus::Failure), |err| Err(err.into()));
        }
        Err(err) => return Err(err.into()),
    };

    // Determine the markers to use for resolution.
    let markers = (!universal).then(|| {
        resolution_markers(
            python_version.as_ref(),
            python_platform.as_ref(),
            interpreter.as_ref().unwrap(),
        )
    });

    // If necessary, look up the latest version of each package.
    let latest = if outdated {
        // Filter to packages that are derived from a registry.
        let packages = lock
            .packages()
            .iter()
            .filter_map(|package| {
                // TODO(charlie): We would need to know the format here.
                let index = match package.index(target.install_path()) {
                    Ok(Some(index)) => index,
                    Ok(None) => return None,
                    Err(err) => return Some(Err(err)),
                };
                Some(Ok((package, index)))
            })
            .collect::<Result<Vec<_>, _>>()?;

        if packages.is_empty() {
            PackageMap::default()
        } else {
            let ResolverSettings {
                index_locations,
                index_strategy: _,
                keyring_provider,
                resolution: _,
                prerelease: _,
                fork_strategy: _,
                dependency_metadata: _,
                config_setting: _,
                config_settings_package: _,
                build_isolation: _,
                extra_build_dependencies: _,
                extra_build_variables: _,
                exclude_newer: _,
                link_mode: _,
                upgrade: _,
                build_options: _,
                sources: _,
                torch_backend: _,
                cuda_driver_version: _,
                amd_gpu_architecture: _,
            } = &settings;

            let capabilities = IndexCapabilities::default();

            // Initialize the registry client.
            let client = RegistryClientBuilder::new(
                client_builder.clone(),
                cache.clone().with_refresh(Refresh::All(Timestamp::now())),
            )
            .index_locations(index_locations.clone())
            .keyring(*keyring_provider)
            .build()?;
            let download_concurrency = concurrency.downloads_semaphore.clone();

            let exclude_newer = lock.exclude_newer();

            // Initialize the client to fetch the latest version of each package.
            let client = LatestClient {
                client: &client,
                capabilities: &capabilities,
                prerelease: lock.prerelease(),
                exclude_newer,
                index_locations,
                requires_python: Some(lock.requires_python()),
                tags: None,
            };

            let reporter = LatestVersionReporter::from(printer).with_length(packages.len() as u64);

            // Fetch the latest version for each package.
            let download_concurrency = &download_concurrency;
            let mut fetches = futures::stream::iter(packages)
                .map(async |(package, index)| {
                    // This probably already doesn't work for `--find-links`?
                    let Some(filename) = client
                        .find_latest(package.name(), Some(&index), download_concurrency)
                        .await?
                    else {
                        return Ok(None);
                    };
                    Ok::<Option<_>, Error>(Some((package, filename.into_version())))
                })
                .buffer_unordered(concurrency.downloads);

            let mut map = PackageMap::default();
            while let Some(entry) = fetches.next().await.transpose()? {
                let Some((package, version)) = entry else {
                    reporter.on_fetch_progress();
                    continue;
                };
                reporter.on_fetch_version(package.name(), &version);
                if package.version().is_some_and(|package| version > *package) {
                    map.insert(package.clone(), version);
                }
            }
            reporter.on_fetch_complete();
            map
        }
    } else {
        PackageMap::default()
    };

    // Construct the tree before retrieving metadata, so pruned or hidden dependencies do not
    // require downloads or builds just to display their version specifiers.
    let tree = TreeDisplay::new(
        &lock,
        markers.as_ref(),
        &latest,
        depth.into(),
        &prune,
        &package,
        &groups,
        no_dedupe,
        invert,
        show_sizes,
    );

    let metadata = if show_version_specifiers {
        let packages = tree.metadata_packages();
        if packages.is_empty() {
            PackageMap::default()
        } else {
            if interpreter.is_none() {
                interpreter = Some(discover_interpreter().await?);
            }
            let interpreter = interpreter
                .as_ref()
                .context("An interpreter is required to retrieve package metadata")?;
            let tags = resolution_tags(
                python_version.as_ref(),
                python_platform.as_ref(),
                interpreter,
            )?;
            fetch_metadata(
                target,
                &lock,
                packages,
                interpreter,
                &tags,
                markers
                    .as_ref()
                    .map_or_else(|| interpreter.markers(), |markers| markers.markers()),
                &settings,
                client_builder,
                &state,
                &concurrency,
                cache,
                workspace_cache,
                preview,
            )
            .await?
        }
    } else {
        PackageMap::default()
    };
    let tree = if show_version_specifiers {
        tree.with_metadata(&metadata)?
    } else {
        tree
    };

    // Render the tree.
    match format {
        TreeFormat::Text => print!("{tree}"),
        TreeFormat::Json => writeln!(
            printer.stdout_important(),
            "{}",
            tree.to_json(match target {
                LockTarget::Workspace(workspace) => {
                    TreeJsonTarget::Workspace(workspace.install_path())
                }
                LockTarget::Script(script) => TreeJsonTarget::Script(&script.path),
            })?
        )?,
    }

    Ok(ExitStatus::Success)
}

/// Retrieve metadata only for the packages whose requirements are displayed in the tree.
async fn fetch_metadata(
    target: LockTarget<'_>,
    lock: &Lock,
    packages: Vec<&Package>,
    interpreter: &Interpreter,
    tags: &Tags,
    markers: &MarkerEnvironment,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    state: &UniversalState,
    concurrency: &Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    preview: Preview,
) -> Result<PackageMap<Metadata>> {
    let client_builder = client_builder.clone().keyring(settings.keyring_provider);
    for index in target.indexes() {
        if let Some(credentials) = index.credentials()? {
            if let Some(root_url) = index.root_url() {
                client_builder.store_credentials(&root_url, credentials.clone());
            }
            client_builder.store_credentials(index.raw_url(), credentials);
        }
    }

    let client = RegistryClientBuilder::new(client_builder, cache.clone())
        .index_locations(settings.index_locations.clone())
        .index_strategy(settings.index_strategy)
        .markers(interpreter.markers())
        .platform(interpreter.platform())
        .build()?;

    let environment;
    let build_isolation = match &settings.build_isolation {
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

    let build_hasher = HashStrategy::default();
    let flat_index = {
        let flat_index_client =
            FlatIndexClient::new(client.cached_client(), client.connectivity(), cache);
        let entries = flat_index_client
            .fetch_all(settings.index_locations.flat_indexes().map(Index::url))
            .await?;
        FlatIndex::from_entries(
            entries,
            Some(interpreter.tags()?),
            &build_hasher,
            &settings.build_options,
        )
    };

    let extra_build_requires = match target {
        LockTarget::Workspace(workspace) => {
            LoweredExtraBuildDependencies::from_workspace(
                settings.extra_build_dependencies.clone(),
                workspace,
                &settings.index_locations,
                &settings.sources,
                cache,
                workspace_cache,
                client.credentials_cache(),
            )
            .await?
        }
        LockTarget::Script(script) => {
            script_extra_build_requires(
                script.into(),
                settings,
                cache,
                workspace_cache,
                client.credentials_cache(),
            )
            .await?
        }
    }
    .into_inner();

    let build_constraints = lock.build_constraints(target.install_path());
    let dependency_metadata = lock.dependency_metadata();
    let build_dispatch = BuildDispatch::new(
        &client,
        cache,
        &build_constraints,
        interpreter,
        &settings.index_locations,
        &flat_index,
        &dependency_metadata,
        state.fork().into_inner(),
        settings.index_strategy,
        &settings.config_setting,
        &settings.config_settings_package,
        build_isolation,
        &extra_build_requires,
        &settings.extra_build_variables,
        settings.link_mode,
        &settings.build_options,
        &build_hasher,
        settings.exclude_newer.clone(),
        settings.sources.clone(),
        SourceTreeEditablePolicy::Project,
        workspace_cache.clone(),
        concurrency.clone(),
        preview,
    );
    let database = DistributionDatabase::new(
        &client,
        &build_dispatch,
        concurrency.downloads_semaphore.clone(),
    );

    let mut fetches = futures::stream::iter(packages)
        .map(async |package| {
            let metadata = Lock::locked_package_metadata(
                package,
                target.install_path(),
                tags,
                markers,
                &settings.build_options,
                state.index(),
                &database,
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to retrieve version specifiers for `{}`",
                    package.name()
                )
            })?;
            Ok::<_, Error>((package.clone(), metadata))
        })
        .buffer_unordered(concurrency.downloads);
    let mut metadata = PackageMap::default();
    while let Some((package, requirements)) = fetches.next().await.transpose()? {
        metadata.insert(package, requirements);
    }
    Ok(metadata)
}
