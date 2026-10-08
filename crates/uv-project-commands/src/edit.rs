use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use anyhow::Result;
use same_file::Handle;
use tracing::debug;

use uv_fs::{FilePublication, Simplified};
use uv_lock_operations::LockTarget;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_scripts::{Pep723Metadata, Pep723Script};
use uv_warnings::warn_user;
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
    pub(super) fn write(&self, content: &str, edit: &ProjectEdit) -> Result<bool, io::Error> {
        match self {
            Self::Script(script) => {
                if content == script.metadata.raw {
                    debug!("No changes to dependencies; skipping update");
                    Ok(false)
                } else {
                    script.write_with(content, |path, contents| {
                        edit.write_file(path, contents.as_bytes())
                    })?;
                    Ok(true)
                }
            }
            Self::Project(project) => {
                if content == project.pyproject_toml().raw {
                    debug!("No changes to dependencies; skipping update");
                    Ok(false)
                } else {
                    let pyproject_path = project.root().join("pyproject.toml");
                    edit.write_file(&pyproject_path, content.as_bytes())?;
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

    /// Publish a tracked file, retaining the written contents and file identity for rollback.
    pub(super) fn write_file(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        self.state.write_file(path, contents)
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
                let original = read_file(&path)?;
                Ok(FileSnapshot {
                    path,
                    original,
                    written: None,
                    created_path: None,
                })
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

    fn write<T>(&self, write: impl FnOnce(&mut [FileSnapshot]) -> io::Result<T>) -> io::Result<T> {
        let mut files = self.files.lock().unwrap_or_else(PoisonError::into_inner);
        if files.finished || self.interrupted.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "project edit is no longer accepting writes",
            ));
        }
        // Keep the guard until the synchronous write completes. Queuing work while holding it
        // would allow a detached worker to publish after rollback.
        write(&mut files.snapshots)
    }

    fn write_file(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        self.write(|files| {
            let snapshot = files
                .iter_mut()
                .find(|file| file.path == path)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "file is not tracked by this project edit",
                    )
                })?;
            snapshot.write(contents)
        })
    }

    async fn write_lockfile(self: Arc<Self>, path: PathBuf, contents: String) -> io::Result<()> {
        tokio::task::spawn_blocking(move || self.write_file(&path, contents.as_bytes()))
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
    original: Option<FileContents>,
    written: Option<FileContents>,
    created_path: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
struct FileContents {
    identity: Handle,
    contents: Vec<u8>,
}

impl FileSnapshot {
    fn write(&mut self, contents: &[u8]) -> io::Result<()> {
        let expected = self.written.as_ref().or(self.original.as_ref());
        if read_file(&self.path)?.as_ref() != expected {
            return Err(io::Error::other(format!(
                "refusing to overwrite `{}` because it changed outside this project edit",
                self.path.user_display()
            )));
        }
        let mut publication = FilePublication::new(&self.path)?;
        let identity = publication
            .original()
            .map(|file| Handle::from_file(file.file().try_clone()?))
            .transpose()?;
        if identity.as_ref() != expected.map(|file| &file.identity) {
            return Err(io::Error::other("file was replaced before publication"));
        }
        if publication.is_staged() {
            publication.writer().write_all(contents)?;
            let identity = Handle::from_file(publication.writer().file().try_clone()?)?;
            let contents = contents.to_vec();
            let created_path = publication.creation_path().map(Path::to_path_buf);
            if read_file(&self.path)?.as_ref() != expected {
                return Err(io::Error::other("file changed before publication"));
            }
            publication.publish()?;
            self.written = Some(FileContents { identity, contents });
            if let Some(created_path) = created_path {
                self.created_path = Some(created_path);
            }
            Ok(())
        } else {
            self.write_opened(publication.writer(), contents)
        }
    }

    fn write_opened(&mut self, file: &mut fs_err::File, contents: &[u8]) -> io::Result<()> {
        let identity = Handle::from_file(file.file().try_clone()?)?;
        if let Some(expected) = self.written.as_ref().or(self.original.as_ref())
            && identity != expected.identity
        {
            return Err(io::Error::other(format!(
                "refusing to overwrite `{}` because it was replaced before publication",
                self.path.user_display()
            )));
        }
        if !file.metadata()?.is_file() {
            // Devices do not retain a file version that can be restored.
            return file.write_all(contents);
        }
        let written_contents = Vec::with_capacity(contents.len());
        file.set_len(0)?;
        let written = self.written.insert(FileContents {
            identity,
            contents: written_contents,
        });
        // Track successful partial writes too, so an I/O failure does not make their bytes look
        // like a foreign edit. Interrupted writes follow `write_all` retry semantics.
        while written.contents.len() < contents.len() {
            let remaining = &contents[written.contents.len()..];
            match file.write(remaining) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write project file",
                    ));
                }
                Ok(count) => written.contents.extend_from_slice(&remaining[..count]),
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    /// Restore only the unchanged version last published by this transaction.
    fn revert(&self) -> io::Result<()> {
        if let Err(err) = self.restore_owned() {
            return match self.save_original() {
                Ok(Some(path)) => Err(io::Error::new(
                    err.kind(),
                    format!(
                        "{err}; original contents saved to `{}`",
                        path.user_display()
                    ),
                )),
                Ok(None) => Err(err),
                Err(recovery) => Err(io::Error::new(
                    err.kind(),
                    format!("{err}; failed to save original contents: {recovery}"),
                )),
            };
        }
        Ok(())
    }

    fn restore_owned(&self) -> io::Result<()> {
        let Some(written) = &self.written else {
            return Ok(());
        };
        if let Some(created_path) = &self.created_path {
            let metadata = match fs_err::symlink_metadata(created_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            };
            if !metadata.is_file()
                || metadata.is_symlink()
                || read_file(created_path)?.as_ref() != Some(written)
            {
                return Err(io::Error::other(
                    "created file changed outside this project edit; leaving it unchanged",
                ));
            }
            return fs_err::remove_file(created_path);
        }
        let current = read_file(&self.path)?;
        if current.as_ref().map(|file| &file.contents)
            == self.original.as_ref().map(|file| &file.contents)
        {
            return Ok(());
        }
        if current.as_ref() != Some(written) {
            return Err(io::Error::other(
                "file changed outside this project edit; leaving it unchanged",
            ));
        }

        // This is conflict detection, not a filesystem compare-and-swap. Uncooperative writers
        // can still race the final comparison and restoration.
        debug!("Reverting changes to `{}`", self.path.user_display());
        if let Some(original) = &self.original {
            uv_fs::write_file(&self.path, &original.contents)
        } else {
            fs_err::remove_file(&self.path)
        }
    }

    fn save_original(&self) -> io::Result<Option<PathBuf>> {
        let Some(original) = &self.original else {
            return Ok(None);
        };
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "project file has no parent directory",
            )
        })?;
        let mut recovery = tempfile::Builder::new()
            .prefix(".uv-project-recovery-")
            .tempfile_in(parent)?;
        recovery.write_all(&original.contents)?;
        let (_, path) = recovery.keep().map_err(|err| err.error)?;
        Ok(Some(path))
    }
}

/// Attempt every restoration even if an earlier file cannot be restored.
fn revert(files: &mut Vec<FileSnapshot>) {
    for file in files.drain(..) {
        if let Err(err) = file.revert() {
            warn_user!("Did not restore `{}`: {err}", file.path.user_display());
        }
    }
}

fn read_file(path: &Path) -> io::Result<Option<FileContents>> {
    let mut file = match fs_err::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;
    Ok(Some(FileContents {
        identity: Handle::from_file(file.into_file())?,
        contents,
    }))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, TryLockError, mpsc};
    use std::thread;
    use std::time::Duration;

    use anyhow::{Result, anyhow, bail};
    use futures::poll;

    use super::{EditState, FileSnapshot, read_file};

    fn recovery_files(directory: &Path) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in fs_err::read_dir(directory)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".uv-project-recovery-")
            {
                paths.push(entry.path());
            }
        }
        Ok(paths)
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn rollback_preserves_dangling_lockfile_links() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let link = directory.path().join("uv.lock");
        let middle = directory.path().join("middle");
        let target = directory.path().join("created.lock");
        fs_err::os::unix::fs::symlink("middle", &link)?;
        fs_err::os::unix::fs::symlink("created.lock", &middle)?;
        let state = Arc::new(EditState::new([link.clone()])?);
        Arc::clone(&state)
            .write_lockfile(link.clone(), "first lockfile".into())
            .await?;
        Arc::clone(&state)
            .write_lockfile(link.clone(), "second lockfile".into())
            .await?;
        assert_eq!(fs_err::read(&target)?, b"second lockfile");
        state.finish(false);
        assert_eq!(fs_err::read_link(&link)?, Path::new("middle"));
        assert_eq!(fs_err::read_link(&middle)?, Path::new("created.lock"));
        assert!(!target.exists());
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn rollback_keeps_a_foreign_created_target() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let link = directory.path().join("uv.lock");
        let target = directory.path().join("created.lock");
        fs_err::os::unix::fs::symlink("created.lock", &link)?;
        let state = EditState::new([link.clone()])?;
        state.write_file(&link, b"published")?;
        fs_err::remove_file(&target)?;
        fs_err::write(&target, "foreign")?;
        state.finish(false);
        assert_eq!(fs_err::read_link(&link)?, Path::new("created.lock"));
        assert_eq!(fs_err::read(&target)?, b"foreign");
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn rollback_cleans_created_target_after_link_retargeting() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let link = directory.path().join("uv.lock");
        let target = directory.path().join("created.lock");
        let foreign = directory.path().join("foreign.lock");
        fs_err::os::unix::fs::symlink("created.lock", &link)?;
        let state = EditState::new([link.clone()])?;
        state.write_file(&link, b"published")?;
        fs_err::write(&foreign, "foreign")?;
        fs_err::remove_file(&link)?;
        fs_err::os::unix::fs::symlink("foreign.lock", &link)?;
        state.finish(false);
        assert_eq!(fs_err::read_link(&link)?, Path::new("foreign.lock"));
        assert_eq!(fs_err::read(&foreign)?, b"foreign");
        assert!(!target.exists());
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn rollback_keeps_a_foreign_link_at_the_created_target() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let link = directory.path().join("uv.lock");
        let target = directory.path().join("created.lock");
        let moved = directory.path().join("moved.lock");
        fs_err::os::unix::fs::symlink("created.lock", &link)?;
        let state = EditState::new([link.clone()])?;
        state.write_file(&link, b"published")?;
        fs_err::rename(&target, &moved)?;
        fs_err::os::unix::fs::symlink("moved.lock", &target)?;
        state.finish(false);
        assert_eq!(fs_err::read_link(&link)?, Path::new("created.lock"));
        assert_eq!(fs_err::read_link(&target)?, Path::new("moved.lock"));
        assert_eq!(fs_err::read(&moved)?, b"published");
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn tracked_special_file_writes_do_not_truncate() -> Result<()> {
        for commit in [false, true] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("uv.lock");
            fs_err::os::unix::fs::symlink("/dev/null", &path)?;
            let state = EditState::new([path.clone()])?;
            state.write_file(&path, b"discarded lock contents")?;
            state.write_file(&path, b"more discarded lock contents")?;
            state.finish(commit);
            assert_eq!(fs_err::read_link(&path)?, Path::new("/dev/null"));
            assert!(fs_err::read(&path)?.is_empty());
            assert!(recovery_files(directory.path())?.is_empty());
        }
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn created_file_setup_failure_is_cleaned() -> Result<()> {
        const CHILD: &str = "UV_TEST_PROJECT_EDIT_DESCRIPTOR_LIMIT";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "edit::tests::created_file_setup_failure_is_cleaned",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()?;
            assert!(output.status.success(), "{output:?}");
            return Ok(());
        }
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let state = EditState::new([path.clone()])?;
        // This limit belongs to the isolated child, never the test runner's other tests.
        uv_unix::set_open_file_limit(64)?;
        let mut descriptors = Vec::new();
        while let Ok(file) = fs_err::File::open("/dev/null") {
            descriptors.push(file);
        }
        // Creation can take the final descriptor, but duplicating it for identity tracking fails.
        drop(descriptors.pop());
        let result = state.write_file(&path, b"lock contents");
        state.finish(false);
        let exists = path.try_exists()?;
        drop(descriptors);
        let Err(error) = result else {
            bail!("descriptor exhaustion did not fail handle setup");
        };
        assert_eq!(error.raw_os_error(), Some(24)); // EMFILE on Unix.
        assert!(
            !exists,
            "setup failure left the newly created lockfile behind"
        );
        Ok(())
    }

    #[test]
    fn opened_replacement_is_rejected_before_truncation() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let mut snapshot = FileSnapshot {
            path: path.clone(),
            original: read_file(&path)?,
            written: None,
            created_path: None,
        };
        assert_eq!(read_file(&path)?, snapshot.original);
        // Replace the path after the initial comparison, before opening it for publication.
        let replacement = directory.path().join("replacement");
        fs_err::write(&replacement, "external")?;
        fs_err::rename(replacement, &path)?;
        let mut opened = fs_err::OpenOptions::new().write(true).open(&path)?;
        assert!(snapshot.write_opened(&mut opened, b"edited").is_err());
        assert_eq!(fs_err::read_to_string(&path)?, "external");
        assert!(snapshot.written.is_none());
        Ok(())
    }

    #[test]
    fn untouched_files_are_not_restored() -> Result<()> {
        for existed in [false, true] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("uv.lock");
            if existed {
                fs_err::write(&path, "original")?;
            }
            let state = EditState::new([path.clone()])?;
            fs_err::write(&path, "external")?;
            state.finish(false);
            assert_eq!(fs_err::read_to_string(&path)?, "external");
            assert!(recovery_files(directory.path())?.is_empty());
        }
        Ok(())
    }

    #[test]
    fn external_changes_keep_the_original_in_recovery() -> Result<()> {
        for replacement in [false, true] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("pyproject.toml");
            fs_err::write(&path, "original")?;
            let state = EditState::new([path.clone()])?;
            state.write_file(&path, b"edited")?;
            let expected = if replacement {
                // Identical bytes on a replacement inode still belong to the external writer.
                let replacement = directory.path().join("replacement");
                fs_err::write(&replacement, "edited")?;
                fs_err::rename(replacement, &path)?;
                "edited"
            } else {
                fs_err::write(&path, "external")?;
                "external"
            };
            state.finish(false);
            assert_eq!(fs_err::read_to_string(&path)?, expected);
            let recovery = recovery_files(directory.path())?;
            assert_eq!(recovery.len(), 1);
            assert_eq!(fs_err::read_to_string(&recovery[0])?, "original");
        }
        Ok(())
    }

    #[test]
    fn external_deletion_is_not_undone() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let state = EditState::new([path.clone()])?;
        state.write_file(&path, b"edited")?;
        fs_err::remove_file(&path)?;
        state.finish(false);
        assert!(!path.exists());
        let recovery = recovery_files(directory.path())?;
        assert_eq!(recovery.len(), 1);
        assert_eq!(fs_err::read_to_string(&recovery[0])?, "original");
        Ok(())
    }

    #[test]
    fn later_publications_reject_external_changes() -> Result<()> {
        for owned in [false, true] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("uv.lock");
            let state = EditState::new([path.clone()])?;
            if owned {
                state.write_file(&path, b"edited")?;
            }
            fs_err::write(&path, "external")?;
            assert!(state.write_file(&path, b"later").is_err());
            state.finish(false);
            assert_eq!(fs_err::read_to_string(&path)?, "external");
            // The snapshot was absent, so there is no original content to save.
            assert!(recovery_files(directory.path())?.is_empty());
        }
        Ok(())
    }

    #[test]
    fn repeated_and_empty_owned_writes_restore_originals() -> Result<()> {
        for existed in [false, true] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("script.py");
            if existed {
                fs_err::write(&path, "original")?;
            }
            let state = EditState::new([path.clone()])?;
            state.write_file(&path, b"first")?;
            state.write_file(&path, b"second")?;
            state.write_file(&path, b"")?;
            state.finish(false);
            assert_eq!(
                read_file(&path)?.map(|file| file.contents),
                existed.then(|| b"original".to_vec())
            );
            assert!(recovery_files(directory.path())?.is_empty());
        }
        Ok(())
    }

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
            move || {
                state.write(|files| {
                    entered.send(()).map_err(io::Error::other)?;
                    resume.recv().map_err(io::Error::other)?;
                    let [file] = files else {
                        return Err(io::Error::other("expected one tracked file"));
                    };
                    file.write(b"published")
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
            .write_file(&path, b"late")
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
        state.write_file(&path, b"edited")?;
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
                state.write_file(&path, b"edited")?;
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
                    read_file(&path)?.map(|file| file.contents),
                    expected,
                    "existed={existed}, commit={commit}"
                );
            }
        }
        Ok(())
    }
}
