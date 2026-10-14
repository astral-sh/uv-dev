//! Integration tests for `uv tool`.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::pypi_proxy;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_audit;

#[cfg(feature = "test-python")]
mod tool_build_hashes;

mod tool_dir;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_install;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_list;

mod tool_list_empty;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_run;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_uninstall;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_upgrade;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod workspace_fixture;
