use std::fmt::Write;

use anyhow::{Result, bail};
use itertools::{Either, Itertools};
use owo_colors::OwoColorize;
use tracing::debug;

use uv_fs::Simplified;
use uv_normalize::PackageName;
use uv_tool::{InstalledTools, Tool};

use uv_command_support::ExitStatus;
use uv_command_support::Printer;

/// Uninstall a tool.
pub async fn uninstall(name: Vec<PackageName>, printer: Printer) -> Result<ExitStatus> {
    let installed_tools = InstalledTools::from_settings()?.init()?;
    let _lock = match installed_tools.lock().await {
        Ok(lock) => lock,
        Err(err)
            if err
                .as_io_error()
                .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound) =>
        {
            if !name.is_empty() {
                for name in name {
                    writeln!(printer.stderr(), "`{name}` is not installed")?;
                }
                return Ok(ExitStatus::Success);
            }
            writeln!(printer.stderr(), "Nothing to uninstall")?;
            return Ok(ExitStatus::Success);
        }
        Err(err) => return Err(err.into()),
    };

    // Keep the root and its lock file in place. Processes already waiting on the lock must not
    // acquire a different lock from processes that start after this uninstallation.
    do_uninstall(&installed_tools, name, printer).await?;

    Ok(ExitStatus::Success)
}

/// Perform the uninstallation.
async fn do_uninstall(
    installed_tools: &InstalledTools,
    names: Vec<PackageName>,
    printer: Printer,
) -> Result<()> {
    // `--all` removes malformed environments; named tools only fall back for missing receipts.
    let tools = if names.is_empty() {
        Either::Left(
            installed_tools
                .tools()?
                .into_iter()
                .map(|(name, receipt)| Ok((name, receipt.ok()))),
        )
    } else {
        Either::Right(names.into_iter().map(|name| {
            installed_tools
                .get_tool_receipt(&name)
                .map(|receipt| (name, receipt))
        }))
    };
    let mut dangling = false;
    let mut executable_names = Vec::new();
    for tool in tools {
        let (name, receipt) = tool?;
        let Some(receipt) = receipt else {
            match installed_tools.remove_environment(&name) {
                Ok(()) => {
                    dangling = true;
                    writeln!(
                        printer.stderr(),
                        "Removed dangling environment for `{name}`"
                    )?;
                    continue;
                }
                Err(err)
                    if err
                        .as_io_error()
                        .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound) =>
                {
                    bail!("`{name}` is not installed");
                }
                Err(err) => return Err(err.into()),
            }
        };

        uninstall_tool(&name, &receipt, installed_tools).await?;
        executable_names.extend(
            receipt
                .into_entrypoints()
                .into_iter()
                .map(|entrypoint| entrypoint.name),
        );
    }
    executable_names.sort_unstable();

    if executable_names.is_empty() {
        // If we removed at least one dangling environment, there's no need to summarize.
        if !dangling {
            writeln!(printer.stderr(), "Nothing to uninstall")?;
        }
        return Ok(());
    }

    let s = if executable_names.len() == 1 { "" } else { "s" };
    writeln!(
        printer.stderr(),
        "Uninstalled {} executable{s}: {}",
        executable_names.len(),
        executable_names.iter().map(|name| name.bold()).join(", ")
    )?;

    Ok(())
}

/// Uninstall a tool.
async fn uninstall_tool(name: &PackageName, receipt: &Tool, tools: &InstalledTools) -> Result<()> {
    // Remove the tool itself.
    tools.remove_environment(name)?;

    #[cfg(windows)]
    let itself = std::env::current_exe().ok();

    // Remove the tool's entrypoints.
    for entrypoint in receipt.entrypoints() {
        debug!(
            "Removing executable: {}",
            entrypoint.install_path.user_display()
        );

        #[cfg(windows)]
        if itself.as_ref().is_some_and(|itself| {
            std::path::absolute(&entrypoint.install_path).is_ok_and(|target| *itself == target)
        }) {
            self_replace::self_delete()?;
            continue;
        }

        match fs_err::tokio::remove_file(&entrypoint.install_path).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                debug!(
                    "Executable not found: {}",
                    entrypoint.install_path.user_display()
                );
            }
            Err(err) => {
                return Err(err.into());
            }
        }
    }

    Ok(())
}
