use std::fmt::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uv_cli::SelfInstallArgs;
use uv_fs::{LockedFile, LockedFileMode, Simplified};
use uv_preview::PreviewFeature;
use uv_static::EnvVars;

use crate::commands::{ExitStatus, update_shell};
use crate::printer::Printer;

/// The receipt remains compatible with installations made by cargo-dist.
#[derive(Debug, Deserialize, Serialize)]
pub(super) struct InstallReceipt {
    pub install_prefix: PathBuf,
    pub binaries: Vec<String>,
    pub source: ReleaseSource,
    pub version: String,
    pub provider: ReceiptProvider,
    #[serde(default = "default_modify_path")]
    pub modify_path: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct ReleaseSource {
    pub release_type: String,
    pub owner: String,
    pub name: String,
    pub app_name: String,
}

impl Default for ReleaseSource {
    fn default() -> Self {
        Self {
            release_type: "github".to_owned(),
            owner: "astral-sh".to_owned(),
            name: "uv".to_owned(),
            app_name: "uv".to_owned(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ReceiptProvider {
    source: String,
    version: String,
}

const fn default_modify_path() -> bool {
    true
}

pub(super) const RECEIPT_NAME: &str = ".uv-receipt.json";

fn legacy_receipt_path() -> Result<PathBuf> {
    if std::env::var_os("AXOUPDATER_CONFIG_WORKING_DIR").is_some() {
        return Ok(std::env::current_dir()?.join("uv-receipt.json"));
    }
    if let Some(path) = std::env::var_os("AXOUPDATER_CONFIG_PATH") {
        return Ok(PathBuf::from(path).join("uv-receipt.json"));
    }
    if let Some(path) = std::env::var_os(EnvVars::XDG_CONFIG_HOME)
        && Path::new(&path).is_absolute()
    {
        let receipt = PathBuf::from(path).join("uv/uv-receipt.json");
        if receipt.try_exists()? {
            return Ok(receipt);
        }
    }
    #[cfg(windows)]
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("Could not determine the local application data directory")?;
    #[cfg(not(windows))]
    let directory = etcetera::home_dir()?.join(".config");
    Ok(directory.join("uv/uv-receipt.json"))
}

pub(super) fn executable_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["uv.exe", "uvx.exe", "uvw.exe"]
    } else {
        &["uv", "uvx"]
    }
}

impl InstallReceipt {
    fn owned_binaries(&self) -> Result<Vec<String>> {
        let mut binaries = Vec::new();
        for name in &self.binaries {
            let normalized = if cfg!(windows)
                && !Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            {
                format!("{name}.exe")
            } else {
                name.clone()
            };
            anyhow::ensure!(
                executable_names().contains(&normalized.as_str()),
                "Install receipt contains an unexpected executable `{name}`"
            );
            if !binaries.contains(&normalized) {
                binaries.push(normalized);
            }
        }
        anyhow::ensure!(
            binaries.iter().any(|name| name == executable_names()[0]),
            "Install receipt does not own the uv executable"
        );
        Ok(binaries)
    }

    pub(super) fn new(install_prefix: PathBuf, modify_path: bool) -> Self {
        Self {
            install_prefix,
            binaries: executable_names()
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            source: ReleaseSource::default(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            provider: ReceiptProvider {
                source: "uv".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            modify_path,
        }
    }

    fn read(path: &Path) -> Result<Self> {
        serde_json::from_slice(&fs_err::read(path)?)
            .with_context(|| format!("Failed to parse install receipt at `{}`", path.display()))
    }

    /// Read an adjacent receipt even if its executable needs to be restored.
    fn for_installation(directory: &Path) -> Result<Self> {
        let directory = fs_err::canonicalize(directory)?;
        let path = directory.join(RECEIPT_NAME);
        let mut receipt = Self::read(&path)?;
        let mut recorded = receipt.install_prefix.clone();
        if uv_fs::is_same_file_allow_missing(&recorded, &directory) != Some(true)
            && receipt.provider.source == "cargo-dist"
        {
            recorded.push("bin");
        }
        anyhow::ensure!(
            uv_fs::is_same_file_allow_missing(&recorded, &directory) == Some(true),
            "The install receipt at `{}` belongs to a different installation at `{}`",
            path.display(),
            recorded.display()
        );
        anyhow::ensure!(
            receipt.source.app_name == "uv",
            "The install receipt is not for uv"
        );
        receipt.install_prefix = directory;
        Ok(receipt)
    }

    pub(super) fn for_executable(executable: &Path) -> Result<(PathBuf, Self)> {
        let executable = fs_err::canonicalize(executable)?;
        let directory = executable
            .parent()
            .context("Executable has no parent directory")?;
        let native = directory.join(RECEIPT_NAME);
        let path = if native.try_exists()? {
            native
        } else {
            legacy_receipt_path()?
        };
        let mut receipt = Self::read(&path)
            .context("Self-management is only available for standalone uv installations")?;
        let mut recorded = receipt
            .install_prefix
            .join(format!("uv{}", std::env::consts::EXE_SUFFIX));
        if uv_fs::is_same_file_allow_missing(&recorded, &executable) != Some(true)
            && receipt.provider.source == "cargo-dist"
        {
            // Prefix-style cargo-dist receipts can name the parent of `bin`.
            recorded = receipt
                .install_prefix
                .join("bin")
                .join(executable_names()[0]);
        }
        anyhow::ensure!(
            uv_fs::is_same_file_allow_missing(&recorded, &executable) == Some(true),
            "The install receipt at `{}` belongs to a different uv executable at `{}`",
            path.display(),
            recorded.display()
        );
        receipt.install_prefix = directory.to_path_buf();
        anyhow::ensure!(
            receipt.source.app_name == "uv",
            "The install receipt is not for uv"
        );
        Ok((path, receipt))
    }

    fn write(&self, path: &Path) -> Result<()> {
        fs_err::create_dir_all(path.parent().context("Receipt has no parent directory")?)?;
        uv_fs::write_atomic_sync(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

/// Serialize changes to one standalone installation.
pub(super) struct LockedInstallation {
    directory: PathBuf,
    _lock: LockedFile,
}

impl LockedInstallation {
    pub(super) async fn acquire(directory: &Path) -> Result<Self> {
        let directory = fs_err::canonicalize(directory)?;
        let lock = LockedFile::acquire(
            directory.join(".uv-install.lock"),
            LockedFileMode::Exclusive,
            "uv installation",
        )
        .await?;
        Ok(Self {
            directory,
            _lock: lock,
        })
    }

    /// Stage a complete distribution before changing installed executables.
    pub(super) fn install_binaries(
        &self,
        source: &Path,
        binaries: &[String],
        receipt: Option<&InstallReceipt>,
        previous: Option<&InstallReceipt>,
    ) -> Result<()> {
        let destination = &self.directory;
        anyhow::ensure!(
            binaries.iter().any(|name| name == executable_names()[0]),
            "Distribution has no uv executable"
        );
        anyhow::ensure!(
            binaries
                .iter()
                .all(|name| executable_names().contains(&name.as_str())),
            "Distribution contains an unexpected executable name"
        );
        let staged = tempfile::tempdir_in(destination)?;
        for name in binaries {
            let source = source.join(name);
            anyhow::ensure!(
                fs_err::symlink_metadata(&source)?.is_file(),
                "Expected a regular executable at `{}`",
                source.display()
            );
            fs_err::copy(&source, staged.path().join(name))?;
        }
        let obsolete = previous
            .map(InstallReceipt::owned_binaries)
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .filter(|name| !binaries.contains(name))
            .collect::<Vec<_>>();
        let backup = staged.path().join("obsolete");
        fs_err::create_dir(&backup)?;
        let mut removed = Vec::new();
        let result = (|| -> Result<()> {
            // Keep obsolete, receipt-owned companions available for rollback until the new receipt
            // is committed. Never recursively remove an unexpected directory at a binary path.
            for name in &obsolete {
                let target = destination.join(name);
                let metadata = match fs_err::symlink_metadata(&target) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                anyhow::ensure!(
                    metadata.is_file() || metadata.file_type().is_symlink(),
                    "Expected a file at obsolete executable `{}`",
                    target.display()
                );
                fs_err::rename(&target, backup.join(name))?;
                removed.push(name);
            }
            // Install the launcher last so it cannot select an incompletely copied uv binary.
            for name in binaries {
                let source = staged.path().join(name);
                let target = destination.join(name);
                #[cfg(windows)]
                if uv_fs::is_same_file_allow_missing(&std::env::current_exe()?, &target)
                    == Some(true)
                {
                    self_replace::self_replace(&source)?;
                    continue;
                }
                uv_fs::copy_atomic_sync(&source, &target)?;
            }
            if let Some(receipt) = receipt {
                receipt.write(&destination.join(RECEIPT_NAME))?;
            } else if destination.join(RECEIPT_NAME).try_exists()? {
                fs_err::remove_file(destination.join(RECEIPT_NAME))?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            for name in removed {
                if let Err(restore_error) =
                    fs_err::rename(backup.join(name), destination.join(name))
                {
                    let backup = staged.keep();
                    return Err(error).with_context(|| {
                        format!(
                            "Failed to restore `{name}`: {restore_error}; installation backup retained at `{}`",
                            backup.display()
                        )
                    });
                }
            }
            return Err(error);
        }
        Ok(())
    }
}

pub(crate) async fn self_install(args: SelfInstallArgs, printer: Printer) -> Result<ExitStatus> {
    anyhow::ensure!(
        uv_preview::is_enabled(PreviewFeature::SelfManagement),
        "Native self-installation is experimental; pass `--preview-features self-management` to enable it"
    );
    let unmanaged = args.unmanaged.is_some();
    let destination = args
        .unmanaged
        .or(args.install_dir)
        .or_else(|| std::env::var_os("CARGO_DIST_FORCE_INSTALL_DIR").map(PathBuf::from))
        .or_else(|| uv_dirs::user_executable_directory(None))
        .context("Could not determine the uv installation directory")?;
    let destination = std::path::absolute(destination)?;
    let executable = std::env::current_exe()?;
    fs_err::create_dir_all(&destination)?;
    let installation = LockedInstallation::acquire(&destination).await?;
    let existing_receipt = destination.join(RECEIPT_NAME);
    let existing_source = if existing_receipt.try_exists()? {
        Some(InstallReceipt::for_installation(&destination)?.source)
    } else {
        InstallReceipt::for_executable(&executable)
            .ok()
            .map(|(_, receipt)| receipt.source)
    };
    let source = executable
        .parent()
        .context("Executable has no parent directory")?;
    let modify_path = !unmanaged
        && !args.no_modify_path
        && std::env::var_os("INSTALLER_NO_MODIFY_PATH").is_none();
    let mut receipt = InstallReceipt::new(installation.directory.clone(), modify_path);
    if let Some(source) = existing_source {
        receipt.source = source;
    }
    installation.install_binaries(
        source,
        &receipt.binaries,
        (!unmanaged).then_some(&receipt),
        None,
    )?;
    writeln!(
        printer.stderr(),
        "Installed uv {} to {}",
        env!("CARGO_PKG_VERSION"),
        destination.simplified_display()
    )?;
    if modify_path {
        update_shell::update_shell(&destination, printer).await?;
    }
    Ok(ExitStatus::Success)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_install_restores_obsolete_binaries() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        fs_err::create_dir_all(&source)?;
        fs_err::create_dir_all(&destination)?;
        let executable = executable_names()[0];
        let companion = executable_names()[1];
        fs_err::write(source.join(executable), b"new uv")?;
        // A directory at the destination prevents committing the replacement executable.
        fs_err::create_dir(destination.join(executable))?;
        fs_err::write(destination.join(companion), b"old companion")?;
        let previous = InstallReceipt::new(destination.clone(), false);
        previous.write(&destination.join(RECEIPT_NAME))?;
        let before = fs_err::read(destination.join(RECEIPT_NAME))?;
        let mut updated = InstallReceipt::new(destination.clone(), false);
        updated.binaries = vec![executable.to_owned()];
        let installation = LockedInstallation::acquire(&destination).await?;
        assert!(
            installation
                .install_binaries(&source, &updated.binaries, Some(&updated), Some(&previous))
                .is_err()
        );
        assert_eq!(fs_err::read(destination.join(companion))?, b"old companion");
        assert_eq!(fs_err::read(destination.join(RECEIPT_NAME))?, before);
        assert!(destination.join(executable).is_dir());
        Ok(())
    }

    #[tokio::test]
    async fn obsolete_directories_are_not_removed() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        fs_err::create_dir_all(&source)?;
        fs_err::create_dir_all(&destination)?;
        let executable = executable_names()[0];
        let companion = destination.join(executable_names()[1]);
        fs_err::write(source.join(executable), b"new uv")?;
        fs_err::write(destination.join(executable), b"old uv")?;
        fs_err::create_dir(&companion)?;
        fs_err::write(companion.join("another-tool"), b"keep")?;
        let previous = InstallReceipt::new(destination.clone(), false);
        previous.write(&destination.join(RECEIPT_NAME))?;
        let before = fs_err::read(destination.join(RECEIPT_NAME))?;
        let mut updated = InstallReceipt::new(destination.clone(), false);
        updated.binaries = vec![executable.to_owned()];
        let installation = LockedInstallation::acquire(&destination).await?;
        assert!(
            installation
                .install_binaries(&source, &updated.binaries, Some(&updated), Some(&previous))
                .is_err()
        );
        assert_eq!(fs_err::read(destination.join(executable))?, b"old uv");
        assert_eq!(fs_err::read(companion.join("another-tool"))?, b"keep");
        assert_eq!(fs_err::read(destination.join(RECEIPT_NAME))?, before);
        Ok(())
    }
}
