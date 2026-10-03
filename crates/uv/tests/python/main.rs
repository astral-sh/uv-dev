//! Integration tests for uv Python commands.

#[cfg(feature = "test-python-managed")]
use uv_platform::Platform;
#[cfg(feature = "test-python-managed")]
use uv_python::managed::Error as ManagedPythonError;

mod python_dir;

#[cfg(feature = "test-python")]
mod python_find;

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

/// Generate a platform portion of a key from the environment.
#[cfg(feature = "test-python-managed")]
fn platform_key_from_env() -> Result<String, ManagedPythonError> {
    Ok(Platform::from_env()?.to_string().to_lowercase())
}
