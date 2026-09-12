//! Integration tests for `uv lock`.

#[cfg(all(feature = "test-python", feature = "test-universal"))]
use uv_test::pypi_proxy;

#[cfg(feature = "test-python")]
mod lock;
