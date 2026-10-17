pub use workspace::{
    DefaultGroupsError, DiscoveryOptions, Editability, MemberDiscovery,
    ProjectEnvironmentSelection, ProjectWorkspace, RequiresPythonDeclaration,
    RequiresPythonSources, VirtualProject, Workspace, WorkspaceCache, WorkspaceError,
    WorkspaceErrorKind, WorkspaceMember, WorkspaceMemberMatcher,
};

pub mod dependency_groups;
pub mod pyproject;
mod workspace;
