//! Integration tests for `uv pip install`.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::pypi_proxy;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod direct_url_hashes;
mod extras;
#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_install;
