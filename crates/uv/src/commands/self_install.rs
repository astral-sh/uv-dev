use std::fmt::Write;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uv_cli::SelfInstallArgs;
use uv_fs::{LockedFile, LockedFileMode, Simplified};
use uv_static::EnvVars;

use crate::commands::ExitStatus;
#[cfg(windows)]
use crate::commands::update_shell;
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

impl ReleaseSource {
    fn from_repository(repository: &str) -> Result<Self> {
        let (owner, name) = repository
            .split_once('/')
            .context("Release repository must be OWNER/REPOSITORY")?;
        anyhow::ensure!(
            !owner.is_empty()
                && owner
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && !matches!(name, "" | "." | "..")
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)),
            "Invalid release repository `{repository}`"
        );
        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
            ..Self::default()
        })
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

pub(super) fn legacy_receipt_path() -> Result<PathBuf> {
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
    pub(super) fn owned_binaries(&self) -> Result<Vec<String>> {
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

    fn owns_executable(&self, executable: &Path) -> bool {
        let name = executable_names()[0];
        uv_fs::is_same_file_allow_missing(&self.install_prefix.join(name), executable) == Some(true)
            || (self.provider.source == "cargo-dist"
                && uv_fs::is_same_file_allow_missing(
                    &self.install_prefix.join("bin").join(name),
                    executable,
                ) == Some(true))
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

    async fn write_legacy(&self, path: &Path, executable: &Path) -> Result<LockedLegacyReceipt> {
        let lock = LockedLegacyReceipt::acquire(path).await?;
        if path.try_exists()? {
            anyhow::ensure!(
                Self::read(path)?.owns_executable(executable),
                "Cannot downgrade uv: the legacy install receipt at `{}` belongs to another installation",
                path.display()
            );
        }
        self.write(path)?;
        Ok(lock)
    }
}

/// Serialize changes to the receipt shared by legacy standalone installations.
pub(super) struct LockedLegacyReceipt {
    path: PathBuf,
    _lock: LockedFile,
}

impl LockedLegacyReceipt {
    async fn acquire(path: &Path) -> Result<Self> {
        fs_err::create_dir_all(path.parent().context("Receipt has no parent directory")?)?;
        let lock = LockedFile::acquire(
            path.with_extension("lock"),
            LockedFileMode::Exclusive,
            "legacy uv installation receipt",
        )
        .await?;
        Ok(Self {
            path: path.to_path_buf(),
            _lock: lock,
        })
    }

    pub(super) async fn acquire_if_exists(path: &Path) -> Result<Option<Self>> {
        if path.try_exists()? {
            Ok(Some(Self::acquire(path).await?))
        } else {
            Ok(None)
        }
    }

    pub(super) fn remove_if_owned(&self, executable: &Path) -> Result<()> {
        if InstallReceipt::read(&self.path).is_ok_and(|receipt| receipt.owns_executable(executable))
        {
            fs_err::remove_file(&self.path)?;
        }
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
    pub(super) async fn install_binaries(
        &self,
        source: &Path,
        binaries: &[String],
        receipt: Option<&InstallReceipt>,
        previous: Option<&InstallReceipt>,
        legacy_receipt: Option<&Path>,
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
        let legacy_to_remove = if receipt.is_none()
            && let Ok(path) = legacy_receipt_path()
        {
            LockedLegacyReceipt::acquire_if_exists(&path).await?
        } else {
            None
        };
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
        let result: Result<()> = async {
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
            // Make an older executable self-updatable before replacing the current binary. A
            // legacy receipt for another installation must never be overwritten by a downgrade.
            let _legacy_lock = if let Some(path) = legacy_receipt {
                Some(
                    receipt
                        .context("Legacy self-management requires an install receipt")?
                        .write_legacy(path, &destination.join(executable_names()[0]))
                        .await?,
                )
            } else {
                None
            };
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
        }
        .await;
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
        if let Some(legacy) = legacy_to_remove {
            legacy.remove_if_owned(&destination.join(executable_names()[0]))?;
        }
        Ok(())
    }
}

/// Make the installation available to subsequent GitHub Actions steps.
fn update_ci_path(directory: &Path, path: &Path) -> Result<()> {
    let mut file = fs_err::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    writeln!(file, "{}", directory.display())?;
    Ok(())
}

pub(crate) async fn self_install(args: SelfInstallArgs, printer: Printer) -> Result<ExitStatus> {
    let unmanaged = args.unmanaged.is_some();
    let destination = args
        .unmanaged
        .or(args.install_dir)
        .or_else(|| std::env::var_os("CARGO_DIST_FORCE_INSTALL_DIR").map(PathBuf::from))
        .or_else(|| uv_dirs::user_executable_directory(None))
        .context("Could not determine the uv installation directory")?;
    let mut destination = std::path::absolute(destination)?;
    // Older standalone updaters pass a Cargo-home prefix rather than its executable directory.
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| etcetera::home_dir().ok().map(|home| home.join(".cargo")));
    if let Some(cargo_home) = cargo_home
        && std::path::absolute(cargo_home)? == destination
    {
        destination.push("bin");
    }
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
        && (std::env::var_os(EnvVars::UV_NO_MODIFY_PATH).is_some()
            || !uv_static::parse_boolish_environment_variable("INSTALLER_NO_MODIFY_PATH")?
                .unwrap_or(false));
    let mut receipt = InstallReceipt::new(installation.directory.clone(), modify_path);
    if let Some(repository) = args.source_repository {
        receipt.source = ReleaseSource::from_repository(&repository)?;
    } else if let Some(source) = existing_source {
        receipt.source = source;
    }
    installation
        .install_binaries(
            source,
            &receipt.binaries,
            (!unmanaged && !args.no_update).then_some(&receipt),
            None,
            None,
        )
        .await?;
    writeln!(
        printer.stderr(),
        "Installed uv {} to {}",
        env!("CARGO_PKG_VERSION"),
        destination.simplified_display()
    )?;
    if modify_path {
        if let Some(path) = std::env::var_os("GITHUB_PATH").filter(|path| !path.is_empty()) {
            update_ci_path(&destination, Path::new(&path))?;
        }
        #[cfg(windows)]
        update_shell::update_shell(&destination, printer).await?;
        #[cfg(unix)]
        super::self_install_shell::update_shell(&destination, printer)?;
    }
    Ok(ExitStatus::Success)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_compatibility_receipt_cannot_replace_another_installation() {
        let temporary = tempfile::tempdir().unwrap();
        let first = temporary.path().join("first");
        let second = temporary.path().join("second");
        fs_err::create_dir_all(&first).unwrap();
        fs_err::create_dir_all(&second).unwrap();
        let first_executable = first.join(executable_names()[0]);
        let second_executable = second.join(executable_names()[0]);
        fs_err::write(&first_executable, b"first").unwrap();
        fs_err::write(&second_executable, b"second").unwrap();
        let path = temporary.path().join("legacy/uv-receipt.json");
        let receipt = InstallReceipt::new(first, false);
        drop(
            receipt
                .write_legacy(&path, &first_executable)
                .await
                .unwrap(),
        );
        drop(
            receipt
                .write_legacy(&path, &first_executable)
                .await
                .unwrap(),
        );
        let before = fs_err::read(&path).unwrap();
        let other = InstallReceipt::new(second, true);
        assert!(other.write_legacy(&path, &second_executable).await.is_err());
        assert_eq!(fs_err::read(&path).unwrap(), before);
    }

    #[test]
    fn appends_install_directory_to_github_path() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let path = temporary.path().join("github-path");
        let directory = temporary.path().join("bin");
        fs_err::write(&path, "another-directory\n")?;
        update_ci_path(&directory, &path)?;
        assert_eq!(
            fs_err::read_to_string(path)?,
            format!("another-directory\n{}\n", directory.display())
        );
        Ok(())
    }

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
                .install_binaries(
                    &source,
                    &updated.binaries,
                    Some(&updated),
                    Some(&previous),
                    None
                )
                .await
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
        let legacy = temporary.path().join("legacy/uv-receipt.json");
        previous.write(&legacy)?;
        let legacy_before = fs_err::read(&legacy)?;
        let mut updated = InstallReceipt::new(destination.clone(), false);
        updated.binaries = vec![executable.to_owned()];
        let installation = LockedInstallation::acquire(&destination).await?;
        assert!(
            installation
                .install_binaries(
                    &source,
                    &updated.binaries,
                    Some(&updated),
                    Some(&previous),
                    Some(&legacy)
                )
                .await
                .is_err()
        );
        assert_eq!(fs_err::read(destination.join(executable))?, b"old uv");
        assert_eq!(fs_err::read(companion.join("another-tool"))?, b"keep");
        assert_eq!(fs_err::read(destination.join(RECEIPT_NAME))?, before);
        assert_eq!(fs_err::read(legacy)?, legacy_before);
        Ok(())
    }

    #[tokio::test]
    async fn conflicting_legacy_receipt_restores_obsolete_binaries() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        let foreign = temporary.path().join("foreign");
        fs_err::create_dir_all(&source)?;
        fs_err::create_dir_all(&destination)?;
        fs_err::create_dir_all(&foreign)?;
        let executable = executable_names()[0];
        let companion = executable_names()[1];
        fs_err::write(source.join(executable), b"new uv")?;
        fs_err::write(destination.join(executable), b"old uv")?;
        fs_err::write(destination.join(companion), b"old companion")?;
        fs_err::write(foreign.join(executable), b"another uv")?;
        let legacy = temporary.path().join("legacy/uv-receipt.json");
        InstallReceipt::new(foreign, false).write(&legacy)?;
        let legacy_before = fs_err::read(&legacy)?;
        let previous = InstallReceipt::new(destination.clone(), false);
        previous.write(&destination.join(RECEIPT_NAME))?;
        let before = fs_err::read(destination.join(RECEIPT_NAME))?;
        let mut updated = InstallReceipt::new(destination.clone(), false);
        updated.binaries = vec![executable.to_owned()];
        let installation = LockedInstallation::acquire(&destination).await?;
        assert!(
            installation
                .install_binaries(
                    &source,
                    &updated.binaries,
                    Some(&updated),
                    Some(&previous),
                    Some(&legacy)
                )
                .await
                .is_err()
        );
        assert_eq!(fs_err::read(destination.join(executable))?, b"old uv");
        assert_eq!(fs_err::read(destination.join(companion))?, b"old companion");
        assert_eq!(fs_err::read(destination.join(RECEIPT_NAME))?, before);
        assert_eq!(fs_err::read(legacy)?, legacy_before);
        Ok(())
    }
}
