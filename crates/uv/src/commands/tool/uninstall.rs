use std::collections::BTreeMap;
use std::fmt::Write;
#[cfg(windows)]
use std::io::Read;
use std::path::Path;

use anyhow::{Result, bail};
use itertools::Itertools;
use owo_colors::OwoColorize;
use tracing::debug;

use uv_fs::Simplified;
use uv_normalize::PackageName;
use uv_tool::{InstalledTools, Tool, ToolEntrypoint};
#[cfg(windows)]
use uv_trampoline_builder::{Launcher, LauncherKind};

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
    // Determine ownership before removing any environment. In particular, copied native
    // executables can have identical contents in multiple tools, so their receipts alone cannot
    // identify the last tool to install them with `--force`.
    let all_tools = installed_tools.tools()?;
    let checks_ownership = all_tools
        .iter()
        .any(|(name, receipt)| receipt.is_ok() && (names.is_empty() || names.contains(name)));
    let mut receipts = Vec::new();
    for (name, receipt) in all_tools {
        match receipt {
            Ok(receipt) => receipts.push((name, receipt)),
            // Missing selected receipts have only a dangling-environment cleanup plan. With
            // `--all`, malformed receipts receive the same cleanup. Neither claims an executable.
            Err(uv_tool::Error::MissingToolReceipt(..)) if names.contains(&name) => {}
            Err(_) if names.is_empty() => {}
            Err(_) if !checks_ownership => {}
            Err(err) => return Err(err.into()),
        }
    }
    receipts.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    let mut planned_entrypoints = BTreeMap::new();
    if names.is_empty() {
        for (name, receipt) in &receipts {
            let entrypoints = owned_entrypoints(name, receipt, &receipts, installed_tools)?;
            planned_entrypoints.insert(name.clone(), entrypoints);
        }
    } else {
        for name in &names {
            if let Some(receipt) = installed_tools.get_tool_receipt(name)? {
                let entrypoints = owned_entrypoints(name, &receipt, &receipts, installed_tools)?;
                planned_entrypoints.insert(name.clone(), entrypoints);
            }
        }
    }

    let mut removed_environment = false;
    let mut entrypoints = if names.is_empty() {
        let mut entrypoints = vec![];
        for (name, receipt) in installed_tools.tools()? {
            let Ok(_receipt) = receipt else {
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

            let Some(planned) = planned_entrypoints.get(&name) else {
                bail!("Missing executable ownership plan for `{name}`");
            };
            let removed_entrypoints = uninstall_tool(&name, planned, installed_tools).await?;
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
            let Some(_receipt) = installed_tools.get_tool_receipt(&name)? else {
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

            let Some(planned) = planned_entrypoints.get(&name) else {
                bail!("Missing executable ownership plan for `{name}`");
            };
            let removed_entrypoints = uninstall_tool(&name, planned, installed_tools).await?;
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

/// Identify the exported executables that still belong to this tool.
pub(super) fn owned_entrypoints(
    name: &PackageName,
    receipt: &Tool,
    receipts: &[(PackageName, Tool)],
    tools: &InstalledTools,
) -> Result<Vec<ToolEntrypoint>> {
    let mut owned = Vec::new();
    for entrypoint in receipt.entrypoints() {
        let mut owner = None;
        for (tool_name, other_receipt) in receipts {
            let mut matches_export = false;
            for other in other_receipt.entrypoints() {
                if same_install_path(&other.install_path, &entrypoint.install_path)?
                    && entrypoint_matches(other, &tools.tool_dir(tool_name))?
                {
                    matches_export = true;
                    break;
                }
            }
            if !matches_export {
                continue;
            }
            if let Some(previous) = owner.replace(tool_name) {
                bail!(
                    "Cannot determine whether executable `{}` belongs to `{previous}` or `{tool_name}`; no tools were removed",
                    entrypoint.install_path.user_display()
                );
            }
        }
        if owner == Some(name) {
            owned.push(entrypoint.clone());
        } else {
            debug!(
                "Retaining executable not owned by `{name}`: {}",
                entrypoint.install_path.user_display()
            );
        }
    }
    Ok(owned)
}

/// Compare receipt paths without confusing differently cased Windows copies with unique owners.
fn same_install_path(left: &Path, right: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        Ok(left == right)
    }
    #[cfg(windows)]
    {
        if left == right {
            return Ok(true);
        }
        for path in [left, right] {
            match fs_err::metadata(path) {
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(err) => return Err(err.into()),
            }
        }
        if let Some(same) = uv_fs::is_same_file_allow_missing(left, right) {
            Ok(same)
        } else {
            bail!(
                "Cannot compare executable ownership paths `{}` and `{}`",
                left.user_display(),
                right.user_display()
            )
        }
    }
}

/// Match an export to the conventional scripts directory of one installed tool.
///
/// Unix exports must be symlinks to the exact script. Windows exports must be regular copies of
/// the exact script. A uv script launcher must also name this environment's Python executable;
/// other copied executables are supported only when the caller finds a unique matching receipt.
fn entrypoint_matches(entrypoint: &ToolEntrypoint, tool_directory: &Path) -> Result<bool> {
    let Some(filename) = entrypoint.install_path.file_name() else {
        return Ok(false);
    };
    let scripts = tool_directory.join(if cfg!(windows) { "Scripts" } else { "bin" });
    let source = scripts.join(filename);
    let metadata = match fs_err::symlink_metadata(&entrypoint.install_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };

    #[cfg(unix)]
    {
        if !metadata.is_symlink() {
            return Ok(false);
        }
        let target = match fs_err::canonicalize(&entrypoint.install_path) {
            Ok(target) => target,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.into()),
        };
        let source = match fs_err::canonicalize(source) {
            Ok(source) => source,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.into()),
        };
        Ok(target == source
            && source.starts_with(fs_err::canonicalize(tool_directory)?)
            && fs_err::metadata(source)?.is_file())
    }

    #[cfg(windows)]
    {
        if !metadata.is_file() || metadata.is_symlink() {
            return Ok(false);
        }
        let source_metadata = match fs_err::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.into()),
        };
        if !source_metadata.is_file()
            || source_metadata.is_symlink()
            || metadata.len() != source_metadata.len()
        {
            return Ok(false);
        }
        if let Some(launcher) = Launcher::try_from_path(&entrypoint.install_path)? {
            match launcher.kind {
                LauncherKind::Script => {}
                LauncherKind::Python => return Ok(false),
            }
            if !launcher.python_path.is_absolute() {
                return Ok(false);
            }
            let python = match launcher.python_path.file_name() {
                Some(filename) if filename == "python.exe" || filename == "pythonw.exe" => {
                    scripts.join(filename)
                }
                _ => return Ok(false),
            };
            let expected = match dunce::canonicalize(python) {
                Ok(expected) => expected,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(err) => return Err(err.into()),
            };
            let actual = match dunce::canonicalize(&launcher.python_path) {
                Ok(actual) => actual,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(err) => return Err(err.into()),
            };
            if actual != expected {
                return Ok(false);
            }
        }
        let mut installed = fs_err::File::open(&entrypoint.install_path)?;
        let mut source = fs_err::File::open(source)?;
        let mut installed_buffer = [0; 8192];
        let mut source_buffer = [0; 8192];
        loop {
            let count = installed.read(&mut installed_buffer)?;
            source.read_exact(&mut source_buffer[..count])?;
            if installed_buffer[..count] != source_buffer[..count] {
                return Ok(false);
            }
            if count == 0 {
                return Ok(source.read(&mut source_buffer)? == 0);
            }
        }
    }
}

/// Uninstall a tool after its executable ownership has been checked.
async fn uninstall_tool(
    name: &PackageName,
    entrypoints: &[ToolEntrypoint],
    tools: &InstalledTools,
) -> Result<Vec<ToolEntrypoint>> {
    // Remove the tool itself, after validating the other tool receipts.
    tools.remove_environment(name)?;

    #[cfg(windows)]
    let itself = std::env::current_exe().ok();

    // Remove the tool's entrypoints.
    let mut removed_entrypoints = Vec::with_capacity(entrypoints.len());
    for entrypoint in entrypoints {
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
