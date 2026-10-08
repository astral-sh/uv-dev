use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

use uv_cache::Cache;
use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileMode, Simplified};
use uv_normalize::PackageName;
use uv_scripts::Pep723Script;
use uv_workspace::{DiscoveryOptions, VirtualProject, Workspace, WorkspaceCache};

/// Own the metadata resource independently of the interpreter or environment used to resolve it.
#[must_use]
pub struct MetadataLock(Arc<LockedFile>);

impl MetadataLock {
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

    async fn acquire(path: &Path, kind: &str) -> Result<Self> {
        let lock = LockedFile::acquire(
            std::env::temp_dir().join(format!("uv-{kind}-metadata-{}.lock", cache_digest(&path))),
            LockedFileMode::Exclusive,
            path.simplified_display(),
        )
        .await?;
        Ok(Self(Arc::new(lock)))
    }

    /// Reload a discovered project after admission, without reusing a pre-admission workspace cache.
    pub async fn project(
        mut project: VirtualProject,
        project_dir: &Path,
        package: Option<&PackageName>,
        options: &DiscoveryOptions,
        cache: &Cache,
    ) -> Result<(VirtualProject, Self, WorkspaceCache)> {
        loop {
            let root = fs_err::canonicalize(project.workspace().install_path())?;
            let lock = Self::workspace(&root).await?;
            let workspace_cache = WorkspaceCache::default();
            project = if let Some(package) = package {
                VirtualProject::discover_with_package(
                    project_dir,
                    options,
                    cache,
                    &workspace_cache,
                    package.clone(),
                )
                .await?
            } else {
                VirtualProject::discover(project_dir, options, cache, &workspace_cache).await?
            };
            if fs_err::canonicalize(project.workspace().install_path())? == root {
                return Ok((project, lock, workspace_cache));
            }
            // Membership may have changed while waiting. Release this workspace before claiming
            // the newly discovered one, rather than nesting resource locks in an arbitrary order.
        }
    }

    /// Reload a workspace from the original directory while holding its metadata resource.
    pub async fn reload_workspace(
        mut workspace: Arc<Workspace>,
        directory: &Path,
        options: &DiscoveryOptions,
        cache: &Cache,
    ) -> Result<(Arc<Workspace>, Self, WorkspaceCache)> {
        loop {
            let root = fs_err::canonicalize(workspace.install_path())?;
            let lock = Self::workspace(&root).await?;
            let workspace_cache = WorkspaceCache::default();
            workspace = Workspace::discover(directory, options, cache, &workspace_cache).await?;
            if fs_err::canonicalize(workspace.install_path())? == root {
                return Ok((workspace, lock, workspace_cache));
            }
        }
    }

    /// Read a script's current metadata after claiming the script, including an absent metadata tag.
    pub async fn read_script(path: &Path) -> Result<(Option<Pep723Script>, Self)> {
        let lock = Self::script(path).await?;
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
        Self(Arc::clone(&self.0))
            .write_owned(move || fs_err::write(path, contents))
            .await
    }

    /// Create metadata without overwriting a file published by another creator.
    pub async fn create_file(&self, path: PathBuf, contents: String) -> io::Result<()> {
        Self(Arc::clone(&self.0))
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
        let lock = self.0;
        tokio::task::spawn_blocking(move || {
            let _lock = lock;
            write()
        })
        .await
        .map_err(io::Error::other)?
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, mpsc};
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
        let task = tokio::spawn(MetadataLock(Arc::clone(&lock.0)).write_owned(move || {
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
