use std::fmt::Write;
use std::path::Path;

use anstream::print;
use anyhow::{Context, Error, Result, bail};
use futures::StreamExt;

use uv_cache::{Cache, Refresh};
use uv_cache_info::Timestamp;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_command_support::{ExitStatus, Printer, UvError};
use uv_configuration::{
    ActiveEnvironment, Concurrency, DependencyGroups, TargetTriple, TreeFormat,
};
use uv_dispatch::{BuildDispatch, UniversalState};
use uv_distribution::{DistributionDatabase, LoweredExtraBuildDependencies, Metadata};
use uv_distribution_types::IndexCapabilities;
use uv_environment_operations::install_target::{InstallTarget, PackageSelection};
use uv_environment_operations::{
    EnvironmentError, ProjectEnvironmentPolicy, ProjectEnvironmentTarget, ProjectInterpreter,
};
use uv_lock::{Lock, Package, PackageMap, TreeDisplay, TreeJsonTarget};
use uv_lock_operations::{DiscoveredProject, FrozenWorkspace, LockMode, LockOperation, LockTarget};
use uv_normalize::{DefaultGroups, PackageName};
use uv_pep508::MarkerEnvironment;
use uv_preview::{Preview, PreviewFeature};
use uv_python_discovery::ConfigDiscovery;
use uv_python_discovery::ProjectPythonRequest;
use uv_python_discovery::ScriptInterpreter;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_python_types::{
    PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest, PythonVersion,
};
use uv_requirements::script_extra_build_requires;
use uv_resolve_operations::latest::LatestClient;
use uv_resolve_operations::loggers::DefaultResolveLogger;
use uv_resolve_operations::reporters::LatestVersionReporter;
use uv_resolve_operations::resolution_markers;
use uv_resolve_operations::resolution_tags;
use uv_resolver::FlatIndex;
use uv_scripts::Pep723Script;
use uv_settings::{FrozenSource, LockCheck, PythonInstallMirrors, ResolverSettings};
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy};
use uv_warnings::warn_user;
use uv_workspace::{DiscoveryOptions, WorkspaceCache};

/// A tree reads an existing workspace lock or resolves a project or script manifest.
#[derive(Clone, Copy)]
enum TreeSource<'a> {
    Manifest(LockTarget<'a>),
    Lockfile(&'a FrozenWorkspace),
}

/// Display the dependency tree for a project, script, or frozen workspace.
#[expect(clippy::fn_params_excessive_bools)]
pub async fn tree(
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
    python_arch: Option<PythonArchitecture>,
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
    let project;
    let source = if let Some(script) = script.as_ref() {
        TreeSource::Manifest(LockTarget::Script(script))
    } else {
        project = DiscoveredProject::discover(
            project_dir,
            &DiscoveryOptions::default(),
            None,
            frozen,
            preview,
            cache,
            workspace_cache,
        )
        .await?;
        match &project {
            DiscoveredProject::Manifest(project) => {
                TreeSource::Manifest(LockTarget::Workspace(project.workspace()))
            }
            DiscoveredProject::Lockfile(workspace) => {
                if outdated {
                    bail!("`--outdated` is not supported without a `pyproject.toml`");
                }
                TreeSource::Lockfile(workspace)
            }
        }
    };

    // Determine the groups to include.
    let groups = match source {
        TreeSource::Manifest(LockTarget::Workspace(workspace)) => {
            groups.with_defaults(workspace.default_groups()?)
        }
        TreeSource::Manifest(LockTarget::Script(_)) => {
            groups.with_defaults(DefaultGroups::default())
        }
        TreeSource::Lockfile(workspace) => workspace
            .resolve_groups(&groups, workspace.lock().root().map(uv_lock::Package::name))?,
    };

    // Find an interpreter only if needed for locking, filtering, or retrieving package metadata.
    let discover_interpreter = async || {
        Ok::<_, Error>(match source {
            TreeSource::Manifest(LockTarget::Script(script)) => ScriptInterpreter::discover(
                script.into(),
                python.as_deref().map(PythonRequest::parse),
                client_builder,
                python_preference,
                python_arch,
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
            TreeSource::Manifest(LockTarget::Workspace(workspace)) => {
                let project_python = ProjectPythonRequest::from_request(
                    python.as_deref().map(PythonRequest::parse),
                    Some(workspace),
                    &groups,
                    project_dir,
                    config_discovery,
                )
                .await?;
                ProjectInterpreter::discover(
                    ProjectEnvironmentTarget::from(workspace),
                    project_python,
                    client_builder,
                    python_preference,
                    python_arch,
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
            TreeSource::Lockfile(workspace) => {
                let root = workspace.root();
                let lock = workspace.lock();
                let discovery_dir = if project_dir.starts_with(root) {
                    project_dir
                } else {
                    root
                };

                let target = InstallTarget::Lockfile {
                    root,
                    project_name: lock.root().map(uv_lock::Package::name),
                    selection: PackageSelection::Workspace,
                    lock,
                };

                let project_python = ProjectPythonRequest::from_requirements(
                    python.as_deref().map(PythonRequest::parse),
                    Some(root),
                    Some(target.python_requirement(&groups)?),
                    discovery_dir,
                    config_discovery,
                )
                .await
                .map_err(EnvironmentError::from)?;
                ProjectInterpreter::discover(
                    ProjectEnvironmentTarget::Lockfile { root, lock },
                    project_python,
                    client_builder,
                    python_preference,
                    python_arch,
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

    // Update the lockfile, if necessary.
    let state = UniversalState::default();
    let resolved_lock;
    let lock = match source {
        TreeSource::Lockfile(workspace) => workspace.lock(),
        TreeSource::Manifest(target) => {
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
            resolved_lock = match Box::pin(
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
                Err(err) => return Err(UvError::from(err).into()),
            };
            &resolved_lock
        }
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
    let latest = if let TreeSource::Manifest(target) = source
        && outdated
    {
        let install_path = target.install_path();
        // Filter to packages that are derived from a registry.
        let packages = lock
            .packages()
            .iter()
            .filter_map(|package| {
                // TODO(charlie): We would need to know the format here.
                let index = match package.index(install_path) {
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
        lock,
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
            fetch_metadata(
                source,
                lock,
                packages,
                interpreter,
                python_version.as_ref(),
                python_platform.as_ref(),
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
            tree.to_json(match source {
                TreeSource::Manifest(LockTarget::Workspace(workspace)) => {
                    TreeJsonTarget::Workspace(workspace.install_path())
                }
                TreeSource::Manifest(LockTarget::Script(script)) =>
                    TreeJsonTarget::Script(&script.path),
                TreeSource::Lockfile(workspace) => TreeJsonTarget::Workspace(workspace.root()),
            })?
        )?,
    }

    Ok(ExitStatus::Success)
}

/// Retrieve metadata only for the packages whose requirements are displayed in the tree.
async fn fetch_metadata(
    source: TreeSource<'_>,
    lock: &Lock,
    packages: Vec<&Package>,
    interpreter: &Interpreter,
    python_version: Option<&PythonVersion>,
    python_platform: Option<&TargetTriple>,
    markers: &MarkerEnvironment,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    state: &UniversalState,
    concurrency: &Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    preview: Preview,
) -> Result<PackageMap<Metadata>> {
    let tags = resolution_tags(python_version, python_platform, interpreter)?;
    let client_builder = client_builder.clone().keyring(settings.keyring_provider);
    if let TreeSource::Manifest(target) = source {
        for index in target.indexes() {
            if let Some(credentials) = index.credentials()? {
                if let Some(root_url) = index.root_url() {
                    client_builder.store_credentials(&root_url, credentials.clone());
                }
                client_builder.store_credentials(index.raw_url(), credentials);
            }
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
    let flat_index = FlatIndex::load(&client, cache, &settings.index_locations).await?;

    let extra_build_requires = match source {
        TreeSource::Manifest(LockTarget::Workspace(workspace)) => {
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
        TreeSource::Manifest(LockTarget::Script(script)) => {
            script_extra_build_requires(
                script.into(),
                &settings.sources,
                &settings.index_locations,
                cache,
                workspace_cache,
                client.credentials_cache(),
            )
            .await?
        }
        TreeSource::Lockfile(_) => LoweredExtraBuildDependencies::from_non_lowered(
            settings.extra_build_dependencies.clone(),
        ),
    }
    .into_inner();

    let install_path = match source {
        TreeSource::Manifest(target) => target.install_path(),
        TreeSource::Lockfile(workspace) => workspace.root(),
    };
    let build_constraints = lock.build_constraints(install_path);
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
                install_path,
                &tags,
                markers,
                &settings.build_options,
                state.index().distributions(),
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
