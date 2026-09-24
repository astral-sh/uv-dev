pub(crate) mod dir;
pub(crate) mod find;
pub(crate) mod install;
pub(crate) mod list;
pub(crate) mod pin;
pub(crate) mod uninstall;
pub(crate) mod update_shell;
mod upgrade_report;

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct PythonVersionParts {
    #[cfg_attr(feature = "schemars", schemars(range(max = u64::MAX)))]
    major: u64,
    #[cfg_attr(feature = "schemars", schemars(range(max = u64::MAX)))]
    minor: u64,
    #[cfg_attr(feature = "schemars", schemars(range(max = u64::MAX)))]
    patch: u64,
}

impl From<&uv_python::PythonInstallationKey> for PythonVersionParts {
    fn from(key: &uv_python::PythonInstallationKey) -> Self {
        let version = key.version();
        let release = version.release();
        Self {
            major: release.first().copied().unwrap_or(0),
            minor: release.get(1).copied().unwrap_or(0),
            patch: release.get(2).copied().unwrap_or(0),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum ChangeEventKind {
    /// The Python version was uninstalled.
    Removed,
    /// The Python version was installed.
    Added,
    /// The Python version was reinstalled.
    Reinstalled,
}

#[derive(Debug)]
pub(super) struct ChangeEvent {
    key: uv_python::PythonInstallationKey,
    kind: ChangeEventKind,
}
