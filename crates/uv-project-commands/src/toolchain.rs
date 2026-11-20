use anyhow::Result;

use uv_configuration::DependencyGroupsWithDefaults;
use uv_environment_operations::install_target::InstallTarget;
use uv_lock::{Installable, Lock, SelectedDependency};
use uv_normalize::{GroupName, PackageName};
use uv_python_interpreter::Interpreter;

/// A locked package selected for use as a project tool.
pub(super) struct LockedTool<'lock> {
    dependency: SelectedDependency<'lock>,
    requires_separate_environment: bool,
}

impl<'lock> LockedTool<'lock> {
    pub(super) fn dependency(&self) -> &SelectedDependency<'lock> {
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
    project_name: Option<&PackageName>,
    target: InstallTarget<'lock>,
    interpreter: &Interpreter,
    package_name: &PackageName,
    dependency_group: &GroupName,
    groups: &DependencyGroupsWithDefaults,
) -> Result<Option<LockedTool<'lock>>> {
    // When no checker is declared, `--no-sync` can use a standalone checker even
    // outside the supported environments.
    if !declares_tool(target.lock(), project_name, package_name, dependency_group)? {
        return Ok(None);
    }
    let selection = target
        .for_environment(interpreter, None)?
        .dependency_selection(project_name, package_name)
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

/// Check for a direct declaration before evaluating its markers.
fn declares_tool(
    lock: &Lock,
    project_name: Option<&PackageName>,
    package_name: &PackageName,
    dependency_group: &GroupName,
) -> Result<bool> {
    let Some(project_name) = project_name else {
        return Ok(lock
            .dependency_groups()
            .get(dependency_group)
            .is_some_and(|group| {
                group
                    .iter()
                    .any(|requirement| &requirement.name == package_name)
            }));
    };
    let Some(project) = lock
        .find_by_name(project_name)
        .map_err(anyhow::Error::msg)?
    else {
        return Ok(false);
    };
    Ok(project
        .dependencies()
        .iter()
        .chain(
            project
                .resolved_dependency_groups()
                .get(dependency_group)
                .into_iter()
                .flatten(),
        )
        .any(|dependency| dependency.package_name() == package_name))
}
