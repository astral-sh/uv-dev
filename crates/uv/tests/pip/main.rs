//! Integration tests for uv pip commands.

#[cfg(feature = "test-python")]
mod pip_check;

mod pip_compile_scenarios;

mod pip_debug;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_exclude_newer_relative;

#[cfg(feature = "test-python")]
mod pip_freeze;

mod pip_install_scenarios;

#[cfg(feature = "test-python")]
mod pip_list;

mod pip_list_input;

mod pip_show;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod pip_sync;

#[cfg(feature = "test-python")]
mod pip_sync_archive;

#[cfg(feature = "test-python")]
mod pip_sync_archive_size;

#[cfg(feature = "test-python")]
mod pip_sync_cache;

mod pip_sync_diagnostics;

#[cfg(feature = "test-python")]
mod pip_sync_hashes;

#[cfg(feature = "test-python")]
mod pip_sync_markers;

#[cfg(feature = "test-python")]
mod pip_sync_reinstall;

#[cfg(feature = "test-python")]
mod pip_sync_sources;

#[cfg(feature = "test-python")]
mod pip_sync_target;

#[cfg(feature = "test-python")]
mod pip_sync_yanked;

#[cfg(feature = "test-python")]
mod pip_tree;

#[cfg(feature = "test-python")]
mod pip_uninstall;

mod pip_uninstall_input;
