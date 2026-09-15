//! Integration tests for uv project commands.

#[cfg(feature = "test-python")]
use uv_test::pypi_proxy;

#[cfg(all(feature = "test-python", feature = "test-r2"))]
mod check;

#[cfg(feature = "test-python")]
mod edit;

#[cfg(feature = "test-python")]
mod export;

#[cfg(all(feature = "test-python", feature = "test-r2"))]
mod format;

#[cfg(all(feature = "test-python", feature = "test-git"))]
mod init;

#[cfg(feature = "test-python")]
mod run;

#[cfg(feature = "test-python")]
mod tree;

#[cfg(feature = "test-python")]
mod workflow;
