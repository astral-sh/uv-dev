use anyhow::Result;

use uv_configuration::{
    BuildOptions, DependencyGroups, DependencyGroupsWithDefaults, InstallOptions,
};
use uv_distribution_types::Resolution;
use uv_lock::{Lock, SelectedDependency};
use uv_normalize::{DEV_DEPENDENCIES, DefaultGroups, GroupName, PackageName};
use uv_python_interpreter::Interpreter;
use uv_resolve_operations::{resolution_markers, resolution_tags};
use uv_workspace::VirtualProject;

/// A locked package selected for use as a project tool.
pub(super) struct LockedTool<'lock> {
    dependency: SelectedDependency<'lock>,
    requires_separate_environment: bool,
}

impl<'lock> LockedTool<'lock> {
    fn dependency(&self) -> &SelectedDependency<'lock> {
        &self.dependency
    }

    /// Returns `true` if the tool must be installed outside the selected project environment.
    pub(super) fn requires_separate_environment(&self) -> bool {
        self.requires_separate_environment
    }
}

/// Find a tool in a dependency group, falling back to the current project's production
/// dependencies.
pub(super) fn find_locked_tool<'lock>(
    project: &VirtualProject,
    lock: &'lock Lock,
    interpreter: &Interpreter,
    package_name: &PackageName,
    dependency_group: &GroupName,
    groups: &DependencyGroupsWithDefaults,
) -> Result<Option<LockedTool<'lock>>> {
    let marker_environment = interpreter.to_resolver_marker_environment();
    let selection = lock
        .dependency_selection(
            project.project_name(),
            package_name,
            marker_environment.markers(),
        )
        .map_err(anyhow::Error::msg)?;
    let (dependency, installed) = if let Some(dependency) = selection.group(dependency_group) {
        (dependency.clone(), groups.contains(dependency_group))
    } else if let Some(dependency) = selection.production() {
        (dependency.clone(), groups.prod())
    } else {
        return Ok(None);
    };

    Ok(Some(LockedTool {
        dependency,
        requires_separate_environment: !installed,
    }))
}

/// The workspace-root declaration used to select an isolated script's checker.
pub(super) enum WorkspaceToolSource<'lock> {
    Group(&'lock GroupName),
    Production,
}

impl WorkspaceToolSource<'_> {
    /// Select only the tool's group when determining its Python requirements.
    pub(super) fn groups(&self) -> DependencyGroupsWithDefaults {
        let groups = match self {
            Self::Group(group) => DependencyGroups::from_group((*group).clone()),
            Self::Production => DependencyGroups::default(),
        };
        groups.with_defaults(DefaultGroups::default())
    }
}

/// Find the tool's declaration before selecting a compatible workspace interpreter.
pub(super) fn find_workspace_tool_source<'lock>(
    project: &VirtualProject,
    lock: &'lock Lock,
    package_name: &PackageName,
) -> Result<Option<WorkspaceToolSource<'lock>>> {
    let (production, groups) = if let Some(project_name) = project.project_name() {
        let Some(package) = lock
            .find_by_name(project_name)
            .map_err(anyhow::Error::msg)?
        else {
            return Ok(None);
        };
        let production = package
            .dependencies()
            .iter()
            .any(|dependency| dependency.package_name() == package_name);
        let groups = package
            .resolved_dependency_groups()
            .iter()
            .filter(|(_, dependencies)| {
                dependencies
                    .iter()
                    .any(|dependency| dependency.package_name() == package_name)
            })
            .map(|(group, _)| group)
            .collect::<Vec<_>>();
        (production, groups)
    } else {
        let groups = lock
            .dependency_groups()
            .iter()
            .filter(|(_, requirements)| {
                requirements
                    .iter()
                    .any(|requirement| &requirement.name == package_name)
            })
            .map(|(group, _)| group)
            .collect::<Vec<_>>();
        (false, groups)
    };

    if let Some(group) = groups.iter().find(|group| **group == &*DEV_DEPENDENCIES) {
        return Ok(Some(WorkspaceToolSource::Group(group)));
    }
    if production {
        return Ok(Some(WorkspaceToolSource::Production));
    }

    // Projects can keep their checker in a dedicated group, such as `check`.
    // Without a selected group, multiple such declarations are ambiguous.
    match groups.as_slice() {
        [] => Ok(None),
        [group] => Ok(Some(WorkspaceToolSource::Group(group))),
        [group, other_group, ..] => {
            anyhow::bail!(
                "Cannot select `{package_name}` from workspace dependency groups `{group}` and `{other_group}`; use `TY` or `--ty-version` to select the checker"
            );
        }
    }
}

/// Find a workspace tool for an isolated script without synchronizing the workspace.
pub(super) fn find_locked_workspace_tool<'lock>(
    project: &VirtualProject,
    lock: &'lock Lock,
    interpreter: &Interpreter,
    package_name: &PackageName,
    source: &WorkspaceToolSource<'lock>,
) -> Result<Option<LockedTool<'lock>>> {
    let marker_environment = interpreter.to_resolver_marker_environment();
    let selection = lock
        .dependency_selection(
            project.project_name(),
            package_name,
            marker_environment.markers(),
        )
        .map_err(anyhow::Error::msg)?;
    let dependency = match source {
        WorkspaceToolSource::Group(group) => selection.group(group),
        WorkspaceToolSource::Production => selection.production(),
    };
    Ok(dependency.map(|dependency| LockedTool {
        dependency: dependency.clone(),
        requires_separate_environment: true,
    }))
}

/// Materialize the exact dependency subgraph for a locked tool selection.
pub(super) fn resolution_from_lock(
    project: &VirtualProject,
    lock: &Lock,
    tool: &LockedTool<'_>,
    interpreter: &Interpreter,
    build_options: &BuildOptions,
) -> Result<Resolution> {
    let marker_environment = resolution_markers(None, None, interpreter);
    let tags = resolution_tags(None, None, interpreter)?;
    Ok(lock.to_resolution_from_dependency(
        project.workspace().install_path(),
        tool.dependency(),
        project.project_name(),
        &marker_environment,
        &tags,
        build_options,
        &InstallOptions::default(),
    )?)
}
