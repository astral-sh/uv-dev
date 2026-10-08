use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use uv_cache::Cache;
use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileError, LockedFileMode, Simplified};

/// Own the destinations being inspected, replaced, or populated independently of their contents.
#[derive(Debug)]
pub struct EnvironmentLock {
    paths: Vec<PathBuf>,
    keys: Vec<Claim>,
    // Drop the worker's cache lease before releasing destination admission.
    _cache: Cache,
    files: Vec<(Claim, LockedFile)>,
}

/// Destination claims precede creation-parent claims, with paths sorted within each kind.
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

    fn lock_path(&self) -> PathBuf {
        let filename = match self {
            Self::Destination(path) => format!("uv-environment-{}.lock", cache_digest(path)),
            Self::CreationParent(path) => {
                format!("uv-environment-parent-{}.lock", cache_digest(path))
            }
        };
        std::env::temp_dir().join(filename)
    }
}

impl EnvironmentLock {
    /// Claim logical slots and canonical destinations in one order, then verify their resolution.
    pub async fn acquire(paths: &[PathBuf], cache: &Cache) -> Result<Arc<Self>, LockedFileError> {
        loop {
            let keys = destination_keys(paths)?;
            let mut files = Vec::with_capacity(keys.len());
            for claim in &keys {
                let file = LockedFile::acquire(
                    claim.key.lock_path(),
                    if claim.exclusive {
                        LockedFileMode::Exclusive
                    } else {
                        LockedFileMode::Shared
                    },
                    claim.key.resource().simplified_display(),
                )
                .await?;
                files.push((claim.clone(), file));
            }
            if keys == destination_keys(paths)? {
                return Ok(Arc::new(Self {
                    paths: paths.to_vec(),
                    keys,
                    files,
                    _cache: cache.clone(),
                }));
            }
            // Release the previous set before following a reference to another destination.
        }
    }

    /// Whether this guard owns exactly the currently resolved destination set.
    pub fn matches(&self, paths: &[PathBuf]) -> io::Result<bool> {
        Ok(self.keys == destination_keys(paths)?)
    }

    /// Release parent admission once creation is covered by an owned canonical destination.
    ///
    /// Call before giving workers or environment descriptions references to this operation.
    pub fn finish_creation(self: &mut Arc<Self>) -> io::Result<()> {
        let Some(guard) = Arc::get_mut(self) else {
            return Ok(());
        };
        let current = destination_keys(&guard.paths)?;
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
        Ok(())
    }
}

/// Resolve the existing prefix without creating it; claim that prefix for any missing tail.
fn canonicalize_destination(
    path: &Path,
    keys: &mut BTreeMap<Key, bool>,
) -> io::Result<(PathBuf, bool)> {
    match fs_err::canonicalize(path) {
        Ok(path) => Ok((path, true)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let absolute = std::path::absolute(path)?;
            let parent = absolute.parent().ok_or(error)?;
            let name = absolute.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "environment requires a filename",
                )
            })?;
            let (parent, exists) = canonicalize_destination(parent, keys)?;
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
            Ok((parent.join(name), false))
        }
        Err(error) => Err(error),
    }
}

fn destination_keys(paths: &[PathBuf]) -> io::Result<Vec<Claim>> {
    let mut keys = BTreeMap::new();
    for path in paths {
        let absolute = std::path::absolute(path)?;
        let (destination, _) = canonicalize_destination(&absolute, &mut keys)?;
        keys.insert(Key::Destination(destination), true);
        if let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) {
            // Replacing a directory link changes the slot independently of its previous target.
            let (parent, _) = canonicalize_destination(parent, &mut keys)?;
            keys.insert(Key::Destination(parent.join(name)), true);
        }
    }
    Ok(keys
        .into_iter()
        .map(|(key, exclusive)| Claim { key, exclusive })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::Result;
    use uv_cache::Cache;

    use super::EnvironmentLock;

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
