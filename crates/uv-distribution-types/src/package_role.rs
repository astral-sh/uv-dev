/// A package's role relative to the project and workspace supplied by an operation.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum PackageRole {
    /// The designated project, without membership in the supplied workspace.
    Project,
    /// The designated project, which is also a member of the supplied workspace.
    WorkspaceProject,
    /// A workspace member other than the designated project.
    WorkspaceMember,
    /// A dependency that is neither the designated project nor a workspace member.
    Dependency,
}

impl PackageRole {
    /// Return whether the package belongs to the supplied workspace.
    pub(crate) fn is_workspace_member(self) -> bool {
        match self {
            Self::WorkspaceProject | Self::WorkspaceMember => true,
            Self::Project | Self::Dependency => false,
        }
    }
}
