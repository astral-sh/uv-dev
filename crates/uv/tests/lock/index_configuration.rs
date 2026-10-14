//! Invalid index configuration must fail before Python discovery and dependency resolution.

use anyhow::Result;
use assert_fs::prelude::*;

use uv_test::uv_snapshot;

/// Index declarations and source references reject invalid names.
#[test]
fn lock_invalid_index() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0", "iniconfig==2.0.0"]

        [tool.uv.sources]
        iniconfig = { index = "internal proxy" }

        [[tool.uv.index]]
        name = "internal proxy"
        url = "https://example.invalid/simple"
        explicit = true
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Failed to parse `pyproject.toml` during settings discovery:
      TOML parse error at line 12, column 16
         |
      12 |         name = "internal proxy"
         |                ^^^^^^^^^^^^^^^^
      Index names may only contain letters, digits, hyphens, underscores, and periods, but found unsupported character (` `) in: `internal proxy`

    error: Failed to parse: pyproject.toml
      cause: TOML parse error at line 9, column 31
               |
             9 |         iniconfig = { index = "internal proxy" }
               |                               ^^^^^^^^^^^^^^^^
             Index names may only contain letters, digits, hyphens, underscores, and periods, but found unsupported character (` `) in: `internal proxy`
    "#);

    Ok(())
}

/// Error when an explicit index does not have a name.
#[test]
fn lock_unnamed_explicit_index() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig==2.0.0"]

        [[tool.uv.index]]
        url = "https://example.invalid/simple"
        explicit = true
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Failed to parse `pyproject.toml` during settings discovery:
      TOML parse error at line 8, column 9
        |
      8 |         [[tool.uv.index]]
        |         ^^^^^^^^^^^^^^^^^
      An index with `explicit = true` requires a `name`: https://example.invalid/simple

    error: Failed to parse: pyproject.toml
      cause: TOML parse error at line 8, column 9
               |
             8 |         [[tool.uv.index]]
               |         ^^^^^^^^^^^^^^^^^
             An index with `explicit = true` requires a `name`: https://example.invalid/simple
    ");

    Ok(())
}

/// Error when an index cache-control override is not a valid HTTP header value.
#[test]
fn lock_invalid_index_cache_control() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig==2.0.0"]

        [[tool.uv.index]]
        name = "test"
        url = "https://example.invalid/simple"
        cache-control.api = """
        max-age=600
        """
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Failed to parse `pyproject.toml` during settings discovery:
      TOML parse error at line 11, column 9
         |
      11 |         cache-control.api = """
         |         ^^^^^^^^^^^^^
      `cache-control.api` must be a valid HTTP header value

    error: Failed to parse: pyproject.toml
      cause: TOML parse error at line 11, column 9
                |
             11 |         cache-control.api = """
                |         ^^^^^^^^^^^^^
             `cache-control.api` must be a valid HTTP header value
    "#);

    Ok(())
}
