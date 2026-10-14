//! Derived from Cargo's `clean` implementation.
//! Cargo is dual-licensed under either Apache 2.0 or MIT, at the user's choice.
//! Source: <https://github.com/rust-lang/cargo/blob/e1ebce1035f9b53bb46a55bd4b0ecf51e24c6458/src/cargo/ops/cargo_clean.rs#L324>

use std::fs::{FileType, Metadata};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use tracing::debug;
use uv_fs::PhysicalSpaceError;

use crate::CleanReporter;

/// How to estimate reclaimed storage when removing cache entries.
#[derive(Debug, Clone, Copy, Default)]
pub enum RemovalAccounting {
    /// Estimate reclaimed storage from ordinary filesystem metadata.
    #[default]
    Coarse,
    /// Inspect filesystem allocation and sharing where supported.
    Fine,
}

/// A builder for a [`Remover`] that can remove files and directories.
#[derive(Default)]
pub(crate) struct Remover {
    reporter: Option<Box<dyn CleanReporter>>,
    removal_accounting: RemovalAccounting,
}

impl Remover {
    /// Create a new [`Remover`] with the given reporter.
    pub(crate) fn new(reporter: Box<dyn CleanReporter>) -> Self {
        Self {
            reporter: Some(reporter),
            ..Self::default()
        }
    }

    /// Set the storage accounting used before each file is removed.
    pub(crate) fn with_removal_accounting(mut self, removal_accounting: RemovalAccounting) -> Self {
        self.removal_accounting = removal_accounting;
        self
    }

    /// Remove a file or directory and all its contents, returning a [`Removal`] with
    /// the number of files and directories removed, along with a total byte count.
    pub(crate) fn rm_rf(
        &self,
        path: impl AsRef<Path>,
        skip_locked_file: bool,
    ) -> io::Result<Removal> {
        let mut removal = Removal::new(self.removal_accounting);
        removal.rm_rf(path.as_ref(), self.reporter.as_deref(), skip_locked_file)?;
        Ok(removal)
    }
}

/// Estimate the storage reclaimed by removing a non-directory entry.
#[cfg(unix)]
fn file_size(metadata: &Metadata) -> u64 {
    if metadata.nlink() == 1 {
        metadata.blocks().saturating_mul(512)
    } else {
        0
    }
}

/// Estimate the storage reclaimed by removing a non-directory entry.
#[cfg(not(unix))]
fn file_size(metadata: &Metadata) -> u64 {
    metadata.len()
}

/// Windows directory symlinks must be removed as directories.
#[cfg(windows)]
fn is_directory_symlink(file_type: FileType) -> bool {
    use std::os::windows::fs::FileTypeExt;

    file_type.is_symlink() && file_type.is_symlink_dir()
}

#[cfg(not(windows))]
fn is_directory_symlink(_file_type: FileType) -> bool {
    false
}

/// A removal operation with statistics on the number of files and directories removed.
#[derive(Debug, Default)]
pub struct Removal {
    /// The number of files removed.
    pub num_files: u64,
    /// The number of directories removed.
    pub num_dirs: u64,
    /// The coarse estimate of the number of bytes occupied by the removed files.
    pub coarse_bytes: u64,
    /// The fine-grained estimate of reclaimed physical file data, when available.
    pub fine_bytes: Option<u64>,
    /// Whether any removed entries could not be measured, making the fine-grained count a lower bound.
    pub fine_bytes_incomplete: bool,
}

impl Removal {
    /// Create an empty removal summary with the requested storage accounting.
    pub(crate) fn new(removal_accounting: RemovalAccounting) -> Self {
        Self {
            fine_bytes: match removal_accounting {
                RemovalAccounting::Coarse => None,
                RemovalAccounting::Fine => Some(0),
            },
            ..Self::default()
        }
    }

    /// Account for a file while its current sharing state can still be inspected.
    fn add_file(&mut self, path: &Path, metadata: &Metadata) {
        self.coarse_bytes += file_size(metadata);

        if let Some(fine_bytes) = self.fine_bytes {
            match uv_fs::physical_space(path, metadata) {
                Ok(bytes) => {
                    self.fine_bytes = Some(fine_bytes.saturating_add(bytes));
                }
                Err(PhysicalSpaceError::UnsupportedFilesystem) => {
                    debug!(
                        "Fine-grained space accounting is unsupported for `{}`; falling back to coarse accounting",
                        path.display()
                    );
                    self.fine_bytes = None;
                    self.fine_bytes_incomplete = false;
                }
                Err(PhysicalSpaceError::UnmeasurableFile(error)) => {
                    debug!(
                        "Failed to measure physical space for `{}`: {error}",
                        path.display()
                    );
                    self.fine_bytes_incomplete = true;
                }
            }
        }
    }

    /// Recursively remove a file or directory and all its contents.
    fn rm_rf(
        &mut self,
        path: &Path,
        reporter: Option<&dyn CleanReporter>,
        skip_locked_file: bool,
    ) -> io::Result<()> {
        let path = uv_fs::verbatim_path(path);

        'retry: loop {
            let metadata = match fs_err::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(err) => return Err(err),
            };

            if !metadata.is_dir() {
                self.num_files += 1;

                // Remove the file.
                self.add_file(&path, &metadata);
                if is_directory_symlink(metadata.file_type()) {
                    remove_dir(&path)?;
                } else {
                    remove_file(&path)?;
                }

                if let Some(reporter) = reporter {
                    reporter.on_clean();
                }

                return Ok(());
            }

            for entry in walkdir::WalkDir::new(&path).contents_first(true) {
                // If we hit a directory that lacks read permissions, try to make it readable.
                if let Err(ref err) = entry
                    && err
                        .io_error()
                        .is_some_and(|err| err.kind() == io::ErrorKind::PermissionDenied)
                    && let Some(dir) = err.path()
                    && set_readable(dir).unwrap_or(false)
                {
                    // Restart the traversal after changing permissions so the walker does not yield
                    // duplicate entries for the directory.
                    continue 'retry;
                }

                let entry = entry?;

                // Keep the root lock in place for other uv processes.
                if skip_locked_file && entry.depth() == 1 && entry.file_name() == ".lock" {
                    continue;
                }

                if is_directory_symlink(entry.file_type()) {
                    self.num_files += 1;
                    remove_dir(entry.path())?;
                } else if entry.file_type().is_dir() {
                    // Keep the root directory containing the lock, too.
                    if skip_locked_file && entry.depth() == 0 {
                        continue;
                    }

                    self.num_dirs += 1;

                    // The contents should have been removed by now, but sometimes a race condition is
                    // hit where other files have been added by the OS. Fall back to `remove_dir_all`,
                    // which will remove the directory robustly across platforms.
                    remove_dir_all(entry.path())?;
                } else {
                    self.num_files += 1;

                    // Remove the file.
                    if let Ok(metadata) = entry.metadata() {
                        self.add_file(entry.path(), &metadata);
                    } else if self.fine_bytes.is_some() {
                        self.fine_bytes_incomplete = true;
                    }
                    remove_file(entry.path())?;
                }

                if let Some(reporter) = reporter {
                    reporter.on_clean();
                }
            }

            if let Some(reporter) = reporter {
                reporter.on_complete();
            }

            return Ok(());
        }
    }
}

impl std::ops::AddAssign for Removal {
    fn add_assign(&mut self, other: Self) {
        self.num_files += other.num_files;
        self.num_dirs += other.num_dirs;
        self.coarse_bytes += other.coarse_bytes;
        self.fine_bytes = self
            .fine_bytes
            .zip(other.fine_bytes)
            .map(|(left, right)| left.saturating_add(right));
        self.fine_bytes_incomplete = self.fine_bytes.is_some()
            && (self.fine_bytes_incomplete || other.fine_bytes_incomplete);
    }
}

/// If the directory isn't readable by the current user, change the permissions to make it readable.
#[cfg_attr(windows, allow(unused_variables, clippy::unnecessary_wraps))]
fn set_readable(path: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs_err::metadata(path)?.permissions();
        if perms.mode() & 0o500 == 0 {
            perms.set_mode(perms.mode() | 0o500);
            fs_err::set_permissions(path, perms)?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// If the file is readonly, change the permissions to make it _not_ readonly.
fn set_not_readonly(path: &Path) -> io::Result<bool> {
    let mut perms = fs_err::metadata(path)?.permissions();
    if !perms.readonly() {
        return Ok(false);
    }

    // We're about to delete the file, so it's fine to set the permissions to world-writable.
    #[expect(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);

    fs_err::set_permissions(path, perms)?;

    Ok(true)
}

/// Like [`fs_err::remove_file`], but attempts to change the permissions to force the file to be
/// deleted (if it is readonly).
fn remove_file(path: &Path) -> io::Result<()> {
    remove_with_permission_retry(|| fs_err::remove_file(path), || set_not_readonly(path))
}

/// Like [`fs_err::remove_dir`], but attempts to change the permissions to force the directory to
/// be deleted (if it is readonly).
fn remove_dir(path: &Path) -> io::Result<()> {
    remove_with_permission_retry(|| fs_err::remove_dir(path), || set_readable(path))
}

/// Like [`fs_err::remove_dir_all`], but attempts to change the permissions to force the directory
/// to be deleted (if it is readonly).
fn remove_dir_all(path: &Path) -> io::Result<()> {
    remove_with_permission_retry(|| fs_err::remove_dir_all(path), || set_readable(path))
}

/// Retry removal after changing permissions, preserving the original error if the change fails.
fn remove_with_permission_retry(
    remove: impl Fn() -> io::Result<()>,
    change_permissions: impl FnOnce() -> io::Result<bool>,
) -> io::Result<()> {
    remove().or_else(|err| {
        if err.kind() == io::ErrorKind::PermissionDenied && change_permissions().unwrap_or(false) {
            remove()
        } else {
            Err(err)
        }
    })
}
