#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ProjectDiscovery {
    /// Discover the project when executing the command.
    #[default]
    Enabled,
    /// Ignore the project when executing the command.
    Disabled,
}

impl ProjectDiscovery {
    /// Determine the [`ProjectDiscovery`] setting based on the command-line arguments.
    pub fn from_args(no_project: bool) -> Self {
        if no_project {
            Self::Disabled
        } else {
            Self::Enabled
        }
    }

    /// Returns `true` if project discovery is enabled.
    pub const fn enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }
}
