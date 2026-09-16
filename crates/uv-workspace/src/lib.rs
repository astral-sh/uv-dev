pub use workspace::{
    DefaultGroupsError, DiscoveryOptions, Editability, MemberDiscovery,
    ProjectEnvironmentSelection, ProjectWorkspace, RequiresPythonDeclaration,
    RequiresPythonSources, VirtualProject, Workspace, WorkspaceCache, WorkspaceError,
    WorkspaceErrorKind, WorkspaceMember,
};
pub use workspace_groups::{ResolvedWorkspaceGroup, WorkspaceGroup};

pub mod dependency_groups;
pub mod pyproject;
mod workspace;
mod workspace_groups;
