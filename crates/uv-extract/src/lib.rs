use std::path::{Path, PathBuf};

pub use error::Error;
use regex::regex;
use rustc_hash::FxHashSet;
pub use sync::*;
use uv_static::EnvVars;

mod archive_path;
pub mod dirhash;
mod error;
pub mod hash;
pub mod stream;
mod sync;
mod vendor;

/// Track directories whose creation has been attempted during archive extraction.
#[derive(Default)]
pub(crate) struct ExtractionDirectories {
    attempted: FxHashSet<PathBuf>,
}

impl ExtractionDirectories {
    pub(crate) fn create(&mut self, path: &Path) -> Result<(), Error> {
        if !self.attempted.contains(path) {
            self.attempted.insert(path.to_path_buf());
            fs_err::create_dir_all(path).map_err(Error::Io)?;
        }
        Ok(())
    }
}

/// Set executable permissions according to pip's wheel extraction behavior.
#[cfg(unix)]
pub(crate) fn set_executable_bits(path: &Path) -> Result<(), Error> {
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
