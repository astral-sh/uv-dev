//! Integration tests for uv build commands and caches.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::pypi_proxy;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod audit;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod build;

#[cfg(feature = "test-python")]
mod build_backend;

mod build_backend_requires;

#[cfg(feature = "test-python")]
mod build_output;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod cache;

mod cache_clean;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod cache_prune;

mod cache_prune_local;

mod cache_size;

mod extract;
