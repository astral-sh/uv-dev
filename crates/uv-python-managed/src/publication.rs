//! Publish a prepared installation without discarding its predecessor on failure.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tracing::warn;

use uv_fs::{LockedFile, Simplified};
use uv_python_types::PythonInstallationKey;

const MARKER: &str = ".uv-replacement";

#[derive(Deserialize, Serialize)]
struct Journal {
    version: u8,
    marker: String,
    committed: bool,
}

struct Recovery {
    destination: PathBuf,
    previous: PathBuf,
    journal: PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error("failed to decode Python replacement journal `{}`", journal.user_display())]
struct JournalDecodeError {
    journal: PathBuf,
    #[source]
    source: serde_json::Error,
}

impl Recovery {
    fn new(destination: &Path, scratch: &Path) -> io::Result<Self> {
        let key = destination
            .file_name()
            .ok_or_else(|| io::Error::other("Python installation has no directory name"))?;
        let mut previous = OsString::from(".replacement-");
        previous.push(key);
        let mut journal = previous.clone();
        journal.push(".json");
        Ok(Self {
            destination: destination.to_path_buf(),
            previous: scratch.join(previous),
            journal: scratch.join(journal),
        })
    }

    fn save(&self, journal: &Journal) -> io::Result<()> {
        let contents = serde_json::to_vec(journal).map_err(io::Error::other)?;
        uv_fs::write_atomic_sync(&self.journal, contents)
    }

    fn discard_failed_journal(&self) {
        if let Err(err) = fs_err::remove_file(&self.journal) {
            // The original operation determines retry eligibility. Recovery can discard this
            // journal later, since its predecessor was never moved or has been restored.
            warn!(
                "Python replacement journal cleanup is pending at `{}`: {err}",
                self.journal.user_display()
            );
        }
    }

    fn finish(&self, journal: &Journal) -> io::Result<()> {
        match fs_err::symlink_metadata(&self.previous) {
            Ok(metadata) if metadata.is_symlink() => fs_err::remove_file(&self.previous)?,
            Ok(metadata) if metadata.is_dir() => fs_err::remove_dir_all(&self.previous)?,
            Ok(_) => {
                return Err(io::Error::other(format!(
                    "unexpected replacement backup at `{}`",
                    self.previous.user_display()
                )));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        let marker = self.destination.join(MARKER);
        match fs_err::read_to_string(&marker) {
            Ok(contents) if contents == journal.marker => fs_err::remove_file(marker)?,
            Ok(_) => {
                return Err(io::Error::other(format!(
                    "replacement marker changed at `{}`",
                    marker.user_display()
                )));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        fs_err::remove_file(&self.journal)
    }

    fn recover(&self) -> io::Result<()> {
        let journal: Journal = match fs_err::read(&self.journal) {
            Ok(contents) => serde_json::from_slice(&contents).map_err(|source| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    JournalDecodeError {
                        journal: self.journal.clone(),
                        source,
                    },
                )
            })?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return match fs_err::symlink_metadata(&self.previous) {
                    Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                    Err(err) => Err(err),
                    Ok(_) => Err(io::Error::other(format!(
                        "replacement backup has no journal at `{}`",
                        self.previous.user_display()
                    ))),
                };
            }
            Err(err) => return Err(err),
        };
        if journal.version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported Python replacement journal version {} at `{}`",
                    journal.version,
                    self.journal.user_display()
                ),
            ));
        }
        if journal.committed {
            if let Err(err) = self.finish(&journal) {
                // A usable published interpreter is independent of deferred backup cleanup.
                warn!(
                    "Python replacement cleanup is pending at `{}`: {err}",
                    self.previous.user_display()
                );
            }
            return Ok(());
        }

        let previous_exists = match fs_err::symlink_metadata(&self.previous) {
            Ok(_) => true,
            Err(err) if err.kind() == io::ErrorKind::NotFound => false,
            Err(err) => return Err(err),
        };
        if !previous_exists {
            // Publication had not moved the predecessor when the process stopped.
            if !self.destination.is_dir() {
                return Err(io::Error::other(format!(
                    "neither Python installation nor replacement backup exists for `{}`",
                    self.destination.user_display()
                )));
            }
            return fs_err::remove_file(&self.journal);
        }
        match fs_err::symlink_metadata(&self.destination) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                rename(&self.previous, &self.destination)?;
                fs_err::remove_file(&self.journal)
            }
            Err(err) => Err(err),
            Ok(_) => {
                // The staged marker identifies a replacement published before its journal was
                // committed. An unrelated entry must not cause the predecessor to be discarded.
                if !fs_err::read_to_string(self.destination.join(MARKER))
                    .is_ok_and(|contents| contents == journal.marker)
                {
                    return Err(io::Error::other(format!(
                        "cannot recover Python replacement; predecessor retained at `{}`",
                        self.previous.user_display()
                    )));
                }
                let journal = Journal {
                    committed: true,
                    ..journal
                };
                self.save(&journal)?;
                if let Err(err) = self.finish(&journal) {
                    warn!(
                        "Python replacement cleanup is pending at `{}`: {err}",
                        self.previous.user_display()
                    );
                }
                Ok(())
            }
        }
    }
}

fn rename(from: &Path, to: &Path) -> io::Result<()> {
    uv_fs::with_retry_sync(from, to, "rename", || fs_err::rename(from, to))
}

fn exchange(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        uv_fs::exchange_paths(from, to)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (from, to);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory exchange is unavailable",
        ))
    }
}

fn publish_inner(
    staged: &Path,
    destination: &Path,
    scratch: &Path,
    marker: &str,
    exchange: impl FnOnce(&Path, &Path) -> io::Result<()>,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    // A retry can arrive after publication and restoration both failed. Recover that attempt
    // before either a fresh install or a native exchange can publish another replacement.
    let recovery = Recovery::new(destination, scratch)?;
    recovery.recover()?;
    if recovery.journal.try_exists()? {
        return Err(io::Error::other(format!(
            "previous Python replacement cleanup must finish at `{}` before reinstalling",
            recovery.previous.user_display()
        )));
    }
    if !destination.is_dir() {
        return rename(staged, destination);
    }
    match exchange(staged, destination) {
        Ok(()) => return Ok(()),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::Unsupported
                    | io::ErrorKind::InvalidInput
                    | io::ErrorKind::CrossesDevices
            ) => {}
        Err(err) => return Err(err),
    }

    // The marker is private until the complete installation is published.
    let marker_path = staged.join(MARKER);
    let mut marker_file = fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker_path)?;
    marker_file.write_all(marker.as_bytes())?;
    drop(marker_file);
    let journal = Journal {
        version: 1,
        marker: marker.to_string(),
        committed: false,
    };
    recovery.save(&journal)?;
    if let Err(err) = rename(destination, &recovery.previous) {
        // The existing installation was never moved.
        recovery.discard_failed_journal();
        return Err(err);
    }
    if let Err(publication) = rename(staged, destination) {
        match fs_err::symlink_metadata(destination) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            current => {
                let context = match current {
                    Ok(_) => "destination is occupied".to_string(),
                    Err(err) => err.to_string(),
                };
                return Err(io::Error::new(
                    publication.kind(),
                    format!(
                        "{publication}; {context}; predecessor retained at `{}`",
                        recovery.previous.user_display()
                    ),
                ));
            }
        }
        if let Err(restoration) = rename(&recovery.previous, destination) {
            return Err(io::Error::new(
                publication.kind(),
                format!(
                    "{publication}; failed to restore the previous Python: {restoration}; predecessor retained at `{}`",
                    recovery.previous.user_display()
                ),
            ));
        }
        recovery.discard_failed_journal();
        return Err(publication);
    }
    let journal = Journal {
        committed: true,
        ..journal
    };
    // A complete replacement is already visible. Keep recovery data if bookkeeping or cleanup
    // fails, without reporting the successfully published interpreter as an installation failure.
    if let Err(err) = recovery
        .save(&journal)
        .and_then(|()| recovery.finish(&journal))
    {
        warn!(
            "Python replacement was published; cleanup is pending at `{}`: {err}",
            recovery.previous.user_display()
        );
    }
    Ok(())
}

/// Recover under the same installation-directory lock used by all download callers.
pub(crate) async fn recover_all(
    installations: PathBuf,
    scratch: PathBuf,
    installation_lock: LockedFile,
) -> io::Result<LockedFile> {
    tokio::task::spawn_blocking(move || {
        let entries = match fs_err::read_dir(&scratch) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(installation_lock),
            Err(err) => return Err(err),
        };
        let mut keys = BTreeSet::new();
        for entry in entries {
            let entry = entry?;
            let filename = entry.file_name();
            let Some(key) = filename
                .to_str()
                .and_then(|filename| filename.strip_prefix(".replacement-"))
                .and_then(|filename| filename.strip_suffix(".json"))
            else {
                continue;
            };
            // Journal names are installation keys, never arbitrary relative destination paths.
            let key = key
                .parse::<PythonInstallationKey>()
                .map_err(io::Error::other)?;
            keys.insert(key.to_string());
        }
        for key in keys {
            Recovery::new(&installations.join(key), &scratch)?.recover()?;
        }
        Ok(installation_lock)
    })
    .await
    .map_err(io::Error::other)?
}

/// Recover a requested key under the existing installation-directory lock.
pub(crate) async fn recover(
    destination: PathBuf,
    scratch: PathBuf,
    installation_lock: Arc<LockedFile>,
) -> io::Result<()> {
    tokio::task::spawn_blocking(move || {
        let _lock = installation_lock;
        Recovery::new(&destination, &scratch)?.recover()
    })
    .await
    .map_err(io::Error::other)?
}

/// Retain staging and the installation lock until publication and its rollback finish.
pub(crate) async fn publish(
    staging: TempDir,
    staged: PathBuf,
    destination: PathBuf,
    scratch: PathBuf,
    installation_lock: Arc<LockedFile>,
) -> io::Result<()> {
    publish_with(
        staging,
        staged,
        destination,
        scratch,
        installation_lock,
        |staged, destination, scratch, marker| {
            publish_inner(staged, destination, scratch, marker, exchange, rename)
        },
    )
    .await
}

async fn publish_with<F>(
    staging: TempDir,
    staged: PathBuf,
    destination: PathBuf,
    scratch: PathBuf,
    installation_lock: Arc<LockedFile>,
    publish: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path, &Path, &str) -> io::Result<()> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let _lock = installation_lock;
        let marker = staging
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::other("Python staging directory has no valid name"))?;
        let result = publish(&staged, &destination, &scratch, marker);
        if let Err(err) = staging.close()
            && err.kind() != io::ErrorKind::NotFound
        {
            warn!("Failed to clean Python replacement staging: {err}");
        }
        result
    })
    .await
    .map_err(io::Error::other)?
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;
    use std::sync::{Arc, mpsc};

    use uv_fs::{LockedFile, LockedFileMode};

    use super::{Journal, MARKER, Recovery, publish_inner, publish_with, rename};

    fn unavailable(_from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn installation(path: &Path, contents: &str) -> io::Result<()> {
        fs_err::create_dir_all(path)?;
        fs_err::write(path.join("interpreter"), contents)
    }

    #[test]
    fn failed_predecessor_move_retains_error_when_cleanup_fails() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("installed");
        let staged = root.path().join("staged");
        installation(&destination, "old")?;
        installation(&staged, "new")?;
        let recovery = Recovery::new(&destination, root.path())?;
        let saved = root.path().join("saved-journal");
        let error = publish_inner(
            &staged,
            &destination,
            root.path(),
            "transaction",
            unavailable,
            |_, _| {
                // Obstruct journal removal with a real filesystem error, retaining its bytes.
                fs_err::rename(&recovery.journal, &saved)?;
                fs_err::create_dir(&recovery.journal)?;
                fs_err::write(recovery.journal.join("sentinel"), "keep")?;
                Err(io::Error::new(io::ErrorKind::TimedOut, "rename timed out"))
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "rename timed out");
        assert_eq!(
            fs_err::read_to_string(destination.join("interpreter"))?,
            "old"
        );
        assert_eq!(
            fs_err::read_to_string(recovery.journal.join("sentinel"))?,
            "keep"
        );
        fs_err::remove_dir_all(&recovery.journal)?;
        fs_err::rename(saved, &recovery.journal)?;
        recovery.recover()?;
        assert!(!recovery.journal.exists());
        Ok(())
    }

    #[test]
    fn failed_publication_retains_error_when_restored_journal_cleanup_fails() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("installed");
        let staged = root.path().join("staged");
        installation(&destination, "old")?;
        installation(&staged, "new")?;
        let recovery = Recovery::new(&destination, root.path())?;
        let saved = root.path().join("saved-journal");
        let error = publish_inner(
            &staged,
            &destination,
            root.path(),
            "transaction",
            unavailable,
            |from, to| {
                if from == staged {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "rename timed out"));
                }
                rename(from, to)?;
                if to == destination {
                    // Restoration succeeds, while the remaining journal cannot be removed.
                    fs_err::rename(&recovery.journal, &saved)?;
                    fs_err::create_dir(&recovery.journal)?;
                    fs_err::write(recovery.journal.join("sentinel"), "keep")?;
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "rename timed out");
        assert_eq!(
            fs_err::read_to_string(destination.join("interpreter"))?,
            "old"
        );
        assert!(!recovery.previous.exists());
        assert_eq!(
            fs_err::read_to_string(recovery.journal.join("sentinel"))?,
            "keep"
        );
        fs_err::remove_dir_all(&recovery.journal)?;
        fs_err::rename(saved, &recovery.journal)?;
        recovery.recover()?;
        assert!(!recovery.journal.exists());
        Ok(())
    }

    #[test]
    fn failed_publication_restores_predecessor() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let old = root.path().join("installed");
        let staged = root.path().join("staged");
        installation(&old, "old")?;
        installation(&staged, "new")?;
        let result = publish_inner(
            &staged,
            &old,
            root.path(),
            "transaction",
            unavailable,
            |from, to| {
                if from == staged && to == old {
                    Err(io::Error::other("publication denied"))
                } else {
                    rename(from, to)
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(fs_err::read_to_string(old.join("interpreter"))?, "old");
        assert_eq!(fs_err::read_to_string(staged.join("interpreter"))?, "new");
        let recovery = Recovery::new(&old, root.path())?;
        assert!(!recovery.previous.exists());
        assert!(!recovery.journal.exists());
        Ok(())
    }

    #[test]
    fn failed_restoration_is_recovered_on_restart() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let old = root.path().join("installed");
        let staged = root.path().join("staged");
        installation(&old, "old")?;
        installation(&staged, "new")?;
        let result = publish_inner(
            &staged,
            &old,
            root.path(),
            "transaction",
            unavailable,
            |from, to| {
                if to == old {
                    Err(io::Error::other("destination unavailable"))
                } else {
                    rename(from, to)
                }
            },
        );
        assert!(result.is_err());
        assert!(!old.exists());
        let recovery = Recovery::new(&old, root.path())?;
        assert_eq!(
            fs_err::read_to_string(recovery.previous.join("interpreter"))?,
            "old"
        );
        recovery.recover()?;
        assert_eq!(fs_err::read_to_string(old.join("interpreter"))?, "old");
        assert!(!recovery.previous.exists());
        assert!(!recovery.journal.exists());
        Ok(())
    }

    #[test]
    fn retry_recovers_failed_restoration_before_publication() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("installed");
        let staged = root.path().join("first-attempt");
        installation(&destination, "old")?;
        installation(&staged, "first")?;
        let error = publish_inner(
            &staged,
            &destination,
            root.path(),
            "first",
            unavailable,
            |from, to| {
                if to == destination {
                    Err(io::Error::new(io::ErrorKind::TimedOut, "rename timed out"))
                } else {
                    rename(from, to)
                }
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(!destination.exists());
        let recovery = Recovery::new(&destination, root.path())?;
        assert_eq!(
            fs_err::read_to_string(recovery.previous.join("interpreter"))?,
            "old"
        );

        let retry = root.path().join("retry");
        installation(&retry, "second")?;
        publish_inner(
            &retry,
            &destination,
            root.path(),
            "second",
            unavailable,
            rename,
        )?;
        // A later command can acquire the installation lock without rejecting the new entry.
        recovery.recover()?;
        assert_eq!(
            fs_err::read_to_string(destination.join("interpreter"))?,
            "second"
        );
        assert!(!recovery.previous.exists());
        assert!(!recovery.journal.exists());
        assert!(!destination.join(MARKER).exists());
        Ok(())
    }

    #[test]
    fn restart_finishes_an_already_published_replacement() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("installed");
        let recovery = Recovery::new(&destination, root.path())?;
        installation(&recovery.previous, "old")?;
        installation(&destination, "new")?;
        fs_err::write(destination.join(MARKER), "transaction")?;
        recovery.save(&Journal {
            version: 1,
            marker: "transaction".to_string(),
            committed: false,
        })?;
        recovery.recover()?;
        assert_eq!(
            fs_err::read_to_string(destination.join("interpreter"))?,
            "new"
        );
        assert!(!recovery.previous.exists());
        assert!(!recovery.journal.exists());
        assert!(!destination.join(MARKER).exists());
        Ok(())
    }

    #[test]
    fn unexpected_destination_retains_the_backup() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("installed");
        let recovery = Recovery::new(&destination, root.path())?;
        installation(&recovery.previous, "old")?;
        installation(&destination, "unrelated")?;
        recovery.save(&Journal {
            version: 1,
            marker: "transaction".to_string(),
            committed: false,
        })?;
        assert!(recovery.recover().is_err());
        assert_eq!(
            fs_err::read_to_string(destination.join("interpreter"))?,
            "unrelated"
        );
        assert_eq!(
            fs_err::read_to_string(recovery.previous.join("interpreter"))?,
            "old"
        );
        assert!(recovery.journal.is_file());
        Ok(())
    }

    #[test]
    fn successful_fallback_discards_committed_backup() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("installed");
        let staged = root.path().join("staged");
        installation(&destination, "old")?;
        installation(&staged, "new")?;
        publish_inner(
            &staged,
            &destination,
            root.path(),
            "transaction",
            unavailable,
            rename,
        )?;
        let recovery = Recovery::new(&destination, root.path())?;
        assert_eq!(
            fs_err::read_to_string(destination.join("interpreter"))?,
            "new"
        );
        assert!(!recovery.previous.exists());
        assert!(!recovery.journal.exists());
        assert!(!destination.join(MARKER).exists());
        Ok(())
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn native_exchange_retains_old_contents_in_staging() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("installed");
        let staged = root.path().join("staged");
        installation(&destination, "old")?;
        installation(&staged, "new")?;
        publish_inner(
            &staged,
            &destination,
            root.path(),
            "transaction",
            super::exchange,
            rename,
        )?;
        assert_eq!(
            fs_err::read_to_string(destination.join("interpreter"))?,
            "new"
        );
        assert_eq!(fs_err::read_to_string(staged.join("interpreter"))?, "old");
        assert!(!Recovery::new(&destination, root.path())?.journal.exists());
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_waiter_retains_staging_and_installation_lock() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let staging = tempfile::tempdir_in(root.path())?;
        let staged = staging.path().to_path_buf();
        installation(&staged, "new")?;
        let destination = root.path().join("installed");
        let lock_path = root.path().join(".lock");
        let lock = Arc::new(
            LockedFile::acquire(&lock_path, LockedFileMode::Exclusive, "test installation")
                .await
                .map_err(io::Error::other)?,
        );
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, resume) = mpsc::channel();
        let (finished, completion) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(publish_with(
            staging,
            staged.clone(),
            destination.clone(),
            root.path().to_path_buf(),
            lock,
            move |staged, destination, scratch, marker| {
                let _ = entered.send(());
                resume.recv().map_err(io::Error::other)?;
                let result =
                    publish_inner(staged, destination, scratch, marker, unavailable, rename);
                let _ = finished.send(());
                result
            },
        ));
        started.await.map_err(io::Error::other)?;
        worker.abort();
        let _ = worker.await;
        assert!(staged.is_dir());
        assert!(
            LockedFile::acquire_no_wait(&lock_path, LockedFileMode::Exclusive, "test installation")
                .is_none()
        );
        release.send(()).map_err(io::Error::other)?;
        completion.await.map_err(io::Error::other)?;
        let _lock = LockedFile::acquire(&lock_path, LockedFileMode::Exclusive, "test installation")
            .await
            .map_err(io::Error::other)?;
        assert!(!staged.exists());
        assert_eq!(
            fs_err::read_to_string(destination.join("interpreter"))?,
            "new"
        );
        Ok(())
    }
}
