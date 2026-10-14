//! Vendored from cachedir 0.3.1 to replace `std::fs` with `fs_err`.

use std::io::Write;
use std::{io, path};

/// The `CACHEDIR.TAG` file header as defined by the [specification](https://bford.info/cachedir/).
const HEADER: &[u8; 43] = b"Signature: 8a477f597d28d172789f06886806bc55";

/// Ensures the tag exists in `directory`.
///
/// This function considers the `CACHEDIR.TAG` file in `directory` existing, regardless of its
/// content, as a success.
///
/// Will return an error if The tag file doesn't exist and can't be created for any reason
/// (the `directory` doesn't exist, permission error, can't write to the file etc.).
pub fn ensure_tag<P: AsRef<path::Path>>(directory: P) -> io::Result<()> {
    let tag = directory.as_ref().join("CACHEDIR.TAG");
    match fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tag)
        .and_then(|mut file| file.write_all(HEADER))
    {
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied && tag.exists() => Ok(()),
        result => result,
    }
}
