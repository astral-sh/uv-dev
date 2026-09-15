//! Integration tests for `uv tool`.

#[cfg(all(feature = "test-python", feature = "test-git"))]
mod git;

#[cfg(feature = "test-python")]
use uv_test::pypi_proxy;

#[cfg(feature = "test-python")]
mod tool_audit;

#[cfg(feature = "test-python")]
mod tool_dir;

#[cfg(feature = "test-python")]
mod tool_install;

#[cfg(feature = "test-python")]
mod tool_list;

#[cfg(feature = "test-python")]
mod tool_run;

#[cfg(feature = "test-python")]
mod tool_uninstall;

#[cfg(feature = "test-python")]
mod tool_upgrade;
