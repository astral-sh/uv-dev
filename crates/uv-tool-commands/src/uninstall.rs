use std::collections::HashSet;
use std::fmt::Write;
use std::{
    io,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use itertools::Itertools;
use owo_colors::OwoColorize;
use tracing::debug;

use uv_fs::Simplified;
use uv_normalize::PackageName;
use uv_tool::{InstalledTools, Tool, ToolEntrypoint};

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
        let mut valid_tools = Vec::new();
        // Remove dangling environments before inspecting the ownership claims of healthy tools.
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

            valid_tools.push((name, receipt));
        }
        for (name, receipt) in valid_tools {
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
    let destinations = receipt
        .entrypoints()
        .iter()
        .map(|entrypoint| executable_destination(&entrypoint.install_path))
        .collect::<io::Result<Vec<_>>>()?;
    let removed_destinations = destinations.iter().collect::<HashSet<_>>();
    let mut retained_entrypoints = HashSet::new();
    for (tool_name, other_receipt) in tools.tools()? {
        if tool_name == *name {
            continue;
        }

        let other_receipt = other_receipt?;
        let tool_directory = tools.tool_dir(&tool_name);
        for entrypoint in other_receipt.entrypoints() {
            let destination = executable_destination(&entrypoint.install_path)?;
            if !removed_destinations.contains(&destination) {
                continue;
            }
            #[cfg(unix)]
            if !symlinked_entrypoint_matches(&tool_directory, entrypoint)? {
                continue;
            }

            #[cfg(windows)]
            if !copied_entrypoint_matches(&tool_directory, entrypoint)? {
                continue;
            }

            retained_entrypoints.insert(destination);
        }
    }

    // Remove the tool itself, after validating the other tool receipts.
    tools.remove_environment(name)?;

    #[cfg(windows)]
    let itself = std::env::current_exe().ok();

    // Remove the tool's entrypoints.
    let entrypoints = receipt.entrypoints();
    let mut removed_entrypoints = Vec::with_capacity(entrypoints.len());
    for (entrypoint, destination) in entrypoints.iter().zip(destinations) {
        if retained_entrypoints.contains(&destination) {
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

#[derive(Debug, PartialEq, Eq, Hash)]
enum ExecutableDestination {
    #[cfg(unix)]
    UnixEntry {
        device: u64,
        inode: u64,
    },
    Path(PathBuf),
}

/// Identify the destination directory without following the executable's own symlink.
fn executable_destination(path: &Path) -> io::Result<ExecutableDestination> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match fs_err::symlink_metadata(path) {
            Ok(metadata) => {
                return Ok(ExecutableDestination::UnixEntry {
                    device: metadata.dev(),
                    inode: metadata.ino(),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    // Windows launchers are regular files; canonicalization also normalizes filename casing.
    #[cfg(windows)]
    match fs_err::canonicalize(path) {
        Ok(destination) => return Ok(ExecutableDestination::Path(destination)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let (Some(parent), Some(filename)) = (path.parent(), path.file_name()) else {
        return Ok(ExecutableDestination::Path(path.to_path_buf()));
    };
    match fs_err::canonicalize(parent) {
        Ok(parent) => Ok(ExecutableDestination::Path(parent.join(filename))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(ExecutableDestination::Path(path.to_path_buf()))
        }
        Err(error) => Err(error),
    }
}

/// Unix launchers link to their environment's script, which may itself link into the cache.
#[cfg(unix)]
fn symlinked_entrypoint_matches(
    tool_directory: &Path,
    entrypoint: &ToolEntrypoint,
) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let target = match fs_err::read_link(&entrypoint.install_path) {
        Ok(target) => target,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
            ) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let Some(parent) = entrypoint.install_path.parent() else {
        return Ok(false);
    };
    let target = parent.join(target);
    let source = tool_directory.join("bin").join(&entrypoint.name);
    match fs_err::metadata(&source) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    }
    // Hardlinked scripts can share a cached inode while belonging to different environments.
    let Some(target_parent) = target.parent() else {
        return Ok(false);
    };
    let target_parent = match fs_err::metadata(target_parent) {
        Ok(parent) => parent,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let source_parent = fs_err::metadata(tool_directory.join("bin"))?;
    if target_parent.dev() != source_parent.dev() || target_parent.ino() != source_parent.ino() {
        return Ok(false);
    }
    Ok(executable_destination(&target)? == executable_destination(&source)?)
}

/// Windows entrypoints are copied from the owning environment's Scripts directory.
#[cfg(windows)]
fn copied_entrypoint_matches(
    tool_directory: &Path,
    entrypoint: &ToolEntrypoint,
) -> io::Result<bool> {
    let Some(filename) = entrypoint.install_path.file_name() else {
        return Ok(false);
    };
    let source = tool_directory.join("Scripts").join(filename);
    if !source.try_exists()? || !entrypoint.install_path.try_exists()? {
        return Ok(false);
    }
    // A Python launcher embeds its interpreter path, so a stale receipt from another
    // environment cannot claim a launcher copied by a later forced installation.
    Ok(fs_err::read(source)? == fs_err::read(&entrypoint.install_path)?)
}
