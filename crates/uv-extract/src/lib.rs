use std::path::{Path, PathBuf};

pub use error::Error;
use regex::regex;
use rustc_hash::FxHashSet;
pub use sync::*;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uv_static::EnvVars;

mod archive_path;
pub mod dirhash;
mod error;
pub mod hash;
pub mod stream;
mod sync;
mod vendor;

/// Default buffer size for archive reads and file copies.
pub(crate) const DEFAULT_BUF_SIZE: usize = 128 * 1024;

/// Cap a ZIP entry's output buffer at 1 MiB, or defer to the writer's default if the entry size
/// exceeds the platform's address space.
pub(crate) fn zip_entry_buffer_capacity(uncompressed_size: u64) -> Option<usize> {
    usize::try_from(uncompressed_size)
        .ok()
        .map(|size| size.min(1024 * 1024))
}

/// Track directories whose creation succeeded during archive extraction.
#[derive(Default)]
pub(crate) struct ExtractionDirectories {
    created: FxHashSet<PathBuf>,
}

impl ExtractionDirectories {
    fn create(&mut self, path: &Path) -> Result<(), Error> {
        if !self.created.contains(path) {
            fs_err::create_dir_all(path).map_err(Error::Io)?;
            self.created.insert(path.to_path_buf());
        }
        Ok(())
    }
}

/// Copy a ZIP entry, preserving ZIP read errors separately from filesystem write errors.
pub(crate) async fn copy_zip_entry(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    buffer: &mut Vec<u8>,
) -> Result<u64, Error> {
    buffer.resize(DEFAULT_BUF_SIZE, 0);
    let mut copied = 0;
    loop {
        let read = reader.read(buffer).await.map_err(Error::io_or_zip)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read]).await.map_err(Error::Io)?;
        copied += read as u64;
    }
    writer.flush().await.map_err(Error::Io)?;
    Ok(copied)
}

/// Validate the empty payload declared by a ZIP directory entry.
pub(crate) fn validate_zip_directory_entry(
    entry: &async_zip::ZipEntry,
    path: &Path,
) -> Result<(), Error> {
    if entry.crc32() != 0 {
        return Err(Error::BadCrc32 {
            path: path.to_path_buf(),
            computed: 0,
            expected: entry.crc32(),
        });
    }
    if entry.uncompressed_size() != 0 {
        return Err(Error::BadUncompressedSize {
            path: path.to_path_buf(),
            computed: 0,
            expected: entry.uncompressed_size(),
        });
    }
    Ok(())
}

/// Set executable permissions according to pip's wheel extraction behavior.
#[cfg(unix)]
fn set_executable_bits(path: &Path) -> Result<(), Error> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    // https://github.com/pypa/pip/blob/3898741e29b7279e7bffe044ecfbe20f6a438b1e/src/pip/_internal/utils/unpacking.py#L88-L100
    let permissions = fs_err::metadata(path).map_err(Error::Io)?.permissions();
    if permissions.mode() & 0o111 == 0o111 {
        return Ok(());
    }

    fs_err::set_permissions(path, Permissions::from_mode(permissions.mode() | 0o111))
        .map_err(Error::Io)
}

static REPLACEMENT_CHARACTER: &str = "\u{FFFD}";

/// Validate that a given filename (e.g. reported by a ZIP archive's
/// local file entries or central directory entries) is "safe" to use.
///
/// "Safe" in this context doesn't refer to directory traversal
/// risk, but whether we believe that other ZIP implementations
/// handle the name correctly and consistently.
///
/// Specifically, we want to avoid names that:
///
/// - Contain *any* non-printable characters
/// - Are empty
///
/// In the future, we may also want to check for names that contain
/// leading/trailing whitespace, or names that are exceedingly long.
pub(crate) fn validate_archive_member_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::EmptyFilename);
    }

    match regex!(r"\p{C}").replace_all(name, REPLACEMENT_CHARACTER) {
        // No replacements mean no control characters.
        std::borrow::Cow::Borrowed(_) => Ok(()),
        std::borrow::Cow::Owned(sanitized) => Err(Error::UnacceptableFilename {
            filename: sanitized,
        }),
    }
}

/// Returns `true` if ZIP validation is disabled.
pub fn insecure_no_validate() -> bool {
    // TODO(charlie) Parse this in `EnvironmentOptions`.
    let Some(value) = std::env::var_os(EnvVars::UV_INSECURE_NO_ZIP_VALIDATION) else {
        return false;
    };
    let Some(value) = value.to_str() else {
        return false;
    };
    matches!(
        value.to_lowercase().as_str(),
        "y" | "yes" | "t" | "true" | "on" | "1"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_extraction_directory_retries_after_failed_creation() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let blocker = temp.path().join("parent");
        let directory = blocker.join("child");
        fs_err::write(&blocker, b"")?;

        let mut directories = super::ExtractionDirectories::default();
        assert!(directories.create(&directory).is_err());
        fs_err::remove_file(&blocker)?;
        directories.create(&directory)?;
        assert!(directory.is_dir());
        Ok(())
    }

    #[test]
    fn test_validate_archive_member_name() {
        for (testcase, ok) in &[
            // Valid cases.
            ("normal.txt", true),
            ("__init__.py", true),
            ("fine i guess.py", true),
            ("🌈.py", true),
            // Invalid cases.
            ("", false),
            ("new\nline.py", false),
            ("carriage\rreturn.py", false),
            ("tab\tcharacter.py", false),
            ("null\0byte.py", false),
            ("control\x01code.py", false),
            ("control\x02code.py", false),
            ("control\x03code.py", false),
            ("control\x04code.py", false),
            ("backspace\x08code.py", false),
            ("delete\x7fcode.py", false),
        ] {
            assert_eq!(
                super::validate_archive_member_name(testcase).is_ok(),
                *ok,
                "testcase: {testcase}"
            );
        }
    }

    #[test]
    fn test_unacceptable_filename_error_replaces_control_characters() {
        let err = super::validate_archive_member_name("bad\nname").unwrap_err();
        match err {
            super::Error::UnacceptableFilename { filename } => {
                assert_eq!(filename, "bad�name");
            }
            _ => panic!("expected UnacceptableFilename error"),
        }
    }
}
