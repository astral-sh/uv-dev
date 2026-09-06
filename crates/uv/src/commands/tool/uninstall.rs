use std::collections::HashSet;
use std::fmt::Write;

use anyhow::{Result, bail};
use itertools::Itertools;
use owo_colors::OwoColorize;
use tracing::debug;

use uv_fs::Simplified;
use uv_normalize::PackageName;
use uv_tool::{InstalledTools, Tool, ToolEntrypoint};

use crate::commands::ExitStatus;
use crate::printer::Printer;

/// Uninstall a tool.
pub(crate) async fn uninstall(name: Vec<PackageName>, printer: Printer) -> Result<ExitStatus> {
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

    // Perform the uninstallation.
    do_uninstall(&installed_tools, name, printer).await?;

    // Clean up any empty directories.
    if uv_fs::directories(installed_tools.root())?.all(|path| uv_fs::is_temporary(&path)) {
        fs_err::tokio::remove_dir_all(&installed_tools.root())
            .await
            .ignore_currently_being_deleted()?;
        if let Some(parent) = installed_tools.root().parent() {
            if uv_fs::directories(parent)?.all(|path| uv_fs::is_temporary(&path)) {
                fs_err::tokio::remove_dir_all(parent)
                    .await
                    .ignore_currently_being_deleted()?;
            }
        }
    }

    Ok(ExitStatus::Success)
}

trait IoErrorExt: std::error::Error + 'static {
    #[inline]
    fn is_in_process_of_being_deleted(&self) -> bool {
        if cfg!(target_os = "windows") {
            use std::error::Error;
            let mut e: &dyn Error = &self;
            loop {
                if e.to_string().contains("The file cannot be opened because it is in the process of being deleted. (os error 303)") {
                    return true;
                }
                e = match e.source() {
                    Some(e) => e,
                    None => break,
                }
            }
        }

        false
    }
}

impl IoErrorExt for std::io::Error {}

/// An extension trait to suppress "cannot open file because it's currently being deleted"
trait IgnoreCurrentlyBeingDeleted {
    fn ignore_currently_being_deleted(self) -> Self;
}

impl IgnoreCurrentlyBeingDeleted for Result<(), std::io::Error> {
    fn ignore_currently_being_deleted(self) -> Self {
        match self {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
            Err(err) if err.is_in_process_of_being_deleted() => Ok(()),
            Err(err) => Err(err),
        }
    }
}

/// Perform the uninstallation.
async fn do_uninstall(
    installed_tools: &InstalledTools,
    names: Vec<PackageName>,
    printer: Printer,
) -> Result<()> {
    let mut removed_environment = false;
    let mut entrypoints = if names.is_empty() {
        let mut entrypoints = vec![];
        for (name, receipt) in installed_tools.tools()? {
            let Ok(receipt) = receipt else {
                // If the tool is not installed properly, attempt to remove the environment anyway.
                match installed_tools.remove_environment(&name) {
                    Ok(()) => {
                        removed_environment = true;
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
                    Err(err) => {
                        return Err(err.into());
                    }
                }
            };

            let removed_entrypoints = uninstall_tool(&name, &receipt, installed_tools).await?;
            if removed_entrypoints.is_empty() {
                removed_environment = true;
                writeln!(printer.stderr(), "Removed environment for `{name}`")?;
            }
            entrypoints.extend(removed_entrypoints);
        }
        entrypoints
    } else {
        let mut entrypoints = vec![];
        for name in names {
            let Some(receipt) = installed_tools.get_tool_receipt(&name)? else {
                // If the tool is not installed properly, attempt to remove the environment anyway.
                match installed_tools.remove_environment(&name) {
                    Ok(()) => {
                        removed_environment = true;
                        writeln!(
                            printer.stderr(),
                            "Removed dangling environment for `{name}`"
                        )?;
                        continue;
                    }
                    Err(uv_tool::Error::VirtualEnvError(uv_virtualenv::Error::Io(err)))
                        if err.kind() == std::io::ErrorKind::NotFound =>
                    {
                        bail!("`{name}` is not installed");
                    }
                    Err(err) => {
                        return Err(err.into());
                    }
                }
            };

            let removed_entrypoints = uninstall_tool(&name, &receipt, installed_tools).await?;
            if removed_entrypoints.is_empty() {
                removed_environment = true;
                writeln!(printer.stderr(), "Removed environment for `{name}`")?;
            }
            entrypoints.extend(removed_entrypoints);
        }
        entrypoints
    };
    entrypoints.sort_unstable_by(|a, b| a.name.cmp(&b.name));

    if entrypoints.is_empty() {
        // If we removed at least one environment without executables, there's no need to summarize.
        if !removed_environment {
            writeln!(printer.stderr(), "Nothing to uninstall")?;
        }
        return Ok(());
    }

    let s = if entrypoints.len() == 1 { "" } else { "s" };
    writeln!(
        printer.stderr(),
        "Uninstalled {} executable{s}: {}",
        entrypoints.len(),
        entrypoints
            .iter()
            .map(|entrypoint| entrypoint.name.bold())
            .join(", ")
    )?;

    Ok(())
}

/// Uninstall a tool.
async fn uninstall_tool(
    name: &PackageName,
    receipt: &Tool,
    tools: &InstalledTools,
) -> Result<Vec<ToolEntrypoint>> {
    let mut retained_entrypoints = HashSet::new();
    for (tool_name, other_receipt) in tools.tools()? {
        if tool_name == *name {
            continue;
        }

        let other_receipt = other_receipt?;
        #[cfg(unix)]
        let tool_directory = tools.tool_dir(&tool_name);
        for entrypoint in other_receipt.entrypoints() {
            #[cfg(unix)]
            if !fs_err::canonicalize(&entrypoint.install_path)
                .is_ok_and(|target| target.starts_with(&tool_directory))
            {
                continue;
            }

            retained_entrypoints.insert(entrypoint.install_path.clone());
        }
    }

    // Remove the tool itself, after validating the other tool receipts.
    tools.remove_environment(name)?;

    #[cfg(windows)]
    let itself = std::env::current_exe().ok();

    // Remove the tool's entrypoints.
    let entrypoints = receipt.entrypoints();
    let mut removed_entrypoints = Vec::with_capacity(entrypoints.len());
    for entrypoint in entrypoints {
        if retained_entrypoints.contains(&entrypoint.install_path) {
            debug!(
                "Retaining executable claimed by another tool: {}",
                entrypoint.install_path.user_display()
            );
            continue;
        }

        debug!(
            "Removing executable: {}",
            entrypoint.install_path.user_display()
        );

        #[cfg(windows)]
        if itself.as_ref().is_some_and(|itself| {
            std::path::absolute(&entrypoint.install_path).is_ok_and(|target| *itself == target)
        }) {
            self_replace::self_delete()?;
            removed_entrypoints.push(entrypoint.clone());
            continue;
        }

        match fs_err::tokio::remove_file(&entrypoint.install_path).await {
            Ok(()) => removed_entrypoints.push(entrypoint.clone()),
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

    Ok(removed_entrypoints)
}
