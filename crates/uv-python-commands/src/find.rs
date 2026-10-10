use anyhow::Result;
use std::fmt::Write;
use std::path::Path;

use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_configuration::{ActiveEnvironment, DependencyGroupsWithDefaults, NoSources};
use uv_errors::ErrorWithHints;
use uv_fs::Simplified;
use uv_python_discovery::ConfigDiscovery;
use uv_python_discovery::PythonInstallation;
use uv_python_types::{
    EnvironmentPreference, PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest,
};
use uv_scripts::Pep723ItemRef;
use uv_settings::PythonInstallMirrors;
use uv_warnings::{warn_user, warn_user_once_with_chain};
use uv_workspace::{DiscoveryOptions, VirtualProject, WorkspaceCache, WorkspaceErrorKind};

use uv_command_support::ExitStatus;
use uv_command_support::Printer;
use uv_python_discovery::ProjectPythonRequest;
use uv_python_discovery::ScriptInterpreter;

/// Find a Python interpreter.
#[expect(clippy::fn_params_excessive_bools)]
pub async fn find(
    project_dir: &Path,
    request: Option<String>,
    show_version: bool,
    resolve_links: bool,
    no_project: bool,
    system: bool,
    config_discovery: ConfigDiscovery,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads_json_url: Option<&str>,
    client_builder: &BaseClientBuilder<'_>,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
) -> Result<ExitStatus> {
    let environment_preference = if system {
        EnvironmentPreference::OnlySystem
    } else {
        EnvironmentPreference::Any
    };

    let project = if no_project {
        None
    } else {
        match VirtualProject::discover(
            project_dir,
            &DiscoveryOptions::default(),
            cache,
            workspace_cache,
        )
        .await
        {
            Ok(project) => Some(project),
            Err(err) => {
                // Ignore missing or unmanaged workspaces in Python discovery.
                if !matches!(
                    err.as_ref(),
                    WorkspaceErrorKind::MissingProject(_)
                        | WorkspaceErrorKind::MissingPyprojectToml
                        | WorkspaceErrorKind::NonWorkspace(_)
                ) {
                    warn_user_once_with_chain!(&err);
                }
                None
            }
        }
    };

    // Don't enable the requires-python settings on groups
    let groups = DependencyGroupsWithDefaults::none();
    // Interpreter-only commands do not build project metadata to refine workspace groups.
    let discovery_workspace = project
        .as_ref()
        .map(|project| {
            let workspace = project.workspace();
            workspace
                .workspace_groups_with_sources(&NoSources::None)
                .and_then(|groups| workspace.with_provisional_workspace_groups(&groups))
        })
        .transpose()?;
    let mut project_python = ProjectPythonRequest::from_request(
        request.map(|request| PythonRequest::parse(&request)),
        discovery_workspace.as_ref(),
        &groups,
        &NoSources::None,
        project_dir,
        config_discovery,
    )
    .await?;

    let probe_request = if project_python.has_environment_constraints() {
        project_python.environment_probe()
    } else {
        project_python.clone()
    };
    let mut python = PythonInstallation::find_existing(
        probe_request
            .python_request
            .as_ref()
            .unwrap_or(&PythonRequest::Default),
        environment_preference,
        python_preference,
        python_arch,
        cache,
    )?;
    if project_python.has_environment_constraints() {
        let requests = match project_python
            .clone()
            .for_environment(python.interpreter().markers())
        {
            Ok(mut requests) => {
                ProjectPythonRequest::prefer_existing(
                    &mut requests,
                    python.interpreter(),
                    environment_preference,
                    python_preference,
                    python_arch,
                    cache,
                )?;
                requests
            }
            // Explicit interpreter queries report project incompatibility without rejecting the
            // requested interpreter, including a platform outside the selected workspace domain.
            Err(error) => {
                warn_user!("{error}");
                vec![project_python.clone()]
            }
        };
        let mut missing = None;
        let mut selected = None;
        for request in requests {
            match PythonInstallation::find_existing(
                request
                    .python_request
                    .as_ref()
                    .unwrap_or(&PythonRequest::Default),
                environment_preference,
                python_preference,
                python_arch,
                cache,
            ) {
                Ok(installation) => {
                    selected = Some((request, installation));
                    break;
                }
                Err(error) if error.can_try_another_request() => missing = Some(error),
                Err(error) => return Err(error.into()),
            }
        }
        let Some((request, installation)) = selected else {
            return Err(missing
                .expect("at least one environment request was attempted")
                .into());
        };
        project_python = request;
        python = installation;
    }
    let python_request = project_python
        .python_request
        .as_ref()
        .unwrap_or(&PythonRequest::Default);
    python
        .download_and_warn_if_outdated_prerelease(
            python_request,
            client_builder,
            cache,
            python_downloads_json_url,
        )
        .await?;

    // Warn if the discovered Python version is incompatible with the current workspace
    if let Err(err) = project_python.check(python.interpreter()) {
        warn_user!("{err}");
    }

    if show_version {
        writeln!(
            printer.stdout(),
            "{}",
            python.interpreter().python_version()
        )?;
    } else {
        let path = if resolve_links {
            dunce::canonicalize(python.interpreter().sys_executable())?
        } else {
            std::path::absolute(python.interpreter().sys_executable())?
        };
        writeln!(printer.stdout(), "{}", path.simplified_display())?;
    }

    Ok(ExitStatus::Success)
}

pub async fn find_script(
    script: Pep723ItemRef<'_>,
    show_version: bool,
    resolve_links: bool,
    client_builder: &BaseClientBuilder<'_>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    config_discovery: ConfigDiscovery,
    cache: &Cache,
    printer: Printer,
) -> Result<ExitStatus> {
    let interpreter = match ScriptInterpreter::discover(
        script,
        None,
        client_builder,
        python_preference,
        python_arch,
        python_downloads,
        &PythonInstallMirrors::default(),
        false,
        config_discovery,
        ActiveEnvironment::Ignore,
        cache,
        printer,
    )
    .await
    {
        Err(error) => {
            writeln!(
                printer.stderr(),
                "{}",
                ErrorWithHints::new(&error, uv_errors::Hinted::hints(&error))
            )?;
            return Ok(ExitStatus::Failure);
        }
        Ok(ScriptInterpreter::Interpreter(selection)) => selection.into_interpreter(),
        Ok(ScriptInterpreter::Environment(environment)) => environment.into_interpreter(),
    };

    if show_version {
        writeln!(printer.stdout(), "{}", interpreter.python_version())?;
    } else {
        let path = if resolve_links {
            dunce::canonicalize(interpreter.sys_executable())?
        } else {
            std::path::absolute(interpreter.sys_executable())?
        };
        writeln!(printer.stdout(), "{}", path.simplified_display())?;
    }

    Ok(ExitStatus::Success)
}
