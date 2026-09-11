use std::fmt::Write;

use anyhow::{Context, Result};
use uv_cli::SelfUninstallArgs;
use uv_fs::Simplified;
use uv_preview::PreviewFeature;

use super::self_install::{
    InstallReceipt, LockedInstallation, LockedLegacyReceipt, RECEIPT_NAME, executable_names,
    legacy_receipt_path,
};
use crate::commands::ExitStatus;
use crate::printer::Printer;

pub(crate) async fn self_uninstall(
    args: SelfUninstallArgs,
    printer: Printer,
) -> Result<ExitStatus> {
    let executable = fs_err::canonicalize(std::env::current_exe()?)?;
    anyhow::ensure!(
        uv_preview::is_enabled(PreviewFeature::SelfManagement)
            || executable.with_file_name(RECEIPT_NAME).try_exists()?,
        "Native self-uninstallation is experimental; pass `--preview-features self-management` to enable it"
    );
    let _installation = LockedInstallation::acquire(
        executable
            .parent()
            .context("Executable has no parent directory")?,
    )
    .await?;
    let legacy = legacy_receipt_path()?;
    let legacy_receipt = LockedLegacyReceipt::acquire_if_exists(&legacy).await?;
    let (receipt_path, receipt) = InstallReceipt::for_executable(&executable)?;
    anyhow::ensure!(
        receipt_path != legacy || legacy_receipt.is_some(),
        "The legacy install receipt changed; run `uv self uninstall` again"
    );
    let binaries = receipt.owned_binaries()?;
    if args.dry_run {
        writeln!(
            printer.stderr_important(),
            "Would uninstall uv from {}",
            receipt.install_prefix.simplified_display()
        )?;
        return Ok(ExitStatus::Success);
    }

    for name in binaries
        .iter()
        .filter(|name| *name != executable_names()[0])
    {
        let path = receipt.install_prefix.join(name);
        match fs_err::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to remove `{}`", path.display()));
            }
        }
    }
    #[cfg(windows)]
    self_replace::self_delete()?;
    #[cfg(not(windows))]
    fs_err::remove_file(&executable)?;
    if receipt_path != legacy {
        fs_err::remove_file(receipt_path)?;
    }
    if let Some(legacy_receipt) = legacy_receipt {
        legacy_receipt.remove_if_owned(&executable)?;
    }
    writeln!(
        printer.stderr(),
        "Uninstalled uv from {}",
        receipt.install_prefix.simplified_display()
    )?;
    Ok(ExitStatus::Success)
}
