//! Integration tests for `uv pip install`.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::pypi_proxy;

#[cfg(feature = "test-python")]
mod binary_policy;
#[cfg(feature = "test-python")]
mod direct_url_hashes;
#[cfg(feature = "test-python")]
mod environment_metadata;
#[cfg(feature = "test-python")]
mod exact;
#[cfg(feature = "test-python")]
mod index;
#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod json;
#[cfg(feature = "test-python")]
mod launcher;
#[cfg(feature = "test-python")]
mod link_mode;
#[cfg(feature = "test-python")]
mod no_index;
#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_install;
#[cfg(feature = "test-python")]
mod pylock;
#[cfg(feature = "test-python")]
mod remote_requirements;

#[cfg(feature = "test-python")]
mod wheel_cache;
#[cfg(feature = "test-python")]
mod wheel_variants;
