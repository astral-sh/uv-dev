use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};

use uv_cache::Cache;
use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileMode, Simplified, normalize_path};
use uv_scripts::Pep723Script;
use uv_workspace::{
    DiscoveryOptions, MemberDiscovery, VirtualProject, Workspace, WorkspaceCache, WorkspaceError,
};

/// The caller's accepted project roots, including whether dependency-group-only roots are valid.
#[derive(Clone, Copy)]
pub enum MetadataDiscovery {
    Project,
    Workspace,
}

impl MetadataDiscovery {
    async fn root(
        self,
        directory: &Path,
        options: &DiscoveryOptions,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
    ) -> Result<PathBuf, WorkspaceError> {
        match self {
            Self::Project => VirtualProject::discover(directory, options, cache, workspace_cache)
                .await
                .map(|project| project.workspace().install_path().clone()),
            Self::Workspace => Workspace::discover(directory, options, cache, workspace_cache)
                .await
                .map(|workspace| workspace.install_path().clone()),
        }
    }
}

/// Own the metadata resource independently of the interpreter or environment used to resolve it.
#[must_use]
#[derive(Clone)]
pub struct MetadataLock {
    file: Arc<LockedFile>,
    resource: PathBuf,
    kind: &'static str,
}

impl MetadataLock {
    /// Claim the workspace before reading configuration or discovering all of its members.
    pub async fn discover(
        directory: &Path,
        cache: &Cache,
        workspace_cache: &mut WorkspaceCache,
        members: MemberDiscovery,
        discovery: MetadataDiscovery,
    ) -> Result<Option<Self>> {
        let directory = std::path::absolute(directory)?;
        let directory = normalize_path(&directory);
        let options = DiscoveryOptions {
            members: MemberDiscovery::None,
            ..DiscoveryOptions::default()
        };
        loop {
            let Ok(root) = discovery
                .root(&directory, &options, cache, &WorkspaceCache::default())
                .await
            else {
                return Ok(None);
            };
            let root = fs_err::canonicalize(root)?;
            let lock = Self::workspace(&root).await?;
            let fresh_cache = WorkspaceCache::default();
            let discovered = discovery
                .root(
                    &directory,
                    &DiscoveryOptions {
                        members: members.clone(),
                        ..DiscoveryOptions::default()
                    },
                    cache,
                    &fresh_cache,
                )
                .await;
            match discovered {
                Ok(current) if fs_err::canonicalize(&current)? != root => {
                    // Drop this guard before retrying admission for the new workspace root.
                }
                Ok(_) | Err(_) => {
                    // Configuration and command discovery share the admitted result, including
                    // discovery errors. Invalid workspaces are reported by the command itself.
                    *workspace_cache = fresh_cache;
                    return Ok(Some(lock));
                }
            }
        }
    }

    fn check_resource(&self, path: &Path, kind: &str) -> Result<()> {
        ensure!(
            self.kind == kind && self.resource == fs_err::canonicalize(path)?,
            "Metadata destination changed after settings were read; run the command again"
        );
        Ok(())
    }

    async fn workspace(root: &Path) -> Result<Self> {
        Self::acquire(&fs_err::canonicalize(root)?, "workspace").await
    }

    /// Claim a script or its prospective filename in an existing parent directory.
    pub async fn script(path: &Path) -> Result<Self> {
        let path = match fs_err::canonicalize(path) {
            Ok(path) => path,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let parent = path
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                let filename = path.file_name().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "script requires a filename")
                })?;
                fs_err::canonicalize(parent)?.join(filename)
            }
            Err(err) => return Err(err.into()),
        };
        Self::acquire(&path, "script").await
    }

    async fn acquire(path: &Path, kind: &'static str) -> Result<Self> {
        let lock = LockedFile::acquire(
            std::env::temp_dir().join(format!("uv-{kind}-metadata-{}.lock", cache_digest(&path))),
            LockedFileMode::Exclusive,
            path.simplified_display(),
        )
        .await?;
        Ok(Self {
            file: Arc::new(lock),
            resource: path.to_path_buf(),
            kind,
        })
    }

    /// Reuse the project discovered while holding its metadata resource.
    pub fn admitted_project(
        admission: Option<Self>,
        project: VirtualProject,
    ) -> Result<(VirtualProject, Self)> {
        let lock = admission
            .context("Workspace changed before metadata admission; run the command again")?;
        lock.check_resource(project.workspace().install_path(), "workspace")?;
        Ok((project, lock))
    }

    /// Reuse the workspace discovered while holding its metadata resource.
    pub fn admitted_workspace(
        admission: Option<Self>,
        workspace: Arc<Workspace>,
    ) -> Result<(Arc<Workspace>, Self)> {
        let lock = admission
            .context("Workspace changed before metadata admission; run the command again")?;
        lock.check_resource(workspace.install_path(), "workspace")?;
        Ok((workspace, lock))
    }

    /// Read a script's current metadata after claiming the script, including an absent metadata tag.
    pub async fn read_script(
        admission: Option<Self>,
        path: &Path,
    ) -> Result<(Option<Pep723Script>, Self)> {
        let lock =
            admission.context("Script changed before metadata admission; run the command again")?;
        lock.check_resource(path, "script")?;
        Ok((Pep723Script::read(path).await?, lock))
    }

    /// Retain admission in the worker that actually publishes the lockfile, including cancellation.
    ///
    /// `None` rejects publication. Read-only lock operations never invoke their writer.
    pub async fn write_lockfile(
        lock: Option<&Self>,
        path: PathBuf,
        contents: String,
    ) -> io::Result<()> {
        let lock =
            lock.ok_or_else(|| io::Error::other("lockfile publication requires a metadata lock"))?;
        lock.write_file(path, contents).await
    }

    /// Write metadata while keeping the resource locked until the worker finishes.
    pub async fn write_file(&self, path: PathBuf, contents: String) -> io::Result<()> {
        self.clone()
            .write_owned(move || {
                // Report open and write failures with the lockfile publication's write context.
                #[expect(clippy::disallowed_methods)]
                std::fs::write(&path, contents).map_err(|cause| {
                    io::Error::new(cause.kind(), MetadataWriteError { path, cause })
                })
            })
            .await
    }

    /// Create metadata without overwriting a file published by another creator.
    pub async fn create_file(&self, path: PathBuf, contents: String) -> io::Result<()> {
        self.clone()
            .write_owned(move || {
                fs_err::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?
                    .write_all(contents.as_bytes())
            })
            .await
    }

    async fn write_owned(
        self,
        write: impl FnOnce() -> io::Result<()> + Send + 'static,
    ) -> io::Result<()> {
        let lock = self.file;
        tokio::task::spawn_blocking(move || {
            let _lock = lock;
            write()
        })
        .await
        .map_err(io::Error::other)?
    }
}

#[derive(Debug, thiserror::Error)]
#[error("failed to write to file `{}`: {cause}", path.display())]
struct MetadataWriteError {
    path: PathBuf,
    cause: io::Error,
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::mpsc;
    use std::time::Duration;

    use uv_cache_key::cache_digest;
    use uv_fs::{LockedFile, LockedFileMode};

    use super::MetadataLock;

    #[tokio::test]
    async fn cancelled_write_retains_metadata_admission() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let root = fs_err::canonicalize(directory.path())?;
        let path = root.join("uv.lock");
        let lock = MetadataLock::workspace(&root).await?;
        let (started, started_receiver) = tokio::sync::oneshot::channel();
        let (release, release_receiver) = mpsc::channel();
        let (finished, finished_receiver) = tokio::sync::oneshot::channel();
        let write_path = path.clone();
        let task = tokio::spawn(lock.clone().write_owned(move || {
            started
                .send(())
                .map_err(|()| io::Error::other("write observer closed"))?;
            release_receiver.recv().map_err(io::Error::other)?;
            let result = fs_err::write(write_path, "completed lockfile");
            let _ = finished.send(());
            result
        }));
        tokio::time::timeout(Duration::from_secs(30), started_receiver).await??;
        task.abort();
        assert!(
            task.await
                .expect_err("cancelled write waiter")
                .is_cancelled()
        );
        drop(lock);

        let contended = LockedFile::acquire_no_wait(
            std::env::temp_dir().join(format!(
                "uv-workspace-metadata-{}.lock",
                cache_digest(&root)
            )),
            LockedFileMode::Exclusive,
            "metadata writer",
        )
        .is_none();
        release.send(())?;
        tokio::time::timeout(Duration::from_secs(30), finished_receiver).await??;
        let _next =
            tokio::time::timeout(Duration::from_secs(30), MetadataLock::workspace(&root)).await??;
        assert!(contended);
        assert_eq!(fs_err::read_to_string(path)?, "completed lockfile");
        Ok(())
    }

    #[tokio::test]
    async fn creation_does_not_overwrite_a_competing_file() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("script.py");
        let lock = MetadataLock::script(&path).await?;
        fs_err::write(&path, "competing creator")?;
        let error = lock
            .create_file(path.clone(), "new metadata".to_owned())
            .await
            .expect_err("destination was created after admission");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs_err::read_to_string(path)?, "competing creator");
        Ok(())
    }
}
