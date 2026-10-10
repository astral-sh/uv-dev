use std::path::PathBuf;
use uv_normalize::PackageName;
use uv_redacted::DisplaySafeUrl;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),

    #[error(transparent)]
    WheelFilename(#[from] uv_distribution_filename::WheelFilenameError),

    #[error("Could not extract path segments from URL: {0}")]
    MissingPathSegments(String),

    #[error("Could not extract wheel filename from path: {}", _0.display())]
    MissingWheelFilename(PathBuf),

    #[error("Distribution not found at: {0}")]
    NotFound(DisplaySafeUrl),

    #[error("Requested package name `{0}` does not match `{1}` in the distribution filename: {2}")]
    PackageNameMismatch(PackageName, PackageName, String),

    #[error(
        "Source distribution `{0}` has a non-PEP 625-compliant filename; only `.tar.gz` and `.zip` archives are accepted"
    )]
    NotPep625Filename(String),
}

/// A wheel selection that does not point into the distribution's wheel collection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Selected wheel index {index} is out of bounds for {wheels} wheels")]
pub struct InvalidWheelSelection {
    pub(crate) index: usize,
    pub(crate) wheels: usize,
}
