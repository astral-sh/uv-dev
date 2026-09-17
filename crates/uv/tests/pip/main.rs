//! Integration tests for uv pip commands.

#[cfg(feature = "test-python")]
mod pip_check;

mod pip_compile_scenarios;

mod pip_debug;

#[cfg(feature = "test-python")]
mod pip_exclude_newer_relative;

#[cfg(feature = "test-python")]
mod pip_freeze;

mod pip_install_scenarios;

mod pip_list;

mod pip_show;

#[cfg(feature = "test-python")]
mod pip_sync;

mod pip_tree;

mod pip_uninstall;
