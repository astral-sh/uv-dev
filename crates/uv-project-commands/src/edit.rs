use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use anyhow::Result;
use tracing::{debug, warn};

use uv_fs::Simplified;
use uv_lock_operations::LockTarget;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_scripts::{Pep723Metadata, Pep723Script};
use uv_workspace::pyproject::PyProjectToml;
use uv_workspace::{VirtualProject, WorkspaceCache};

use crate::ProjectError;

/// A project manifest or script metadata to edit.
#[derive(Debug, Clone)]
#[expect(clippy::large_enum_variant)]
pub(super) enum EditTarget {
    /// A PEP 723 script, with inline metadata.
    Script(Pep723Script),
    /// A project with a `pyproject.toml`.
    Project(VirtualProject),
}

impl<'lock> From<&'lock EditTarget> for LockTarget<'lock> {
    fn from(value: &'lock EditTarget) -> Self {
        match value {
            EditTarget::Script(script) => Self::Script(script),
            EditTarget::Project(project) => Self::Workspace(project.workspace()),
        }
    }
}

impl EditTarget {
    /// Write the updated metadata, returning whether the content changed.
    pub(super) fn write(&self, content: &str) -> Result<bool, io::Error> {
        match self {
            Self::Script(script) => {
                if content == script.metadata.raw {
                    debug!("No changes to dependencies; skipping update");
                    Ok(false)
                } else {
                    script.write(content)?;
                    Ok(true)
                }
            }
            Self::Project(project) => {
                if content == project.pyproject_toml().raw {
                    debug!("No changes to dependencies; skipping update");
                    Ok(false)
                } else {
                    let pyproject_path = project.root().join("pyproject.toml");
                    fs_err::write(pyproject_path, content)?;
                    Ok(true)
                }
            }
        }
    }

    /// Update parsed metadata and the workspace cache after writing the target.
    pub(super) fn update(
        self,
        content: &str,
        workspace_cache: &WorkspaceCache,
    ) -> Result<Self, ProjectError> {
        match self {
            Self::Script(mut script) => {
                script.metadata = Pep723Metadata::from_str(content)
                    .map_err(ProjectError::Pep723ScriptTomlParse)?;
                Ok(Self::Script(script))
            }
            Self::Project(project) => {
                let pyproject_path = project.root().join("pyproject.toml");
                let project = project
                    .update_member(
                        PyProjectToml::from_string(content.to_string(), &pyproject_path)
                            .map_err(ProjectError::PyprojectTomlParse)?,
                        workspace_cache,
                    )?
                    .ok_or(ProjectError::PyprojectTomlUpdate)?;
                Ok(Self::Project(project))
            }
        }
    }
}

/// The interpreter used for resolution, or an environment that can also be synchronized.
#[derive(Debug, Clone)]
#[expect(clippy::large_enum_variant)]
pub(super) enum PythonTarget {
    Interpreter(Interpreter),
    Environment(PythonEnvironment),
}

impl PythonTarget {
    /// Return the interpreter from either form of Python discovery.
    pub(super) fn interpreter(&self) -> &Interpreter {
        match self {
            Self::Interpreter(interpreter) => interpreter,
            Self::Environment(venv) => venv.interpreter(),
        }
    }
}

/// Restore project or script files on errors and Ctrl-C, unless the edit is committed.
///
/// Only changed files are restored. Callers must exclude files they cannot modify, such as
/// lockfiles when editing with `--frozen`.
pub(super) struct ProjectEdit {
    state: Arc<EditState>,
}

impl ProjectEdit {
    /// Snapshot the files an operation can modify and install its Ctrl-C handler.
    pub(super) fn new(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self> {
        let state = Arc::new(EditState::new(paths)?);
        let _ = ctrlc::set_handler({
            let state = Arc::clone(&state);
            move || {
                state.interrupted.store(true, Ordering::Release);
                state.finish(false);
                #[expect(clippy::cast_possible_wrap)]
                std::process::exit(if cfg!(windows) {
                    0xC000_013A_u32 as i32
                } else {
                    130
                });
            }
        });
        Ok(Self { state })
    }

    /// Publish a synchronous project or script edit inside the transaction's write fence.
    ///
    /// The callback must complete publication before returning.
    pub(super) fn write<T>(&self, write: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        self.state.write(write)
    }

    /// Publish a lockfile while retaining the transaction fence in the blocking worker.
    pub(super) async fn write_lockfile(&self, path: PathBuf, contents: String) -> io::Result<()> {
        Arc::clone(&self.state).write_lockfile(path, contents).await
    }

    /// Keep the edited files when the operation succeeds.
    pub(super) fn commit(self) {
        self.state.finish(true);
    }
}

impl Drop for ProjectEdit {
    fn drop(&mut self) {
        self.state.finish(false);
        // The signal handler owns the interrupt exit status. An interrupted command must not
        // finish unwinding and exit before that handler, even if it completed rollback first.
        if self.state.interrupted.load(Ordering::Acquire) {
            loop {
                thread::park();
            }
        }
    }
}

/// State retained by every owned write, including a detached blocking worker.
struct EditState {
    files: Mutex<EditFiles>,
    interrupted: AtomicBool,
}

struct EditFiles {
    snapshots: Vec<FileSnapshot>,
    finished: bool,
}

impl EditState {
    fn new(paths: impl IntoIterator<Item = PathBuf>) -> io::Result<Self> {
        let snapshots = paths
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|path| {
                let contents = read_file(&path)?;
                Ok(FileSnapshot { path, contents })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            files: Mutex::new(EditFiles {
                snapshots,
                finished: false,
            }),
            interrupted: AtomicBool::new(false),
        })
    }

    fn write<T>(&self, write: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        let files = self.files.lock().unwrap_or_else(PoisonError::into_inner);
        if files.finished || self.interrupted.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "project edit is no longer accepting writes",
            ));
        }
        // Keep the guard until the synchronous write completes. Queuing work while holding it
        // would allow a detached worker to publish after rollback.
        write()
    }

    async fn write_lockfile(self: Arc<Self>, path: PathBuf, contents: String) -> io::Result<()> {
        tokio::task::spawn_blocking(move || self.write(|| fs_err::write(path, contents)))
            .await
            .map_err(io::Error::other)?
    }

    fn finish(&self, commit: bool) {
        let mut files = self.files.lock().unwrap_or_else(PoisonError::into_inner);
        if files.finished {
            return;
        }
        files.finished = true;
        if commit && !self.interrupted.load(Ordering::Acquire) {
            files.snapshots.clear();
        } else {
            revert(&mut files.snapshots);
        }
    }
}

struct FileSnapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
}

impl FileSnapshot {
    /// Restore the original contents, or remove a file created by the operation.
    fn revert(&self) -> io::Result<()> {
        // An unchanged file may be read-only, even when another file in the edit is writable.
        if let Ok(contents) = read_file(&self.path)
            && contents == self.contents
        {
            return Ok(());
        }

        debug!("Reverting changes to `{}`", self.path.user_display());
        if let Some(contents) = &self.contents {
            fs_err::write(&self.path, contents)
        } else {
            match fs_err::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            }
        }
    }
}

/// Attempt every restoration even if an earlier file cannot be restored.
fn revert(files: &mut Vec<FileSnapshot>) {
    for file in files.drain(..) {
        if let Err(err) = file.revert() {
            warn!("Failed to restore `{}`: {err}", file.path.user_display());
        }
    }
}

fn read_file(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs_err::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, TryLockError, mpsc};
    use std::thread;
    use std::time::Duration;

    use anyhow::{Result, anyhow, bail};
    use futures::poll;

    use super::{EditState, read_file};

    #[test]
    fn interrupt_waits_for_active_publication() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let state = Arc::new(EditState::new([path.clone()])?);
        let (entered, started) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let writer = thread::spawn({
            let state = Arc::clone(&state);
            let path = path.clone();
            move || {
                state.write(|| {
                    entered.send(()).map_err(io::Error::other)?;
                    resume.recv().map_err(io::Error::other)?;
                    fs_err::write(path, "published")
                })
            }
        });
        started.recv()?;
        // The write is paused before publication, while its transaction guard is still held.
        match state.files.try_lock() {
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Poisoned(_)) => bail!("the write guard was poisoned"),
            Ok(_) => bail!("the write released its guard before publication"),
        }
        let (cancelled, cancellation) = mpsc::channel();
        let rollback = thread::spawn({
            let state = Arc::clone(&state);
            move || {
                state.interrupted.store(true, Ordering::Release);
                cancelled.send(())?;
                state.finish(false);
                Ok::<_, mpsc::SendError<()>>(())
            }
        });
        cancellation.recv()?;
        release.send(())?;
        writer
            .join()
            .map_err(|_| anyhow!("write worker panicked"))??;
        rollback
            .join()
            .map_err(|_| anyhow!("rollback worker panicked"))??;
        assert_eq!(fs_err::read_to_string(&path)?, "original");
        let error = state
            .write(|| fs_err::write(&path, "late"))
            .expect_err("an interrupted transaction must reject new writes");
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs_err::read_to_string(&path)?, "original");
        Ok(())
    }

    #[test]
    fn interrupted_commit_restores_snapshots() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let state = EditState::new([path.clone()])?;
        state.write(|| fs_err::write(&path, "edited"))?;
        state.interrupted.store(true, Ordering::Release);
        state.finish(true);
        assert_eq!(fs_err::read_to_string(&path)?, "original");
        Ok(())
    }

    #[test]
    fn detached_queued_write_cannot_publish_after_finish() -> Result<()> {
        for existed in [false, true] {
            for commit in [false, true] {
                let directory = tempfile::tempdir()?;
                let path = directory.path().join("uv.lock");
                if existed {
                    fs_err::write(&path, "original")?;
                }
                let state = Arc::new(EditState::new([path.clone()])?);
                state.write(|| fs_err::write(&path, "edited"))?;
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(1)
                    .build()?;
                runtime.block_on(async {
                    let (entered, started) = mpsc::channel();
                    let (release, resume) = mpsc::channel();
                    let blocker = tokio::task::spawn_blocking(move || {
                        entered.send(()).map_err(io::Error::other)?;
                        resume.recv().map_err(io::Error::other)
                    });
                    started.recv()?;
                    // Poll once to queue the actual lockfile writer behind the occupied worker,
                    // then drop its future. Its blocking job remains owned by the transaction.
                    let mut write = Box::pin(
                        Arc::clone(&state).write_lockfile(path.clone(), "queued".to_owned()),
                    );
                    assert!(poll!(&mut write).is_pending());
                    drop(write);
                    state.finish(commit);
                    release.send(())?;
                    blocker.await??;
                    // Observe the detached job release its state while the runtime is still live.
                    tokio::time::timeout(Duration::from_secs(5), async {
                        while Arc::strong_count(&state) > 1 {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await?;
                    Ok::<_, anyhow::Error>(())
                })?;
                // Runtime shutdown waits for the detached blocking job to finish.
                drop(runtime);
                let expected = if commit {
                    Some(b"edited".to_vec())
                } else if existed {
                    Some(b"original".to_vec())
                } else {
                    None
                };
                assert_eq!(
                    read_file(&path)?,
                    expected,
                    "existed={existed}, commit={commit}"
                );
            }
        }
        Ok(())
    }
}
