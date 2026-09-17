pub use resolution_axes::{
    ResolvedWorkspaceAxes, WorkspaceAxes, WorkspaceAxis, WorkspaceAxisAssignment,
    WorkspaceAxisDomain, WorkspaceAxisEnvironment, WorkspaceAxisError, WorkspaceAxisName,
    WorkspaceAxisResolutionView, WorkspaceAxisSection, WorkspaceAxisSelection,
    WorkspaceSectionName,
};
pub use resolution_axis_groups::{WorkspaceAxisDependencyGroup, WorkspaceAxisGroupMetadata};
pub use workspace::{
    DefaultGroupsError, DiscoveryOptions, Editability, MemberDiscovery,
    ProjectEnvironmentSelection, ProjectWorkspace, RequiresPythonSources, VirtualProject,
    Workspace, WorkspaceCache, WorkspaceError, WorkspaceErrorKind, WorkspaceMember,
};
pub use workspace_groups::{ResolvedWorkspaceGroup, WorkspaceGroup};

pub mod dependency_groups;
pub mod pyproject;
pub mod pyproject_mut;
mod resolution_axes;
mod resolution_axis_groups;
mod workspace;
mod workspace_groups;
