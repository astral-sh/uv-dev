//! Read-result contracts for the installed-sidecar benchmarks.

use std::io::{self, Read};
use std::path::Path;

pub(super) type ReadResult = io::Result<Option<Vec<u8>>>;

/// Use the same contextual file wrapper as installed-distribution metadata reads.
pub(super) fn read_file(path: &Path) -> ReadResult {
    read_opened(fs_err::File::open(path))
}

/// An untimed oracle that retains operating-system errors instead of adding `fs_err` context.
#[expect(
    clippy::disallowed_types,
    reason = "The raw-I/O oracle must retain the original operating-system error"
)]
pub(super) fn read_raw(path: &Path) -> ReadResult {
    read_opened(std::fs::File::open(path))
}

fn read_opened(file: io::Result<impl Read>) -> ReadResult {
    let mut file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;
    Ok(Some(contents))
}

/// `fs_err` retains the error kind but contextualizes the error, hiding the outer raw errno.
pub(super) fn comparable(result: &ReadResult) -> Result<Option<&[u8]>, io::ErrorKind> {
    comparable_raw(result).map_err(|(kind, _)| kind)
}

/// Compare raw I/O against the independent standard-library oracle without weakening errno checks.
pub(super) fn comparable_raw(
    result: &ReadResult,
) -> Result<Option<&[u8]>, (io::ErrorKind, Option<i32>)> {
    match result {
        Ok(contents) => Ok(contents.as_deref()),
        Err(error) => Err((error.kind(), error.raw_os_error())),
    }
}
