use std::collections::BTreeSet;
use std::fmt::Write;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use futures::{Stream, StreamExt};
use indexmap::IndexSet;
use itertools::Itertools;
use owo_colors::OwoColorize;
use rustc_hash::{FxHashMap, FxHashSet};
use tracing::{Span, debug, warn};

use uv_command_support::Printer;
use uv_command_support::{ExitStatus, elapsed};
use uv_fs::{LockedFile, Simplified};
use uv_python_managed::{
    ManagedPythonInstallation, ManagedPythonInstallations, PythonMinorVersionLink,
    python_executable_dir,
};
use uv_python_types::{
    PythonDownloadRequest, PythonInstallationKey, PythonInstallationMinorVersionKey, PythonRequest,
};

use crate::install::format_executables;
use crate::{ChangeEvent, ChangeEventKind};

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

    let lock = Arc::new(installations.lock().await?);

    // Perform the uninstallation.
    do_uninstall(&installations, lock.clone(), targets, all, printer).await?;

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

            if uv_fs::directories(top_level)?.all(|path| uv_fs::is_temporary(&path)) {
                fs_err::tokio::remove_dir_all(top_level).await?;
            }
        }
    }

    Ok(ExitStatus::Success)
}

/// Perform the uninstallation of managed Python installations.
async fn do_uninstall(
    installations: &ManagedPythonInstallations,
    lock: Arc<LockedFile>,
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
            PythonDownloadRequest::from_request(request).ok_or_else(|| {
                anyhow::anyhow!("Cannot uninstall managed Python for request: {request}")
            })
        })
        // Always include pre-releases in uninstalls
        .map(|result| result.map(|request| request.with_prereleases(true)))
        .collect::<Result<Vec<_>>>()?;
    let installed_installations: Vec<_> = installations.find_all()?.collect();
    let mut matching_installations = BTreeSet::default();
    for (request, download_request) in requests.iter().zip(download_requests) {
        if matches!(requests.as_slice(), [PythonRequest::Default]) {
            writeln!(printer.stderr(), "Searching for Python installations")?;
        } else {
            writeln!(
                printer.stderr(),
                "Searching for Python versions matching: {}",
                request.cyan()
            )?;
        }
        let mut found = false;
        for installation in installed_installations
            .iter()
            .filter(|installation| download_request.satisfied_by_key(installation.key()))
        {
            found = true;
            matching_installations.insert(installation.clone());
        }
        if !found {
            // Clear any remnants in the registry
            #[cfg(windows)]
            {
                uv_python_managed::windows_registry::remove_orphan_registry_entries(
                    &installed_installations,
                );
            }

            if matches!(requests.as_slice(), [PythonRequest::Default]) {
                writeln!(printer.stderr(), "No Python installations found")?;
                return Ok(ExitStatus::Failure);
            }

            writeln!(
                printer.stderr(),
                "No existing installations found for: {}",
                request.cyan()
            )?;
        }
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
            matching_installations.iter().any(|installation| {
                let name = path.file_name().and_then(|name| name.to_str());
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

    let mut tasks = removal_tasks(&matching_installations, lock);

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
    let remaining_installations: Vec<_> = installed_installations
        .into_iter()
        .filter(|installation| !uninstalled.contains(installation.key()))
        .collect();

    let remaining_minor_versions =
        ManagedPythonInstallation::highest_by_minor_version_key(remaining_installations.iter());

    for (_, installation) in remaining_minor_versions
        .iter()
        .filter(|(minor_version, _)| uninstalled_minor_versions.contains(minor_version))
    {
        installation.ensure_minor_version_link()?;
    }
    // For each uninstalled installation, check if there are no remaining installations
    // for its minor version. If there are none remaining, remove the symlink directory
    // (or junction on Windows) if it exists.
    for installation in &matching_installations {
        if !remaining_minor_versions.contains_key(installation.minor_version_key()) {
            if let Some(minor_version_link) =
                PythonMinorVersionLink::from_installation(installation)
            {
                if minor_version_link.exists() {
                    uv_fs::remove_symlink(&minor_version_link.symlink_directory).map_err(
                        |source| MinorVersionLinkRemovalError {
                            path: minor_version_link.symlink_directory.clone(),
                            source,
                        },
                    )?;
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
        }
    }

    // Report on any uninstalled installations.
    if let Some(first_uninstalled) = uninstalled.first() {
        if uninstalled.len() == 1 {
            // Ex) "Uninstalled Python 3.9.7 in 1.68s"
            writeln!(
                printer.stderr(),
                "{}",
                format!(
                    "Uninstalled {} {}",
                    format!("Python {}", first_uninstalled.version()).bold(),
                    format!("in {}", elapsed(start.elapsed())).dimmed()
                )
                .dimmed()
            )?;
        } else {
            // Ex) "Uninstalled 2 versions in 1.68s"
            writeln!(
                printer.stderr(),
                "{}",
                format!(
                    "Uninstalled {} {}",
                    format!("{} versions", uninstalled.len()).bold(),
                    format!("in {}", elapsed(start.elapsed())).dimmed()
                )
                .dimmed()
            )?;
        }

        for event in uninstalled
            .into_iter()
            .map(|key| ChangeEvent {
                key,
                kind: ChangeEventKind::Removed,
            })
            .sorted_unstable_by(|a, b| a.key.cmp(&b.key).then_with(|| a.kind.cmp(&b.kind)))
        {
            let executables = format_executables(&event, &uninstalled_executables);
            match event.kind {
                ChangeEventKind::Removed => {
                    writeln!(
                        printer.stderr(),
                        " {} {}{}",
                        "-".red(),
                        event.key.bold(),
                        executables,
                    )?;
                }
                _ => unreachable!(),
            }
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

/// Limit simultaneous recursive removals and their open filesystem handles.
const MAX_CONCURRENT_REMOVALS: usize = 8;

fn removal_tasks(
    installations: &BTreeSet<ManagedPythonInstallation>,
    lock: Arc<LockedFile>,
) -> impl Stream<Item = (&PythonInstallationKey, io::Result<()>)> {
    futures::stream::iter(installations)
        .map(move |installation| {
            let path = installation.path().to_path_buf();
            let lock = lock.clone();
            let span = Span::current();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    // A cancelled waiter cannot release the installation lock while deletion runs.
                    let _lock = lock;
                    let _entered = span.enter();
                    fs_err::remove_dir_all(path)
                })
                .await;
                let result = match result {
                    Ok(result) => result,
                    Err(error) => Err(io::Error::other(error)),
                };
                (installation.key(), result)
            }
        })
        .buffer_unordered(MAX_CONCURRENT_REMOVALS)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use futures::{FutureExt, StreamExt};
    use tokio::runtime::Builder;
    use tokio::sync::oneshot;
    use uv_cache::Cache;
    use uv_fs::{LockedFile, LockedFileMode};
    use uv_python_managed::downloads::ManagedPythonDownloadList;
    use uv_python_managed::{ManagedPythonInstallation, ManagedPythonInstallations};
    use uv_python_types::PythonDownloadRequest;

    use super::{MAX_CONCURRENT_REMOVALS, removal_tasks};

    fn fixture(root: &Path, count: usize) -> anyhow::Result<BTreeSet<ManagedPythonInstallation>> {
        let downloads = ManagedPythonDownloadList::new_only_embedded()?;
        let download = downloads.find(&PythonDownloadRequest::default())?;
        (0..count)
            .map(|index| {
                let path = root.join(format!("python-{index}"));
                fs_err::create_dir_all(path.join("lib"))?;
                fs_err::write(path.join("lib/module.py"), "value = 1\n")?;
                Ok(ManagedPythonInstallation::new(path, download)?)
            })
            .collect()
    }

    #[test]
    fn cancelled_removals_are_bounded_and_keep_the_lock() -> anyhow::Result<()> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()?;
        let cache = Cache::temp()?;
        let installations =
            ManagedPythonInstallations::from_settings(Some(cache.root().join("python")))?.init()?;
        let matching = fixture(installations.root(), MAX_CONCURRENT_REMOVALS * 4)?;
        let lock_path = installations.root().join(".lock");
        runtime.block_on(async {
            let lock = Arc::new(installations.lock().await?);
            let (started, start) = oneshot::channel();
            let (release, finish) = mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                finish.recv()
            });
            start.await?;
            assert!(
                removal_tasks(&matching, lock)
                    .collect::<Vec<_>>()
                    .now_or_never()
                    .is_none()
            );
            assert!(
                matching
                    .iter()
                    .all(|installation| installation.path().is_dir())
            );
            assert!(
                LockedFile::acquire_no_wait(&lock_path, LockedFileMode::Exclusive, "test removals")
                    .is_none()
            );
            tokio::task::yield_now().await;
            release.send(())?;
            blocker.await??;
            let _reacquired = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Some(lock) = LockedFile::acquire_no_wait(
                        &lock_path,
                        LockedFileMode::Exclusive,
                        "test removals",
                    ) {
                        break lock;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await?;
            // Only the admitted workers run after cancellation; remaining paths are untouched.
            assert_eq!(
                matching
                    .iter()
                    .filter(|installation| !installation.path().exists())
                    .count(),
                MAX_CONCURRENT_REMOVALS
            );
            Ok(())
        })
    }

    #[tokio::test]
    async fn removal_failure_does_not_skip_other_installations() -> anyhow::Result<()> {
        let cache = Cache::temp()?;
        let installations =
            ManagedPythonInstallations::from_settings(Some(cache.root().join("python")))?.init()?;
        let matching = fixture(installations.root(), MAX_CONCURRENT_REMOVALS * 2)?;
        let failed = matching
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing fixture"))?
            .path()
            .to_path_buf();
        fs_err::remove_dir_all(&failed)?;
        fs_err::write(&failed, "not a directory")?;
        let lock = Arc::new(installations.lock().await?);
        let results = removal_tasks(&matching, lock).collect::<Vec<_>>().await;
        assert_eq!(results.len(), matching.len());
        assert_eq!(
            results.iter().filter(|(_, result)| result.is_err()).count(),
            1
        );
        assert!(failed.is_file());
        assert!(
            matching
                .iter()
                .filter(|installation| installation.path() != failed)
                .all(|installation| !installation.path().exists())
        );
        Ok(())
    }
}
