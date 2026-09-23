use serde::Serialize;
use uv_fs::PortablePathBuf;
use uv_pep440::Version;
use uv_python::PythonInstallationKey;
use uv_python::managed::ManagedPythonInstallation;

use super::PythonVersionParts;
use crate::commands::report::SchemaReport;

#[derive(Debug, Default, Serialize)]
pub(super) struct UpgradeReport {
    pub(super) schema: SchemaReport,
    pub(super) upgrades: Vec<UpgradeEntry>,
    pub(super) errors: Vec<UpgradeError>,
}

impl UpgradeReport {
    /// Capture an installation before any later finalization or bytecode error can stop the run.
    pub(super) fn record_installation(&mut self, installation: &ManagedPythonInstallation) {
        let key = installation.key().to_string();
        for entry in &mut self.upgrades {
            if entry.selected_key == key {
                let to = InstallationReport::from(installation);
                entry.outcome = installation_outcome(
                    &entry.from,
                    Some(&to),
                    true,
                    false,
                    !entry.errors.is_empty(),
                );
                entry.to = Some(to);
            }
        }
    }

    pub(super) fn fail_installation(
        &mut self,
        key: &PythonInstallationKey,
        kind: UpgradeErrorKind,
        message: String,
    ) {
        let key = key.to_string();
        for entry in &mut self.upgrades {
            if entry.selected_key == key {
                entry.outcome = UpgradeOutcome::Failed;
                entry.errors.push(UpgradeError {
                    kind,
                    message: message.clone(),
                });
            }
        }
    }
}

/// The result of one resolved upgrade request, including any completed side effects.
#[derive(Debug, Serialize)]
pub(super) struct UpgradeEntry {
    #[serde(skip)]
    pub(super) selected_key: String,
    pub(super) request: String,
    pub(super) outcome: UpgradeOutcome,
    /// All matching installations observed before the upgrade. They need not be removed.
    pub(super) from: Vec<InstallationReport>,
    /// The selected installation, only when it was already present or installed successfully.
    pub(super) to: Option<InstallationReport>,
    pub(super) executables: Vec<ExecutableChange>,
    pub(super) errors: Vec<UpgradeError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UpgradeOutcome {
    NotCompleted,
    NoOp,
    Installed,
    Upgraded,
    Reinstalled,
    UpdateExecutables,
    Failed,
}

pub(super) fn installation_outcome(
    from: &[InstallationReport],
    to: Option<&InstallationReport>,
    downloaded: bool,
    executable_changes: bool,
    errors: bool,
) -> UpgradeOutcome {
    if errors {
        UpgradeOutcome::Failed
    } else if let Some(to) = to {
        if downloaded {
            if from.is_empty() {
                UpgradeOutcome::Installed
            } else if from.contains(to) {
                UpgradeOutcome::Reinstalled
            } else {
                UpgradeOutcome::Upgraded
            }
        } else if executable_changes {
            UpgradeOutcome::UpdateExecutables
        } else {
            UpgradeOutcome::NoOp
        }
    } else {
        UpgradeOutcome::NotCompleted
    }
}

/// Installation identity follows `python list`, with the build recorded separately from the key.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub(super) struct InstallationReport {
    pub(super) key: String,
    version: Version,
    version_parts: PythonVersionParts,
    path: PortablePathBuf,
    pub(super) build: Option<String>,
    os: String,
    variant: String,
    implementation: String,
    arch: String,
    libc: String,
}

impl From<&ManagedPythonInstallation> for InstallationReport {
    fn from(installation: &ManagedPythonInstallation) -> Self {
        let key = installation.key();
        Self {
            key: key.to_string(),
            version: key.version().version().clone(),
            version_parts: key.into(),
            path: installation.path().into(),
            build: installation.build().map(str::to_owned),
            os: key.os().to_string(),
            variant: key.variant().to_string(),
            implementation: key.implementation().to_string(),
            arch: key.arch().to_string(),
            libc: key.libc().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ExecutableChange {
    pub(super) path: PortablePathBuf,
    pub(super) from: Option<InstallationReport>,
    pub(super) to: InstallationReport,
}

#[derive(Debug, Serialize)]
pub(super) struct UpgradeError {
    pub(super) kind: UpgradeErrorKind,
    pub(super) message: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UpgradeErrorKind {
    Download,
    Executable,
    Registry,
    DownloadsDisabled,
    Finalize,
    MinorVersionLink,
    Bytecode,
    Operation,
}
