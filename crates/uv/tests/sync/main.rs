//! Integration tests for uv synchronization and settings.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod centralized_project_envs;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod show_settings;

#[cfg(feature = "test-python")]
mod source_preparation;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod sync;
