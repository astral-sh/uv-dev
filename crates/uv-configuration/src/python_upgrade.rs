/// The command that requested a managed Python upgrade.
#[derive(Debug, Clone, Copy)]
pub enum PythonUpgradeSource {
    /// The user invoked `uv python install --upgrade`
    Install,
    /// The user invoked `uv python upgrade`
    Upgrade,
}

impl std::fmt::Display for PythonUpgradeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Install => write!(f, "uv python install --upgrade"),
            Self::Upgrade => write!(f, "uv python upgrade"),
        }
    }
}

/// Whether managed Python installations should be upgraded.
#[derive(Debug, Clone, Copy)]
pub enum PythonUpgrade {
    /// Python upgrades are enabled.
    Enabled(PythonUpgradeSource),
    /// Python upgrades are disabled.
    Disabled,
}

/// Whether to reinstall matching Python versions.
#[derive(Debug, Clone, Copy)]
pub enum PythonReinstall {
    /// Reinstall matching Python versions.
    Enabled,
    /// Respect existing Python installations.
    Disabled,
}

impl PythonReinstall {
    pub fn is_enabled(self) -> bool {
        match self {
            Self::Enabled => true,
            Self::Disabled => false,
        }
    }
}

impl From<bool> for PythonReinstall {
    fn from(value: bool) -> Self {
        if value { Self::Enabled } else { Self::Disabled }
    }
}

/// Whether to replace existing Python executables.
#[derive(Debug, Clone, Copy)]
pub enum PythonInstallForce {
    /// Replace existing Python executables.
    Enabled,
    /// Respect existing Python executables.
    Disabled,
}

impl PythonInstallForce {
    pub fn is_enabled(self) -> bool {
        match self {
            Self::Enabled => true,
            Self::Disabled => false,
        }
    }
}

impl From<bool> for PythonInstallForce {
    fn from(value: bool) -> Self {
        if value { Self::Enabled } else { Self::Disabled }
    }
}

/// Whether to create default Python executable links.
#[derive(Debug, Clone, Copy)]
pub enum PythonInstallDefault {
    /// Create default Python executable links.
    Enabled,
    /// Do not explicitly request default Python executable links.
    ///
    /// Default installs can still create these links implicitly.
    Disabled,
}

impl PythonInstallDefault {
    pub fn is_enabled(self) -> bool {
        match self {
            Self::Enabled => true,
            Self::Disabled => false,
        }
    }
}

impl From<bool> for PythonInstallDefault {
    fn from(value: bool) -> Self {
        if value { Self::Enabled } else { Self::Disabled }
    }
}
