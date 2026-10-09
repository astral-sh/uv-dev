use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use uv_cache::Cache;
use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileError, LockedFileMode, Simplified};

const MAX_LINK_EXPANSIONS: usize = 256;

/// Failure to establish environment coordination or acquire its advisory locks.
#[derive(Debug, thiserror::Error)]
pub enum EnvironmentLockError {
    #[error("Could not locate the OS account profile for environment coordination")]
    Profile(#[source] io::Error),
    #[error("Could not initialize environment coordination at `{}`", path.user_display())]
    Registry {
        path: PathBuf,
        #[source]
        source: LockedFileError,
    },
    #[error(
        "Environment destination `{}` overlaps the coordination directory `{}`",
        destination.user_display(), registry.user_display()
    )]
    ProtectedRegistry {
        destination: PathBuf,
        registry: PathBuf,
    },
    #[error(transparent)]
    Destination(#[from] io::Error),
    #[error(transparent)]
    Lock(#[from] LockedFileError),
}

/// Coordinate within one OS account profile and filesystem namespace, independently of process
/// configuration. Lock files remain in this directory: unlinking one can split waiting processes
/// between its old inode and a replacement.
fn coordination_directory() -> Result<PathBuf, EnvironmentLockError> {
    #[cfg(unix)]
    let profile = homedir::UserIdentifier::my_id().and_then(|user| user.to_home());
    #[cfg(windows)]
    let profile = homedir::my_home();
    let profile = profile
        .map_err(|error| EnvironmentLockError::Profile(io::Error::other(error)))?
        .ok_or_else(|| {
            EnvironmentLockError::Profile(io::Error::new(
                io::ErrorKind::NotFound,
                "the OS account has no profile directory",
            ))
        })?;
    let profile = fs_err::canonicalize(profile).map_err(EnvironmentLockError::Profile)?;
    Ok(profile.join(".uv-environment-locks-v1"))
}

fn initialize_registry(path: &Path) -> Result<(), EnvironmentLockError> {
    let initialize = || {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.recursive(false).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs_err::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "environment coordination requires a directory, not a symbolic link",
            ));
        }
        match fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join(".gitignore"))
        {
            Ok(mut file) => file.write_all(b"*")?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        Ok(())
    };
    initialize().map_err(|source| EnvironmentLockError::Registry {
        path: path.to_path_buf(),
        source: source.into(),
    })
}

/// Own the destinations being inspected, replaced, or populated independently of their contents.
#[derive(Debug)]
pub struct EnvironmentLock {
    paths: Vec<PathBuf>,
    registry: PathBuf,
    creating: bool,
    keys: Vec<Claim>,
    // Drop the worker's cache lease before releasing destination admission.
    _cache: Cache,
    files: Vec<(Claim, LockedFile)>,
}

/// Tree claims precede creation-parent claims, with parents before their descendants.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Key {
    Destination(PathBuf),
    CreationParent(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Claim {
    key: Key,
    exclusive: bool,
}

impl Key {
    fn resource(&self) -> &Path {
        match self {
            Self::Destination(path) | Self::CreationParent(path) => path,
        }
    }

    fn lock_path(&self, registry: &Path) -> PathBuf {
        let filename = match self {
            Self::Destination(path) => format!("uv-environment-{}.lock", cache_digest(path)),
            Self::CreationParent(path) => {
                format!("uv-environment-parent-{}.lock", cache_digest(path))
            }
        };
        registry.join(filename)
    }
}

impl EnvironmentLock {
    /// Claim logical slots and canonical destinations in one order, then verify their resolution.
    async fn acquire(paths: &[PathBuf], cache: &Cache) -> Result<Arc<Self>, EnvironmentLockError> {
        // Replacing the current directory can invalidate `current_dir`; retain absolute inputs
        // before any caller mutates the destination.
        let paths = paths
            .iter()
            .map(std::path::absolute)
            .collect::<io::Result<Vec<_>>>()?;
        let registry = coordination_directory()?;
        validate_destinations(&paths, &registry)?;
        initialize_registry(&registry)?;
        let registry =
            fs_err::canonicalize(&registry).map_err(|source| EnvironmentLockError::Registry {
                path: registry,
                source: source.into(),
            })?;
        validate_destinations(&paths, &registry)?;
        loop {
            let keys = destination_keys(&paths, true)?;
            let mut files = Vec::with_capacity(keys.len());
            for claim in &keys {
                let lock_path = claim.key.lock_path(&registry);
                let file = LockedFile::acquire(
                    &lock_path,
                    if claim.exclusive {
                        LockedFileMode::Exclusive
                    } else {
                        LockedFileMode::Shared
                    },
                    claim.key.resource().simplified_display(),
                )
                .await
                .map_err(|error| match error {
                    source @ (LockedFileError::CreateTemporary(_)
                    | LockedFileError::PersistTemporary { .. }
                    | LockedFileError::Io(_)) => EnvironmentLockError::Registry {
                        path: lock_path,
                        source,
                    },
                    error @ (LockedFileError::Lock { .. }
                    | LockedFileError::Timeout { .. }
                    | LockedFileError::JoinError(_)) => EnvironmentLockError::Lock(error),
                })?;
                files.push((claim.clone(), file));
            }
            if keys == destination_keys(&paths, true)? {
                return Ok(Arc::new(Self {
                    paths,
                    registry,
                    creating: true,
                    keys,
                    files,
                    _cache: cache.clone(),
                }));
            }
            // Release the previous set before following a reference to another destination.
        }
    }

    /// Tolerate advisory lock failures, but never change or bypass the coordination registry.
    pub async fn acquire_optional(
        paths: &[PathBuf],
        cache: &Cache,
    ) -> Result<Option<Arc<Self>>, EnvironmentLockError> {
        match Self::acquire(paths, cache).await {
            Ok(lock) => Ok(Some(lock)),
            Err(EnvironmentLockError::Lock(error)) => {
                tracing::warn!("Failed to acquire environment lock: {error}");
                Ok(None)
            }
            Err(EnvironmentLockError::Destination(error)) => {
                tracing::warn!("Failed to acquire environment lock: {error}");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Whether this guard owns exactly the currently resolved destination set.
    pub fn matches(&self, paths: &[PathBuf]) -> Result<bool, EnvironmentLockError> {
        validate_destinations(paths, &self.registry)?;
        Ok(self.keys == destination_keys(paths, self.creating)?)
    }

    /// Release parent admission once creation is covered by an owned canonical destination.
    ///
    /// Call before giving workers or environment descriptions references to this operation.
    pub fn finish_creation(self: &mut Arc<Self>) -> Result<(), EnvironmentLockError> {
        let Some(guard) = Arc::get_mut(self) else {
            return Ok(());
        };
        validate_destinations(&guard.paths, &guard.registry)?;
        let current = destination_keys(&guard.paths, false)?;
        // Keep the hierarchy until all protected paths exist and their canonical destinations
        // are covered. A partial ancestor creation must not open a gap for another spelling.
        if current.iter().any(|claim| match &claim.key {
            Key::Destination(_) => !guard.keys.contains(claim),
            Key::CreationParent(_) => true,
        }) {
            return Ok(());
        }
        guard.files.retain(|(claim, _)| match &claim.key {
            Key::Destination(_) => true,
            Key::CreationParent(_) => false,
        });
        guard.keys.retain(|claim| match &claim.key {
            Key::Destination(_) => true,
            Key::CreationParent(_) => false,
        });
        guard.creating = false;
        Ok(())
    }
}

/// Resolve the existing prefix without creating it; claim that prefix for any missing tail.
fn canonicalize_destination(
    path: &Path,
    keys: &mut BTreeMap<Key, bool>,
) -> io::Result<(PathBuf, bool)> {
    resolve_destination(path, keys, &mut 0)
}

fn resolve_destination(
    path: &Path,
    keys: &mut BTreeMap<Key, bool>,
    links: &mut usize,
) -> io::Result<(PathBuf, bool)> {
    match fs_err::canonicalize(path) {
        Ok(path) => Ok((path, true)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let absolute = std::path::absolute(path)?;
            let parent = absolute.parent().ok_or(error)?;
            let (parent, exists) = resolve_destination(parent, keys, links)?;
            let destination = match absolute.components().next_back() {
                Some(Component::Normal(name)) => parent.join(name),
                Some(Component::ParentDir) => parent.parent().unwrap_or(&parent).to_path_buf(),
                Some(Component::Prefix(_) | Component::RootDir | Component::CurDir) | None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "environment requires a filename",
                    ));
                }
            };
            match fs_err::symlink_metadata(&destination) {
                Ok(metadata) if metadata.is_symlink() => {
                    // A dangling link already selects a future destination. Reconstruct its
                    // target, rather than treating the link's name as a missing directory.
                    *links += 1;
                    if *links > MAX_LINK_EXPANSIONS {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "too many symbolic links while resolving environment admission",
                        ));
                    }
                    let parent = destination.parent().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "environment link requires a parent",
                        )
                    })?;
                    let (target, _) = resolve_destination(
                        &parent.join(fs_err::read_link(&destination)?),
                        keys,
                        links,
                    )?;
                    // Required intermediate directories can still be missing even if the
                    // target reached after `..` exists. Keep their creation-parent claims.
                    return Ok((target, false));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            if exists {
                // A missing filename has no canonical spelling. The nearest existing parent is
                // exclusive; shared ancestors keep this claim connected when another creator
                // observes a newly created intermediate directory.
                for (index, ancestor) in parent.ancestors().enumerate() {
                    keys.entry(Key::CreationParent(ancestor.to_path_buf()))
                        .and_modify(|exclusive| *exclusive |= index == 0)
                        .or_insert(index == 0);
                }
            }
            Ok((destination, false))
        }
        Err(error) => Err(error),
    }
}

fn destination_keys(paths: &[PathBuf], replacing: bool) -> io::Result<Vec<Claim>> {
    let mut keys = BTreeMap::new();
    for path in paths {
        let absolute = std::path::absolute(path)?;
        let (destination, _) = canonicalize_destination(&absolute, &mut keys)?;
        insert_tree(&destination, &mut keys);
        if replacing {
            insert_creation_parent(&destination, &mut keys);
        }
        if let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) {
            // Replacing a directory link changes the slot independently of its previous target.
            let (parent, _) = canonicalize_destination(parent, &mut keys)?;
            let slot = parent.join(name);
            insert_tree(&slot, &mut keys);
            if replacing {
                insert_creation_parent(&slot, &mut keys);
            }
        }
        insert_traversed_slots(&absolute, &mut keys)?;
    }
    Ok(keys
        .into_iter()
        .map(|(key, exclusive)| Claim { key, exclusive })
        .collect())
}

/// Workers retain their original paths, so every entry traversed by a selected root must remain
/// usable, including entries hidden inside a symlink's target spelling.
fn insert_traversed_slots(path: &Path, keys: &mut BTreeMap<Key, bool>) -> io::Result<()> {
    let mut pending = vec![path.to_path_buf()];
    let mut visited = BTreeSet::new();
    let mut links = 0;
    while let Some(path) = pending.pop() {
        for prefix in path.ancestors() {
            let (Some(parent), Some(name)) = (prefix.parent(), prefix.file_name()) else {
                continue;
            };
            let (parent, _) = canonicalize_destination(parent, keys)?;
            let slot = parent.join(name);
            if !visited.insert(slot.clone()) {
                continue;
            }
            for ancestor in slot.ancestors() {
                keys.entry(Key::Destination(ancestor.to_path_buf()))
                    .or_insert(false);
            }
            let metadata = match fs_err::symlink_metadata(&slot) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if metadata.is_symlink() {
                // Canonicalization enforces the OS link limit. Also bound traversal if links
                // keep changing between the individual filesystem observations.
                links += 1;
                if links > MAX_LINK_EXPANSIONS {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "too many symbolic links while collecting environment admission",
                    ));
                }
                // Retain the target spelling: on Unix, `..` applies after earlier link
                // expansion. Windows Path operations retain native prefix/root semantics.
                pending.push(parent.join(fs_err::read_link(&slot)?));
            }
        }
    }
    Ok(())
}

fn insert_tree(destination: &Path, keys: &mut BTreeMap<Key, bool>) {
    keys.insert(Key::Destination(destination.to_path_buf()), true);
    for parent in destination.ancestors().skip(1) {
        keys.entry(Key::Destination(parent.to_path_buf()))
            .or_insert(false);
    }
}

fn insert_creation_parent(destination: &Path, keys: &mut BTreeMap<Key, bool>) {
    for (index, parent) in destination.ancestors().skip(1).enumerate() {
        keys.entry(Key::CreationParent(parent.to_path_buf()))
            .and_modify(|exclusive| *exclusive |= index == 0)
            .or_insert(index == 0);
    }
}

fn validate_destinations(paths: &[PathBuf], registry: &Path) -> Result<(), EnvironmentLockError> {
    let logical_registry = registry;
    let (registry, _) =
        canonicalize_destination(registry, &mut BTreeMap::new()).map_err(|source| {
            EnvironmentLockError::Registry {
                path: registry.to_path_buf(),
                source: source.into(),
            }
        })?;
    for path in paths {
        let path = std::path::absolute(path)?;
        let (destination, _) = canonicalize_destination(&path, &mut BTreeMap::new())?;
        let mut destinations = vec![destination];
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            let (parent, _) = canonicalize_destination(parent, &mut BTreeMap::new())?;
            destinations.push(parent.join(name));
        }
        for destination in destinations {
            if registry.starts_with(&destination)
                || destination.starts_with(&registry)
                || logical_registry.starts_with(&destination)
            {
                return Err(EnvironmentLockError::ProtectedRegistry {
                    destination,
                    registry,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod process_tests;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::Result;
    use uv_cache::Cache;

    use super::EnvironmentLock;

    #[test]
    #[cfg(unix)]
    fn dangling_relative_link_resolves_links_before_parent_components() -> Result<()> {
        let root = tempfile::tempdir()?;
        let root = fs_err::canonicalize(root.path())?;
        fs_err::create_dir(root.join("a"))?;
        fs_err::create_dir(root.join("b"))?;
        fs_err::os::unix::fs::symlink("../b", root.join("a/through"))?;
        fs_err::os::unix::fs::symlink("through/../future", root.join("a/alias"))?;

        let path = root.join("a/alias/env");
        let (destination, exists) =
            super::canonicalize_destination(&path, &mut std::collections::BTreeMap::new())?;
        assert_eq!(destination, root.join("future/env"));
        assert!(!exists);
        let claims = super::destination_keys(&[path], true)?;
        assert!(claims.contains(&super::Claim {
            key: super::Key::Destination(root.join("future/env")),
            exclusive: true,
        }));
        assert!(claims.contains(&super::Claim {
            key: super::Key::Destination(root.join("a/through")),
            exclusive: false,
        }));
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn dangling_link_retains_missing_traversal_before_another_link() -> Result<()> {
        let root = tempfile::tempdir()?;
        let root = fs_err::canonicalize(root.path())?;
        fs_err::create_dir(root.join("a"))?;
        fs_err::create_dir(root.join("b"))?;
        fs_err::os::unix::fs::symlink("missing/../other", root.join("a/alias"))?;
        fs_err::os::unix::fs::symlink("../b", root.join("a/other"))?;

        let mut claims = std::collections::BTreeMap::new();
        let (destination, exists) =
            super::canonicalize_destination(&root.join("a/alias"), &mut claims)?;
        assert_eq!(destination, root.join("b"));
        assert!(!exists);
        assert_eq!(
            claims.get(&super::Key::CreationParent(root.join("a"))),
            Some(&true)
        );
        assert!(!root.join("a/missing").exists());
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn dangling_link_expansion_is_bounded() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("alias");
        fs_err::os::unix::fs::symlink("missing/../alias", &path)?;
        assert_eq!(
            fs_err::canonicalize(&path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        let error = super::canonicalize_destination(&path, &mut std::collections::BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        Ok(())
    }

    #[tokio::test]
    async fn absent_case_alias_waits_for_creation_owner() -> Result<()> {
        let cache = Cache::temp()?;
        let parent = tempfile::tempdir()?;
        let probe = parent.path().join("CaseProbe");
        fs_err::write(&probe, "")?;
        if !parent.path().join("caseprobe").exists() {
            // The collision exists only on case-insensitive filesystems.
            return Ok(());
        }
        let first = parent.path().join("NewEnv");
        let second = parent.path().join("newenv");
        let mut owner = EnvironmentLock::acquire(std::slice::from_ref(&first), &cache).await?;
        let paths = [second.clone()];
        let waiter = EnvironmentLock::acquire(&paths, &cache);
        tokio::pin!(waiter);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiter)
                .await
                .is_err()
        );

        fs_err::create_dir(&first)?;
        owner.finish_creation()?;
        // The waiter must now join the canonical destination, not its earlier absent spelling.
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiter)
                .await
                .is_err()
        );
        drop(owner);
        let waiter = tokio::time::timeout(Duration::from_secs(30), waiter).await??;
        assert!(waiter.matches(&[second])?);
        Ok(())
    }

    #[tokio::test]
    async fn partial_parent_creation_keeps_case_aliases_connected() -> Result<()> {
        let cache = Cache::temp()?;
        let parent = tempfile::tempdir()?;
        fs_err::write(parent.path().join("CaseProbe"), "")?;
        if !parent.path().join("caseprobe").exists() {
            return Ok(());
        }
        let first = parent.path().join("NewParent").join("NewEnv");
        let mut owner = EnvironmentLock::acquire(std::slice::from_ref(&first), &cache).await?;
        fs_err::create_dir(parent.path().join("NewParent"))?;

        let second = parent.path().join("newparent").join("newenv");
        let paths = [second.clone()];
        let waiter = EnvironmentLock::acquire(&paths, &cache);
        tokio::pin!(waiter);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiter)
                .await
                .is_err()
        );

        fs_err::create_dir(&first)?;
        owner.finish_creation()?;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiter)
                .await
                .is_err()
        );
        drop(owner);
        let waiter = tokio::time::timeout(Duration::from_secs(30), waiter).await??;
        assert!(waiter.matches(&[second])?);
        Ok(())
    }

    #[tokio::test]
    async fn initialized_destination_releases_sibling_creation() -> Result<()> {
        let cache = Cache::temp()?;
        let parent = tempfile::tempdir()?;
        let first = parent.path().join("first");
        let second = parent.path().join("second");
        let mut owner = EnvironmentLock::acquire(std::slice::from_ref(&first), &cache).await?;
        fs_err::create_dir(&first)?;
        owner.finish_creation()?;
        let sibling = tokio::time::timeout(
            Duration::from_secs(30),
            EnvironmentLock::acquire(&[second], &cache),
        )
        .await??;
        drop(sibling);
        drop(owner);
        Ok(())
    }
}
