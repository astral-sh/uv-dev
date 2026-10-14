//! Integration tests for uv Python commands.

#[cfg(feature = "test-python-managed")]
mod build_upgrade;

#[cfg(feature = "test-python-managed")]
mod managed_venv;

mod python_dir;

#[cfg(feature = "test-python")]
mod python_find;

#[cfg(feature = "test-python-managed")]
mod python_install;

mod python_list;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod python_module;

#[cfg(feature = "test-python")]
mod python_pin;

#[cfg(feature = "test-python-managed")]
mod python_upgrade;

#[cfg(feature = "test-python")]
mod venv;

mod venv_config;

#[cfg(all(
    windows,
    feature = "test-python-managed",
    feature = "test-windows-registry"
))]
mod windows_registry;
