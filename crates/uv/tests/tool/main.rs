//! Integration tests for `uv tool`.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::pypi_proxy;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_audit;

#[cfg(feature = "test-python")]
mod tool_build_hashes;

mod tool_dir;

#[cfg(feature = "test-python")]
mod tool_failed_update;

#[cfg(feature = "test-python")]
mod tool_fixture;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_install;

#[cfg(feature = "test-python")]
mod tool_install_directories;

#[cfg(feature = "test-python")]
mod tool_install_platform;

#[cfg(feature = "test-python")]
mod tool_install_validation;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_list;

mod tool_list_empty;

#[cfg(feature = "test-python")]
mod tool_list_installed;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_run;

#[cfg(feature = "test-python")]
mod tool_run_python;

mod tool_run_validation;

#[cfg(feature = "test-python")]
mod tool_uninstall;

mod tool_uninstall_empty;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_upgrade;

mod tool_upgrade_local;

#[cfg(feature = "test-python")]
mod tool_workspace;

#[cfg(feature = "test-python")]
mod workspace_fixture;
