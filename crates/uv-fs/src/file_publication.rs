//! Publication of user-owned files without replacing their link or access semantics.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use std::os::macos::fs::MetadataExt as _;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use same_file::Handle;
use tempfile::TempPath;

use crate::verbatim_path;

/// A file writer which stages complete bytes when replacing the inode is compatible with its
/// access metadata. Hard links, extended access metadata, and unsupported platforms use the
/// authorized original descriptor instead, so aliases and access restrictions remain intact.
///
/// Existing Windows files use in-place writes: `ReplaceFileW` preserves DACLs but can remove the
/// destination on failure, while a plain replacing rename does not retain its DACL.
/// This provides neither a multi-file transaction nor power-loss durability.
pub struct FilePublication {
    path: PathBuf,
    writer: fs_err::File,
    staging: Option<Staging>,
}

struct Staging {
    target: PathBuf,
    temporary: TempPath,
    original: Option<fs_err::File>,
}

impl FilePublication {
    /// Open the destination with the same write authorization as an in-place write, without
    /// truncating it. A missing destination is staged with ordinary file-creation permissions.
    pub fn new(path: &Path) -> io::Result<Self> {
        let original = match fs_err::OpenOptions::new().write(true).open(path) {
            Ok(file) => Some(file),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };
        if let Some(file) = original.as_ref()
            && !can_replace(file)
        {
            return Ok(Self {
                path: path.to_owned(),
                writer: file.try_clone()?,
                staging: None,
            });
        }
        let target = destination(path)?;
        if let Some(file) = original.as_ref() {
            same_identity(file, &target)?;
        }
        let parent = target.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "file has no parent directory")
        })?;
        let mut builder = tempfile::Builder::new();
        builder.prefix(".uv-publish-");
        #[cfg(unix)]
        {
            builder.permissions(std::fs::Permissions::from_mode(0o666));
        }
        let temporary = match builder.tempfile_in(verbatim_path(parent)) {
            Ok(temporary) => temporary,
            // Writing an existing file does not require write access to its directory.
            Err(err) => {
                if err.kind() == io::ErrorKind::PermissionDenied
                    && let Some(original) = original
                {
                    return Ok(Self {
                        path: path.to_owned(),
                        writer: original,
                        staging: None,
                    });
                }
                return Err(err);
            }
        };
        let (file, temporary) = temporary.into_parts();
        let writer = fs_err::File::from_parts(file, path);
        if let Some(original) = original.as_ref()
            && !prepare_permissions(original, &writer)?
        {
            return Ok(Self {
                path: path.to_owned(),
                writer: original.try_clone()?,
                staging: None,
            });
        }
        Ok(Self {
            path: path.to_owned(),
            writer,
            staging: Some(Staging {
                target,
                temporary,
                original,
            }),
        })
    }

    /// The file whose write authorization was checked, if the destination already existed.
    pub fn original(&self) -> Option<&fs_err::File> {
        match &self.staging {
            Some(staging) => staging.original.as_ref(),
            None => Some(&self.writer),
        }
    }

    /// Whether writes are private until [`Self::publish`].
    pub fn is_staged(&self) -> bool {
        self.staging.is_some()
    }

    /// The descriptor receiving bytes. In-place callers must retain partial-write ownership.
    pub fn writer(&mut self) -> &mut fs_err::File {
        &mut self.writer
    }

    /// Publish the completed bytes, returning the descriptor of the resulting file.
    ///
    /// The identity check detects replaced paths but is not a filesystem compare-and-swap.
    pub fn publish(self) -> io::Result<fs_err::File> {
        if let Some(staging) = self.staging {
            if let Some(original) = &staging.original {
                same_identity(original, &self.path)?;
                same_identity(original, &staging.target)?;
                // A newly added ACL or hard link must not be discarded by inode replacement.
                if !can_replace(original) || !prepare_permissions(original, &self.writer)? {
                    return Err(io::Error::other(
                        "file access metadata changed before publication",
                    ));
                }
                staging.temporary.persist(verbatim_path(&staging.target))
            } else {
                if destination(&self.path)? != staging.target {
                    return Err(io::Error::other("file target changed before publication"));
                }
                staging
                    .temporary
                    .persist_noclobber(verbatim_path(&staging.target))
            }
            .map_err(|err| err.error)?;
        }
        Ok(self.writer)
    }
}

/// Write complete bytes, staging compatible files before publication.
pub fn write_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut publication = FilePublication::new(path)?;
    publication.writer().set_len(0)?;
    publication.writer().write_all(contents)?;
    publication.publish()?;
    Ok(())
}

/// Run the complete publication in one blocking worker.
#[cfg(feature = "tokio")]
pub async fn write_file_async(path: PathBuf, contents: Vec<u8>) -> io::Result<()> {
    tokio::task::spawn_blocking(move || write_file(&path, &contents))
        .await
        .map_err(io::Error::other)?
}

fn same_identity(file: &fs_err::File, path: &Path) -> io::Result<()> {
    if Handle::from_file(file.file().try_clone()?)? != Handle::from_path(path)? {
        return Err(io::Error::other("file was replaced before publication"));
    }
    Ok(())
}

/// Resolve the final link too, including dangling links whose target can be created.
fn destination(path: &Path) -> io::Result<PathBuf> {
    let mut target = path.to_owned();
    for _ in 0..40 {
        match fs_err::symlink_metadata(&target) {
            Ok(metadata) if metadata.is_symlink() => {
                let link = fs_err::read_link(&target)?;
                target = target.parent().unwrap_or_else(|| Path::new("")).join(link);
            }
            Ok(_) => return fs_err::canonicalize(target),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let parent = target
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                let name = target.file_name().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "file has no name")
                })?;
                return Ok(fs_err::canonicalize(parent)?.join(name));
            }
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::other(
        "too many symbolic links in file destination",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn can_replace(file: &fs_err::File) -> bool {
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    metadata.is_file()
        && metadata.nlink() == 1
        && metadata.mode() & 0o7000 == 0
        && plain_access_metadata(file)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn can_replace(_file: &fs_err::File) -> bool {
    false
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn prepare_permissions(original: &fs_err::File, staged: &fs_err::File) -> io::Result<bool> {
    let original_metadata = original.metadata()?;
    let staged_metadata = staged.metadata()?;
    if original_metadata.uid() != staged_metadata.uid()
        || original_metadata.gid() != staged_metadata.gid()
        || !plain_access_metadata(staged)
    {
        return Ok(false);
    }
    staged.set_permissions(original_metadata.permissions())?;
    Ok(true)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn prepare_permissions(_original: &fs_err::File, _staged: &fs_err::File) -> io::Result<bool> {
    Ok(false)
}

#[cfg(target_os = "linux")]
fn plain_access_metadata(file: &fs_err::File) -> bool {
    // POSIX ACLs, capabilities and security labels are extended attributes. Inode replacement is
    // conservative when the filesystem cannot report them, including unsupported queries.
    rustix::fs::flistxattr(file, &mut [0u8; 0]).is_ok_and(|length| length == 0)
}

#[cfg(target_os = "macos")]
fn plain_access_metadata(file: &fs_err::File) -> bool {
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    metadata.st_flags() == 0
        && rustix::fs::flistxattr(file, &mut [0u8; 0]).is_ok_and(|length| length == 0)
        && macos_acl::is_empty(file)
}

#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "libc does not expose macOS descriptor ACL queries"
)]
mod macos_acl {
    use std::ffi::c_void;
    use std::io;
    use std::os::fd::AsRawFd;

    // These types and constants follow <sys/acl.h> in the macOS SDK.
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_uint) -> *mut c_void;
        fn acl_get_entry(
            acl: *mut c_void,
            entry_id: libc::c_int,
            entry: *mut *mut c_void,
        ) -> libc::c_int;
        fn acl_free(acl: *mut c_void) -> libc::c_int;
    }

    pub(super) fn is_empty(file: &fs_err::File) -> bool {
        const ACL_TYPE_EXTENDED: libc::c_uint = 0x100;
        const ACL_FIRST_ENTRY: libc::c_int = 0;
        // SAFETY: The descriptor is valid for the query. A successful result is an independently
        // allocated ACL; the entry query receives valid storage and the allocation is freed once.
        unsafe {
            let acl = acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED);
            if acl.is_null() {
                // The descriptor already exists; ENOENT means it has no extended ACL.
                return io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT);
            }
            let mut entry = std::ptr::null_mut();
            let result = acl_get_entry(acl, ACL_FIRST_ENTRY, &raw mut entry);
            // macOS reports EINVAL when the first entry is beyond an empty ACL's end.
            let empty =
                result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL);
            acl_free(acl);
            empty
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::io::Read;
    use std::io::{self, Write};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(target_os = "macos")]
    use std::process::Command;

    use super::{FilePublication, write_file};

    #[test]
    fn new_file_is_invisible_until_publication() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let mut publication = FilePublication::new(&path)?;
        assert!(publication.is_staged());
        publication.writer().write_all(b"complete")?;
        assert!(!path.exists());
        publication.publish()?;
        assert_eq!(fs_err::read(&path)?, b"complete");
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn new_file_does_not_replace_a_competing_creator() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let mut publication = FilePublication::new(&path)?;
        publication.writer().write_all(b"edited")?;
        fs_err::write(&path, "external")?;
        assert!(publication.publish().is_err());
        assert_eq!(fs_err::read(&path)?, b"external");
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn staging_failure_keeps_original_bytes() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        {
            let mut publication = FilePublication::new(&path)?;
            assert!(publication.is_staged());
            publication.writer().write_all(b"partial")?;
            // A failed or cancelled write drops staging without publishing its partial bytes.
        }
        assert_eq!(fs_err::read(&path)?, b"original");
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn readers_keep_complete_versions() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("script.py");
        fs_err::write(&path, "original")?;
        fs_err::set_permissions(&path, std::fs::Permissions::from_mode(0o751))?;
        let mut reader = fs_err::File::open(&path)?;
        write_file(&path, b"replacement")?;
        let mut previous = String::new();
        reader.read_to_string(&mut previous)?;
        assert_eq!(previous, "original");
        assert_eq!(fs_err::read(&path)?, b"replacement");
        assert_eq!(fs_err::metadata(&path)?.permissions().mode() & 0o777, 0o751);
        write_file(&path, b"")?;
        assert!(fs_err::read(&path)?.is_empty());
        Ok(())
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn publication_failure_keeps_original_bytes() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let mut publication = FilePublication::new(&path)?;
        assert!(publication.is_staged());
        publication.writer().write_all(b"replacement")?;
        let permissions = fs_err::metadata(directory.path())?.permissions();
        fs_err::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o500))?;
        let can_create = fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.path().join("authorization-probe"))
            .is_ok();
        let result = publication.publish();
        fs_err::set_permissions(directory.path(), permissions)?;
        // Privileged callers may rename through a mode-read-only directory.
        assert_eq!(result.is_ok(), can_create);
        if can_create {
            assert_eq!(fs_err::read(&path)?, b"replacement");
        } else {
            assert_eq!(fs_err::read(&path)?, b"original");
        }
        Ok(())
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn writable_file_in_read_only_directory_uses_in_place_fallback() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let permissions = fs_err::metadata(directory.path())?.permissions();
        fs_err::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o500))?;
        let result = write_file(&path, b"replacement");
        fs_err::set_permissions(directory.path(), permissions)?;
        result?;
        assert_eq!(fs_err::read(&path)?, b"replacement");
        Ok(())
    }

    #[test]
    fn hard_link_aliases_observe_the_write() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        let alias = directory.path().join("alias");
        fs_err::write(&path, "original")?;
        fs_err::hard_link(&path, &alias)?;
        assert!(!FilePublication::new(&path)?.is_staged());
        write_file(&path, b"replacement")?;
        assert_eq!(fs_err::read(&alias)?, b"replacement");
        assert!(same_file::is_same_file(&path, &alias)?);
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn symbolic_links_are_written_through() -> io::Result<()> {
        for exists in [false, true] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("script.py");
            let target = directory.path().join("target");
            if exists {
                fs_err::write(&target, "original")?;
            }
            fs_err::os::unix::fs::symlink("target", &path)?;
            write_file(&path, b"replacement")?;
            assert_eq!(fs_err::read_link(&path)?, Path::new("target"));
            assert_eq!(fs_err::read(&target)?, b"replacement");
        }
        Ok(())
    }

    #[test]
    fn read_only_file_requires_write_authorization() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let permissions = fs_err::metadata(&path)?.permissions();
        let mut read_only = permissions.clone();
        read_only.set_readonly(true);
        fs_err::set_permissions(&path, read_only)?;
        let ordinary = fs_err::OpenOptions::new().write(true).open(&path);
        let publication = FilePublication::new(&path);
        fs_err::set_permissions(&path, permissions)?;
        // Privileged users can legitimately open a mode-read-only file; compare authorization.
        assert_eq!(publication.is_ok(), ordinary.is_ok());
        assert_eq!(fs_err::read(&path)?, b"original");
        Ok(())
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_acl_uses_in_place_publication() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let status = Command::new("chmod")
            .args(["+a", "everyone deny delete"])
            .arg(&path)
            .status()?;
        assert!(status.success());
        let mut publication = FilePublication::new(&path)?;
        assert!(!publication.is_staged());
        publication.writer().set_len(0)?;
        publication.writer().write_all(b"replacement")?;
        publication.publish()?;
        let acl = Command::new("ls").arg("-le").arg(&path).output()?;
        assert!(acl.status.success());
        let cleanup = Command::new("chmod").arg("-N").arg(&path).status()?;
        assert!(cleanup.success());
        assert!(String::from_utf8_lossy(&acl.stdout).contains("everyone deny delete"));
        Ok(())
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn extended_metadata_uses_in_place_publication() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let file = fs_err::File::open(&path)?;
        rustix::fs::fsetxattr(
            &file,
            "user.uv-test",
            b"metadata",
            rustix::fs::XattrFlags::empty(),
        )?;
        assert!(!FilePublication::new(&path)?.is_staged());
        write_file(&path, b"replacement")?;
        let mut value = [0; 8];
        let length = rustix::fs::fgetxattr(&file, "user.uv-test", &mut value)?;
        assert_eq!(&value[..length], b"metadata");
        Ok(())
    }
}
