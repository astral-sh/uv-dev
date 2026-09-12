//! Integration tests for uv Python commands.

mod python_dir;

mod python_install_cleanup;

#[cfg(feature = "test-python")]
mod python_find;

#[cfg(feature = "test-python-managed")]
mod python_install;

#[cfg(feature = "test-python-managed")]
mod python_install_links;

#[cfg(all(feature = "test-python-managed", target_os = "macos"))]
mod python_install_publication;

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
