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

fn legacy_receipt_path() -> Result<PathBuf> {
    let directory = if std::env::var_os("AXOUPDATER_CONFIG_WORKING_DIR").is_some() {
        std::env::current_dir()?
    } else if let Some(path) = std::env::var_os("AXOUPDATER_CONFIG_PATH") {
        PathBuf::from(path)
    } else if let Some(path) = std::env::var_os(EnvVars::XDG_CONFIG_HOME)
        && Path::new(&path).is_absolute()
    {
        PathBuf::from(path).join("uv")
    } else {
        #[cfg(windows)]
        let directory = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .context("Could not determine the local application data directory")?;
        #[cfg(not(windows))]
        let directory = etcetera::home_dir()?.join(".config");
        directory.join("uv")
    };
    Ok(directory.join("uv-receipt.json"))
}

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

    fn for_executable(executable: &Path) -> Result<(PathBuf, Self)> {
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
        let receipt = Self::read(&path)
            .context("Self-management is only available for standalone uv installations")?;
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

    fn write(&self, path: &Path) -> Result<()> {
        fs_err::create_dir_all(path.parent().context("Receipt has no parent directory")?)?;
        uv_fs::write_atomic_sync(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

/// Copy a complete distribution before replacing any installed executable.
async fn install_binaries(source: &Path, destination: &Path) -> Result<()> {
    fs_err::create_dir_all(destination)?;
    let _lock = LockedFile::acquire(
        destination.join(".uv-install.lock"),
        LockedFileMode::Exclusive,
        "uv installation",
    )
    .await?;
    let staged = tempfile::tempdir_in(destination)?;
    for name in executable_names() {
        let source = source.join(name);
        anyhow::ensure!(
            fs_err::symlink_metadata(&source)?.is_file(),
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
        if uv_fs::is_same_file_allow_missing(&std::env::current_exe()?, &target) == Some(true) {
            self_replace::self_replace(&source)?;
            continue;
        }
        uv_fs::copy_atomic_sync(&source, &target)?;
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
    install_binaries(source, &destination).await?;
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
