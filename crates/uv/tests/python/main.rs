//! Integration tests for uv Python commands.

mod python_dir;

#[cfg(feature = "test-python")]
mod python_find;

#[cfg(all(unix, feature = "test-python"))]
mod python_find_versions;

#[cfg(feature = "test-python-managed")]
mod python_install;

#[cfg(feature = "test-python")]
mod python_list;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod python_module;

#[cfg(feature = "test-python")]
mod python_pin;

#[cfg(feature = "test-python-managed")]
mod python_upgrade;

#[cfg(feature = "test-python")]
mod venv;
