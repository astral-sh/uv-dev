//! Integration tests for uv synchronization and settings.

#[cfg(feature = "test-python")]
mod cached_source;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod centralized_project_envs;

#[cfg(feature = "test-python")]
mod environment_path;

#[cfg(feature = "test-python")]
mod legacy_lockfile;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod show_settings;

#[cfg(feature = "test-python")]
mod source_url;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod sync;
