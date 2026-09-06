//! Miscellaneous integration tests for uv.

use uv_test::pypi_proxy;

mod auth;

#[cfg(all(feature = "test-pypi", feature = "test-universal"))]
mod branching_urls;

#[cfg(all(
    feature = "test-python",
    feature = "test-pypi",
    feature = "test-ecosystem"
))]
mod ecosystem;

mod help;

#[cfg(feature = "test-pypi")]
mod issue_20678;

mod network;

#[cfg(feature = "test-pypi")]
mod publish;

#[cfg(unix)]
mod resource_limits;

#[cfg(feature = "self-update")]
mod self_update;

#[cfg(not(windows))]
mod update_shell;

mod upgrade;

mod version;
