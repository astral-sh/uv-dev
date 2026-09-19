//! Integration tests for `uv lock`.

#[cfg(all(
    feature = "test-python",
    feature = "test-pypi",
    feature = "test-universal"
))]
use uv_test::pypi_proxy;

#[cfg(all(
    feature = "test-python",
    feature = "test-pypi",
    feature = "test-universal"
))]
mod coordinated_budget;

#[cfg(all(
    feature = "test-python",
    feature = "test-pypi",
    feature = "test-universal"
))]
mod coordinated_observations;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod coordinated_scopes;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod coordinated_source;

#[cfg(all(
    feature = "test-python",
    feature = "test-pypi",
    feature = "test-universal"
))]
mod coordinated_trace;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod lock;

#[cfg(all(feature = "test-python", feature = "test-universal"))]
mod minimum_libc;
