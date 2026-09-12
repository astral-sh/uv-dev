//! Integration tests for uv build commands and caches.

#[cfg(feature = "test-python")]
use uv_test::pypi_proxy;

#[cfg(feature = "test-python")]
mod audit;

#[cfg(feature = "test-python")]
mod build;

#[cfg(feature = "test-python")]
mod build_backend;

#[cfg(feature = "test-python")]
mod cache;

#[cfg(feature = "test-python")]
mod cache_clean;

#[cfg(feature = "test-python")]
mod cache_prune;

#[cfg(feature = "test-python")]
mod cache_size;

mod extract;
