#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum InstallerMetadata {
    /// Write installer metadata to the `.dist-info` directory.
    #[default]
    Enabled,
    /// Do not write installer metadata to the `.dist-info` directory.
    Disabled,
}

impl InstallerMetadata {
    /// Determine the [`InstallerMetadata`] setting based on the command-line arguments.
    pub fn from_args(installer_metadata: bool) -> Self {
        if installer_metadata {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }

    /// Returns `true` if installer metadata is enabled.
    pub(crate) const fn enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}
