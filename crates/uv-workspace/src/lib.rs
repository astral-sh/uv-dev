pub use uv_normalize::PackageName;

pub use workspace::{
    DefaultGroupsError, DiscoveryOptions, Editability, MemberDiscovery,
    ProjectEnvironmentSelection, ProjectWorkspace, RequiresPythonDeclaration,
    RequiresPythonSources, VirtualProject, Workspace, WorkspaceCache, WorkspaceError,
    WorkspaceErrorKind, WorkspaceMember, format_requires_python_sources,
};

pub mod dependency_groups;
pub mod pyproject;
mod workspace;
