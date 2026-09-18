//! Identity primitives for opened Windows filesystem objects.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
use windows::Win32::Storage::FileSystem::{
    CheckNameLegalDOS8Dot3W, FILE_CASE_SENSITIVE_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FileCaseSensitiveInfo, FileIdInfo,
    GetFileInformationByHandleEx,
};
use windows::Win32::System::SystemServices::FILE_CS_FLAG_CASE_SENSITIVE_DIR;
use windows::core::{BOOL, PCWSTR};

/// A volume serial number and the complete 128-bit file identifier.
///
/// Retain an open handle when using this identity across a filesystem mutation: an identifier
/// can be reused after its file has been deleted and all handles have been closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    volume: u64,
    file: [u8; 16],
}

/// Open a file entry without traversing a reparse point at the final component.
pub fn open_file_entry(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

/// Open a directory, resolving directory aliases to the directory they designate.
pub fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)
}

impl FileIdentity {
    /// Query the complete identity of an open file or directory.
    #[expect(unsafe_code)]
    pub fn from_file(file: &File) -> io::Result<Self> {
        let mut info = FILE_ID_INFO::default();
        // SAFETY: `file` owns the live handle and `info` has the exact layout and size required
        // for `FileIdInfo`. The API writes the buffer synchronously.
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(file.as_raw_handle()),
                FileIdInfo,
                std::ptr::from_mut(&mut info).cast(),
                u32::try_from(size_of::<FILE_ID_INFO>()).map_err(io::Error::other)?,
            )
        }
        .map_err(io::Error::other)?;
        Ok(Self {
            volume: info.VolumeSerialNumber,
            file: info.FileId.Identifier,
        })
    }
}

/// Read the case-sensitive-name flag from an opened directory.
#[expect(unsafe_code)]
pub fn directory_is_case_sensitive(directory: &File) -> io::Result<bool> {
    let mut info = FILE_CASE_SENSITIVE_INFO::default();
    // SAFETY: `directory` owns the live handle and the buffer matches `FileCaseSensitiveInfo`.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(directory.as_raw_handle()),
            FileCaseSensitiveInfo,
            std::ptr::from_mut(&mut info).cast(),
            u32::try_from(size_of::<FILE_CASE_SENSITIVE_INFO>()).map_err(io::Error::other)?,
        )
    }
    .map_err(io::Error::other)?;
    Ok(info.Flags & FILE_CS_FLAG_CASE_SENSITIVE_DIR != 0)
}

/// Compare filenames using Windows' ordinal case-insensitive comparison.
#[expect(unsafe_code)]
pub fn names_equal_ordinal(left: &OsStr, right: &OsStr) -> io::Result<bool> {
    let left = left.encode_wide().collect::<Vec<_>>();
    let right = right.encode_wide().collect::<Vec<_>>();
    i32::try_from(left.len()).map_err(io::Error::other)?;
    i32::try_from(right.len()).map_err(io::Error::other)?;
    // SAFETY: The slices are initialized UTF-16 code units; their lengths fit the API's i32.
    let result = unsafe { CompareStringOrdinal(&left, &right, true) };
    if result.0 == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(result == CSTR_EQUAL)
}

/// Whether a missing filename could be a historical DOS short spelling.
///
/// DOS-name legality does not predict which alias a filesystem assigns. The length fallback
/// deliberately also covers short names whose original OEM code page is no longer available.
#[expect(unsafe_code)]
pub fn could_be_dos_short_name(name: &OsStr) -> io::Result<bool> {
    let mut name = name.encode_wide().collect::<Vec<_>>();
    if name.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in filename",
        ));
    }
    // GetLongPathName documents that a component longer than 12 characters, or with an
    // extension longer than three characters, cannot require short-name expansion.
    let bounded = name.len() <= 12
        && name
            .iter()
            .rposition(|unit| *unit == u16::from(b'.'))
            .is_none_or(|dot| name.len() - dot - 1 <= 3);
    name.push(0);
    let mut legal = BOOL::default();
    // SAFETY: `name` is NUL-terminated with no embedded NUL, and `legal` is a live output.
    unsafe { CheckNameLegalDOS8Dot3W(PCWSTR(name.as_ptr()), None, None, &raw mut legal) }
        .map_err(io::Error::other)?;
    Ok(legal.as_bool() || bounded)
}
