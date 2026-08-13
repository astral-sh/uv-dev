use std::fmt::{Display, Formatter};
use std::io;
use std::io::Write;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use rustc_hash::FxHashMap;
use tracing::{debug, trace, warn};

use uv_cache_info::Timestamp;
use uv_fs::{LockedFile, LockedFileError, LockedFileMode, Simplified, cachedir, directories};
use uv_normalize::PackageName;
use uv_pypi_types::ResolutionMetadata;

pub use crate::by_timestamp::CachedByTimestamp;
#[cfg(feature = "clap")]
pub use crate::cli::CacheArgs;
use crate::removal::Remover;
pub use crate::removal::{Removal, RemovalAccounting};
pub use crate::wheel::WheelCache;
use crate::wheel::WheelCacheKind;
pub use archive::{ArchiveFileId, ArchiveId};

mod archive;
mod by_timestamp;
#[cfg(feature = "clap")]
mod cli;
mod removal;
mod wheel;

/// The version of the archive bucket.
///
/// Keep this value in sync with the version in [`CacheBucket::to_str`].
pub const ARCHIVE_VERSION: u8 = 0;

/// An error that occurs when locking a cache entry or shard.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Failed to initialize cache at `{}`", _0.user_display())]
    Init(PathBuf, #[source] io::Error),
    #[error("Could not make the path absolute")]
    Absolute(#[source] io::Error),
    #[error("Could not acquire lock")]
    Acquire(#[from] LockedFileError),
}

/// A cache entry that might not exist yet.
#[derive(Debug, Clone)]
pub struct CacheEntry(PathBuf);

impl CacheEntry {
    /// Create a new [`CacheEntry`] from a directory and a file name.
    pub fn new(dir: impl Into<PathBuf>, file: impl AsRef<Path>) -> Self {
        Self(dir.into().join(file))
    }

    /// Create a new [`CacheEntry`] from a path.
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// Return the cache entry's parent directory.
    pub fn shard(&self) -> CacheShard {
        CacheShard(self.dir().to_path_buf())
    }

    /// Convert the [`CacheEntry`] into a [`PathBuf`].
    #[inline]
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }

    /// Return the path to the [`CacheEntry`].
    #[inline]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Return the cache entry's parent directory.
    #[inline]
    pub fn dir(&self) -> &Path {
        self.0.parent().expect("Cache entry has no parent")
    }

    /// Create a new [`CacheEntry`] with the given file name.
    #[must_use]
    pub fn with_file(&self, file: impl AsRef<Path>) -> Self {
        Self(self.dir().join(file))
    }

    /// Acquire the [`CacheEntry`] as an exclusive lock.
    pub async fn lock(&self) -> Result<LockedFile, Error> {
        fs_err::create_dir_all(self.dir())?;
        Ok(LockedFile::acquire(
            self.path(),
            LockedFileMode::Exclusive,
            self.path().display(),
        )
        .await?)
    }
}

impl AsRef<Path> for CacheEntry {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// A subdirectory within the cache.
#[derive(Debug, Clone)]
pub struct CacheShard(PathBuf);

impl CacheShard {
    /// Return a [`CacheEntry`] within this shard.
    pub fn entry(&self, file: impl AsRef<Path>) -> CacheEntry {
        CacheEntry::new(&self.0, file)
    }

    /// Return a [`CacheShard`] within this shard.
    #[must_use]
    pub fn shard(&self, dir: impl AsRef<Path>) -> Self {
        Self(self.0.join(dir.as_ref()))
    }

    /// Acquire the cache entry as an exclusive lock.
    pub async fn lock(&self) -> Result<LockedFile, Error> {
        fs_err::create_dir_all(self.as_ref())?;
        Ok(LockedFile::acquire(
            self.join(".lock"),
            LockedFileMode::Exclusive,
            self.display(),
        )
        .await?)
    }

    /// Return the [`CacheShard`] as a [`PathBuf`].
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

impl AsRef<Path> for CacheShard {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Deref for CacheShard {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// The main cache abstraction.
///
/// An active cache holds a shared read lock to prevent cache cleaning.
#[derive(Debug, Clone)]
pub struct Cache {
    /// The cache directory.
    root: PathBuf,
    /// The refresh strategy for cache reads.
    refresh: Refresh,
    /// A temporary cache directory, if the user requested `--no-cache`.
    ///
    /// Keep the temporary directory until the operation ends. Drop it after the operation.
    temp_dir: Option<Arc<tempfile::TempDir>>,
    /// Prevent `uv cache` commands from removing entries that another uv process uses.
    lock_file: Option<Arc<LockedFile>>,
    /// The storage accounting used when removing cache entries.
    removal_accounting: RemovalAccounting,
}

impl Cache {
    /// Create a persistent cache configuration for `root`.
    pub fn from_path(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            refresh: Refresh::None(Timestamp::now()),
            temp_dir: None,
            lock_file: None,
            removal_accounting: RemovalAccounting::Coarse,
        }
    }

    /// Create a temporary cache directory.
    pub fn temp() -> Result<Self, io::Error> {
        let temp_dir = tempfile::tempdir()?;
        Ok(Self {
            root: temp_dir.path().to_path_buf(),
            refresh: Refresh::None(Timestamp::now()),
            temp_dir: Some(Arc::new(temp_dir)),
            lock_file: None,
            removal_accounting: RemovalAccounting::Coarse,
        })
    }

    /// Set the [`Refresh`] policy for the cache.
    #[must_use]
    pub fn with_refresh(self, refresh: Refresh) -> Self {
        Self { refresh, ..self }
    }

    /// Set the storage accounting used when removing cache entries.
    ///
    /// Use [`RemovalAccounting::Coarse`] if fine-grained accounting is unsupported.
    #[must_use]
    pub fn with_removal_accounting(self, removal_accounting: RemovalAccounting) -> Self {
        let removal_accounting = match removal_accounting {
            RemovalAccounting::Fine if !uv_fs::supports_fine_grained_accounting() => {
                RemovalAccounting::Coarse
            }
            removal_accounting => removal_accounting,
        };
        Self {
            removal_accounting,
            ..self
        }
    }

    /// Create an empty removal summary using the cache's configured accounting.
    pub fn removal(&self) -> Removal {
        Removal::new(self.removal_accounting)
    }

    /// Acquire a lock that permits cache entries to be removed.
    pub async fn with_exclusive_lock(self) -> Result<Self, LockedFileError> {
        let Self {
            root,
            refresh,
            temp_dir,
            lock_file,
            removal_accounting,
        } = self;

        // Release the existing lock to prevent a deadlock from a cloned cache.
        if let Some(lock_file) = lock_file {
            drop(
                Arc::try_unwrap(lock_file).expect(
                    "cloning the cache before acquiring an exclusive lock causes a deadlock",
                ),
            );
        }
        let lock_file = LockedFile::acquire(
            root.join(".lock"),
            LockedFileMode::Exclusive,
            root.simplified_display(),
        )
        .await?;

        Ok(Self {
            root,
            refresh,
            temp_dir,
            lock_file: Some(Arc::new(lock_file)),
            removal_accounting,
        })
    }

    /// Acquire a lock that permits cache entries to be removed, if available.
    ///
    /// If the lock is not immediately available, return [`Err`] with the original cache.
    pub fn with_exclusive_lock_no_wait(self) -> Result<Self, Self> {
        let Self {
            root,
            refresh,
            temp_dir,
            lock_file,
            removal_accounting,
        } = self;

        match LockedFile::acquire_no_wait(
            root.join(".lock"),
            LockedFileMode::Exclusive,
            root.simplified_display(),
        ) {
            Some(lock_file) => Ok(Self {
                root,
                refresh,
                temp_dir,
                lock_file: Some(Arc::new(lock_file)),
                removal_accounting,
            }),
            None => Err(Self {
                root,
                refresh,
                temp_dir,
                lock_file,
                removal_accounting,
            }),
        }
    }

    /// Return the root of the cache.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the directory for a specific cache bucket.
    pub fn bucket(&self, cache_bucket: CacheBucket) -> PathBuf {
        self.root.join(cache_bucket.to_str())
    }

    /// Return a shard in the cache.
    pub fn shard(&self, cache_bucket: CacheBucket, dir: impl AsRef<Path>) -> CacheShard {
        CacheShard(self.bucket(cache_bucket).join(dir.as_ref()))
    }

    /// Return an entry in the cache.
    pub fn entry(
        &self,
        cache_bucket: CacheBucket,
        dir: impl AsRef<Path>,
        file: impl AsRef<Path>,
    ) -> CacheEntry {
        CacheEntry::new(self.bucket(cache_bucket).join(dir), file)
    }

    /// Return the path to an archive in the cache.
    pub fn archive(&self, id: &ArchiveId) -> PathBuf {
        self.bucket(CacheBucket::Archive).join(id)
    }

    /// Return the path to an archive file in the cache.
    pub fn archive_file(&self, id: &ArchiveFileId) -> PathBuf {
        self.bucket(CacheBucket::Files).join(id)
    }

    /// Create a temporary directory for a Python virtual environment.
    pub fn venv_dir(&self) -> io::Result<tempfile::TempDir> {
        fs_err::create_dir_all(self.bucket(CacheBucket::Builds))?;
        tempfile::tempdir_in(self.bucket(CacheBucket::Builds))
    }

    /// Create a temporary directory for PEP 517 source distribution builds.
    pub fn build_dir(&self) -> io::Result<tempfile::TempDir> {
        fs_err::create_dir_all(self.bucket(CacheBucket::Builds))?;
        tempfile::tempdir_in(self.bucket(CacheBucket::Builds))
    }

    /// Return `true` if the [`Refresh`] policy requires this package to be revalidated.
    pub fn must_revalidate_package(&self, package: &PackageName) -> bool {
        match &self.refresh {
            Refresh::None(_) => false,
            Refresh::All(_) => true,
            Refresh::Packages(packages, _, _) => packages.contains(package),
        }
    }

    /// Return `true` if the [`Refresh`] policy requires this path to be revalidated.
    pub fn must_revalidate_path(&self, path: &Path) -> bool {
        match &self.refresh {
            Refresh::None(_) => false,
            Refresh::All(_) => true,
            Refresh::Packages(_, paths, _) => paths
                .iter()
                .any(|target| same_file::is_same_file(path, target).unwrap_or(false)),
        }
    }

    /// Return the [`Freshness`] of a cache entry under the [`Refresh`] policy.
    ///
    /// An entry is fresh if it was created after the cache was initialized.
    /// An entry is also fresh if the [`Refresh`] policy does not require revalidation.
    pub fn freshness(
        &self,
        entry: &CacheEntry,
        package: Option<&PackageName>,
        path: Option<&Path>,
    ) -> io::Result<Freshness> {
        // Get the cutoff timestamp if the refresh policy requires one.
        let timestamp = match &self.refresh {
            Refresh::None(_) => return Ok(Freshness::Fresh),
            Refresh::All(timestamp) => timestamp,
            Refresh::Packages(packages, paths, timestamp) => {
                if package.is_none_or(|package| packages.contains(package))
                    || path.is_some_and(|path| {
                        paths
                            .iter()
                            .any(|target| same_file::is_same_file(path, target).unwrap_or(false))
                    })
                {
                    timestamp
                } else {
                    return Ok(Freshness::Fresh);
                }
            }
        };

        match fs_err::metadata(entry.path()) {
            Ok(metadata) => {
                if Timestamp::from_metadata(&metadata) >= *timestamp {
                    Ok(Freshness::Fresh)
                } else {
                    Ok(Freshness::Stale)
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Freshness::Missing),
            Err(err) => Err(err),
        }
    }

    /// Move a temporary directory to the artifact store and return its unique ID.
    pub async fn persist(
        &self,
        temp_dir: impl AsRef<Path>,
        path: impl AsRef<Path>,
    ) -> io::Result<ArchiveId> {
        // Create a unique ID for the artifact.
        let id = ArchiveId::new();

        // Move the temporary directory into the directory store.
        let archive_entry = self.entry(CacheBucket::Archive, "", &id);
        fs_err::create_dir_all(archive_entry.dir())?;
        uv_fs::rename_with_retry(temp_dir.as_ref(), archive_entry.path()).await?;

        // Create a symlink to the directory store.
        fs_err::create_dir_all(path.as_ref().parent().expect("Cache entry to have parent"))?;
        self.create_link(&id, path.as_ref())?;

        Ok(id)
    }

    /// Persist a temporary directory to the artifact store under a caller-selected ID.
    ///
    /// If another writer has already persisted the same ID, discard `temp_dir` and link `path` to
    /// the existing archive entry. The ID must therefore uniquely identify the directory contents.
    pub async fn persist_with_id(
        &self,
        temp_dir: tempfile::TempDir,
        path: impl AsRef<Path>,
        id: ArchiveId,
    ) -> io::Result<ArchiveId> {
        // Move the temporary directory into the directory store.
        let archive_entry = self.entry(CacheBucket::Archive, "", &id);
        fs_err::create_dir_all(archive_entry.dir())?;
        if let Err(err) = uv_fs::rename_with_retry(temp_dir.path(), archive_entry.path()).await {
            if !archive_entry.path().is_dir() {
                return Err(err);
            }
        }

        // Create a symlink to the directory store.
        fs_err::create_dir_all(path.as_ref().parent().expect("Cache entry to have parent"))?;
        self.create_link(&id, path.as_ref())?;

        Ok(id)
    }

    /// Return `true` if the [`Cache`] is temporary.
    pub fn is_temporary(&self) -> bool {
        self.temp_dir.is_some()
    }

    /// Create the base cache files.
    fn create_base_files(root: &PathBuf) -> io::Result<()> {
        // Create the cache directory if it does not exist.
        fs_err::create_dir_all(root)?;

        // Add `CACHEDIR.TAG`.
        cachedir::ensure_tag(root)?;

        // Add `.gitignore`.
        match fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(".gitignore"))
        {
            Ok(mut file) => file.write_all(b"*")?,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => (),
            Err(err) => return Err(err),
        }

        // Add an empty `.gitignore` to the build bucket. This prevents the cache's `.gitignore`
        // from affecting source distribution builds. Backends such as hatchling search parent
        // directories for `.gitignore` files.
        fs_err::create_dir_all(root.join(CacheBucket::SourceDistributions.to_str()))?;
        match fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(
                root.join(CacheBucket::SourceDistributions.to_str())
                    .join(".gitignore"),
            ) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => (),
            Err(err) => return Err(err),
        }

        // Add a placeholder `.git` file so the cache is not treated as part of a Git repository.
        // Otherwise, packages can include Git metadata, such as a commit hash, in built versions.
        // Place this file below `.gitignore`. Otherwise, a backend that uses the Rust `ignore`
        // crate can find the top-level `.gitignore` and ignore Python source files.
        let phony_git = root
            .join(CacheBucket::SourceDistributions.to_str())
            .join(".git");
        match fs_err::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&phony_git)
        {
            Ok(_) => {}
            // Support read-only caches, including caches in sandboxed environments.
            Err(err) if err.kind() == io::ErrorKind::ReadOnlyFilesystem => {
                if !phony_git.exists() {
                    return Err(err);
                }
            }
            Err(err) => return Err(err),
        }

        Ok(())
    }

    /// Initialize the [`Cache`].
    pub async fn init(self) -> Result<Self, Error> {
        let root = &self.root;

        Self::create_base_files(root).map_err(|err| Error::Init(root.clone(), err))?;

        // Prevent cache removal operations from interfering.
        let lock_file = match LockedFile::acquire(
            root.join(".lock"),
            LockedFileMode::Shared,
            root.simplified_display(),
        )
        .await
        {
            Ok(lock_file) => Some(Arc::new(lock_file)),
            Err(err)
                if err
                    .as_io_error()
                    .is_some_and(|err| err.kind() == io::ErrorKind::Unsupported) =>
            {
                warn!(
                    "Shared locking is not supported by the current platform or filesystem, \
                        reduced parallel process safety with `uv cache clean` and `uv cache prune`."
                );
                None
            }
            Err(err) => return Err(err.into()),
        };

        Ok(Self {
            root: std::path::absolute(root).map_err(Error::Absolute)?,
            lock_file,
            ..self
        })
    }

    /// Initialize the [`Cache`] when no other uv processes are running.
    pub fn init_no_wait(self) -> Result<Option<Self>, Error> {
        let root = &self.root;

        Self::create_base_files(root).map_err(|err| Error::Init(root.clone(), err))?;

        // Prevent cache removal operations from interfering.
        let Some(lock_file) = LockedFile::acquire_no_wait(
            root.join(".lock"),
            LockedFileMode::Shared,
            root.simplified_display(),
        ) else {
            return Ok(None);
        };
        Ok(Some(Self {
            root: std::path::absolute(root).map_err(Error::Absolute)?,
            lock_file: Some(Arc::new(lock_file)),
            ..self
        }))
    }

    /// Remove all entries from the cache.
    pub fn clear(self, reporter: Box<dyn CleanReporter>) -> Result<Removal, io::Error> {
        // Remove everything except `.lock`. Windows cannot remove a locked file.
        let mut removal = Remover::new(reporter)
            .with_removal_accounting(self.removal_accounting)
            .rm_rf(&self.root, true)?;
        let Self {
            root, lock_file, ..
        } = self;

        // Unlock and remove the `.lock` file.
        if let Some(lock) = lock_file {
            drop(lock);
            fs_err::remove_file(root.join(".lock"))?;
        }
        removal.num_files += 1;

        // Remove the root directory.
        match fs_err::remove_dir(root) {
            Ok(()) => {
                removal.num_dirs += 1;
            }
            // On Windows, `--force` can leave a `.lock` file that cannot be removed.
            // Do not treat this case as an error.
            Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => {
                trace!("Failed to remove root cache directory: not empty");
            }
            Err(err) => return Err(err),
        }

        Ok(removal)
    }

    /// Remove a package from the cache.
    ///
    /// Unreferenced file objects are removed separately by [`Cache::prune_archive_files`].
    ///
    /// Return the number of entries removed from the cache.
    pub fn remove(&self, name: &PackageName) -> io::Result<Removal> {
        // Collect all referenced archives.
        let references = self.find_archive_references()?;

        // Remove any entries for the package from the cache.
        let mut summary = self.removal();
        for bucket in CacheBucket::iter() {
            summary += bucket.remove(self, name)?;
        }

        if references.is_empty() {
            return Ok(summary);
        }

        // Remove targets only from the archive bucket. Cache entries can link to paths outside
        // the cache.
        let archive_root = fs_err::canonicalize(&self.root)?.join(CacheBucket::Archive.to_str());

        // Remove any archives that are no longer referenced.
        for (target, references) in references {
            if target.starts_with(&archive_root) && references.iter().all(|path| !path.exists()) {
                debug!("Removing dangling cache entry: {}", target.display());
                summary += self.remove_path(target)?;
            }
        }

        Ok(summary)
    }

    /// Remove file objects with no hardlinks outside the files bucket.
    ///
    /// Archives refer to these objects via hardlinks, independently of the installation link mode.
    /// Installed copies and reflinks remain valid when the cached file is removed, so they do not
    /// need to keep the file object alive.
    pub fn prune_archive_files(&self) -> Result<Removal, io::Error> {
        let root = self.bucket(CacheBucket::Files);
        if !root.exists() {
            return Ok(self.removal());
        }

        let mut summary = self.removal();
        let mut directories = Vec::new();
        let mut entries = walkdir::WalkDir::new(&root).min_depth(1).into_iter();
        while let Some(entry) = entries.next() {
            let entry = entry?;
            if entry.file_type().is_file() {
                match uv_fs::hardlink_count(entry.path()) {
                    Ok(1) => summary += self.remove_path(entry.path())?,
                    Ok(_) => {}
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
                }
            } else if entry.file_type().is_dir() {
                if let Some(files) = uv_fs::files_with_one_hardlink(entry.path())? {
                    entries.skip_current_dir();
                    for file in files {
                        summary += self.remove_path(file)?;
                    }
                }
                directories.push(entry.into_path());
            }
        }
        // The walk visits parents first so the bulk path can skip their contents.
        // Remove directories in reverse order so children are removed before parents.
        for directory in directories.into_iter().rev() {
            match fs_err::remove_dir(directory) {
                Ok(()) => {
                    summary.num_dirs += 1;
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::DirectoryNotEmpty | io::ErrorKind::NotFound
                    ) => {}
                Err(err) => return Err(err),
            }
        }

        Ok(summary)
    }

    /// Remove unused cache entries and cached environments.
    pub fn prune(&self, ci: bool) -> Result<Removal, io::Error> {
        let mut summary = self.removal();

        // First, remove unused top-level directories. These usually contain outdated cache
        // buckets, such as `wheels-v0` when the current version is `wheels-v1`.
        for entry in fs_err::read_dir(&self.root)? {
            let entry = entry?;
            let metadata = entry.metadata()?;

            if entry.file_name() == "CACHEDIR.TAG"
                || entry.file_name() == ".gitignore"
                || entry.file_name() == ".git"
                || entry.file_name() == ".lock"
            {
                continue;
            }

            if metadata.is_dir() {
                // If the directory is not a cache bucket, remove it.
                if CacheBucket::iter().all(|bucket| entry.file_name() != bucket.to_str()) {
                    let path = entry.path();
                    debug!("Removing dangling cache bucket: {}", path.display());
                    summary += self.remove_path(path)?;
                }
            } else {
                // If the file is not a marker file, remove it.
                let path = entry.path();
                debug!("Removing dangling cache bucket: {}", path.display());
                summary += self.remove_path(path)?;
            }
        }

        // Second, remove all cached environments. A `.venv` link can reference a centralized
        // project environment, but uv recreates that environment when needed.
        match fs_err::read_dir(self.bucket(CacheBucket::Environments)) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let path = entry.path();
                    debug!("Removing cached environment: {}", path.display());
                    summary += self.remove_path(path)?;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => (),
            Err(err) => return Err(err),
        }

        // Third, if enabled, remove unpacked wheels and keep only wheel archives.
        if ci {
            // Remove the complete prebuilt wheel cache because every entry is an unpacked wheel.
            match fs_err::read_dir(self.bucket(CacheBucket::Wheels)) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = entry?;
                        let path = entry.path();
                        if path.is_dir() {
                            debug!("Removing unzipped wheel entry: {}", path.display());
                            summary += self.remove_path(path)?;
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => (),
                Err(err) => return Err(err),
            }

            let source_distributions = self.bucket(CacheBucket::SourceDistributions);
            if source_distributions.try_exists()? {
                for entry in walkdir::WalkDir::new(source_distributions) {
                    let entry = entry?;

                    // A directory that contains `metadata.msgpack` is a built wheel revision.
                    if !entry.file_type().is_dir() {
                        continue;
                    }

                    if !entry.path().join("metadata.msgpack").exists() {
                        continue;
                    }

                    // Remove everything except the built wheel archive and the metadata.
                    for entry in fs_err::read_dir(entry.path())? {
                        let entry = entry?;
                        let path = entry.path();

                        // Retain the resolved metadata (`metadata.msgpack`).
                        if path
                            .file_name()
                            .is_some_and(|file_name| file_name == "metadata.msgpack")
                        {
                            continue;
                        }

                        // Retain any built wheel archives.
                        if path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("whl"))
                        {
                            continue;
                        }

                        debug!("Removing unzipped built wheel entry: {}", path.display());
                        summary += self.remove_path(path)?;
                    }
                }
            }
        }

        // Fourth, remove archives that no cache entry references.
        let references = self.find_archive_references()?;

        match fs_err::read_dir(self.bucket(CacheBucket::Archive)) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let path = entry.path();
                    let target = fs_err::canonicalize(&path)?;
                    if !references.contains_key(&target) {
                        debug!("Removing dangling cache archive: {}", path.display());
                        summary += self.remove_path(path)?;
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => (),
            Err(err) => return Err(err),
        }

        summary += self.prune_archive_files()?;

        Ok(summary)
    }

    /// Remove a cache path using the cache's configured storage accounting.
    pub fn remove_path(&self, path: impl AsRef<Path>) -> io::Result<Removal> {
        Remover::default()
            .with_removal_accounting(self.removal_accounting)
            .rm_rf(path, false)
    }

    /// Find all references to entries in the archive bucket.
    ///
    /// Symlinks in other cache buckets often reference archive entries.
    /// This method finds every such reference.
    ///
    /// Return a map from each archive path to the paths that reference it.
    fn find_archive_references(&self) -> Result<FxHashMap<PathBuf, Vec<PathBuf>>, io::Error> {
        let mut references = FxHashMap::<PathBuf, Vec<PathBuf>>::default();
        for bucket in [CacheBucket::SourceDistributions, CacheBucket::Wheels] {
            let bucket_path = self.bucket(bucket);
            if bucket_path.is_dir() {
                let walker = walkdir::WalkDir::new(&bucket_path).into_iter();
                for entry in walker.filter_entry(|entry| {
                    !(
                        // Ignore `.lock`, `.whl`, `.msgpack`, `.rev`, and `.http` files.
                        // Also ignore the `src` directory, which contains the unpacked source
                        // distribution.
                        entry.file_name() == "src"
                            || entry.file_name() == ".lock"
                            || entry.file_name() == ".gitignore"
                            || entry.path().extension().is_some_and(|ext| {
                                ext.eq_ignore_ascii_case("lock")
                                    || ext.eq_ignore_ascii_case("whl")
                                    || ext.eq_ignore_ascii_case("http")
                                    || ext.eq_ignore_ascii_case("rev")
                                    || ext.eq_ignore_ascii_case("msgpack")
                            })
                    )
                }) {
                    let entry = entry?;

                    // On Unix, archive references use symlinks.
                    if cfg!(unix) {
                        if !entry.file_type().is_symlink() {
                            continue;
                        }
                    }

                    // On Windows, archive references are files containing structured data.
                    if cfg!(windows) {
                        if !entry.file_type().is_file() {
                            continue;
                        }
                    }

                    if let Ok(target) = self.resolve_link(entry.path()) {
                        references
                            .entry(target)
                            .or_default()
                            .push(entry.path().to_path_buf());
                    }
                }
            }
        }
        Ok(references)
    }

    /// Create a link to a directory in the archive bucket.
    ///
    /// On Windows, write a [`Link`] file with the archive ID and version.
    /// On Unix, create a symlink to the target directory.
    #[cfg(windows)]
    #[expect(clippy::unused_self)]
    fn create_link(&self, id: &ArchiveId, dst: impl AsRef<Path>) -> io::Result<()> {
        // Serialize the link.
        let link = Link::new(id.clone());
        let contents = link.to_string();

        // First, create the file only if it does not already exist.
        match fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dst.as_ref())
        {
            Ok(mut file) => {
                // Write the target path to the file.
                file.write_all(contents.as_bytes())?;
                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                // Write to a temporary file, then move it into place.
                let temp_dir = tempfile::tempdir_in(dst.as_ref().parent().unwrap())?;
                let temp_file = temp_dir.path().join("link");
                fs_err::write(&temp_file, contents.as_bytes())?;

                // Move the link into the target location.
                fs_err::rename(&temp_file, dst.as_ref())?;

                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    /// Resolve an archive link and return the complete path.
    ///
    /// Return an error if the link target does not exist.
    #[cfg(windows)]
    pub fn resolve_link(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        // Deserialize the link.
        let contents = fs_err::read_to_string(path.as_ref())?;
        let link = Link::from_str(&contents)?;

        // Ignore stale links.
        if link.version != ARCHIVE_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "The link target does not exist.",
            ));
        }

        // Reconstruct the path.
        let path = self.archive(&link.id);
        path.canonicalize()
    }

    /// Create a link to a directory in the archive bucket.
    ///
    /// On Windows, write a [`Link`] file with the archive ID and version.
    /// On Unix, create a symlink to the target directory.
    #[cfg(unix)]
    fn create_link(&self, id: &ArchiveId, dst: impl AsRef<Path>) -> io::Result<()> {
        let dst = dst.as_ref();
        let dst_parent = dst.parent().expect("Cache entry to have parent");
        // Construct the relative link target.
        let src = uv_fs::relative_to(self.archive(id), dst_parent)?;

        // Attempt to create the symlink directly.
        match fs_err::os::unix::fs::symlink(&src, dst) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                // Use a temporary file to create the symlink atomically.
                let temp_dir = tempfile::tempdir_in(dst_parent)?;
                let temp_file = temp_dir.path().join("link");
                fs_err::os::unix::fs::symlink(&src, &temp_file)?;

                // Move the symlink into the target location.
                fs_err::rename(&temp_file, dst)?;

                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    /// Resolve an archive link and return the complete path.
    ///
    /// Return an error if the link target does not exist.
    #[cfg(unix)]
    pub fn resolve_link(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        path.as_ref().canonicalize()
    }
}

/// A link to an unpacked wheel in the local cache.
#[derive(Debug, Clone)]
#[allow(unused)]
struct Link {
    /// The unique ID of the entry in the archive bucket.
    id: ArchiveId,
    /// The version of the archive bucket.
    version: u8,
}

#[allow(unused)]
impl Link {
    /// Create a [`Link`] with the given archive ID.
    fn new(id: ArchiveId) -> Self {
        Self {
            id,
            version: ARCHIVE_VERSION,
        }
    }
}

impl Display for Link {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "archive-v{}/{}", self.version, self.id)
    }
}

impl FromStr for Link {
    type Err = io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, '/');
        let version = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing version"))?;
        let id = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing ID"))?;

        // Parse the archive version from `archive-v{version}/{id}`.
        let version = version
            .strip_prefix("archive-v")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing version prefix"))?;
        let version = u8::from_str(version).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse version: {err}"),
            )
        })?;

        // Parse the ID from `archive-v{version}/{id}`.
        let id = ArchiveId::from_str(id).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse ID: {err}"),
            )
        })?;

        Ok(Self { id, version })
    }
}

pub trait CleanReporter: Send + Sync {
    /// Run after one file or directory is removed.
    fn on_clean(&self);

    /// Run after all files and directories are removed.
    fn on_complete(&self);
}

/// A cache bucket that stores one kind of data in a directory under the cache root.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum CacheBucket {
    /// Wheels, their metadata, and their cache policy. This excludes locally built wheels.
    ///
    /// Entries contain wheel metadata and policy as `MsgPack` files, wheel files, or unpacked
    /// wheel archives. If a wheel exceeds the in-memory size threshold, download its archive into
    /// the cache. Then unpack it into a directory with the same name, without the `.whl` extension.
    ///
    /// Cache structure:
    ///  * `wheel-metadata-v0/pypi/foo/{foo-1.0.0-py3-none-any.msgpack, foo-1.0.0-py3-none-any.whl}`
    ///  * `wheel-metadata-v0/<digest(index-url)>/foo/{foo-1.0.0-py3-none-any.msgpack, foo-1.0.0-py3-none-any.whl}`
    ///  * `wheel-metadata-v0/url/<digest(url)>/foo/{foo-1.0.0-py3-none-any.msgpack, foo-1.0.0-py3-none-any.whl}`
    ///
    /// See `uv_client::RegistryClient::wheel_metadata` for details about fetching wheel metadata.
    ///
    /// # Example
    ///
    /// Consider this `requirements.in` file:
    /// ```text
    /// # pypi wheel
    /// pandas
    /// # url wheel
    /// flask @ https://files.pythonhosted.org/packages/36/42/015c23096649b908c809c69388a805a571a3bea44362fe87e33fc3afa01f/flask-3.0.0-py3-none-any.whl
    /// ```
    ///
    /// The `pip compile` command fetches and caches only metadata and the cache policy.
    /// It does not need the wheel files yet:
    /// ```text
    /// wheel-v0
    /// ├── pypi
    /// │   ...
    /// │   ├── pandas
    /// │   │   └── pandas-2.1.3-cp310-cp310-manylinux_2_17_x86_64.manylinux2014_x86_64.msgpack
    /// │   ...
    /// └── url
    ///     └── 4b8be67c801a7ecb
    ///         └── flask
    ///             └── flask-3.0.0-py3-none-any.msgpack
    /// ```
    ///
    /// The `pip compile` command creates this `requirements.txt` file:
    ///
    /// ```text
    /// [...]
    /// flask @ https://files.pythonhosted.org/packages/36/42/015c23096649b908c809c69388a805a571a3bea44362fe87e33fc3afa01f/flask-3.0.0-py3-none-any.whl
    /// [...]
    /// pandas==2.1.3
    /// [...]
    /// ```
    ///
    /// If `pip sync` uses `requirements.txt` on another machine, it also fetches the wheels:
    ///
    /// TODO(konstin): This is still wrong, we need to store the cache policy too!
    /// ```text
    /// wheel-v0
    /// ├── pypi
    /// │   ...
    /// │   ├── pandas
    /// │   │   ├── pandas-2.1.3-cp310-cp310-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
    /// │   │   ├── pandas-2.1.3-cp310-cp310-manylinux_2_17_x86_64.manylinux2014_x86_64
    /// │   ...
    /// └── url
    ///     └── 4b8be67c801a7ecb
    ///         └── flask
    ///             └── flask-3.0.0-py3-none-any.whl
    ///                 ├── flask
    ///                 │   └── ...
    ///                 └── flask-3.0.0.dist-info
    ///                     └── ...
    /// ```
    ///
    /// If `pip compile` and `pip sync` run on the same machine, the cache contains both:
    ///
    /// ```text
    /// wheels-v0
    /// ├── pypi
    /// │   ├── ...
    /// │   ├── pandas
    /// │   │   ├── pandas-2.1.3-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.msgpack
    /// │   │   ├── pandas-2.1.3-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
    /// │   │   └── pandas-2.1.3-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64
    /// │   │       ├── pandas
    /// │   │       │   ├── ...
    /// │   │       ├── pandas-2.1.3.dist-info
    /// │   │       │   ├── ...
    /// │   │       └── pandas.libs
    /// │   ├── ...
    /// └── url
    ///     └── 4b8be67c801a7ecb
    ///         └── flask
    ///             ├── flask-3.0.0-py3-none-any.msgpack
    ///             ├── flask-3.0.0-py3-none-any.msgpack
    ///             └── flask-3.0.0-py3-none-any
    ///                 ├── flask
    ///                 │   └── ...
    ///                 └── flask-3.0.0.dist-info
    ///                     └── ...
    Wheels,
    /// Source distributions, built wheels, extracted metadata, and source distribution cache
    /// policies.
    ///
    /// The structure resembles the `Wheel` bucket but adds a source distribution filename layer.
    /// Metadata belongs to the source distribution, not the wheel.
    ///
    /// TODO(konstin): The cache policy should be on the source distribution level, the metadata we
    /// can put next to the wheels as in the `Wheels` bucket.
    ///
    /// Store the unpacked source distribution in a directory named after its archive.
    ///
    /// PEP 517 builds source distributions into wheel archives. During resolution, build the
    /// wheel and store its archive in the cache. During installation, unpack the archive into a
    /// directory with the same name, without the `.whl` extension. The cache can contain both
    /// wheel archives and unpacked wheel directories.
    ///
    /// Cache structure:
    ///  * `built-wheels-v0/pypi/foo/34a17436ed1e9669/{manifest.msgpack, metadata.msgpack, foo-1.0.0.zip, foo-1.0.0-py3-none-any.whl, ...other wheels}`
    ///  * `built-wheels-v0/<digest(index-url)>/foo/foo-1.0.0.zip/{manifest.msgpack, metadata.msgpack, foo-1.0.0-py3-none-any.whl, ...other wheels}`
    ///  * `built-wheels-v0/url/<digest(url)>/foo/foo-1.0.0.zip/{manifest.msgpack, metadata.msgpack, foo-1.0.0-py3-none-any.whl, ...other wheels}`
    ///  * `built-wheels-v0/git/<digest(url)>/<git sha>/foo/foo-1.0.0.zip/{metadata.msgpack, foo-1.0.0-py3-none-any.whl, ...other wheels}`
    ///
    /// A URL filename does not need to be a valid source distribution filename
    /// (<https://github.com/search?q=path%3A**%2Frequirements.txt+master.zip&type=code>),
    /// so accept any filename. For example:
    ///  * `built-wheels-v0/url/<sha256(url)>/master.zip/metadata.msgpack`
    ///
    /// # Example
    ///
    /// Consider these requirements:
    /// ```text
    /// # git source dist
    /// pydantic-extra-types @ git+https://github.com/pydantic/pydantic-extra-types.git
    /// # pypi source dist
    /// django_allauth==0.51.0
    /// # url source dist
    /// werkzeug @ https://files.pythonhosted.org/packages/0d/cc/ff1904eb5eb4b455e442834dabf9427331ac0fa02853bf83db817a7dd53d/werkzeug-3.0.1.tar.gz
    /// ```
    ///
    /// These requirements can produce this cache structure:
    /// ```text
    /// built-wheels-v4/
    /// ├── git
    /// │   └── 2122faf3e081fb7a
    /// │       └── 7a2d650a4a7b4d04
    /// │           ├── metadata.msgpack
    /// │           └── pydantic_extra_types-2.9.0-py3-none-any.whl
    /// ├── pypi
    /// │   └── django-allauth
    /// │       └── 0.51.0
    /// │           ├── 0gH-_fwv8tdJ7JwwjJsUc
    /// │           │   ├── django-allauth-0.51.0.tar.gz
    /// │           │   │   └── [UNZIPPED CONTENTS]
    /// │           │   ├── django_allauth-0.51.0-py3-none-any.whl
    /// │           │   └── metadata.msgpack
    /// │           └── revision.http
    /// └── url
    ///     └── 6781bd6440ae72c2
    ///         ├── APYY01rbIfpAo_ij9sCY6
    ///         │   ├── metadata.msgpack
    ///         │   ├── werkzeug-3.0.1-py3-none-any.whl
    ///         │   └── werkzeug-3.0.1.tar.gz
    ///         │       └── [UNZIPPED CONTENTS]
    ///         └── revision.http
    /// ```
    ///
    /// The `manifest.msgpack` file contains only the information needed to invalidate the cache.
    /// The `metadata.msgpack` file contains the source distribution metadata.
    SourceDistributions,
    /// Flat index responses in a format similar to the Simple API.
    ///
    /// Cache structure:
    ///  * `flat-index-v0/index/<digest(flat_index_url)>.msgpack`
    ///
    /// Store the response as `Vec<File>`.
    FlatIndex,
    /// Git repositories.
    Git,
    /// Information about an interpreter at a path.
    ///
    /// Cache interpreter information only when the executable path matches `sys.executable`.
    /// This excludes pyenv shims, which can select a new Python version without changing the shim.
    ///
    /// Cache structure: `interpreter-v0/<digest(path)>.msgpack`
    ///
    /// # Example
    ///
    /// Each `MsgPack` file contains a Unix timestamp, [PEP 508] markers, and information from
    /// the `sys` and `sysconfig` modules.
    ///
    /// ```json
    /// {
    ///   "timestamp": 1698047994491,
    ///   "data": {
    ///     "markers": {
    ///       "implementation_name": "cpython",
    ///       "implementation_version": "3.12.0",
    ///       "os_name": "posix",
    ///       "platform_machine": "x86_64",
    ///       "platform_python_implementation": "CPython",
    ///       "platform_release": "6.5.0-13-generic",
    ///       "platform_system": "Linux",
    ///       "platform_version": "#13-Ubuntu SMP PREEMPT_DYNAMIC Fri Nov  3 12:16:05 UTC 2023",
    ///       "python_full_version": "3.12.0",
    ///       "python_version": "3.12",
    ///       "sys_platform": "linux"
    ///     },
    ///     "base_exec_prefix": "/home/ferris/.pyenv/versions/3.12.0",
    ///     "base_prefix": "/home/ferris/.pyenv/versions/3.12.0",
    ///     "sys_executable": "/home/ferris/projects/uv/.venv/bin/python"
    ///   }
    /// }
    /// ```
    ///
    /// [PEP 508]: https://peps.python.org/pep-0508/#environment-markers
    Interpreter,
    /// Index responses from the Simple API.
    ///
    /// Cache structure:
    ///  * `simple-v0/pypi/<package_name>.rkyv`
    ///  * `simple-v0/<digest(index_url)>/<package_name>.rkyv`
    ///
    /// Parse the response into `uv_client::SimpleDetailMetadata` before storage.
    Simple,
    /// Unpacked wheels stored as directories for internal cache use.
    /// When another bucket needs a directory, store it in [`CacheBucket::Archive`] first.
    /// Then link it into the required bucket. This permits cache entries to be replaced and
    /// removed atomically.
    Archive,
    /// Content-addressed files that are hardlinked into cached archives.
    Files,
    /// Temporary virtual environments for PEP 517 builds and other operations.
    Builds,
    /// Reusable virtual environments for Python tools and projects.
    Environments,
    /// Cached Python downloads.
    Python,
    /// Downloaded tool binaries, such as Ruff.
    Binaries,
    /// Cached vulnerability data from [OSV](https://osv.dev/).
    ///
    /// Cache structure:
    ///  * `osv-v0/vulnerability/<vuln_id>.msgpack` — cached full vulnerability records
    Osv,
}

impl CacheBucket {
    fn to_str(self) -> &'static str {
        match self {
            // Update `crates/uv/tests/build/cache_prune.rs` when this version changes.
            Self::SourceDistributions => "sdists-v9",
            // Update `crates/uv/tests/lock/lock.rs` when this version changes.
            Self::FlatIndex => "flat-index-v4",
            Self::Git => "git-v0",
            Self::Interpreter => "interpreter-v4",
            // Update `crates/uv/tests/build/cache_clean.rs` when this version changes.
            Self::Simple => "simple-v24",
            // Update `crates/uv/tests/build/cache_prune.rs` when this version changes.
            Self::Wheels => "wheels-v6",
            // Update `ARCHIVE_VERSION` in `crates/uv-cache/src/lib.rs` when this version changes.
            Self::Archive => "archive-v0",
            Self::Files => "files-v0",
            Self::Builds => "builds-v0",
            Self::Environments => "environments-v2",
            Self::Python => "python-v0",
            Self::Binaries => "binaries-v0",
            Self::Osv => "osv-v0",
        }
    }

    /// Remove a package from the cache bucket.
    ///
    /// Return the number of entries removed from the cache.
    fn remove(self, cache: &Cache, name: &PackageName) -> Result<Removal, io::Error> {
        /// Return `true` if the [`Path`] contains a built wheel for the given package.
        fn is_match(path: &Path, name: &PackageName) -> bool {
            let Ok(metadata) = fs_err::read(path.join("metadata.msgpack")) else {
                return false;
            };
            let Ok(metadata) = rmp_serde::from_slice::<ResolutionMetadata>(&metadata) else {
                return false;
            };
            metadata.name == *name
        }

        let mut summary = cache.removal();
        match self {
            Self::Wheels => {
                // PyPI wheels use one directory per package name.
                let root = cache.bucket(self).join(WheelCacheKind::Pypi);
                summary += cache.remove_path(root.join(name.to_string()))?;

                // Alternate indexes use one directory per index, then one directory per package.
                let root = cache.bucket(self).join(WheelCacheKind::Index);
                for directory in directories(root)? {
                    summary += cache.remove_path(directory.join(name.to_string()))?;
                }

                // Direct URLs use one directory per URL, then one directory per package.
                let root = cache.bucket(self).join(WheelCacheKind::Url);
                for directory in directories(root)? {
                    summary += cache.remove_path(directory.join(name.to_string()))?;
                }
            }
            Self::SourceDistributions => {
                // PyPI source distributions use one directory per package name.
                let root = cache.bucket(self).join(WheelCacheKind::Pypi);
                summary += cache.remove_path(root.join(name.to_string()))?;

                // Alternate indexes use one directory per index, then one directory per package.
                let root = cache.bucket(self).join(WheelCacheKind::Index);
                for directory in directories(root)? {
                    summary += cache.remove_path(directory.join(name.to_string()))?;
                }

                // Direct URLs use one directory per URL, then one directory per version.
                // Search for a wheel with the requested package name.
                let root = cache.bucket(self).join(WheelCacheKind::Url);
                for url in directories(root)? {
                    if directories(&url)?.any(|version| is_match(&version, name)) {
                        summary += cache.remove_path(url)?;
                    }
                }

                // Local dependencies use one directory per path, then one directory per version.
                // Search for a wheel with the requested package name.
                let root = cache.bucket(self).join(WheelCacheKind::Path);
                for path in directories(root)? {
                    if directories(&path)?.any(|version| is_match(&version, name)) {
                        summary += cache.remove_path(path)?;
                    }
                }

                // Git dependencies use one directory per repository, then one directory per SHA.
                // Search for a wheel with the requested package name.
                let root = cache.bucket(self).join(WheelCacheKind::Git);
                for repository in directories(root)? {
                    for sha in directories(repository)? {
                        if is_match(&sha, name) {
                            summary += cache.remove_path(sha)?;
                        }
                    }
                }
            }
            Self::Simple => {
                // PyPI responses use one rkyv file per package name.
                let root = cache.bucket(self).join(WheelCacheKind::Pypi);
                summary += cache.remove_path(root.join(format!("{name}.rkyv")))?;

                // Alternate indexes use one directory per index and one rkyv file per package.
                let root = cache.bucket(self).join(WheelCacheKind::Index);
                for directory in directories(root)? {
                    summary += cache.remove_path(directory.join(format!("{name}.rkyv")))?;
                }
            }
            Self::FlatIndex => {
                // A flat index does not identify its packages, so remove the complete cache entry.
                let root = cache.bucket(self);
                summary += cache.remove_path(root)?;
            }
            Self::Git
            | Self::Interpreter
            | Self::Archive
            | Self::Files
            | Self::Builds
            | Self::Environments
            | Self::Python
            | Self::Binaries
            | Self::Osv => {
                // Nothing to do.
            }
        }
        Ok(summary)
    }

    /// Return an iterator over all cache buckets.
    fn iter() -> impl Iterator<Item = Self> {
        [
            Self::Wheels,
            Self::SourceDistributions,
            Self::FlatIndex,
            Self::Git,
            Self::Interpreter,
            Self::Simple,
            Self::Archive,
            Self::Files,
            Self::Builds,
            Self::Environments,
            Self::Python,
            Self::Binaries,
            Self::Osv,
        ]
        .iter()
        .copied()
    }
}

impl Display for CacheBucket {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.to_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// The cache entry is fresh according to the [`Refresh`] policy.
    Fresh,
    /// The cache entry is stale according to the [`Refresh`] policy.
    Stale,
    /// The cache entry does not exist.
    Missing,
}

impl Freshness {
    pub const fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh)
    }
}

/// A refresh policy for cache entries.
#[derive(Debug, Clone)]
pub enum Refresh {
    /// Do not refresh any entries.
    None(Timestamp),
    /// Refresh entries for the specified packages if they were created before the timestamp.
    Packages(Vec<PackageName>, Vec<Box<Path>>, Timestamp),
    /// Refresh all entries created before the given timestamp.
    All(Timestamp),
}

impl Refresh {
    /// Determine the refresh strategy from the command-line arguments.
    pub fn from_args(refresh: Option<bool>, refresh_package: Vec<PackageName>) -> Self {
        let timestamp = Timestamp::now();
        match refresh {
            Some(true) => Self::All(timestamp),
            Some(false) => Self::None(timestamp),
            None => {
                if refresh_package.is_empty() {
                    Self::None(timestamp)
                } else {
                    Self::Packages(refresh_package, vec![], timestamp)
                }
            }
        }
    }

    /// Combine two [`Refresh`] policies and use the more comprehensive policy.
    #[must_use]
    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            // If the policy is `None`, keep the other policy and the later timestamp.
            (Self::None(t1), Self::None(t2)) => Self::None(t1.max(t2)),
            (Self::None(t1), Self::All(t2)) => Self::All(t1.max(t2)),
            (Self::None(t1), Self::Packages(packages, paths, t2)) => {
                Self::Packages(packages, paths, t1.max(t2))
            }

            // If the policy is `All`, refresh all packages.
            (Self::All(t1), Self::None(t2) | Self::All(t2) | Self::Packages(.., t2)) => {
                Self::All(t1.max(t2))
            }

            // If the policy is `Packages`, combine both policies and use the later timestamp.
            (Self::Packages(packages, paths, t1), Self::None(t2)) => {
                Self::Packages(packages, paths, t1.max(t2))
            }
            (Self::Packages(.., t1), Self::All(t2)) => Self::All(t1.max(t2)),
            (Self::Packages(packages1, paths1, t1), Self::Packages(packages2, paths2, t2)) => {
                Self::Packages(
                    packages1.into_iter().chain(packages2).collect(),
                    paths1.into_iter().chain(paths2).collect(),
                    t1.max(t2),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::ArchiveId;

    use super::Link;

    #[test]
    fn test_link_round_trip() {
        let id = ArchiveId::new();
        let link = Link::new(id);
        let s = link.to_string();
        let parsed = Link::from_str(&s).unwrap();
        assert_eq!(link.id, parsed.id);
        assert_eq!(link.version, parsed.version);
    }

    #[test]
    fn test_link_deserialize() {
        assert!(Link::from_str("archive-v0/foo").is_ok());
        assert!(Link::from_str("archive/foo").is_err());
        assert!(Link::from_str("v1/foo").is_err());
        assert!(Link::from_str("archive-v0/").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn prune_does_not_follow_environment_symlinks() {
        use super::{Cache, CacheBucket};

        let cache_root = tempfile::tempdir().unwrap();
        let victim_root = tempfile::tempdir().unwrap();
        let environments = cache_root.path().join(CacheBucket::Environments.to_str());
        let victim_dir = victim_root.path().join("victim-dir");

        fs_err::create_dir_all(&environments).unwrap();
        fs_err::create_dir_all(&victim_dir).unwrap();
        fs_err::write(victim_dir.join("payload.txt"), "payload").unwrap();
        fs_err::os::unix::fs::symlink(&victim_dir, environments.join("escape")).unwrap();

        let summary = Cache::from_path(cache_root.path()).prune(false).unwrap();

        assert_eq!(summary.num_files, 1);
        assert_eq!(summary.num_dirs, 0);
        assert!(victim_dir.is_dir());
        assert!(victim_dir.join("payload.txt").is_file());
        assert!(fs_err::symlink_metadata(environments.join("escape")).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn prune_ci_does_not_follow_wheel_symlinks() {
        use super::{Cache, CacheBucket};

        let cache_root = tempfile::tempdir().unwrap();
        let victim_root = tempfile::tempdir().unwrap();
        let wheels = cache_root.path().join(CacheBucket::Wheels.to_str());
        let source_distributions = cache_root
            .path()
            .join(CacheBucket::SourceDistributions.to_str());
        let victim_dir = victim_root.path().join("victim-dir");
        let symlink = wheels.join("escape");

        fs_err::create_dir_all(&wheels).unwrap();
        fs_err::create_dir_all(&source_distributions).unwrap();
        fs_err::create_dir_all(&victim_dir).unwrap();
        fs_err::write(victim_dir.join("payload.txt"), "payload").unwrap();
        fs_err::os::unix::fs::symlink(&victim_dir, &symlink).unwrap();

        let summary = Cache::from_path(cache_root.path()).prune(true).unwrap();

        assert_eq!(summary.num_files, 1);
        assert_eq!(summary.num_dirs, 0);
        assert!(victim_dir.is_dir());
        assert!(victim_dir.join("payload.txt").is_file());
        assert!(fs_err::symlink_metadata(symlink).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn prune_does_not_follow_archive_symlinks() {
        use super::{Cache, CacheBucket};

        let cache_root = tempfile::tempdir().unwrap();
        let victim_root = tempfile::tempdir().unwrap();
        let archives = cache_root.path().join(CacheBucket::Archive.to_str());
        let victim_dir = victim_root.path().join("victim-dir");
        let symlink = archives.join("escape");

        fs_err::create_dir_all(&archives).unwrap();
        fs_err::create_dir_all(&victim_dir).unwrap();
        fs_err::write(victim_dir.join("payload.txt"), "payload").unwrap();
        fs_err::os::unix::fs::symlink(&victim_dir, &symlink).unwrap();

        let summary = Cache::from_path(cache_root.path()).prune(false).unwrap();

        assert_eq!(summary.num_files, 1);
        assert_eq!(summary.num_dirs, 0);
        assert!(victim_dir.is_dir());
        assert!(victim_dir.join("payload.txt").is_file());
        assert!(fs_err::symlink_metadata(symlink).is_err());
    }
}
