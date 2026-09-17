//! Integration tests for `uv pip install`.

#[cfg(feature = "test-python")]
use uv_test::pypi_proxy;

#[cfg(feature = "test-python")]
mod direct_url_hashes;
#[cfg(feature = "test-python")]
mod pip_install;
