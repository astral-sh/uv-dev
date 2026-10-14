//! Integration tests for uv pip commands.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_check;

mod pip_compile_scenarios;

mod pip_debug;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_exclude_newer_relative;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_freeze;

mod pip_install_scenarios;

mod pip_list;

mod pip_show;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_sync;

#[cfg(feature = "test-python")]
mod pip_sync_archive;

#[cfg(feature = "test-python")]
mod pip_sync_cache;

#[cfg(feature = "test-python")]
mod pip_sync_hashes;

#[cfg(feature = "test-python")]
mod pip_sync_markers;

#[cfg(feature = "test-python")]
mod pip_sync_reinstall;

#[cfg(feature = "test-python")]
mod pip_sync_target;

mod pip_tree;

mod pip_uninstall;
