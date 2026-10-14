use std::collections::BTreeSet;
use std::fmt::Write;
use std::io;
use std::path::PathBuf;

use anyhow::Result;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use indexmap::IndexSet;
use itertools::Itertools;
use owo_colors::OwoColorize;
use rustc_hash::{FxHashMap, FxHashSet};
use tracing::{debug, warn};

use uv_command_support::Printer;
use uv_command_support::{ExitStatus, elapsed};
use uv_fs::Simplified;
use uv_python_managed::{
    ManagedPythonInstallation, ManagedPythonInstallations, PythonMinorVersionLink,
    python_executable_dir,
};
use uv_python_types::{
    PythonDownloadRequest, PythonInstallationKey, PythonInstallationMinorVersionKey, PythonRequest,
};

use crate::install::format_executables;

#[derive(Debug, thiserror::Error)]
#[error("Failed to remove symlink directory `{}`", path.display())]
struct MinorVersionLinkRemovalError {
    path: PathBuf,
    #[source]
    source: io::Error,
}

/// Uninstall managed Python versions.
pub async fn uninstall(
    install_dir: Option<PathBuf>,
    targets: Vec<String>,
    all: bool,
    printer: Printer,
) -> Result<ExitStatus> {
    let installations = ManagedPythonInstallations::from_settings(install_dir)?.init()?;

    let _lock = installations.lock().await?;

    // Perform the uninstallation.
    do_uninstall(&installations, targets, all, printer).await?;

    // Clean up any empty directories.
    if uv_fs::directories(installations.root())?.all(|path| uv_fs::is_temporary(&path)) {
        fs_err::tokio::remove_dir_all(&installations.root()).await?;

        if let Some(top_level) = installations.root().parent() {
            // Remove the `toolchains` symlink.
            match fs_err::tokio::remove_file(top_level.join("toolchains")).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }

            // Best-effort cleanup: the parent may contain user files or be in use.
            if let Err(err) = fs_err::tokio::remove_dir(top_level).await {
                debug!(
                    "Skipping removal of parent directory `{}`: {err}",
                    top_level.user_display()
                );
            }
        }
    }

    Ok(ExitStatus::Success)
}

/// Perform the uninstallation of managed Python installations.
async fn do_uninstall(
    installations: &ManagedPythonInstallations,
    targets: Vec<String>,
    all: bool,
    printer: Printer,
) -> Result<ExitStatus> {
    let start = std::time::Instant::now();

    let requests = if all {
        vec![PythonRequest::Default]
    } else {
        let targets = targets.into_iter().collect::<BTreeSet<_>>();
        targets
            .iter()
            .map(|target| PythonRequest::parse(target.as_str()))
            .collect::<Vec<_>>()
    };

    let download_requests = requests
        .iter()
        .map(|request| {
            PythonDownloadRequest::from_request(request)
                // Always include pre-releases in uninstalls.
                .map(|request| request.with_prereleases(true))
                .ok_or_else(|| {
                    anyhow::anyhow!("Cannot uninstall managed Python for request: {request}")
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let installed_installations: Vec<_> = installations.find_all()?.collect();
    let mut matching_installations = BTreeSet::default();
    let is_default_request = matches!(requests.as_slice(), [PythonRequest::Default]);
    for (request, download_request) in requests.iter().zip(download_requests) {
        if is_default_request {
            writeln!(printer.stderr(), "Searching for Python installations")?;
        } else {
            writeln!(
                printer.stderr(),
                "Searching for Python versions matching: {}",
                request.cyan()
            )?;
        }
        let mut candidates = installed_installations
            .iter()
            .filter(|installation| download_request.satisfied_by_key(installation.key()))
            .peekable();
        if candidates.peek().is_none() {
            // Clear any remnants in the registry
            #[cfg(windows)]
            {
                uv_python_managed::windows_registry::remove_orphan_registry_entries(
                    &installed_installations,
                );
            }

            if is_default_request {
                writeln!(printer.stderr(), "No Python installations found")?;
                return Ok(ExitStatus::Failure);
            }

            writeln!(
                printer.stderr(),
                "No existing installations found for: {}",
                request.cyan()
            )?;
        }
        matching_installations.extend(candidates.cloned());
    }

    if matching_installations.is_empty() {
        writeln!(
            printer.stderr(),
            "No Python installations found matching the requests"
        )?;
        return Ok(ExitStatus::Failure);
    }

    // Remove registry entries first, so we don't have dangling entries between the file removal
    // and the registry removal.
    let mut errors = vec![];
    #[cfg(windows)]
    {
        uv_python_managed::windows_registry::remove_registry_entry(
            &matching_installations,
            all,
            &mut errors,
        );
        uv_python_managed::windows_registry::remove_orphan_registry_entries(
            &installed_installations,
        );
    }

    // Find and remove all relevant Python executables
    let mut uninstalled_executables: FxHashMap<PythonInstallationKey, FxHashSet<PathBuf>> =
        FxHashMap::default();
    for executable in python_executable_dir()?
        .read_dir()
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry),
            Err(err) => {
                warn!("Failed to read executable: {}", err);
                None
            }
        })
        .filter(|entry| entry.file_type().is_ok_and(|file_type| !file_type.is_dir()))
        .map(|entry| entry.path())
        // Only include files that match the expected Python executable names
        // TODO(zanieb): This is a minor optimization to avoid opening more files, but we could
        // leave broken links behind, i.e., if the user created them.
        .filter(|path| {
            let name = path.file_name().and_then(|name| name.to_str());
            matching_installations.iter().any(|installation| {
                name == Some(&installation.key().executable_name_minor())
                    || name == Some(&installation.key().executable_name_major())
                    || name == Some(&installation.key().executable_name())
            })
        })
        .sorted()
    {
        let Some(installation) = matching_installations
            .iter()
            .find(|installation| installation.is_bin_link(executable.as_path()))
        else {
            continue;
        };

        fs_err::remove_file(&executable)?;
        debug!(
            "Removed `{}` for `{}`",
            executable.simplified_display(),
            installation.key()
        );
        uninstalled_executables
            .entry(installation.key().clone())
            .or_default()
            .insert(executable);
    }

    let mut tasks: FuturesUnordered<_> = matching_installations
        .iter()
        .map(|installation| async move {
            (
                installation.key(),
                fs_err::tokio::remove_dir_all(installation.path()).await,
            )
        })
        .collect();

    let mut uninstalled = IndexSet::<PythonInstallationKey>::default();
    while let Some((key, result)) = tasks.next().await {
        if let Err(err) = result {
            errors.push((key.clone(), anyhow::Error::new(err)));
        } else {
            uninstalled.insert(key.clone());
        }
    }

    // Read all existing managed installations and find the highest installed patch
    // for each installed minor version. Ensure the minor version link directory
    // is still valid.
    let uninstalled_minor_versions: IndexSet<_> = uninstalled
        .iter()
        .map(PythonInstallationMinorVersionKey::ref_cast)
        .collect();
    let remaining_minor_versions = ManagedPythonInstallation::highest_by_minor_version_key(
        installed_installations
            .iter()
            .filter(|installation| !uninstalled.contains(installation.key())),
    );

    for (_, installation) in remaining_minor_versions
        .iter()
        .filter(|(minor_version, _)| uninstalled_minor_versions.contains(minor_version))
    {
        installation.ensure_minor_version_link().await?;
    }
    // For each uninstalled installation, check if there are no remaining installations
    // for its minor version. If there are none remaining, remove the symlink directory
    // (or junction on Windows) if it exists.
    for installation in &matching_installations {
        if !remaining_minor_versions.contains_key(installation.minor_version_key())
            && let Some(minor_version_link) =
                PythonMinorVersionLink::from_installation(installation)
            && minor_version_link.exists()
        {
            uv_fs::remove_symlink(&minor_version_link.symlink_directory).map_err(|source| {
                MinorVersionLinkRemovalError {
                    path: minor_version_link.symlink_directory.clone(),
                    source,
                }
            })?;
            let symlink_term = if cfg!(windows) {
                "junction"
            } else {
                "symlink directory"
            };
            debug!(
                "Removed {}: {}",
                symlink_term,
                minor_version_link.symlink_directory.to_string_lossy()
            );
        }
    }

    // Report on any uninstalled installations.
    if let Some(first_uninstalled) = uninstalled.first() {
        let summary = if uninstalled.len() == 1 {
            format!("Python {}", first_uninstalled.version())
        } else {
            format!("{} versions", uninstalled.len())
        };
        writeln!(
            printer.stderr(),
            "{}",
            format!(
                "Uninstalled {} {}",
                summary.bold(),
                format!("in {}", elapsed(start.elapsed())).dimmed()
            )
            .dimmed()
        )?;

        for key in uninstalled.into_iter().sorted_unstable() {
            let executables = format_executables(&key, &uninstalled_executables);
            writeln!(
                printer.stderr(),
                " {} {}{}",
                "-".red(),
                key.bold(),
                executables
            )?;
        }
    }

    if !errors.is_empty() {
        for (key, err) in errors {
            writeln!(
                printer.stderr(),
                "Failed to uninstall {}: {}",
                key.green(),
                err.to_string().trim()
            )?;
        }
        return Ok(ExitStatus::Failure);
    }

    Ok(ExitStatus::Success)
}
