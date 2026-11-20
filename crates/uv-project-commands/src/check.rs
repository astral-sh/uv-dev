use std::path::{Path, PathBuf};
use std::sync::LazyLock;

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
use uv_environment_operations::LockedToolInstaller;
use uv_environment_operations::install_target::SyncSelection;
use uv_environment_operations::install_target::{InstallTarget, PackageSelection};
use uv_environment_operations::malware::MalwareCheckContext;
use uv_environment_operations::{
    LinkErrorReporting, ProjectEnvironment, ProjectEnvironmentPolicy, ProjectEnvironmentTarget,
    ProjectInterpreter, ScriptEnvironment, sync_from_lock,
};
use uv_fs::normalize_path;
use uv_install_operations::loggers::SummaryInstallLogger;
use uv_lock_operations::{LockMode, LockOperation, LockTarget};
use uv_normalize::{DefaultExtras, PackageName};
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

pub use self::source::WorkspaceSettings;
use self::source::{TySource, WorkspaceLookup};

mod source;
mod ty;

static TY: LazyLock<PackageName> = LazyLock::new(|| "ty".parse().expect("valid package name"));

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
    resolve_workspace_settings: impl FnOnce(&Path) -> Result<WorkspaceSettings>,
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

    if no_project && script.is_none() {
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
                .values()
                .map(|member| member.root().clone())
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
        && !defacto_all_packages
    {
        project
            .workspace()
            .packages()
            .iter()
            .filter(|(name, member)| {
                let selected = if package.is_empty() {
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

    let groups = if let Some(project) = &project {
        groups.with_defaults(project.default_groups()?)
    } else {
        DependencyGroupsWithDefaults::none()
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
            let workspace = project.as_ref().map(VirtualProject::workspace);
            let project_python = ProjectPythonRequest::from_request(
                python.as_deref().map(PythonRequest::parse),
                workspace,
                &groups,
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

    // Prepare the analysis environment and select the checker that will inspect it.
    let state = UniversalState::default();
    let requested = TySource::requested(ty_path, ty_version)?;
    let (venv, ty_source) = if let Some(script) = &script {
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
        let environment_lock = venv
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

        let target = InstallTarget::Script {
            script,
            lock: result.lock(),
        };
        match sync_from_lock(
            target,
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
        drop(environment_lock);

        if no_sync {
            warn_user!(
                "`--no-sync` is a no-op for Python scripts with inline metadata, which always run in isolation"
            );
        }

        let workspace = (!no_project).then_some(WorkspaceLookup {
            script_dir: &target_dir,
            workspace_cache,
            config_discovery,
            resolve_settings: resolve_workspace_settings,
        });
        let source =
            TySource::for_script(requested, target, &venv, workspace, cache, printer).await?;
        (Some(venv), source)
    } else if let Some(project) = &project {
        let extras = extras.with_defaults(DefaultExtras::default());
        let mut malware_context = MalwareCheckContext::from(&malware_settings);
        let install_options = InstallOptions::new(
            no_install_project,
            false,
            false,
            false,
            false,
            false,
            Vec::new(),
            Vec::new(),
        );

        let venv = if let Some(venv) = isolated_venv {
            venv
        } else {
            ProjectEnvironment::get_or_init(
                ProjectEnvironmentTarget::from(project.workspace()),
                None,
                &groups,
                python.as_deref().map(PythonRequest::parse),
                &install_mirrors,
                &client_builder,
                python_preference,
                python_arch,
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

        // `--no-sync` intentionally permits an incompatible project environment, but locking must
        // still use an interpreter that satisfies the project and any explicit Python request.
        let lock_interpreter = if no_sync && !isolated && frozen.is_none() {
            let project_python = ProjectPythonRequest::from_request(
                python.as_deref().map(PythonRequest::parse),
                Some(project.workspace()),
                &groups,
                project_dir,
                config_discovery,
            )
            .await?;
            Some(
                ProjectInterpreter::discover(
                    ProjectEnvironmentTarget::from(project.workspace()),
                    project_python,
                    &client_builder,
                    python_preference,
                    python_arch,
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
        };

        let target = InstallTarget::from_project(project, result.lock(), selection);

        target.validate_extras(&extras)?;
        target.validate_groups(&groups)?;

        let tool_installer = LockedToolInstaller {
            settings: &settings,
            client_builder: &client_builder,
            state: &state,
            installer_metadata,
            concurrency: &concurrency,
            cache,
            printer,
            preview,
        };
        let source = TySource::for_project(
            requested,
            project.project_name(),
            target,
            lock_interpreter,
            (!no_sync).then_some(SyncSelection {
                extras: &extras,
                groups: &groups,
                install_options: &install_options,
                build_options: &settings.resolver.build_options,
            }),
            &venv,
            &tool_installer,
            &mut malware_context,
        )
        .await?;

        if no_sync {
            debug!("Skipping environment synchronization due to `--no-sync`");
        } else {
            let sync_state = state.fork();
            match sync_from_lock(
                target,
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

        (Some(venv), source)
    } else {
        (isolated_venv, requested.unwrap_or_default())
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
        .exclude_newer_package_for_index(&TY, None);

    ty::run(
        ty_source,
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
