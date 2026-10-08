//! File replacement with a caller-owned recovery backup.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use fs_err::os::windows::fs::OpenOptionsExt;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{
    EqualSid, GROUP_SECURITY_INFORMATION, GetFileSecurityW, GetSecurityDescriptorGroup,
    GetSecurityDescriptorOwner, GetSecurityDescriptorSacl, LABEL_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SCOPE_SECURITY_INFORMATION,
};
use windows::Win32::Storage::FileSystem::{
    DELETE, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO_EX,
    FILE_DISPOSITION_INFO_EX_FLAGS, FILE_FLAG_OPEN_REPARSE_POINT, FileDispositionInfoEx,
    REPLACE_FILE_FLAGS, ReplaceFileW, SetFileInformationByHandle,
};
use windows::core::{BOOL, PCWSTR};

/// Replace a file while merging its DACL and supported attributes into the replacement.
///
/// The caller must authorize replacement and retain recovery information for all three paths.
/// A failed call can move the original to `backup` without publishing `replacement`; this is not
/// rollback-safe by itself.
#[expect(unsafe_code)]
pub fn replace_file_with_backup(
    destination: &Path,
    replacement: &Path,
    backup: &Path,
) -> io::Result<()> {
    let destination = terminated_path(destination)?;
    let replacement = terminated_path(replacement)?;
    let backup = terminated_path(backup)?;
    // ReplaceFileW retains the DACL, but does not promise to retain ownership or mandatory policy.
    // Refuse a replacement whose freshly created inode would change those access restrictions.
    require_matching_access_policy(&destination, &replacement)?;
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

#[expect(unsafe_code)]
fn require_matching_access_policy(destination: &[u16], replacement: &[u16]) -> io::Result<()> {
    let mut destination_descriptor = access_descriptor(destination)?;
    let mut replacement_descriptor = access_descriptor(replacement)?;
    let destination = PSECURITY_DESCRIPTOR(destination_descriptor.as_mut_ptr().cast());
    let replacement = PSECURITY_DESCRIPTOR(replacement_descriptor.as_mut_ptr().cast());
    let mut destination_owner = PSID::default();
    let mut replacement_owner = PSID::default();
    let mut destination_group = PSID::default();
    let mut replacement_group = PSID::default();
    let mut defaulted = BOOL::default();
    // SAFETY: Both buffers contain complete security descriptors returned by GetFileSecurityW.
    // Their aligned backing allocations remain alive while the returned SID pointers are used.
    unsafe {
        GetSecurityDescriptorOwner(destination, &raw mut destination_owner, &raw mut defaulted)
            .map_err(io::Error::other)?;
        GetSecurityDescriptorOwner(replacement, &raw mut replacement_owner, &raw mut defaulted)
            .map_err(io::Error::other)?;
        GetSecurityDescriptorGroup(destination, &raw mut destination_group, &raw mut defaulted)
            .map_err(io::Error::other)?;
        GetSecurityDescriptorGroup(replacement, &raw mut replacement_group, &raw mut defaulted)
            .map_err(io::Error::other)?;
        if destination_owner.0.is_null()
            || replacement_owner.0.is_null()
            || destination_group.0.is_null()
            || replacement_group.0.is_null()
            || EqualSid(destination_owner, replacement_owner).is_err()
            || EqualSid(destination_group, replacement_group).is_err()
            || mandatory_policy(destination)? != mandatory_policy(replacement)?
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Executable replacement would change its ownership or mandatory access policy",
            ));
        }
    }
    Ok(())
}

#[expect(unsafe_code)]
fn access_descriptor(path: &[u16]) -> io::Result<Vec<u32>> {
    let information = OWNER_SECURITY_INFORMATION.0
        | GROUP_SECURITY_INFORMATION.0
        | LABEL_SECURITY_INFORMATION.0
        | SCOPE_SECURITY_INFORMATION.0;
    let mut bytes = 0;
    // SAFETY: `path` is a live NUL-terminated UTF-16 buffer. A null buffer requests the required
    // size; the second call receives an aligned allocation of at least that many bytes.
    unsafe {
        let _ = GetFileSecurityW(PCWSTR(path.as_ptr()), information, None, 0, &raw mut bytes);
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut descriptor = vec![
            0u32;
            usize::try_from(bytes)
                .map_err(io::Error::other)?
                .div_ceil(4)
        ];
        if !GetFileSecurityW(
            PCWSTR(path.as_ptr()),
            information,
            Some(PSECURITY_DESCRIPTOR(descriptor.as_mut_ptr().cast())),
            bytes,
            &raw mut bytes,
        )
        .as_bool()
        {
            return Err(io::Error::last_os_error());
        }
        Ok(descriptor)
    }
}

#[expect(unsafe_code)]
fn mandatory_policy(descriptor: PSECURITY_DESCRIPTOR) -> io::Result<Vec<u8>> {
    let mut present = BOOL::default();
    let mut defaulted = BOOL::default();
    let mut policy = std::ptr::null_mut();
    // SAFETY: The descriptor comes from GetFileSecurityW and its backing allocation remains
    // alive. Only mandatory-label and central-access-policy entries were requested in its SACL.
    // A non-null ACL contains AclSize initialized bytes within that descriptor allocation.
    unsafe {
        GetSecurityDescriptorSacl(
            descriptor,
            &raw mut present,
            &raw mut policy,
            &raw mut defaulted,
        )
        .map_err(io::Error::other)?;
        if !present.as_bool() || policy.is_null() {
            return Ok(Vec::new());
        }
        Ok(
            std::slice::from_raw_parts(policy.cast::<u8>(), usize::from((*policy).AclSize))
                .to_vec(),
        )
    }
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

/// Remove a recovery link without changing the attributes shared by its other hard links.
///
/// Ordinary deletion works on older Windows versions. A read-only link requires support for
/// ignoring that attribute during deletion; otherwise the caller retains its recovery state.
#[expect(unsafe_code)]
pub fn remove_file_preserving_attributes(path: &Path) -> io::Result<()> {
    match fs_err::remove_file(path) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {}
        Err(error) => return Err(error),
    }
    let file = fs_err::OpenOptions::new()
        .access_mode(DELETE.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    let disposition = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
            FILE_DISPOSITION_FLAG_DELETE.0
                | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0
                | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0,
        ),
    };
    // SAFETY: `file` owns the live handle with DELETE access. The initialized buffer has the
    // layout and size required by FileDispositionInfoEx and stays alive for the synchronous call.
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfoEx,
            std::ptr::from_ref(&disposition).cast(),
            u32::try_from(std::mem::size_of::<FILE_DISPOSITION_INFO_EX>())
                .map_err(io::Error::other)?,
        )
    }
    .map_err(io::Error::other)
}
