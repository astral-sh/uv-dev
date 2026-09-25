//! Integration tests for uv Python commands.

mod python_dir;

#[cfg(feature = "test-python")]
mod python_find;

#[cfg(feature = "test-python-managed")]
mod python_install;

#[cfg(all(unix, feature = "test-python-managed"))]
mod python_install_pyodide;

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
