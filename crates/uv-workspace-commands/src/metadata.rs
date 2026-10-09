use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use uv_cache::{Cache, Refresh};
use uv_client::BaseClientBuilder;
use uv_command_support::{ExitStatus, Printer, Stdout, UvError};
use uv_configuration::{
    ActiveEnvironment, Concurrency, DependencyGroupsWithDefaults, DryRun, Modifications,
};
use uv_dispatch::UniversalState;
use uv_environment_operations::install_target::{InstallTarget, PackageSelection};
use uv_environment_operations::{
    LinkErrorReporting, ProjectEnvironment, ProjectEnvironmentPolicy, ProjectEnvironmentTarget,
    ProjectInterpreter, ScriptEnvironment,
};
use uv_lock::{Lock, Metadata, Package};
use uv_lock_operations::{
    DiscoveredProject, FrozenWorkspace, LockError, LockMode, LockOperation, LockTarget,
    handle_missing_script_lockfile,
};
use uv_preview::{Preview, PreviewFeature};
use uv_python_discovery::ConfigDiscovery;
use uv_python_discovery::ProjectPythonRequest;
use uv_python_discovery::ScriptInterpreter;
use uv_python_types::{PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest};
use uv_resolve_operations::loggers::DefaultResolveLogger;
use uv_scripts::{Pep723Item, Pep723ItemRef, Pep723Script};
use uv_settings::{
    FrozenSource, LockCheck, MalwareCheckSettings, PythonInstallMirrors, ResolverSettings,
};
use uv_warnings::warn_user;
use uv_workspace::{DiscoveryOptions, WorkspaceCache};

use super::module_owners::collect_module_owners;

/// The input used to obtain metadata and its locked resolution.
enum MetadataSource<'a> {
    Manifest(LockTarget<'a>),
    Lockfile(&'a FrozenWorkspace),
}

/// Display metadata about the workspace.
pub async fn metadata(
    project_dir: &Path,
    lock_check: LockCheck,
    frozen: Option<FrozenSource>,
    refresh: Refresh,
    sync: Option<Modifications>,
    active: ActiveEnvironment,
    python: Option<String>,
    install_mirrors: PythonInstallMirrors,
    malware_settings: MalwareCheckSettings,
    settings: ResolverSettings,
    client_builder: BaseClientBuilder<'_>,
    script: Option<Pep723Item>,
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
    let stdin = match script.as_ref() {
        Some(Pep723Item::Stdin(_)) => true,
        Some(Pep723Item::Script(_) | Pep723Item::Remote(..)) | None => false,
    };
    let (lock_check, frozen) = if stdin {
        handle_missing_script_lockfile(lock_check, frozen)?;
        (LockCheck::Disabled, None)
    } else {
        (lock_check, frozen)
    };

    if !preview.is_enabled(PreviewFeature::WorkspaceMetadata) {
        warn_user!(
            "The `uv workspace metadata` command is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::WorkspaceMetadata
        );
    }

    let project;
    let stdin_script;
    let source = match script.as_ref() {
        Some(Pep723Item::Script(script)) => MetadataSource::Manifest(LockTarget::Script(script)),
        Some(Pep723Item::Stdin(metadata)) => {
            stdin_script = Pep723Script {
                path: Pep723ItemRef::Stdin(metadata).directory()?.join("-"),
                metadata: metadata.clone(),
                prelude: String::new(),
                postlude: String::new(),
            };
            MetadataSource::Manifest(LockTarget::Script(&stdin_script))
        }
        Some(Pep723Item::Remote(..)) => {
            bail!("Remote scripts are not supported by `uv workspace metadata`")
        }
        None => {
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
                    MetadataSource::Manifest(LockTarget::Workspace(project.workspace()))
                }
                DiscoveredProject::Lockfile(workspace) => MetadataSource::Lockfile(workspace),
            }
        }
    };
    let script_item = script.as_ref().map(Pep723ItemRef::from);

    // Don't enable any groups' requires-python for interpreter discovery.
    let groups = DependencyGroupsWithDefaults::none();
    let state = UniversalState::default();

    let resolved_lock;
    let lock: &Lock = match &source {
        MetadataSource::Lockfile(workspace) => workspace.lock(),
        MetadataSource::Manifest(target) => {
            let target = *target;
            let interpreter;
            let mode = if let Some(frozen_source) = frozen {
                LockMode::Frozen(frozen_source.into())
            } else {
                interpreter = match target {
                    LockTarget::Script(script) => ScriptInterpreter::discover(
                        script_item.unwrap_or_else(|| script.into()),
                        python.as_deref().map(PythonRequest::parse),
                        &client_builder,
                        python_preference,
                        python_arch,
                        python_downloads,
                        &install_mirrors,
                        false,
                        config_discovery,
                        active,
                        cache,
                        printer,
                    )
                    .await?
                    .into_interpreter(),
                    LockTarget::Workspace(workspace) => {
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
                            &client_builder,
                            python_preference,
                            python_arch,
                            python_downloads,
                            &install_mirrors,
                            if sync.is_some() {
                                ProjectEnvironmentPolicy::Compatible
                            } else {
                                ProjectEnvironmentPolicy::Optional
                            },
                            active,
                            cache,
                            printer,
                        )
                        .await?
                        .into_interpreter()
                    }
                };

                if let LockCheck::Enabled(lock_check) = lock_check {
                    LockMode::Locked(&interpreter, lock_check)
                } else if stdin
                    || sync.is_none()
                    || (matches!(target, LockTarget::Script(_)) && !target.lock_path().is_file())
                {
                    LockMode::DryRun(&interpreter)
                } else {
                    LockMode::Write(&interpreter)
                }
            };

            resolved_lock = match Box::pin(
                LockOperation::new(
                    mode,
                    &settings,
                    &client_builder,
                    &state,
                    Box::new(DefaultResolveLogger),
                    &concurrency,
                    cache,
                    workspace_cache,
                    printer,
                    preview,
                )
                .with_refresh(&refresh)
                .with_existing_lockfile(!stdin)
                .execute(target),
            )
            .await
            {
                Ok(lock) => lock.into_lock(),
                Err(err @ LockError::LockMismatch(..)) => return Err(UvError::user(err).into()),
                Err(err) => return Err(UvError::from(err).into()),
            };
            &resolved_lock
        }
    };

    let install_target = match &source {
        MetadataSource::Manifest(LockTarget::Workspace(workspace)) => InstallTarget::Workspace {
            workspace,
            project_name: None,
            lock,
        },
        MetadataSource::Manifest(LockTarget::Script(script)) => {
            InstallTarget::Script { script, lock }
        }
        MetadataSource::Lockfile(workspace) => InstallTarget::Lockfile {
            root: workspace.root(),
            project_name: lock.root().map(Package::name),
            selection: PackageSelection::Workspace,
            lock,
        },
    };
    let mut export = metadata_for_target(install_target);
    let environment = if sync.is_some() {
        Some(match &source {
            MetadataSource::Manifest(LockTarget::Workspace(workspace)) => {
                ProjectEnvironment::get_or_init(
                    ProjectEnvironmentTarget::from(*workspace),
                    None,
                    &groups,
                    python.as_deref().map(PythonRequest::parse),
                    &install_mirrors,
                    &client_builder,
                    python_preference,
                    python_arch,
                    python_downloads,
                    false,
                    config_discovery,
                    active,
                    cache,
                    DryRun::Disabled,
                    LinkErrorReporting::User,
                    printer,
                )
                .await?
                .into_environment()?
            }
            MetadataSource::Manifest(LockTarget::Script(script)) => ScriptEnvironment::get_or_init(
                script_item.unwrap_or_else(|| (*script).into()),
                python.as_deref().map(PythonRequest::parse),
                &client_builder,
                python_preference,
                python_arch,
                python_downloads,
                &install_mirrors,
                false,
                config_discovery,
                active,
                cache,
                DryRun::Disabled,
                printer,
            )
            .await?
            .into_environment()?,
            MetadataSource::Lockfile(workspace) => ProjectEnvironment::get_or_init(
                ProjectEnvironmentTarget::Lockfile {
                    root: workspace.root(),
                    lock,
                },
                Some(install_target),
                &groups,
                python.as_deref().map(PythonRequest::parse),
                &install_mirrors,
                &client_builder,
                python_preference,
                python_arch,
                python_downloads,
                false,
                config_discovery,
                active,
                cache,
                DryRun::Disabled,
                LinkErrorReporting::User,
                printer,
            )
            .await?
            .into_environment()?,
        })
    } else {
        match &source {
            MetadataSource::Manifest(LockTarget::Workspace(workspace)) => {
                ProjectInterpreter::discover_existing(workspace.install_path(), active, cache)?
            }
            MetadataSource::Manifest(LockTarget::Script(script)) => {
                ScriptInterpreter::discover_existing(
                    script_item.unwrap_or_else(|| (*script).into()),
                    active,
                    cache,
                )
            }
            MetadataSource::Lockfile(workspace) => {
                ProjectInterpreter::discover_existing(workspace.root(), active, cache)?
            }
        }
    };

    if let Some(environment) = environment {
        let _lock = environment
            .lock()
            .await
            .inspect_err(|err| {
                tracing::warn!("Failed to acquire environment lock: {err}");
            })
            .ok();
        let module_owners = collect_module_owners(
            install_target,
            &environment,
            &settings,
            &client_builder,
            &state,
            &concurrency,
            cache,
            workspace_cache,
            preview,
            &malware_settings,
            sync,
        )
        .await
        .context("Failed to collect module owners")?;
        export = export
            .with_environment(&environment)
            .with_module_owners(module_owners);
    }

    print_metadata(&export, printer)
}

fn metadata_for_target(target: InstallTarget<'_>) -> Metadata {
    match target {
        InstallTarget::Project {
            workspace, lock, ..
        }
        | InstallTarget::Projects {
            workspace, lock, ..
        }
        | InstallTarget::Workspace {
            workspace, lock, ..
        }
        | InstallTarget::NonProjectWorkspace { workspace, lock } => {
            Metadata::from_lockfile(workspace.install_path(), lock)
        }
        InstallTarget::Script { script, lock } => Metadata::from_script(&script.path, lock),
        InstallTarget::Lockfile { root, lock, .. } => Metadata::from_lockfile(root, lock),
    }
}

fn print_metadata(export: &Metadata, printer: Printer) -> Result<ExitStatus> {
    if printer.stdout_important() == Stdout::Enabled {
        let mut stdout = BufWriter::new(anstream::stdout().lock());
        export.write_json(&mut stdout)?;
        writeln!(stdout)?;
        stdout.flush()?;
    }

    Ok(ExitStatus::Success)
}
