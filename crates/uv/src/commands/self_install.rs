use std::fmt::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uv_cli::SelfInstallArgs;
use uv_command_support::{ExitStatus, Printer, update_shell};
use uv_fs::{LockedFile, LockedFileMode, Simplified};
use uv_preview::PreviewFeature;

use super::self_receipt::find_receipt_path;

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

#[derive(Debug, Deserialize, Serialize)]
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

const RECEIPT_NAME: &str = ".uv-receipt.json";

fn executable_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["uv.exe", "uvx.exe", "uvw.exe"]
    } else {
        &["uv", "uvx"]
    }
}

impl InstallReceipt {
    fn new(install_prefix: PathBuf, modify_path: bool) -> Self {
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
        let recorded = receipt.normalized_install_prefix(&directory);
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

    fn for_executable(executable: &Path) -> Result<(PathBuf, Self)> {
        let executable = fs_err::canonicalize(executable)?;
        let directory = executable
            .parent()
            .context("Executable has no parent directory")?;
        let native = directory.join(RECEIPT_NAME);
        let path = if native.try_exists()? {
            native
        } else {
            find_receipt_path("uv")?
                .context("Self-management is only available for standalone uv installations")?
        };
        let mut receipt = Self::read(&path)
            .context("Self-management is only available for standalone uv installations")?;
        receipt.install_prefix = receipt.normalized_install_prefix(directory);
        let recorded = receipt
            .install_prefix
            .join(format!("uv{}", std::env::consts::EXE_SUFFIX));
        anyhow::ensure!(
            uv_fs::is_same_file_allow_missing(&recorded, &executable) == Some(true),
            "The install receipt at `{}` belongs to a different uv executable at `{}`",
            path.display(),
            recorded.display()
        );
        anyhow::ensure!(
            receipt.source.app_name == "uv",
            "The install receipt is not for uv"
        );
        Ok((path, receipt))
    }

    /// Cargo-dist receipts can name the prefix above the executable's `bin` directory.
    fn normalized_install_prefix(&self, directory: &Path) -> PathBuf {
        let mut prefix = self.install_prefix.clone();
        if uv_fs::is_same_file_allow_missing(&prefix, directory) != Some(true)
            && self.provider.source == "cargo-dist"
        {
            prefix.push("bin");
        }
        prefix
    }

    fn write(&self, path: &Path) -> Result<()> {
        fs_err::create_dir_all(path.parent().context("Receipt has no parent directory")?)?;
        uv_fs::write_atomic_sync(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(windows)]
const BACKUP_PREFIX: &str = ".uv-install-backup-";
#[cfg(windows)]
const BACKUP_OWNER: &str = "uv-self-install-v1\n";

/// Remove owned backups once their executable mappings have been released.
#[cfg(windows)]
fn cleanup_previous_installations(destination: &Path) {
    let Ok(entries) = fs_err::read_dir(destination) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(BACKUP_PREFIX)
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        let path = entry.path();
        if fs_err::read_to_string(path.join("owner")).ok().as_deref() != Some(BACKUP_OWNER) {
            continue;
        }
        let Ok(contents) =
            fs_err::read_dir(&path).and_then(Iterator::collect::<std::io::Result<Vec<_>>>)
        else {
            continue;
        };
        if !contents.iter().all(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && (entry.file_name() == "owner"
                    || executable_names()
                        .iter()
                        .any(|name| entry.file_name() == *name))
        }) {
            continue;
        }
        let mut pending = false;
        for entry in contents.iter().filter(|entry| entry.file_name() != "owner") {
            if let Err(error) = fs_err::remove_file(entry.path())
                && error.kind() != std::io::ErrorKind::NotFound
            {
                pending = true;
                tracing::debug!(
                    "Retaining executable backup at `{}`: {error}",
                    path.display()
                );
            }
        }
        // Retain the ownership marker until every old executable can be removed.
        if !pending && fs_err::remove_file(path.join("owner")).is_ok() {
            let _ = fs_err::remove_dir(&path);
        }
    }
}

#[cfg(windows)]
fn replace_windows_binary(source: &Path, target: &Path) -> Result<()> {
    if let Err(error) = fs_err::rename(source, target) {
        if !fs_err::symlink_metadata(target).is_ok_and(|metadata| metadata.is_file()) {
            return Err(error.into());
        }
        let backup = tempfile::Builder::new().prefix(BACKUP_PREFIX).tempdir_in(
            target
                .parent()
                .context("Executable has no parent directory")?,
        )?;
        fs_err::write(backup.path().join("owner"), BACKUP_OWNER)?;
        let previous = backup
            .path()
            .join(target.file_name().context("Executable has no filename")?);
        // A running uvw launcher waits for this child, so retries cannot release its mapping.
        // Keep the old executable in an owned directory for cleanup on a later installation.
        fs_err::rename(target, &previous)?;
        if let Err(error) = fs_err::rename(source, target) {
            fs_err::rename(&previous, target)
                .context("Failed to restore the previous executable")?;
            return Err(error.into());
        }
        let _ = backup.keep();
    }
    Ok(())
}

/// Copy a complete distribution before replacing any installed executable.
fn install_binaries(executable: &Path, destination: &Path) -> Result<()> {
    let source = executable
        .parent()
        .context("Executable has no parent directory")?;
    let uv_name = format!("uv{}", std::env::consts::EXE_SUFFIX);
    anyhow::ensure!(
        uv_fs::is_same_file_allow_missing(&source.join(&uv_name), executable) == Some(true),
        "Cannot install from `{}`: `{uv_name}` does not identify the running executable `{}`",
        source.display(),
        executable.display()
    );
    #[cfg(windows)]
    cleanup_previous_installations(destination);
    let staged = tempfile::tempdir_in(destination)?;
    for name in executable_names() {
        let source = source.join(name);
        anyhow::ensure!(
            fs_err::metadata(&source)?.is_file(),
            "Expected a regular executable at `{}`",
            source.display()
        );
        fs_err::copy(&source, staged.path().join(name))?;
    }
    // Install the launcher last so it cannot select an incompletely copied uv binary.
    for name in executable_names() {
        let source = staged.path().join(name);
        let target = destination.join(name);
        #[cfg(windows)]
        if uv_fs::is_same_file_allow_missing(executable, &target) == Some(true) {
            self_replace::self_replace(&source)?;
            continue;
        }
        #[cfg(windows)]
        replace_windows_binary(&source, &target)?;
        // Keep renames synchronous under the installation lock so cancellation cannot leave a
        // queued filesystem operation running after its guard is released.
        #[cfg(not(windows))]
        uv_fs::with_retry_sync(&source, &target, "renaming", || {
            fs_err::rename(&source, &target)
        })?;
    }
    Ok(())
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
    let lock = LockedFile::acquire(
        destination.join(".uv-install.lock"),
        LockedFileMode::Exclusive,
        "uv installation",
    )
    .await?;
    let existing_receipt = destination.join(RECEIPT_NAME);
    let existing_source = if existing_receipt.try_exists()? {
        Some(InstallReceipt::for_installation(&destination)?.source)
    } else {
        InstallReceipt::for_executable(&executable)
            .ok()
            .map(|(_, receipt)| receipt.source)
    };
    install_binaries(&executable, &destination)?;
    let modify_path = !unmanaged
        && !args.no_modify_path
        && std::env::var_os("INSTALLER_NO_MODIFY_PATH").is_none();
    if !unmanaged {
        let mut receipt = InstallReceipt::new(fs_err::canonicalize(&destination)?, modify_path);
        if let Some(source) = existing_source {
            receipt.source = source;
        }
        receipt.write(&destination.join(RECEIPT_NAME))?;
    }
    drop(lock);
    writeln!(
        printer.stderr(),
        "Installed uv {} to {}",
        env!("CARGO_PKG_VERSION"),
        destination.simplified_display()
    )?;
    if modify_path
        && let update_shell::ShellUpdate::AlreadyConfigured(_) =
            update_shell::configure_shell(&destination, printer).await?
    {
        writeln!(printer.stderr(), "Restart your shell to apply changes")?;
    }
    Ok(ExitStatus::Success)
}
