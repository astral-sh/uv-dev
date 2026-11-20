use std::path::{Path, PathBuf};

use uv_bin_install::BinVersion;
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

use super::TY;

fn tool_scopes() -> [DependencyScope<'static>; 2] {
    [
        DependencyScope::Group(&DEV_DEPENDENCIES),
        DependencyScope::Production,
    ]
}

/// The executable to run, or the version request for a standalone download.
pub(super) enum TySource {
    Executable(PathBuf),
    Standalone(BinVersion),
}

impl Default for TySource {
    fn default() -> Self {
        Self::Standalone(BinVersion::Default)
    }
}

impl TySource {
    /// Select an explicit override, or leave the source open for lockfile lookup.
    pub(super) fn requested(
        path: Option<PathBuf>,
        version: Option<String>,
    ) -> anyhow::Result<Option<Self>> {
        if let Some(path) = path {
            return Ok(Some(Self::Executable(path)));
        }
        Ok(version
            .map(|v| v.parse().map(Self::Standalone))
            .transpose()?)
    }

    fn installed(environment: &PythonEnvironment) -> Self {
        Self::Executable(
            environment
                .scripts()
                .join(format!("ty{}", std::env::consts::EXE_SUFFIX)),
        )
    }

    /// Prefer the script's direct checker to one supplied by its workspace root.
    pub(super) async fn for_script(
        requested: Option<Self>,
        target: InstallTarget<'_>,
        environment: &PythonEnvironment,
        workspace: Option<WorkspaceLookup<'_>>,
        installer: &LockedToolInstaller<'_>,
    ) -> Result<Self, UvError> {
        if let Some(source) = requested {
            return Ok(source);
        }
        if target
            .for_environment(environment.interpreter(), None)?
            .dependency_selection(None, &TY)?
            .root()
            .is_some()
        {
            return Ok(Self::installed(environment));
        }
        let Some(workspace) = workspace else {
            return Ok(Self::default());
        };
        let source = workspace.checker(installer).await.map_err(|error| {
            error.context("Failed to look up or install `ty` from the script's workspace. Use `--no-project` to skip the workspace lookup")
        })?;
        Ok(source.unwrap_or_default())
    }

    /// Use the selected project environment when synchronization includes the tool.
    pub(super) async fn for_project(
        requested: Option<Self>,
        project_name: Option<&PackageName>,
        target: InstallTarget<'_>,
        interpreter: &Interpreter,
        sync: Option<SyncSelection<'_>>,
        environment: &PythonEnvironment,
        installer: &LockedToolInstaller<'_>,
        malware_context: &mut MalwareCheckContext<'_>,
    ) -> Result<Self, UvError> {
        if let Some(source) = requested {
            return Ok(source);
        }
        if let Some(sync) = sync {
            match target
                .for_environment(environment.interpreter(), None)?
                .tool_location(project_name, &TY, &tool_scopes(), &sync)?
            {
                ToolLocation::Inactive => return Ok(Self::default()),
                ToolLocation::Project => return Ok(Self::installed(environment)),
                ToolLocation::Cache => {}
            }
        }
        for scope in tool_scopes() {
            let Some(tool) = LockedTool::find(target, project_name, &TY, scope)? else {
                continue;
            };
            if let Some(environment) = installer
                .install(tool, interpreter, malware_context)
                .await?
            {
                return Ok(Self::installed(&environment));
            }
        }
        Ok(Self::default())
    }
}

/// Inputs to workspace discovery, used only when the script needs a checker.
pub(super) struct WorkspaceLookup<'a> {
    pub(super) script_dir: &'a Path,
    pub(super) workspace_cache: &'a WorkspaceCache,
    pub(super) config_discovery: ConfigDiscovery,
    pub(super) python_preference: PythonPreference,
    pub(super) python_arch: Option<PythonArchitecture>,
    pub(super) python_downloads: PythonDownloads,
    pub(super) malware_settings: &'a MalwareCheckSettings,
    pub(super) install_mirrors: &'a PythonInstallMirrors,
}

impl WorkspaceLookup<'_> {
    async fn checker(
        &self,
        installer: &LockedToolInstaller<'_>,
    ) -> Result<Option<TySource>, UvError> {
        let Some(project) = VirtualProject::discover_for_script(
            self.script_dir,
            installer.cache,
            self.workspace_cache,
        )
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
            if let Some(tool) = LockedTool::find(target, project_name, &TY, scope)? {
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
            self.config_discovery,
        )
        .await
        .map_err(EnvironmentError::from)?;
        let interpreter = ProjectInterpreter::discover(
            ProjectEnvironmentTarget::from(workspace),
            project_python,
            installer.client_builder,
            self.python_preference,
            self.python_arch,
            self.python_downloads,
            self.install_mirrors,
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
                    &mut MalwareCheckContext::from(self.malware_settings),
                )
                .await?
            {
                return Ok(Some(TySource::installed(&environment)));
            }
        }
        warn_user!(
            "No locked `ty` dependency applies to workspace Python {}. Using the standalone checker.",
            interpreter.python_minor_version()
        );
        Ok(None)
    }
}
