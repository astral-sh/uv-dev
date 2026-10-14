//! Integration tests for uv synchronization and settings.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod centralized_project_envs;

#[cfg(feature = "test-python")]
mod environment_path;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod show_settings;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod sync;
