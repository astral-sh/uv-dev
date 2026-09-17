use std::path::{Path, PathBuf};

use tracing::info_span;

use uv_lock::{Lock, LockError, LockParseError};
use uv_pep508::MarkerTree;
use uv_preview::PreviewFeature;
use uv_resolver::Preference;
use uv_warnings::warn_user_once;

use crate::commands::locked_requirements::read_inherited_lock_preferences;

/// An immutable snapshot of the lockfile used to align a child workspace.
#[derive(Debug)]
pub(crate) struct ParentLockSnapshot {
    root: PathBuf,
    lock: Lock,
}

impl ParentLockSnapshot {
    /// Read a registered parent workspace's lockfile without updating it.
    pub(crate) async fn read(root: PathBuf) -> Result<Self, ParentLockError> {
        let lock_path = root.join("uv.lock");
        let encoded = match fs_err::tokio::read_to_string(&lock_path).await {
            Ok(encoded) => encoded,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(ParentLockError::Missing(lock_path));
            }
            Err(err) => return Err(ParentLockError::Read(lock_path, err)),
        };
        let lock = info_span!("parse parent uv lock", path = %lock_path.display())
            .in_scope(|| Lock::from_toml(&encoded))
            .map_err(|err| ParentLockError::Parse(lock_path, Box::new(err)))?;
        Ok(Self { root, lock })
    }

    /// Return the parent workspace root.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Extract the registry-version preferences applicable to a child resolution.
    pub(crate) fn preferences(
        &self,
        child_environment: MarkerTree,
    ) -> Result<Vec<Preference>, ParentLockError> {
        read_inherited_lock_preferences(&self.lock, &self.root, child_environment)
            .map_err(|err| ParentLockError::Preferences(self.root.join("uv.lock"), Box::new(err)))
    }
}

/// An error obtaining the baseline of a registered parent workspace.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ParentLockError {
    #[error(
        "Unable to find the parent workspace lockfile at `{0}`. Run `uv lock` in the parent workspace first."
    )]
    Missing(PathBuf),
    #[error("Failed to read the parent workspace lockfile at `{0}`")]
    Read(PathBuf, #[source] std::io::Error),
    #[error("Failed to parse the parent workspace lockfile at `{0}`")]
    Parse(PathBuf, #[source] Box<LockParseError>),
    #[error("Failed to read preferences from the parent workspace lockfile at `{0}`")]
    Preferences(PathBuf, #[source] Box<LockError>),
}

/// Warn when nested workspaces are used without enabling the preview feature.
pub(crate) fn warn_nested_workspaces() {
    if !uv_preview::is_enabled(PreviewFeature::NestedWorkspaces) {
        warn_user_once!(
            "Nested workspaces are experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::NestedWorkspaces
        );
    }
}
