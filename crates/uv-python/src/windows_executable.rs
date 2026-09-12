//! Conservative version hints from Windows CPython executables.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;
use std::ptr;

use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
};
use windows::core::HSTRING;

/// Read the major and minor version of a recognized CPython console executable.
///
/// Version resources on launchers and virtual-environment redirectors describe the launcher, not
/// necessarily the interpreter it starts. Missing, unreadable, or unrecognized metadata must fall
/// back to querying Python.
pub(crate) fn cpython_version(path: &Path) -> Option<(u8, u8)> {
    if path
        .extension()
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("exe"))
    {
        return None;
    }

    let metadata = VersionInfo::from_path(path)?;
    from_metadata(
        metadata.fixed()?,
        &metadata.string(r"\StringFileInfo\000004b0\ProductName")?,
        &metadata.string(r"\StringFileInfo\000004b0\InternalName")?,
        &metadata.string(r"\StringFileInfo\000004b0\OriginalFilename")?,
    )
}

fn from_metadata(
    info: VS_FIXEDFILEINFO,
    product_name: &str,
    internal_name: &str,
    original_filename: &str,
) -> Option<(u8, u8)> {
    if info.dwSignature != 0xfeef_04bd
        || product_name != "Python"
        || internal_name != "Python Console"
        || !matches!(original_filename, "python.exe" | "python_d.exe")
    {
        return None;
    }

    // CPython encodes its patch and release level together in the third fixed-file version
    // component. Only the major and minor components can be read as Python version segments.
    let major = u8::try_from(info.dwFileVersionMS >> 16).ok()?;
    let minor = u8::try_from(info.dwFileVersionMS & 0xffff).ok()?;
    Some((major, minor))
}

/// An initialized Win32 version-information buffer.
struct VersionInfo {
    data: Vec<u16>,
}

impl VersionInfo {
    fn from_path(path: &Path) -> Option<Self> {
        let path = HSTRING::from(path.as_os_str());
        if path.contains(&0) {
            return None;
        }

        // SAFETY: `path` is a null-terminated Windows string.
        #[expect(unsafe_code)]
        let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
        // This is only an optimization. Do not allocate an excessive buffer for an unusual or
        // malformed resource when executing the interpreter remains available as a fallback.
        if size == 0 || size > 1024 * 1024 {
            return None;
        }
        let mut data = vec![0_u16; usize::try_from(size).ok()?.div_ceil(size_of::<u16>())];

        // SAFETY: `data` is initialized and has at least `size` writable bytes. `path` remains
        // valid for the duration of the call.
        #[expect(unsafe_code)]
        unsafe {
            GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()).ok()?;
        }
        Some(Self { data })
    }

    fn query(&self, key: &str) -> Option<(*mut c_void, usize)> {
        let key = HSTRING::from(key);
        let mut value = ptr::null_mut();
        let mut length = 0;

        // SAFETY: `data` was initialized by `GetFileVersionInfoW`, `key` is a valid
        // null-terminated string, and both output pointers are valid for the call.
        #[expect(unsafe_code)]
        let found = unsafe {
            VerQueryValueW(
                self.data.as_ptr().cast(),
                &key,
                &raw mut value,
                &raw mut length,
            )
        };
        if !found.as_bool() {
            return None;
        }
        Some((value, usize::try_from(length).ok()?))
    }

    /// Check that an API-returned value lies inside the initialized buffer.
    fn byte_offset(&self, value: *const c_void, length: usize) -> Option<usize> {
        let offset = value.addr().checked_sub(self.data.as_ptr().addr())?;
        let end = offset.checked_add(length)?;
        let capacity = self.data.len().checked_mul(size_of::<u16>())?;
        (end <= capacity).then_some(offset)
    }

    fn fixed(&self) -> Option<VS_FIXEDFILEINFO> {
        let (value, length) = self.query("\\")?;
        if length < size_of::<VS_FIXEDFILEINFO>() {
            return None;
        }
        self.byte_offset(value, size_of::<VS_FIXEDFILEINFO>())?;

        // SAFETY: The complete fixed-file structure is inside the initialized buffer. Use an
        // unaligned read because the buffer only guarantees UTF-16 alignment.
        #[expect(unsafe_code)]
        Some(unsafe { value.cast::<VS_FIXEDFILEINFO>().read_unaligned() })
    }

    fn string(&self, key: &str) -> Option<String> {
        let (value, length) = self.query(key)?;
        let offset = self.byte_offset(value, length.checked_mul(size_of::<u16>())?)?;
        if !offset.is_multiple_of(size_of::<u16>()) {
            return None;
        }
        let start = offset / size_of::<u16>();
        let end = start.checked_add(length)?;
        let value = self.data.get(start..end)?.strip_suffix(&[0])?;
        String::from_utf16(value).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{VS_FIXEDFILEINFO, cpython_version, from_metadata};

    fn metadata(major: u16, minor: u16) -> VS_FIXEDFILEINFO {
        VS_FIXEDFILEINFO {
            dwSignature: 0xfeef_04bd,
            dwFileVersionMS: (u32::from(major) << 16) | u32::from(minor),
            // CPython 3.13.7 final: patch 7, release level 15, release serial 0.
            dwFileVersionLS: (0x1bee << 16) | 0x03f5,
            ..VS_FIXEDFILEINFO::default()
        }
    }

    #[test]
    fn cpython_major_minor_ignores_encoded_patch() {
        assert_eq!(
            from_metadata(metadata(3, 13), "Python", "Python Console", "python.exe"),
            Some((3, 13)),
        );
        assert_eq!(
            from_metadata(metadata(3, 13), "Python", "Python Console", "python_d.exe"),
            Some((3, 13)),
        );
    }

    #[test]
    fn only_cpython_console_resources_are_version_hints() {
        for (product, internal, original) in [
            ("Python", "Python Launcher", "py.exe"),
            ("Python", "Python Console", "py.exe"),
            ("Python", "Python Windowed", "pythonw.exe"),
            ("PyPy", "Python Console", "python.exe"),
        ] {
            assert_eq!(
                from_metadata(metadata(3, 13), product, internal, original),
                None
            );
        }
        assert_eq!(
            from_metadata(metadata(256, 13), "Python", "Python Console", "python.exe"),
            None,
        );
        assert_eq!(
            from_metadata(metadata(3, 256), "Python", "Python Console", "python.exe"),
            None,
        );
        assert_eq!(
            from_metadata(
                VS_FIXEDFILEINFO::default(),
                "Python",
                "Python Console",
                "python.exe"
            ),
            None,
        );
    }

    #[test]
    fn unknown_executables_have_no_version_hint() -> std::io::Result<()> {
        let directory = tempfile::tempdir()?;
        assert_eq!(cpython_version(&directory.path().join("missing.exe")), None);
        let executable = directory.path().join("python.exe");
        fs_err::write(&executable, b"MZ")?;
        assert_eq!(cpython_version(&executable), None);
        let batch = directory.path().join("python.bat");
        fs_err::write(&batch, b"@echo off\r\n")?;
        assert_eq!(cpython_version(&batch), None);
        Ok(())
    }
}
