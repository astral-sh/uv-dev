//! Integration tests for `uv lock`.

#[cfg(all(
    feature = "test-python",
    feature = "test-pypi",
    feature = "test-universal"
))]
use uv_test::pypi_proxy;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod lock;

#[cfg(feature = "test-python")]
mod lockfile_format;

#[cfg(all(feature = "test-python", feature = "test-universal"))]
mod marker_normalization;

#[cfg(all(feature = "test-python", feature = "test-universal"))]
mod minimum_libc;

#[cfg(feature = "test-python")]
mod source_validation;

mod validation;

#[cfg(all(
    feature = "test-python",
    feature = "test-pypi",
    feature = "test-universal"
))]
mod workspace_sources;
