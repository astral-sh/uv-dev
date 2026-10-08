//! File replacement with a caller-owned recovery backup.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW};
use windows::core::PCWSTR;

/// Replace a file while merging its DACL and supported attributes into the replacement.
///
/// The caller must own all three paths and retain recovery information. A failed call can move
/// the original to `backup` without publishing `replacement`; this is not rollback-safe by itself.
#[expect(unsafe_code)]
pub fn replace_file_with_backup(
    destination: &Path,
    replacement: &Path,
    backup: &Path,
) -> io::Result<()> {
    let destination = terminated_path(destination)?;
    let replacement = terminated_path(replacement)?;
    let backup = terminated_path(backup)?;
    // SAFETY: Each path is a live NUL-terminated UTF-16 buffer without embedded NULs. Reserved
    // pointers are null, and no flags permit ignoring failures to retain access metadata.
    unsafe {
        ReplaceFileW(
            PCWSTR(destination.as_ptr()),
            PCWSTR(replacement.as_ptr()),
            PCWSTR(backup.as_ptr()),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    }
    .map_err(io::Error::other)
}

fn terminated_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in file replacement path",
        ));
    }
    encoded.push(0);
    Ok(encoded)
}
