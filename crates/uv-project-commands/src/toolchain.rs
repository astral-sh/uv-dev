use std::path::Path;

use uv_command_support::UvError;
use uv_configuration::{ActiveEnvironment, DependencyGroupsWithDefaults};
use uv_environment_operations::install_target::{InstallTarget, PackageSelection};
use uv_environment_operations::install_target::{SyncSelection, ToolLocation};
use uv_environment_operations::malware::MalwareCheckContext;
use uv_environment_operations::{
    EnvironmentError, ProjectEnvironmentPolicy, ProjectEnvironmentTarget, ProjectInterpreter,
};
use uv_environment_operations::{LockedTool, LockedToolInstaller};
use uv_lock::{DependencyScope, Package};
use uv_lock_operations::LockTarget;
use uv_normalize::{DEV_DEPENDENCIES, PackageName};
use uv_python_discovery::{ConfigDiscovery, ProjectPythonRequest};
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_python_types::{PythonArchitecture, PythonDownloads, PythonPreference};
use uv_settings::{MalwareCheckSettings, PythonInstallMirrors};
use uv_warnings::warn_user;
use uv_workspace::{VirtualProject, WorkspaceCache};

fn tool_scopes() -> [DependencyScope<'static>; 2] {
    [
        DependencyScope::Group(&DEV_DEPENDENCIES),
        DependencyScope::Production,
    ]
}

pub(super) async fn project_tool(
    project_name: Option<&PackageName>,
    target: InstallTarget<'_>,
    interpreter: &Interpreter,
    name: &PackageName,
    sync: Option<SyncSelection<'_>>,
    environment: &PythonEnvironment,
    installer: &LockedToolInstaller<'_>,
    malware_context: &mut MalwareCheckContext<'_>,
) -> Result<Option<PythonEnvironment>, UvError> {
    if let Some(sync) = sync {
        match target
            .for_environment(environment.interpreter(), None)?
            .tool_location(project_name, name, &tool_scopes(), &sync)?
        {
            ToolLocation::Inactive => return Ok(None),
            ToolLocation::Project => return Ok(Some(environment.clone())),
            ToolLocation::Cache => {}
        }
    }
    for scope in tool_scopes() {
        let Some(tool) = LockedTool::find(target, project_name, name, scope)? else {
            continue;
        };
        if let Some(environment) = installer
            .install(tool, interpreter, malware_context)
            .await?
        {
            return Ok(Some(environment));
        }
    }
    Ok(None)
}

/// Use the script's containing workspace only to locate and install the checker.
pub(super) async fn install_from_script_workspace(
    installer: &LockedToolInstaller<'_>,
    malware_settings: &MalwareCheckSettings,
    script_dir: &Path,
    name: &PackageName,
    workspace_cache: &WorkspaceCache,
    config_discovery: ConfigDiscovery,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    install_mirrors: &PythonInstallMirrors,
) -> Result<Option<PythonEnvironment>, UvError> {
    let Some(project) =
        VirtualProject::discover_for_script(script_dir, installer.cache, workspace_cache)
            .await
            .map_err(EnvironmentError::from)?
    else {
        return Ok(None);
    };
    let workspace = project.workspace();
    let Some(lock) = LockTarget::Workspace(workspace).read().await? else {
        return Ok(None);
    };
    let root = lock.root();
    let project_name = root.map(Package::name);
    let selection = PackageSelection::from_args(false, &[], project_name);
    let target = InstallTarget::from_project(&project, &lock, selection);
    let mut tools = Vec::new();
    for scope in tool_scopes() {
        if let Some(tool) = LockedTool::find(target, project_name, name, scope)? {
            tools.push(tool);
        }
    }
    if tools.is_empty() {
        return Ok(None);
    }

    // Only the checker subgraph is installed. Select one workspace interpreter without
    // group-specific bounds, then evaluate both declarations against its markers.
    let groups = DependencyGroupsWithDefaults::none();
    let requirement = target.python_requirement(&groups)?;
    let project_python = ProjectPythonRequest::from_requirements(
        None,
        Some(workspace.install_path()),
        Some(requirement),
        workspace.install_path(),
        config_discovery,
    )
    .await
    .map_err(EnvironmentError::from)?;
    let interpreter = ProjectInterpreter::discover(
        ProjectEnvironmentTarget::from(workspace),
        project_python,
        installer.client_builder,
        python_preference,
        python_arch,
        python_downloads,
        install_mirrors,
        ProjectEnvironmentPolicy::Optional,
        ActiveEnvironment::Ignore,
        installer.cache,
        installer.printer,
    )
    .await?
    .into_interpreter();
    for tool in tools {
        if let Some(environment) = installer
            .install(
                tool,
                &interpreter,
                &mut MalwareCheckContext::from(malware_settings),
            )
            .await?
        {
            return Ok(Some(environment));
        }
    }
    warn_user!(
        "No locked `{name}` dependency applies to workspace Python {}. Using the standalone checker.",
        interpreter.python_minor_version()
    );
    Ok(None)
}
