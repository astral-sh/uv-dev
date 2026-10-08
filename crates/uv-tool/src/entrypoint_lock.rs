use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use uv_fs::{LockedFile, LockedFileMode, Simplified};

use crate::{Error, ToolEntrypoint, tool_executable_dir};

pub(crate) const ENTRYPOINT_LOCK_DIRECTORY: &str = ".uv-tool-lock";

/// Admission for executable directories shared by otherwise independent tool stores.
///
/// Acquire the tool-store lock first. Keep these guards through the final ownership checks,
/// executable changes, and receipt publication. Directory locks are acquired in canonical path
/// order, including previous destinations when a tool moves to another bin directory.
#[derive(Debug)]
#[must_use]
pub struct ToolEntrypointLocks {
    _locks: Vec<LockedFile>,
}

impl ToolEntrypointLocks {
    /// Lock the configured destination and any directories recorded by the previous receipt.
    pub async fn for_installation(previous: &[ToolEntrypoint]) -> Result<Self, Error> {
        let directory = tool_executable_dir()?;
        fs_err::create_dir_all(&directory)?;
        Self::for_directories(
            std::iter::once(directory).chain(
                previous
                    .iter()
                    .filter_map(|entrypoint| entrypoint.install_path.parent().map(PathBuf::from)),
            ),
        )
        .await
    }

    /// Lock destinations for a receipt repair without creating a directory for an empty receipt.
    pub async fn for_repair(previous: &[ToolEntrypoint]) -> Result<Self, Error> {
        if previous.is_empty() {
            return Self::for_removal(previous).await;
        }
        Self::for_installation(previous).await
    }

    /// Lock existing recorded destinations without recreating directories removed by the user.
    pub async fn for_removal<'a>(
        entrypoints: impl IntoIterator<Item = &'a ToolEntrypoint>,
    ) -> Result<Self, Error> {
        Self::for_directories(
            entrypoints
                .into_iter()
                .filter_map(|entrypoint| entrypoint.install_path.parent().map(PathBuf::from)),
        )
        .await
    }

    /// Lock all existing destinations, including directories retained by recovery records.
    ///
    /// Create a new publication directory before admission; absent historical destinations are
    /// skipped. Aliases share one guard, and all guards are acquired in canonical path order.
    async fn for_directories(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self, Error> {
        let mut directories = BTreeSet::new();
        for path in paths {
            match fs_err::canonicalize(path) {
                Ok(directory) => {
                    directories.insert(directory);
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }
        let mut locks = Vec::with_capacity(directories.len());
        for directory in directories {
            let sidecar = directory.join(ENTRYPOINT_LOCK_DIRECTORY);
            match fs_err::create_dir(&sidecar) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(err.into()),
            }
            let metadata = fs_err::symlink_metadata(&sidecar)?;
            if !metadata.is_dir() || metadata.is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "tool executable lock directory `{}` is not a directory",
                        sidecar.user_display()
                    ),
                )
                .into());
            }
            // Retain the sidecar after release: removing it would split later waiters across
            // different lock inodes. A directory cannot be replaced by an exported command.
            locks.push(
                LockedFile::acquire(
                    sidecar.join("lock"),
                    LockedFileMode::Exclusive,
                    format!("tool executable directory {}", directory.user_display()),
                )
                .await?,
            );
        }
        Ok(Self { _locks: locks })
    }
}
