use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};

use uv_cache::Cache;
use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileMode, Simplified, normalize_path};
use uv_normalize::PackageName;
use uv_scripts::Pep723Script;
use uv_workspace::{
    DiscoveryOptions, MemberDiscovery, VirtualProject, Workspace, WorkspaceCache, WorkspaceError,
    WorkspaceErrorKind,
};

/// The caller's accepted project roots, including whether dependency-group-only roots are valid.
#[derive(Clone, Copy)]
pub enum MetadataDiscovery<'a> {
    Project,
    ProjectEdit(Option<&'a PackageName>),
    Workspace,
}

impl MetadataDiscovery<'_> {
    async fn root(
        self,
        directory: &Path,
        options: &DiscoveryOptions,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
    ) -> Result<PathBuf, WorkspaceError> {
        // Workspace discovery reports deprecations and retains root-level sources and indexes.
        // Dependency-group-only roots additionally require project discovery.
        match (
            self,
            Workspace::discover(directory, options, cache, workspace_cache).await,
        ) {
            (Self::Project | Self::ProjectEdit(_) | Self::Workspace, Ok(workspace)) => {
                Ok(workspace.install_path().clone())
            }
            (Self::Project | Self::ProjectEdit(_), Err(error))
                if matches!(
                    error.as_ref(),
                    WorkspaceErrorKind::MissingProject(_) | WorkspaceErrorKind::NonWorkspace(_)
                ) =>
            {
                VirtualProject::discover(directory, options, cache, &WorkspaceCache::default())
                    .await
                    .map(|project| project.workspace().install_path().clone())
            }
            (Self::Project | Self::ProjectEdit(_) | Self::Workspace, Err(error)) => Err(error),
        }
    }
}

/// Own the metadata resource independently of the interpreter or environment used to resolve it.
#[must_use]
#[derive(Clone)]
pub struct MetadataLock {
    files: BTreeMap<PathBuf, Arc<LockedFile>>,
    project: Option<PathBuf>,
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
        discovery: MetadataDiscovery<'_>,
    ) -> Result<Option<Self>> {
        let directory = std::path::absolute(directory)?;
        let directory = normalize_path(&directory);
        let options = DiscoveryOptions {
            members: MemberDiscovery::None,
            suppress_warnings: true,
            ..DiscoveryOptions::default()
        };
        let mut resources = BTreeSet::new();
        loop {
            let Ok(root) = discovery
                .root(&directory, &options, cache, &WorkspaceCache::default())
                .await
            else {
                return Ok(None);
            };
            let root = fs_err::canonicalize(root)?;
            resources.insert(root.clone());
            let mut files = BTreeMap::new();
            for resource in &resources {
                files.insert(
                    resource.clone(),
                    Arc::new(Self::acquire_file(resource, "workspace").await?),
                );
            }
            let mut lock = Self {
                files,
                project: None,
                resource: root.clone(),
                kind: "workspace",
            };
            let fresh_cache = WorkspaceCache::default();
            let discovered = discovery
                .root(
                    &directory,
                    &DiscoveryOptions {
                        members: members.clone(),
                        suppress_warnings: members == MemberDiscovery::None,
                        ..DiscoveryOptions::default()
                    },
                    cache,
                    &fresh_cache,
                )
                .await;
            let mut current_resources = BTreeSet::new();
            if let Ok(current) = discovered {
                current_resources.insert(fs_err::canonicalize(current)?);
                if let MetadataDiscovery::ProjectEdit(package) = discovery {
                    // Project selection can name a member outside the workspace's directory tree.
                    // Its physical metadata resource also admits writers invoked in that member.
                    let project_cache = WorkspaceCache::default();
                    let selected = if let Some(package) = package {
                        VirtualProject::discover_with_package(
                            &directory,
                            &DiscoveryOptions::default(),
                            cache,
                            &fresh_cache,
                            package.clone(),
                        )
                        .await
                    } else {
                        VirtualProject::discover(
                            &directory,
                            &DiscoveryOptions::default(),
                            cache,
                            // A group-only project must not populate the strict workspace cache
                            // used to select filesystem configuration.
                            if Workspace::discover(
                                &directory,
                                &DiscoveryOptions::default(),
                                cache,
                                &fresh_cache,
                            )
                            .await
                            .is_ok()
                            {
                                &fresh_cache
                            } else {
                                &project_cache
                            },
                        )
                        .await
                    };
                    if let Ok(project) = selected {
                        let selected = fs_err::canonicalize(project.root())?;
                        current_resources.insert(selected.clone());
                        lock.project = Some(selected);
                    }
                }
                if current_resources != resources {
                    resources = current_resources;
                    // Release the entire ordered set before retrying, while no settings or
                    // command metadata have been derived from the admitted discovery.
                    continue;
                }
            }
            *workspace_cache = fresh_cache;
            if matches!(discovery, MetadataDiscovery::ProjectEdit(_)) && lock.project.is_none() {
                // A later command may rediscover through a different cache. A failed selection
                // must not become a valid edit admission without its physical project guard.
                return Ok(None);
            }
            return Ok(Some(lock));
        }
    }

    fn check_resource(&self, path: &Path, kind: &str) -> Result<()> {
        ensure!(
            self.kind == kind && self.resource == fs_err::canonicalize(path)?,
            "Metadata destination changed after settings were read; run the command again"
        );
        Ok(())
    }

    #[cfg(test)]
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

    async fn acquire_file(path: &Path, kind: &'static str) -> Result<LockedFile> {
        Ok(LockedFile::acquire(
            std::env::temp_dir().join(format!("uv-{kind}-metadata-{}.lock", cache_digest(&path))),
            LockedFileMode::Exclusive,
            path.simplified_display(),
        )
        .await?)
    }

    async fn acquire(path: &Path, kind: &'static str) -> Result<Self> {
        let lock = Self::acquire_file(path, kind).await?;
        Ok(Self {
            files: BTreeMap::from([(path.to_path_buf(), Arc::new(lock))]),
            project: None,
            resource: path.to_path_buf(),
            kind,
        })
    }

    /// Admit a prospective member and its previous workspace before resolving its metadata.
    ///
    /// The original resources stay locked because settings have already been read. A contended
    /// lower key requires a fresh command rather than introducing a reverse-order wait.
    pub async fn admit_members(&mut self, paths: &[PathBuf], cache: &Cache) -> Result<()> {
        loop {
            let mut resources = BTreeSet::new();
            for path in paths {
                resources.insert(fs_err::canonicalize(path)?);
                if let Ok(workspace) = Workspace::discover(
                    path,
                    &DiscoveryOptions {
                        members: MemberDiscovery::None,
                        suppress_warnings: true,
                        ..DiscoveryOptions::default()
                    },
                    cache,
                    &WorkspaceCache::default(),
                )
                .await
                {
                    resources.insert(fs_err::canonicalize(workspace.install_path())?);
                }
            }
            if resources
                .iter()
                .all(|resource| self.files.contains_key(resource))
            {
                return Ok(());
            }
            for resource in resources {
                if self.files.contains_key(&resource) {
                    continue;
                }
                let file = if self
                    .files
                    .last_key_value()
                    .is_some_and(|(held, _)| resource < *held)
                {
                    LockedFile::acquire_no_wait(
                    std::env::temp_dir().join(format!("uv-workspace-metadata-{}.lock", cache_digest(&resource))),
                    LockedFileMode::Exclusive, resource.simplified_display(),
                ).with_context(|| format!(
                    "Could not immediately acquire metadata for `{}` while admitting a workspace member; run the command again",
                    resource.user_display(),
                ))?
                } else {
                    Self::acquire_file(&resource, "workspace").await?
                };
                self.files.insert(resource, Arc::new(file));
            }
            // A candidate may have joined another workspace while admission was queued. Re-read
            // its roots with the physical project guard held before deriving build metadata.
        }
    }

    /// Reuse the project discovered while holding its metadata resource.
    pub fn admitted_project(
        admission: Option<Self>,
        project: VirtualProject,
    ) -> Result<(VirtualProject, Self)> {
        let lock = admission
            .context("Workspace changed before metadata admission; run the command again")?;
        lock.check_resource(project.workspace().install_path(), "workspace")?;
        if let Some(selected) = &lock.project {
            ensure!(
                *selected == fs_err::canonicalize(project.root())?,
                "Selected project changed after settings were read; run the command again"
            );
        }
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
        tokio::task::spawn_blocking(move || {
            let _lock = self;
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

    use uv_cache::Cache;
    use uv_cache_key::cache_digest;
    use uv_fs::{LockedFile, LockedFileMode};

    use uv_normalize::PackageName;
    use uv_workspace::{DiscoveryOptions, MemberDiscovery, VirtualProject, WorkspaceCache};

    use super::{MetadataDiscovery, MetadataLock};

    #[tokio::test]
    async fn cancelled_write_retains_metadata_admission() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let root = fs_err::canonicalize(directory.path())?;
        let path = root.join("uv.lock");
        let mut lock = MetadataLock::workspace(&root).await?;
        let member = root.join("member");
        fs_err::create_dir(&member)?;
        lock.admit_members(std::slice::from_ref(&member), &Cache::temp()?)
            .await?;
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
        let member_contended = LockedFile::acquire_no_wait(
            std::env::temp_dir().join(format!(
                "uv-workspace-metadata-{}.lock",
                cache_digest(&member)
            )),
            LockedFileMode::Exclusive,
            "member metadata writer",
        )
        .is_none();
        release.send(())?;
        tokio::time::timeout(Duration::from_secs(30), finished_receiver).await??;
        let _next =
            tokio::time::timeout(Duration::from_secs(30), MetadataLock::workspace(&root)).await??;
        assert!(contended);
        assert!(member_contended);
        assert_eq!(fs_err::read_to_string(path)?, "completed lockfile");
        Ok(())
    }

    #[tokio::test]
    async fn failed_member_discovery_cannot_authorize_a_later_edit() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("workspace");
        let member = directory.path().join("external-member");
        fs_err::create_dir(&root)?;
        fs_err::create_dir(&member)?;
        fs_err::write(
            root.join("pyproject.toml"),
            format!(
                "[tool.uv.workspace]\nmembers = [{}]\n",
                toml::Value::String(member.to_string_lossy().into_owned()),
            ),
        )?;
        let manifest = member.join("pyproject.toml");
        fs_err::write(&manifest, "[invalid")?;
        let _writer = MetadataLock::workspace(&member).await?;
        let cache = Cache::temp()?;
        let package: PackageName = "dep".parse()?;
        let admission = MetadataLock::discover(
            &root,
            &cache,
            &mut WorkspaceCache::default(),
            MemberDiscovery::All,
            MetadataDiscovery::ProjectEdit(Some(&package)),
        )
        .await?;

        fs_err::write(&manifest, "[project]\nname = 'dep'\nversion = '1.0.0'\n")?;
        // Filesystem configuration can select a new cache directory after initial admission.
        let project = VirtualProject::discover_with_package(
            &root,
            &DiscoveryOptions::default(),
            &cache,
            &WorkspaceCache::default(),
            package,
        )
        .await?;
        assert!(MetadataLock::admitted_project(admission, project).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn contended_lower_member_requires_fresh_admission() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let earlier = directory.path().join("a-member");
        let later = directory.path().join("z-workspace");
        fs_err::create_dir(&earlier)?;
        fs_err::create_dir(&later)?;
        let _member = MetadataLock::workspace(&earlier).await?;
        let mut lock = MetadataLock::workspace(&later).await?;
        let error = tokio::time::timeout(
            Duration::from_secs(1),
            lock.admit_members(&[earlier], &Cache::temp()?),
        )
        .await?
        .expect_err("a reverse-order wait requires a fresh command");
        assert!(error.to_string().contains("run the command again"));
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
