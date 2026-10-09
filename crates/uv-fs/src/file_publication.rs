//! Publication of user-owned files without replacing their link or access semantics.

use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use std::os::macos::fs::MetadataExt as _;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
use fs_err::os::unix::fs::OpenOptionsExt as _;

#[cfg(target_os = "linux")]
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, openat2};
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
            Ok(file) if can_replace(&file) && can_replace_path(path) => Some(file),
            Ok(file) => {
                return Ok(Self {
                    path: path.to_owned(),
                    writer: file,
                    staging: None,
                });
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };
        let target = destination(path)?;
        let original = match original {
            Some(original) => {
                same_identity(&original, &target)?;
                if !shares_parent_mount(&original, &target) {
                    // File mountpoints allow descriptor writes, but cannot be renamed over.
                    return Ok(Self {
                        path: path.to_owned(),
                        writer: original,
                        staging: None,
                    });
                }
                Some(original)
            }
            None => None,
        };
        let parent = target.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "file has no parent directory")
        })?;
        let temporary = match create_staging_file(parent, original.is_some()) {
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
                return Err(io::Error::new(
                    err.kind(),
                    PublicationError {
                        path: path.to_owned(),
                        source: err,
                    },
                ));
            }
        };
        let (file, temporary) = temporary.into_parts();
        let writer = fs_err::File::from_parts(file, path);
        let original = match original {
            Some(original) => {
                if !prepare_permissions(&original, &writer) {
                    return Ok(Self {
                        path: path.to_owned(),
                        writer: original,
                        staging: None,
                    });
                }
                Some(original)
            }
            None => None,
        };
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

    /// The resolved destination that publication will create, if no file existed there.
    ///
    /// This can differ from the requested path when writing through a dangling symbolic link.
    pub fn creation_path(&self) -> Option<&Path> {
        self.staging
            .as_ref()
            .filter(|staging| staging.original.is_none())
            .map(|staging| staging.target.as_path())
    }

    /// The descriptor receiving bytes. In-place callers must retain partial-write ownership.
    pub fn writer(&mut self) -> &mut fs_err::File {
        &mut self.writer
    }

    /// Publish the completed bytes, returning the identity of the resulting file.
    ///
    /// The identity check detects replaced paths but is not a filesystem compare-and-swap.
    /// Filesystems without no-clobber persistence use exclusive creation; readers can observe
    /// that newly created file while its contents are written.
    pub fn publish(self) -> io::Result<Handle> {
        self.publish_with(
            |temporary, target| temporary.persist_noclobber(target),
            |writer, contents| writer.write_all(contents),
        )
    }

    fn publish_with(
        self,
        persist_new: impl FnOnce(TempPath, &Path) -> Result<(), tempfile::PathPersistError>,
        write_new: impl FnOnce(&mut dyn Write, &[u8]) -> io::Result<()>,
    ) -> io::Result<Handle> {
        if let Some(staging) = self.staging {
            if destination(&self.path)? != staging.target {
                return Err(io::Error::other("file target changed before publication"));
            }
            if let Some(original) = &staging.original {
                same_identity(original, &self.path)?;
                same_identity(original, &staging.target)?;
                // A newly added ACL or hard link must not be discarded by inode replacement.
                if !can_replace(original)
                    || !can_replace_path(&self.path)
                    || !shares_parent_mount(original, &staging.target)
                    || !prepare_permissions(original, &self.writer)
                {
                    return Err(io::Error::other(
                        "file access metadata no longer permits replacement",
                    ));
                }
                // Obtain identity before changing the directory entry, without duplicating its
                // descriptor after publication has already succeeded.
                let identity = Handle::from_file(self.writer.into_file())?;
                staging
                    .temporary
                    .persist(verbatim_path(&staging.target))
                    .map_err(|error| publication_error(&self.path, error.error))?;
                Ok(identity)
            } else {
                let identity = Handle::from_file(self.writer.into_file())?;
                match persist_new(staging.temporary, &verbatim_path(&staging.target)) {
                    Ok(()) => Ok(identity),
                    Err(error) if unsupported_persistence(&error.error) => {
                        // Retain the private staging path until copying or its cleanup finishes.
                        let _temporary = error.path;
                        create_from_staging(identity, &staging.target, write_new)
                            .map_err(|error| publication_error(&self.path, error))
                    }
                    Err(error) => Err(publication_error(&self.path, error.error)),
                }
            }
        } else {
            Handle::from_file(self.writer.into_file())
        }
    }
}

fn publication_error(path: &Path, source: io::Error) -> io::Error {
    io::Error::new(
        source.kind(),
        PublicationError {
            path: path.to_owned(),
            source,
        },
    )
}

fn unsupported_persistence(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    #[cfg(unix)]
    {
        // Match the filesystem fallback used by LockedFile::create, including macOS ENOTSUP.
        matches!(
            rustix::io::Errno::from_io_error(error),
            Some(rustix::io::Errno::NOTSUP | rustix::io::Errno::INVAL)
        )
    }
    #[cfg(not(unix))]
    false
}

fn create_from_staging(
    mut staged: Handle,
    target: &Path,
    write: impl FnOnce(&mut dyn Write, &[u8]) -> io::Result<()>,
) -> io::Result<Handle> {
    let permissions = staged.as_file().metadata()?.permissions();
    staged.as_file_mut().rewind()?;
    let mut contents = Vec::new();
    staged.as_file_mut().read_to_end(&mut contents)?;
    drop(staged);

    let mut options = fs_err::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    options.mode(permissions.mode() & 0o7777);
    let file = options.open(verbatim_path(target))?;
    // Capture the new identity before writing any bytes. It can differ from the staged inode.
    let mut identity = Handle::from_file(file.into_file())?;
    let result = write(identity.as_file_mut(), &contents);
    #[cfg(not(unix))]
    let result = result.and_then(|()| {
        if permissions.readonly() {
            identity.as_file().set_permissions(permissions)?;
        }
        Ok(())
    });
    if let Err(source) = result {
        let cleanup = identity.as_file_mut().stream_position().and_then(|length| {
            let length = usize::try_from(length).map_err(io::Error::other)?;
            let written = contents.get(..length).ok_or_else(|| {
                io::Error::other("new file no longer matches its staged contents")
            })?;
            remove_incomplete(&mut identity, target, written)
        });
        return match cleanup {
            Ok(()) => Err(source),
            Err(cleanup) => Err(io::Error::new(
                source.kind(),
                IncompleteFileError {
                    path: target.to_owned(),
                    source,
                    cleanup,
                },
            )),
        };
    }
    Ok(identity)
}

/// Remove only the same entry with the bytes written by this failed creation.
fn remove_incomplete(identity: &mut Handle, path: &Path, written: &[u8]) -> io::Result<()> {
    let metadata = match fs_err::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.is_symlink() || metadata.len() != written.len() as u64 {
        return Err(io::Error::other("new file was changed outside publication"));
    }
    #[cfg(unix)]
    let same = {
        let opened = identity.as_file().metadata()?;
        opened.dev() == metadata.dev() && opened.ino() == metadata.ino()
    };
    #[cfg(not(unix))]
    let same = Handle::from_path(path)? == *identity;
    if !same {
        return Err(io::Error::other(
            "new file was replaced outside publication",
        ));
    }
    // Creation can authorize this descriptor even when the resulting mode denies fresh reads.
    let current = identity.as_file_mut();
    current.rewind()?;
    let mut buffer = [0; 8192];
    for expected in written.chunks(buffer.len()) {
        let actual = &mut buffer[..expected.len()];
        current.read_exact(actual)?;
        if actual != expected {
            return Err(io::Error::other("new file was changed outside publication"));
        }
    }
    if current.read(&mut [0])? != 0 {
        return Err(io::Error::other("new file was changed outside publication"));
    }
    fs_err::remove_file(path)
}

#[derive(Debug, thiserror::Error)]
#[error("failed to write to file `{}`", path.display())]
struct PublicationError {
    path: PathBuf,
    #[source]
    source: io::Error,
}

#[derive(Debug, thiserror::Error)]
#[error("could not remove incomplete file `{}`: {cleanup}", path.display())]
struct IncompleteFileError {
    path: PathBuf,
    #[source]
    source: io::Error,
    cleanup: io::Error,
}

fn create_staging_file(parent: &Path, replacing: bool) -> io::Result<tempfile::NamedTempFile> {
    #[expect(
        clippy::disallowed_types,
        reason = "report the destination, not the temporary path"
    )]
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        // Replacements start private: tightening permissions later cannot revoke an open descriptor.
        options.mode(if replacing { 0o600 } else { 0o666 });
    }
    #[cfg(not(unix))]
    let _ = replacing;
    // Keep the I/O cause free of the random staging path; callers report the destination instead.
    tempfile::Builder::new()
        .prefix(".uv-publish-")
        .make_in(verbatim_path(parent), |path| options.open(path))
}

/// Write complete bytes, staging compatible files before publication.
pub fn write_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut publication = FilePublication::new(path)?;
    if publication.writer().metadata()?.is_file() {
        publication.writer().set_len(0)?;
    }
    publication.writer().write_all(contents)?;
    if publication.is_staged() {
        publication.publish()?;
    }
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
    #[cfg(unix)]
    let same = {
        // Keep the descriptor open to prevent inode reuse, without requiring read access to bytes.
        let opened = file.metadata()?;
        let current = fs_err::metadata(path)?;
        opened.dev() == current.dev() && opened.ino() == current.ino()
    };
    #[cfg(not(unix))]
    let same = Handle::from_file(file.file().try_clone()?)? == Handle::from_path(path)?;
    if !same {
        return Err(io::Error::other("file was replaced before publication"));
    }
    Ok(())
}

/// Resolve the final link too, including dangling links whose target can be created.
fn destination(path: &Path) -> io::Result<PathBuf> {
    let mut target = path.to_owned();
    for _ in 0..=40 {
        match fs_err::symlink_metadata(&target) {
            Ok(metadata) if metadata.is_symlink() => {
                let link = fs_err::read_link(&target)?;
                target = target.parent().unwrap_or_else(|| Path::new("")).join(link);
            }
            Ok(_) => return fs_err::canonicalize(target),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                // `file_name` normalizes trailing separators and `.` components, but these
                // require a directory and must not become a newly created regular file.
                let bytes = target.as_os_str().as_encoded_bytes();
                let final_component = bytes
                    .rsplit(|byte| std::path::is_separator(char::from(*byte)))
                    .next();
                if let Some(b"" | b"." | b"..") = final_component {
                    return Err(err);
                }
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

#[cfg(target_os = "linux")]
fn can_replace_path(path: &Path) -> bool {
    // Descriptor-backed links refer to an inode, not the pathname reported by read_link.
    // Replacing that pathname would leave the requested link reading the previous bytes.
    // An unavailable or denied query keeps the authorized in-place write path.
    openat2(
        CWD,
        path,
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_MAGICLINKS,
    )
    .is_ok()
}

#[cfg(not(target_os = "linux"))]
fn can_replace_path(_path: &Path) -> bool {
    true
}

#[cfg(target_os = "linux")]
fn shares_parent_mount(file: &fs_err::File, target: &Path) -> bool {
    use rustix::fs::{AtFlags, CWD, StatxFlags, statx};

    let Some(parent) = target.parent() else {
        return false;
    };
    let mask = StatxFlags::MNT_ID;
    let Ok(file_metadata) = statx(file, "", AtFlags::EMPTY_PATH, mask) else {
        return false;
    };
    let Ok(parent_metadata) = statx(CWD, parent, AtFlags::empty(), mask) else {
        return false;
    };
    file_metadata.stx_mask & mask.bits() != 0
        && parent_metadata.stx_mask & mask.bits() != 0
        && file_metadata.stx_mnt_id == parent_metadata.stx_mnt_id
}

#[cfg(not(target_os = "linux"))]
fn shares_parent_mount(_file: &fs_err::File, _target: &Path) -> bool {
    true
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn prepare_permissions(original: &fs_err::File, staged: &fs_err::File) -> bool {
    let (Ok(original_metadata), Ok(staged_metadata)) = (original.metadata(), staged.metadata())
    else {
        return false;
    };
    if original_metadata.uid() != staged_metadata.uid()
        || original_metadata.gid() != staged_metadata.gid()
        || !plain_access_metadata(staged)
    {
        return false;
    }
    staged
        .set_permissions(original_metadata.permissions())
        .is_ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn prepare_permissions(_original: &fs_err::File, _staged: &fs_err::File) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn plain_access_metadata(file: &fs_err::File) -> bool {
    // POSIX ACLs, capabilities and security labels are extended attributes. Inode flags also
    // carry policy, such as `NODUMP` or `NOATIME`. Extents describe storage layout and are the only
    // permitted flag; unknown flags and unsupported queries require an in-place write.
    rustix::fs::flistxattr(file, &mut [0u8; 0]).is_ok_and(|length| length == 0)
        && rustix::fs::ioctl_getflags(file)
            .is_ok_and(|flags| flags.bits() & !linux_raw_sys::general::FS_EXTENT_FL == 0)
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
    #[cfg(target_os = "linux")]
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "macos")]
    use std::os::macos::fs::MetadataExt as _;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::os::unix::fs::MetadataExt;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    #[cfg(unix)]
    use std::process::Command;

    #[cfg(target_os = "linux")]
    use rustix::fs::{
        AtFlags, CWD, IFlags, Mode, OFlags, ResolveFlags, StatxFlags, ioctl_getflags,
        ioctl_setflags, openat2, statx,
    };

    use super::{FilePublication, write_file};

    /// Check the fixture's filesystem capabilities independently of publication's admission rules.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[expect(
        clippy::print_stderr,
        reason = "report unavailable staging prerequisites"
    )]
    fn staging_prerequisites(path: &Path) -> io::Result<bool> {
        let file = fs_err::File::open(path)?;
        let parent = path.parent().expect("fixture has a parent");
        let peer = tempfile::Builder::new()
            .prefix(".uv-publish-probe-")
            .tempfile_in(parent)?;
        let metadata = file.metadata()?;
        let peer_metadata = peer.as_file().metadata()?;
        let mut supported = metadata.is_file()
            && metadata.nlink() == 1
            && metadata.mode() & 0o7000 == 0
            && metadata.uid() == peer_metadata.uid()
            && metadata.gid() == peer_metadata.gid()
            && [file.file(), peer.as_file()].into_iter().all(|file| {
                rustix::fs::flistxattr(file, &mut [0u8; 0]).is_ok_and(|length| length == 0)
            });
        #[cfg(target_os = "linux")]
        {
            supported &= [file.file(), peer.as_file()].into_iter().all(|file| {
                ioctl_getflags(file)
                    .is_ok_and(|flags| flags.bits() & !linux_raw_sys::general::FS_EXTENT_FL == 0)
            });
            let mask = StatxFlags::MNT_ID;
            supported &= match (
                statx(&file, "", AtFlags::EMPTY_PATH, mask),
                statx(CWD, parent, AtFlags::empty(), mask),
            ) {
                (Ok(file), Ok(parent)) => {
                    file.stx_mask & mask.bits() != 0
                        && parent.stx_mask & mask.bits() != 0
                        && file.stx_mnt_id == parent.stx_mnt_id
                }
                _ => false,
            };
            supported &= openat2(
                CWD,
                path,
                OFlags::PATH | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::NO_MAGICLINKS,
            )
            .is_ok();
        }
        #[cfg(target_os = "macos")]
        {
            supported &= metadata.st_flags() == 0 && peer_metadata.st_flags() == 0;
            for path in [path, peer.path()] {
                let acl = Command::new("ls")
                    .args(["-lde"])
                    .env("LC_ALL", "C")
                    .arg(path)
                    .output()?;
                supported &= acl.status.success()
                    && String::from_utf8_lossy(&acl.stdout)
                        .split_whitespace()
                        .next()
                        .is_some_and(|permissions| !permissions.contains('+'));
            }
        }
        if !supported {
            eprintln!(
                "skipping staged replacement assertions: filesystem metadata or path lookup is unsupported for `{}`",
                path.display()
            );
        }
        Ok(supported)
    }

    #[test]
    #[cfg(unix)]
    fn replacement_staging_starts_private() -> io::Result<()> {
        const CHILD: &str = "UV_TEST_STAGING_CREATION_MODE";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new("sh")
                .args(["-c", r#"umask 022; exec "$@""#, "sh"])
                .arg(std::env::current_exe()?)
                .args([
                    "--exact",
                    "file_publication::tests::replacement_staging_starts_private",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()?;
            assert!(output.status.success(), "{output:?}");
            return Ok(());
        }
        let directory = tempfile::tempdir()?;
        let replacement = super::create_staging_file(directory.path(), true)?;
        assert_eq!(
            replacement.as_file().metadata()?.permissions().mode() & 0o777,
            0o600
        );
        let new_file = super::create_staging_file(directory.path(), false)?;
        assert_eq!(
            new_file.as_file().metadata()?.permissions().mode() & 0o777,
            0o644
        );
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn special_file_writes_do_not_truncate() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        fs_err::os::unix::fs::symlink("/dev/null", &path)?;
        write_file(&path, b"lock contents")?;
        assert_eq!(fs_err::read_link(&path)?, Path::new("/dev/null"));
        assert!(fs_err::read(&path)?.is_empty());
        Ok(())
    }

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
        let error = publication.publish().expect_err("competing creator wins");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            error.to_string(),
            format!("failed to write to file `{}`", path.display())
        );
        let source = error
            .get_ref()
            .and_then(|error| error.source())
            .and_then(|error| error.downcast_ref::<io::Error>())
            .expect("the original I/O error is retained");
        assert_eq!(source.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs_err::read(&path)?, b"external");
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    fn unsupported_noclobber(
        path: tempfile::TempPath,
        _target: &Path,
    ) -> Result<(), tempfile::PathPersistError> {
        #[cfg(unix)]
        let error = rustix::io::Errno::NOTSUP.into();
        #[cfg(not(unix))]
        let error = io::ErrorKind::Unsupported.into();
        Err(tempfile::PathPersistError { error, path })
    }

    #[test]
    fn unsupported_noclobber_returns_created_file_identity() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let mut publication = FilePublication::new(&path)?;
        publication.writer().write_all(b"complete")?;
        let staged = same_file::Handle::from_file(publication.writer().file().try_clone()?)?;
        let identity = publication.publish_with(unsupported_noclobber, |writer, contents| {
            writer.write_all(contents)
        })?;
        assert_eq!(identity, same_file::Handle::from_path(&path)?);
        assert_ne!(identity, staged);
        drop(staged);
        assert_eq!(fs_err::read(&path)?, b"complete");
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn unsupported_noclobber_writes_through_a_dangling_link() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let target = directory.path().join("created.lock");
        fs_err::os::unix::fs::symlink("created.lock", &path)?;
        let mut publication = FilePublication::new(&path)?;
        publication.writer().write_all(b"complete")?;
        let identity = publication.publish_with(unsupported_noclobber, |writer, contents| {
            writer.write_all(contents)
        })?;
        assert_eq!(identity, same_file::Handle::from_path(&target)?);
        assert_eq!(fs_err::read(&target)?, b"complete");
        assert_eq!(fs_err::read_link(&path)?, Path::new("created.lock"));
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 2);
        Ok(())
    }

    #[test]
    fn unsupported_noclobber_keeps_a_competing_creator() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let mut publication = FilePublication::new(&path)?;
        publication.writer().write_all(b"complete")?;
        let error = publication
            .publish_with(
                |temporary, target| {
                    fs_err::write(target, b"foreign").expect("publish competing file");
                    #[cfg(unix)]
                    let error = rustix::io::Errno::INVAL.into();
                    #[cfg(not(unix))]
                    let error = io::ErrorKind::Unsupported.into();
                    Err(tempfile::PathPersistError {
                        error,
                        path: temporary,
                    })
                },
                |writer, contents| writer.write_all(contents),
            )
            .expect_err("exclusive creation rejects the competing entry");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs_err::read(&path)?, b"foreign");
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn unsupported_noclobber_cleans_a_failed_partial_creation() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let mut publication = FilePublication::new(&path)?;
        publication.writer().write_all(b"complete")?;
        let error = publication
            .publish_with(unsupported_noclobber, |writer, contents| {
                writer.write_all(&contents[..3])?;
                Err(io::ErrorKind::WriteZero.into())
            })
            .expect_err("injected partial write failure");
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
        assert!(!path.try_exists()?);
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 0);
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn unsupported_noclobber_cleans_a_write_only_partial_creation() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let mut publication = FilePublication::new(&path)?;
        publication.writer().write_all(b"complete")?;
        publication
            .writer()
            .set_permissions(std::fs::Permissions::from_mode(0o200))?;
        let error = publication
            .publish_with(unsupported_noclobber, |writer, contents| {
                writer.write_all(&contents[..3])?;
                Err(io::ErrorKind::WriteZero.into())
            })
            .expect_err("injected partial write failure on a write-only file");
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
        assert!(!path.try_exists()?);
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn failed_creation_keeps_foreign_in_place_edits() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let mut publication = FilePublication::new(&path)?;
        publication.writer().write_all(b"complete")?;
        let error = publication
            .publish_with(unsupported_noclobber, |writer, contents| {
                writer.write_all(&contents[..3])?;
                fs_err::write(&path, b"new")?;
                Err(io::ErrorKind::WriteZero.into())
            })
            .expect_err("injected failure after a foreign edit");
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
        assert_eq!(fs_err::read(&path)?, b"new");
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn failed_creation_keeps_a_foreign_replacement() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let moved = directory.path().join("moved.lock");
        let mut publication = FilePublication::new(&path)?;
        publication.writer().write_all(b"complete")?;
        let error = publication
            .publish_with(unsupported_noclobber, |writer, contents| {
                writer.write_all(&contents[..3])?;
                fs_err::rename(&path, &moved)?;
                fs_err::write(&path, b"com")?;
                Err(io::ErrorKind::WriteZero.into())
            })
            .expect_err("injected failure after a foreign replacement");
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
        assert_eq!(fs_err::read(&path)?, b"com");
        assert_eq!(fs_err::read(&moved)?, b"com");
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 2);
        Ok(())
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn staging_failure_keeps_original_bytes() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        if !staging_prerequisites(&path)? {
            return Ok(());
        }
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
        if !staging_prerequisites(&path)? {
            return Ok(());
        }
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
        if !staging_prerequisites(&path)? {
            return Ok(());
        }
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
    #[cfg(target_os = "linux")]
    fn descriptor_links_are_written_in_place() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        let target = directory.path().join("target");
        fs_err::write(&target, "original")?;
        if !staging_prerequisites(&target)? {
            return Ok(());
        }
        assert!(FilePublication::new(&target)?.is_staged());
        let mut descriptor = fs_err::File::open(&target)?;
        let link = format!("/proc/self/fd/{}", descriptor.as_raw_fd());
        fs_err::os::unix::fs::symlink(&link, &path)?;

        write_file(&path, b"replacement")?;
        assert_eq!(fs_err::read(&path)?, b"replacement");
        assert_eq!(fs_err::read(&target)?, b"replacement");
        assert_eq!(fs_err::read_link(&path)?, Path::new(&link));
        let mut contents = String::new();
        descriptor.read_to_string(&mut contents)?;
        assert_eq!(contents, "replacement");
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
    #[cfg(unix)]
    fn dangling_link_to_directory_suffix_is_not_created() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        fs_err::os::unix::fs::symlink("missing/", &path)?;
        let error = write_file(&path, b"replacement").expect_err("target requires a directory");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(fs_err::read_link(&path)?, Path::new("missing/"));
        assert!(!directory.path().join("missing").exists());
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn dangling_link_to_dot_directory_is_not_created() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("uv.lock");
        fs_err::os::unix::fs::symlink("missing/.", &path)?;
        let error = write_file(&path, b"replacement").expect_err("target requires a directory");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(fs_err::read_link(&path)?, Path::new("missing/."));
        assert!(!directory.path().join("missing").exists());
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn missing_directory_destinations_are_not_created() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        assert!(write_file(&directory.path().join("missing/"), b"bytes").is_err());
        assert!(write_file(&directory.path().join("missing/."), b"bytes").is_err());
        assert!(write_file(&directory.path().join("missing//"), b"bytes").is_err());
        assert!(write_file(&directory.path().join("missing/../"), b"bytes").is_err());
        #[cfg(windows)]
        {
            assert!(write_file(&directory.path().join("missing\\"), b"bytes").is_err());
            assert!(write_file(&directory.path().join("missing\\."), b"bytes").is_err());
        }
        assert_eq!(fs_err::read_dir(directory.path())?.count(), 0);
        Ok(())
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn a_retargeted_path_keeps_its_symbolic_link() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        let moved = directory.path().join("moved.toml");
        fs_err::write(&path, "original")?;
        if !staging_prerequisites(&path)? {
            return Ok(());
        }
        let mut publication = FilePublication::new(&path)?;
        publication.writer().write_all(b"replacement")?;
        fs_err::rename(&path, &moved)?;
        fs_err::os::unix::fs::symlink("moved.toml", &path)?;
        assert!(publication.publish().is_err());
        assert_eq!(fs_err::read_link(&path)?, Path::new("moved.toml"));
        assert_eq!(fs_err::read(&moved)?, b"original");
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
    #[cfg(unix)]
    fn writing_does_not_require_read_authorization() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("pyproject.toml");
        fs_err::write(&path, "original")?;
        let permissions = fs_err::metadata(&path)?.permissions();
        fs_err::set_permissions(&path, std::fs::Permissions::from_mode(0o200))?;
        let opened = fs_err::OpenOptions::new().write(true).open(&path)?;
        let identity = super::same_identity(&opened, &path);
        let result = write_file(&path, b"replacement");
        fs_err::set_permissions(&path, permissions)?;
        identity?;
        result?;
        assert_eq!(fs_err::read(&path)?, b"replacement");
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
    fn linux_inode_flags_use_in_place_publication() -> io::Result<()> {
        for flag in [IFlags::NODUMP, IFlags::NOATIME] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("pyproject.toml");
            fs_err::write(&path, "original")?;
            let file = fs_err::OpenOptions::new().write(true).open(&path)?;
            let Ok(original) = ioctl_getflags(&file) else {
                // Filesystems without flag queries must use the conservative fallback too.
                assert!(!FilePublication::new(&path)?.is_staged());
                write_file(&path, b"replacement")?;
                assert_eq!(fs_err::read(&path)?, b"replacement");
                return Ok(());
            };
            ioctl_setflags(&file, original | flag)?;
            assert!(!FilePublication::new(&path)?.is_staged());
            write_file(&path, b"replacement")?;
            assert_eq!(fs_err::read(&path)?, b"replacement");
            assert_eq!(ioctl_getflags(&file)?, original | flag);
            let published = fs_err::File::open(&path)?;
            assert_eq!(ioctl_getflags(&published)?, original | flag);
        }
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

    #[test]
    #[cfg(target_os = "linux")]
    #[expect(clippy::print_stderr, reason = "report unavailable namespace fixtures")]
    fn linux_file_mountpoints_use_in_place_publication() -> io::Result<()> {
        const CHILD: &str = "UV_TEST_FILE_MOUNTPOINT";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = std::path::PathBuf::from(root);
            let source = root.join("source.toml");
            let target = root.join("pyproject.toml");
            fs_err::write(&source, "original")?;
            fs_err::write(&target, "covered file")?;
            if !staging_prerequisites(&source)? {
                return Ok(());
            }
            let mounted = Command::new("mount")
                .arg("--bind")
                .arg(&source)
                .arg(&target)
                .output()?;
            assert!(mounted.status.success(), "{mounted:?}");
            write_file(&target, b"replacement")?;
            assert!(!FilePublication::new(&target)?.is_staged());
            assert_eq!(fs_err::read(&source)?, b"replacement");
            assert_eq!(fs_err::read(&target)?, b"replacement");
            let unmounted = Command::new("umount").arg(&target).output()?;
            assert!(unmounted.status.success(), "{unmounted:?}");
            assert_eq!(fs_err::read(&target)?, b"covered file");
            return Ok(());
        }

        // The child owns a private mount namespace. Its exit releases mounts even after a panic,
        // and the parent's directory owner removes the fixture only after the child has exited.
        let directory = tempfile::tempdir()?;
        let output = match Command::new("unshare")
            .args([
                "--user",
                "--map-root-user",
                "--mount",
                "--propagation",
                "private",
            ])
            .arg(std::env::current_exe()?)
            .args([
                "--exact",
                "file_publication::tests::linux_file_mountpoints_use_in_place_publication",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, directory.path())
            .env("LC_ALL", "C")
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                eprintln!("skipping file-mount regression: unshare is unavailable");
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success()
            && stderr.starts_with("unshare:")
            && (stderr.contains("Operation not permitted") || stderr.contains("Permission denied"))
        {
            eprintln!("skipping file-mount regression: private user namespaces are unavailable");
            return Ok(());
        }
        assert!(output.status.success(), "{output:?}");
        Ok(())
    }
}
