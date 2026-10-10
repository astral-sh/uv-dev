use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Result;
use tracing::debug;

use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_command_support::{ExitStatus, Printer, UvError};
use uv_configuration::{
    ActiveEnvironment, ColorChoice, Concurrency, DependencyGroups, DependencyGroupsWithDefaults,
    DryRun, ExtrasSpecification, InstallOptions, Modifications,
};
use uv_dispatch::UniversalState;
use uv_environment_operations::environment::CachedEnvironment;
use uv_environment_operations::install_target::{InstallTarget, PackageSelection};
use uv_environment_operations::malware::MalwareCheckContext;
use uv_environment_operations::{
    LinkErrorReporting, ProjectEnvironment, ProjectEnvironmentPolicy, ProjectEnvironmentTarget,
    ProjectInterpreter, ScriptEnvironment, discover_workspace_groups,
    store_credentials_from_target, sync_from_lock,
};
use uv_fs::normalize_path;
use uv_install_operations::loggers::SummaryInstallLogger;
use uv_lock_operations::{LockMode, LockOperation, LockResult, LockTarget};
use uv_normalize::{DEV_DEPENDENCIES, DefaultExtras, PackageName};
use uv_preview::{Preview, PreviewFeature};
use uv_python_discovery::ConfigDiscovery;
use uv_python_discovery::ProjectPythonRequest;
use uv_python_discovery::PythonDownloadReporter;
use uv_python_discovery::PythonInstallation;
use uv_python_discovery::ScriptInterpreter;
use uv_python_interpreter::PythonEnvironment;
use uv_python_types::{
    EnvironmentPreference, PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest,
};
use uv_resolve_operations::loggers::SummaryResolveLogger;
use uv_scripts::Pep723Script;
use uv_settings::{
    FrozenSource, LockCheck, MalwareCheckSettings, PythonInstallMirrors, ResolverInstallerSettings,
};
use uv_virtualenv::UpgradePolicy;
use uv_warnings::warn_user;
use uv_workspace::{DiscoveryOptions, VirtualProject, WorkspaceCache, WorkspaceErrorKind};

use crate::lock::{
    CommandWorkspaceSelection, FinalizedCommandWorkspaceSelection,
    command_workspace_group_from_lock, project_workspace_selection, workspace_selection_members,
};
use crate::toolchain;

mod ty;

/// Run project checks.
#[expect(clippy::fn_params_excessive_bools)]
pub async fn check(
    project_dir: &Path,
    ty_path: Option<PathBuf>,
    fix: bool,
    lock_check: LockCheck,
    frozen: Option<FrozenSource>,
    no_sync: bool,
    no_install_project: bool,
    isolated: bool,
    all_packages: bool,
    package: Vec<PackageName>,
    extras: ExtrasSpecification,
    groups: DependencyGroups,
    python: Option<String>,
    install_mirrors: PythonInstallMirrors,
    settings: ResolverInstallerSettings,
    ty_version: Option<String>,
    show_version: bool,
    show_command: bool,
    script: Option<Pep723Script>,
    client_builder: BaseClientBuilder<'_>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    installer_metadata: bool,
    concurrency: Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    color: ColorChoice,
    printer: Printer,
    preview: Preview,
    no_project: bool,
    config_discovery: ConfigDiscovery,
    malware_settings: MalwareCheckSettings,
) -> Result<ExitStatus> {
    if !preview.is_enabled(PreviewFeature::CheckCommand) {
        warn_user!(
            "`uv check` is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::CheckCommand
        );
    }

    // Discover the project.
    let project = if no_project || script.is_some() {
        None
    } else {
        let discovery = if let [name] = package.as_slice() {
            VirtualProject::discover_with_package(
                project_dir,
                &DiscoveryOptions::default(),
                cache,
                workspace_cache,
                name.clone(),
            )
            .await
        } else {
            VirtualProject::discover(
                project_dir,
                &DiscoveryOptions::default(),
                cache,
                workspace_cache,
            )
            .await
        };

        match discovery {
            Ok(project) => {
                for name in &package {
                    if !project.workspace().packages().contains_key(name) {
                        anyhow::bail!("Package `{name}` not found in workspace");
                    }
                }
                Some(project)
            }
            Err(err) => {
                if let WorkspaceErrorKind::NoSuchMember(name, _) = err.as_ref() {
                    anyhow::bail!("Package `{name}` not found in workspace");
                }
                if !all_packages
                    && package.is_empty()
                    && matches!(
                        err.as_ref(),
                        WorkspaceErrorKind::MissingPyprojectToml
                            | WorkspaceErrorKind::MissingProject(_)
                            | WorkspaceErrorKind::NonWorkspace(_),
                    )
                {
                    None
                } else {
                    return Err(err.into());
                }
            }
        }
    };

    if no_project {
        for flag in extras.history().as_flags_pretty() {
            warn_user!("`{flag}` has no effect when used alongside `--no-project`");
        }
        for flag in groups.history().as_flags_pretty() {
            warn_user!("`{flag}` has no effect when used alongside `--no-project`");
        }
        if let LockCheck::Enabled(lock_check) = lock_check {
            warn_user!("`{lock_check}` has no effect when used alongside `--no-project`");
        }
        if frozen.is_some() {
            warn_user!("`--frozen` has no effect when used alongside `--no-project`");
        }
        if no_sync {
            warn_user!("`--no-sync` has no effect when used alongside `--no-project`");
        }
    } else if project.is_none() && script.is_none() {
        for flag in extras.history().as_flags_pretty() {
            warn_user!("`{flag}` has no effect when used outside of a project");
        }
        for flag in groups.history().as_flags_pretty() {
            warn_user!("`{flag}` has no effect when used outside of a project");
        }
        if let LockCheck::Enabled(lock_check) = lock_check {
            warn_user!("`{lock_check}` has no effect when used outside of a project");
        }
        if frozen.is_some() {
            warn_user!("`--frozen` has no effect when used outside of a project");
        }
        if no_sync {
            warn_user!("`--no-sync` has no effect when used outside of a project");
        }
    }

    let is_virtual_workspace = project
        .as_ref()
        .is_some_and(|project| project.project_name().is_none());
    let defacto_all_packages = all_packages || (is_virtual_workspace && package.is_empty());
    // Running within a project selects that project, even if workspace configuration excludes it.
    let explicit_targets = all_packages
        || !package.is_empty()
        || script.is_some()
        || project
            .as_ref()
            .is_some_and(|project| project.project_name().is_some());

    let target_dir = script
        .as_ref()
        .and_then(|script| script.path.parent())
        .map(Path::to_path_buf)
        .or_else(|| {
            project.as_ref().map(|project| {
                // If multiple packages are selected, or the package is outside the workspace dir,
                // require analysis to run in the workspace dir.
                if defacto_all_packages
                    || package.len() > 1
                    || !normalize_path(project.root())
                        .starts_with(project.workspace().install_path())
                {
                    project.workspace().install_path().to_owned()
                } else {
                    project.root().to_owned()
                }
            })
        })
        .unwrap_or_else(|| project_dir.to_owned());

    let groups = if let Some(project) = &project {
        groups.with_defaults(project.default_groups()?)
    } else {
        DependencyGroupsWithDefaults::none()
    };

    let mut frozen_workspace_lock =
        if let (Some(source), Some(project)) = (frozen, project.as_ref()) {
            Some(
                LockTarget::Workspace(project.workspace())
                    .read_frozen(source.into())
                    .await
                    .map_err(UvError::from)?,
            )
        } else {
            None
        };
    let frozen_grouped_tool = ty_path.is_none()
        && ty_version.is_none()
        && frozen_workspace_lock
            .as_ref()
            .is_some_and(|lock| !lock.workspace_groups().is_empty());
    let state = UniversalState::default();
    let project_install_options = InstallOptions::new(
        no_install_project,
        false,
        false,
        false,
        false,
        false,
        Vec::new(),
        Vec::new(),
    );
    let mut resolved_before_environment = None;
    let mut selected_workspace_members = None;
    let discovery_workspace = if let Some(project) = &project {
        let workspace = project.workspace();
        let selection = if let Some(lock) = frozen_workspace_lock.as_ref() {
            let members = workspace_selection_members(project, &package, all_packages);
            let mut selection = command_workspace_group_from_lock(
                lock,
                None,
                Some(&members),
                all_packages || (package.is_empty() && project.is_non_project()),
            )?;
            if let Some(selected) = selection
                .as_mut()
                .and_then(FinalizedCommandWorkspaceSelection::take_selected_lock)
            {
                frozen_workspace_lock = Some(selected);
            }
            selection.map(CommandWorkspaceSelection::Finalized)
        } else {
            let selection =
                PackageSelection::from_args(all_packages, &package, project.project_name());
            let exclusions = selection.first_party_exclusions(
                workspace,
                project.project_name(),
                &project_install_options,
            );
            project_workspace_selection(
                project,
                &package,
                all_packages,
                &discover_workspace_groups(
                    workspace,
                    project_dir,
                    python.as_deref(),
                    lock_check,
                    &settings.resolver,
                    &client_builder,
                    &state,
                    &exclusions,
                    python_preference,
                    python_arch,
                    python_downloads,
                    &install_mirrors,
                    &concurrency,
                    config_discovery,
                    cache,
                    workspace_cache,
                    printer,
                    preview,
                )
                .await?,
            )?
        };
        let finalized = match selection {
            Some(CommandWorkspaceSelection::Pending(selection)) => {
                let workspace = selection.provisional_workspace(project.workspace());
                let project_python = ProjectPythonRequest::from_request(
                    python.as_deref().map(PythonRequest::parse),
                    Some(&workspace),
                    &groups,
                    &settings.resolver.sources,
                    project_dir,
                    config_discovery,
                )
                .await?;
                let interpreter = ProjectInterpreter::discover(
                    ProjectEnvironmentTarget::from(&workspace),
                    project_python.environment_probe(),
                    &client_builder,
                    python_preference,
                    python_arch,
                    python_downloads,
                    &install_mirrors,
                    ProjectEnvironmentPolicy::Optional,
                    ActiveEnvironment::Ignore,
                    cache,
                    if printer == Printer::Verbose {
                        printer
                    } else {
                        Printer::Silent
                    },
                )
                .await?
                .into_interpreter();
                let mode = if let LockCheck::Enabled(source) = lock_check {
                    LockMode::Locked(&interpreter, source)
                } else if isolated {
                    LockMode::DryRun(&interpreter)
                } else {
                    LockMode::Write(&interpreter)
                };
                let exclusions =
                    PackageSelection::from_args(all_packages, &package, project.project_name())
                        .first_party_exclusions(
                            project.workspace(),
                            project.project_name(),
                            &project_install_options,
                        );
                let result = Box::pin(
                    LockOperation::new(
                        mode,
                        &settings.resolver,
                        &client_builder,
                        &state,
                        Box::new(SummaryResolveLogger),
                        &concurrency,
                        cache,
                        workspace_cache,
                        printer,
                        preview,
                    )
                    .with_first_party_exclusions(exclusions)
                    .execute(project.workspace().into()),
                )
                .await
                .map_err(UvError::from)?;
                let finalized = selection.finalize(result.lock())?;
                resolved_before_environment = Some(result);
                Some(finalized)
            }
            Some(CommandWorkspaceSelection::Finalized(selection)) => Some(selection),
            None => None,
        };
        Some(finalized.map_or_else(
            || workspace.clone(),
            |selection| {
                selected_workspace_members = Some(selection.target_members().clone());
                selection.environment_workspace(workspace)
            },
        ))
    } else {
        None
    };

    let check_targets = if let Some(script) = script.as_ref() {
        vec![script.path.clone()]
    } else if let Some(project) = project.as_ref() {
        if defacto_all_packages {
            // In --all-packages mode, and anything equivalent like virtual workspaces,
            // we can't just pass ty the root of the project because:
            //
            // * It excludes members of the workspace that aren't nested under the root,
            //   as constructs like `members = ["../foo"]` are legal.
            // * For virtual workspaces, this can include files that are not strictly
            //   part of any member, such as `scripts/myscript.py`
            //
            // The first issue is definitely important to handle, but the second issue
            // is debatable. It is in fact Useful for ty to find and check all your
            // random scripts, and indeed this is the default ty behaviour. Attempting
            // to manually suppress this behaviour is an attempt to maintain "uv-like"
            // behaviour, but if anyone disagrees we can change this by just always
            // including the workspace root, even for virtual workspaces.
            project
                .workspace()
                .packages()
                .iter()
                .filter(|(name, _)| {
                    selected_workspace_members
                        .as_ref()
                        .is_none_or(|members| members.contains(*name))
                })
                .map(|(_, member)| member.root().clone())
                .collect()
        } else if !package.is_empty() {
            // If the user has specified a list of packages, tell ty to only check those packages.
            package
                .iter()
                .map(|name| {
                    project
                        .workspace()
                        .packages()
                        .get(name)
                        .map(|member| member.root().clone())
                        .ok_or_else(|| anyhow::anyhow!("Package `{name}` not found in workspace"))
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            // Otherwise we're checking just this one package (nearest ancestor).
            vec![project.root().to_owned()]
        }
    } else {
        Vec::new()
    };

    // Any selected package can contain other workspace members, even in a virtual workspace.
    // Explicitly exclude any workspace members that aren't selected *and are nested under
    // a selected one*, so ty doesn't emit diagnostics for them (if they're dependencies
    // of selected packages that's fine, ty will still find them for those purposes).
    //
    // The most common case this is handling is a non-virtual workspace, where the root
    // package will almost always have the other packages nested under it, and we need a
    // way to select just the workspace root.
    let excluded_targets = if let Some(project) = project.as_ref()
        && (!defacto_all_packages || selected_workspace_members.is_some())
    {
        project
            .workspace()
            .packages()
            .iter()
            .filter(|(name, member)| {
                let selected = if let Some(members) = selected_workspace_members.as_ref() {
                    members.contains(*name)
                } else if package.is_empty() {
                    project.project_name() == Some(*name)
                } else {
                    package.contains(name)
                };
                !selected
                    && check_targets
                        .iter()
                        .any(|target| member.root().starts_with(target))
            })
            .map(|(_, member)| member.root().clone())
            .collect()
    } else {
        Vec::new()
    };

    // Create an isolated environment, if requested.
    let temp_dir;
    let isolated_venv = if isolated {
        debug!("Creating isolated virtual environment");

        let interpreter = if let Some(script) = script.as_ref() {
            ScriptInterpreter::discover(
                script.into(),
                python.as_deref().map(PythonRequest::parse),
                &client_builder,
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
            .into_interpreter()
        } else {
            let workspace = discovery_workspace.as_ref();
            let project_python = ProjectPythonRequest::from_request(
                python.as_deref().map(PythonRequest::parse),
                workspace,
                &groups,
                &settings.resolver.sources,
                project_dir,
                config_discovery,
            )
            .await?;

            let reporter = PythonDownloadReporter::single(printer);
            project_python
                .find_or_download(
                    EnvironmentPreference::Any,
                    python_preference,
                    python_arch,
                    None,
                    python_downloads,
                    &client_builder,
                    cache,
                    &reporter,
                    &install_mirrors,
                )
                .await?
                .into_interpreter()
        };

        temp_dir = cache.venv_dir()?;
        Some(uv_virtualenv::create_venv(
            temp_dir.path(),
            interpreter,
            uv_virtualenv::Prompt::None,
            false,
            uv_virtualenv::OnExisting::Remove(uv_virtualenv::RemovalReason::TemporaryEnvironment),
            false,
            uv_virtualenv::Seed::Disabled,
            UpgradePolicy::Fixed,
        )?)
    } else {
        None
    };

    // Select an environment and, if we found a project, sync it before running checks.
    let mut locked_ty_path = None;
    let venv = if let Some(script) = &script {
        let extras = extras.with_defaults(DefaultExtras::default());
        let venv = if let Some(venv) = isolated_venv {
            venv
        } else {
            ScriptEnvironment::get_or_init(
                script.into(),
                python.as_deref().map(PythonRequest::parse),
                &client_builder,
                python_preference,
                python_arch,
                python_downloads,
                &install_mirrors,
                no_sync,
                config_discovery,
                ActiveEnvironment::Ignore,
                cache,
                DryRun::Disabled,
                printer,
            )
            .await?
            .into_environment()?
        };

        let lock_target = LockTarget::Script(script);
        // Scripts always run in an isolated environment, so `--no-sync` has no effect.
        let _environment_lock = venv
            .lock()
            .await
            .inspect_err(|err| {
                tracing::warn!("Failed to acquire environment lock: {err}");
            })
            .ok();
        let sync_state = state.fork();
        let mode = if let Some(frozen_source) = frozen {
            LockMode::Frozen(frozen_source.into())
        } else if let LockCheck::Enabled(lock_check) = lock_check {
            LockMode::Locked(venv.interpreter(), lock_check)
        } else if isolated || !lock_target.lock_path().is_file() {
            LockMode::DryRun(venv.interpreter())
        } else {
            LockMode::Write(venv.interpreter())
        };
        let result = match Box::pin(
            LockOperation::new(
                mode,
                &settings.resolver,
                &client_builder,
                &state,
                Box::new(SummaryResolveLogger),
                &concurrency,
                cache,
                workspace_cache,
                printer,
                preview,
            )
            .execute(lock_target),
        )
        .await
        {
            Ok(result) => result,
            Err(err) => return Err(UvError::from(err).into()),
        };

        let marker_environment = venv.interpreter().to_resolver_marker_environment();
        if ty_path.is_none()
            && ty_version.is_none()
            && result
                .lock()
                .dependency_selection(
                    None,
                    &PackageName::from_str("ty")?,
                    marker_environment.markers(),
                )
                .map_err(anyhow::Error::msg)?
                .root()
                .is_some()
        {
            locked_ty_path = Some(
                venv.scripts()
                    .join(format!("ty{}", std::env::consts::EXE_SUFFIX)),
            );
        }

        let target = InstallTarget::Script {
            script,
            lock: result.lock(),
        };
        match sync_from_lock(
            &target.select_workspace_context()?,
            &venv,
            &extras,
            &groups,
            None,
            InstallOptions::default(),
            Modifications::Sufficient,
            None,
            (&settings).into(),
            &client_builder,
            &sync_state,
            Box::new(SummaryInstallLogger),
            installer_metadata,
            &concurrency,
            cache,
            workspace_cache,
            DryRun::Disabled,
            printer,
            preview,
            MalwareCheckContext::from(&malware_settings),
        )
        .await
        {
            Ok(_) => {}
            Err(err) => return Err(UvError::from(err).into()),
        }

        if no_sync {
            warn_user!(
                "`--no-sync` is a no-op for Python scripts with inline metadata, which always run in isolation"
            );
        }

        Some(venv)
    } else if let Some(project) = &project {
        let workspace = discovery_workspace
            .as_ref()
            .unwrap_or_else(|| project.workspace());
        let extras = extras.with_defaults(DefaultExtras::default());
        let mut malware_context = MalwareCheckContext::from(&malware_settings);
        let install_options = project_install_options;

        let venv = if let Some(venv) = isolated_venv {
            venv
        } else {
            ProjectEnvironment::get_or_init(
                ProjectEnvironmentTarget::from(workspace),
                None,
                &groups,
                &settings.resolver.sources,
                python.as_deref().map(PythonRequest::parse),
                &install_mirrors,
                &client_builder,
                python_preference,
                python_arch,
                None,
                python_downloads,
                no_sync,
                config_discovery,
                ActiveEnvironment::Warn,
                cache,
                DryRun::Disabled,
                LinkErrorReporting::User,
                printer,
            )
            .await?
            .into_environment()?
        };

        // `--no-sync` permits an incompatible project environment. Locking and grouped frozen tool
        // lookup still need an interpreter in the selected context to evaluate dependency markers.
        let lock_interpreter = if no_sync && !isolated && (frozen.is_none() || frozen_grouped_tool)
        {
            let project_python = ProjectPythonRequest::from_request(
                python.as_deref().map(PythonRequest::parse),
                Some(workspace),
                &groups,
                &settings.resolver.sources,
                project_dir,
                config_discovery,
            )
            .await?;
            Some(
                ProjectInterpreter::discover_for_environment(
                    ProjectEnvironmentTarget::from(workspace),
                    project_python,
                    &client_builder,
                    python_preference,
                    python_arch,
                    None,
                    python_downloads,
                    &install_mirrors,
                    ProjectEnvironmentPolicy::Optional,
                    ActiveEnvironment::Warn,
                    cache,
                    printer,
                )
                .await?
                .into_interpreter(),
            )
        } else {
            None
        };
        let lock_interpreter = lock_interpreter
            .as_ref()
            .unwrap_or_else(|| venv.interpreter());

        // Keep the environment locked through synchronization and metadata collection.
        let _environment_lock;
        if !no_sync {
            _environment_lock = venv
                .lock()
                .await
                .inspect_err(|err| {
                    tracing::warn!("Failed to acquire environment lock: {err}");
                })
                .ok();
        }

        let mode = if let Some(frozen_source) = frozen {
            LockMode::Frozen(frozen_source.into())
        } else if let LockCheck::Enabled(lock_check) = lock_check {
            LockMode::Locked(lock_interpreter, lock_check)
        } else if isolated {
            LockMode::DryRun(lock_interpreter)
        } else {
            LockMode::Write(lock_interpreter)
        };

        let selection = PackageSelection::from_args(all_packages, &package, project.project_name());
        let result = if let Some(lock) = frozen_workspace_lock.take() {
            LockResult::Unchanged(lock)
        } else if let Some(result) = resolved_before_environment.take() {
            result
        } else {
            match Box::pin(
                LockOperation::new(
                    mode,
                    &settings.resolver,
                    &client_builder,
                    &state,
                    Box::new(SummaryResolveLogger),
                    &concurrency,
                    cache,
                    workspace_cache,
                    printer,
                    preview,
                )
                .with_first_party_exclusions(selection.first_party_exclusions(
                    project.workspace(),
                    project.project_name(),
                    &install_options,
                ))
                .execute(project.workspace().into()),
            )
            .await
            {
                Ok(result) => result,
                Err(err) => return Err(UvError::from(err).into()),
            }
        };

        let target = InstallTarget::from_project(project, result.lock(), selection)
            .select_workspace_context()?;
        let lock = target.lock();
        let install_target = target.as_target();

        install_target.validate_extras(&extras)?;
        install_target.validate_groups(&groups)?;

        if ty_path.is_none()
            && ty_version.is_none()
            && let Some(tool) = toolchain::find_locked_tool(
                project,
                &target,
                lock_interpreter,
                &PackageName::from_str("ty")?,
                &DEV_DEPENDENCIES,
                &groups,
            )?
        {
            locked_ty_path = Some(if !tool.requires_separate_environment() && !no_sync {
                // Synchronization will install the locked tool into the selected project or
                // isolated environment.
                venv.scripts()
                    .join(format!("ty{}", std::env::consts::EXE_SUFFIX))
            } else {
                // Do not modify the selected environment when synchronization is disabled or the
                // locked tool is excluded from it. Install only the locked `ty` subgraph.
                let base_interpreter =
                    CachedEnvironment::base_interpreter(lock_interpreter, cache)?;
                let resolution = toolchain::resolution_from_lock(
                    project,
                    &target,
                    &tool,
                    &base_interpreter,
                    &settings.resolver.build_options,
                )?;
                store_credentials_from_target(install_target, &client_builder)?;
                let ty_state = state.fork();
                let environment = match CachedEnvironment::from_locked_resolution(
                    &resolution,
                    lock.build_constraints(project.workspace().install_path()),
                    &base_interpreter,
                    &settings,
                    &malware_settings,
                    &client_builder,
                    &ty_state,
                    Box::new(SummaryInstallLogger),
                    installer_metadata,
                    &concurrency,
                    cache,
                    printer,
                    preview,
                )
                .await
                {
                    Ok(environment) => environment,
                    Err(err) => return Err(UvError::from(err).into()),
                };
                malware_context.record_resolution(&resolution);
                PythonEnvironment::from(environment)
                    .scripts()
                    .join(format!("ty{}", std::env::consts::EXE_SUFFIX))
            });
        }

        if no_sync {
            debug!("Skipping environment synchronization due to `--no-sync`");
        } else {
            let sync_state = state.fork();
            match sync_from_lock(
                &target,
                &venv,
                &extras,
                &groups,
                None,
                install_options,
                Modifications::Sufficient,
                None,
                (&settings).into(),
                &client_builder,
                &sync_state,
                Box::new(SummaryInstallLogger),
                installer_metadata,
                &concurrency,
                cache,
                workspace_cache,
                DryRun::Disabled,
                printer,
                preview,
                malware_context,
            )
            .await
            {
                Ok(_) => {}
                Err(err) => return Err(UvError::from(err).into()),
            }
        }

        Some(venv)
    } else {
        isolated_venv
    };

    // Forward the user's explicit Python request so ty can apply its own version selection rules.
    let python_version = if let Some(python) = python {
        let request = PythonRequest::parse(&python);
        if let Some(venv) = venv.as_ref()
            && venv.interpreter().matches_request(
                &request.with_default_arch(python_arch.map(PythonArchitecture::into_inner)),
                cache,
            )
        {
            Some(venv.interpreter().python_minor_version())
        } else {
            // Without syncing, the environment may not satisfy the explicit request.
            let reporter = PythonDownloadReporter::single(printer);
            let installation = PythonInstallation::find_or_download(
                Some(&request),
                EnvironmentPreference::Any,
                python_preference,
                python_arch,
                python_downloads,
                &client_builder,
                cache,
                Some(&reporter),
                install_mirrors.mirrors(),
                install_mirrors.python_downloads_json_url.as_deref(),
            )
            .await?;
            Some(installation.interpreter().python_minor_version())
        }
    } else {
        None
    };

    let exclude_newer = settings
        .resolver
        .exclude_newer
        .exclude_newer_package_for_index(&PackageName::from_str("ty")?, None);

    ty::run(
        ty_version,
        ty_path.or(locked_ty_path),
        fix,
        &target_dir,
        project
            .as_ref()
            .map(|project| project.workspace().install_path().as_path()),
        lock_check,
        frozen,
        &check_targets,
        &excluded_targets,
        explicit_targets,
        venv.as_ref().map(PythonEnvironment::root),
        python_version.as_ref(),
        exclude_newer,
        show_version,
        show_command,
        &client_builder,
        cache,
        color,
        printer,
    )
    .await
}
