//! Integration tests for uv workspaces.

#[cfg(all(feature = "test-python", feature = "test-universal"))]
mod nested_workspaces;

mod workspace;

mod workspace_dir;

mod workspace_list;

mod workspace_metadata;
