//! Integration tests for uv Python commands.

mod python_dir;

#[cfg(all(feature = "test-python", target_os = "linux", target_arch = "x86_64"))]
mod linux_personality;

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
