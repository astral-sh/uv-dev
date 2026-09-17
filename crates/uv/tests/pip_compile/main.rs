//! Integration tests for `uv pip compile`.

#[cfg(feature = "test-python")]
use uv_test::pypi_proxy;

#[cfg(feature = "test-python")]
mod pip_compile;
