//! Integration tests for uv workspaces.

#[cfg(feature = "test-python")]
mod workspace;

mod workspace_dir;

mod workspace_list;

#[cfg(feature = "test-python")]
mod workspace_metadata;
