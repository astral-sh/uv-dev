//! Validation of project source declarations before dependency resolution.

use anyhow::Result;
use assert_fs::prelude::*;
use uv_test::uv_snapshot;

/// An index source scoped to an undefined extra is invalid.
#[test]
fn index_source_rejects_undefined_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["jinja2"]

        [tool.uv.sources]
        jinja2 = [
            { index = "torch-cu118", extra = "cu118" },
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "https://example.invalid/simple"
        explicit = true
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `project @ file://[TEMP_DIR]/`
      cause: Source entry for `jinja2` only applies to extra `cu118`, but the `cu118` extra does not exist. When an extra is present on a source (e.g., `extra = "cu118"`), the relevant package must be included in the `project.optional-dependencies` section for that extra (e.g., `project.optional-dependencies = { "cu118" = ["jinja2"] }`).
    "#);

    Ok(())
}

/// An index source scoped to an extra must name a dependency in that extra.
#[test]
fn index_source_rejects_package_missing_from_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["jinja2>=3"]

        [project.optional-dependencies]
        cu118 = []

        [tool.uv.sources]
        jinja2 = [
            { index = "torch-cu118", extra = "cu118" },
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "https://example.invalid/simple"
        explicit = true
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `project @ file://[TEMP_DIR]/`
      cause: Source entry for `jinja2` only applies to extra `cu118`, but `jinja2` was not found under the `project.optional-dependencies` section for that extra. When an extra is present on a source (e.g., `extra = "cu118"`), the relevant package must be included in the `project.optional-dependencies` section for that extra (e.g., `project.optional-dependencies = { "cu118" = ["jinja2"] }`).
    "#);

    Ok(())
}

/// An index source scoped to an undefined dependency group is invalid.
#[test]
fn index_source_rejects_undefined_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["jinja2"]

        [tool.uv.sources]
        jinja2 = [
            { index = "torch-cu118", group = "cu118" },
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "https://example.invalid/simple"
        explicit = true
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `project @ file://[TEMP_DIR]/`
      cause: Source entry for `jinja2` only applies to dependency group `cu118`, but the `cu118` group does not exist. When a group is present on a source (e.g., `group = "cu118"`), the relevant package must be included in the `dependency-groups` section for that extra (e.g., `dependency-groups = { "cu118" = ["jinja2"] }`).
    "#);

    Ok(())
}

/// An index source scoped to a dependency group must name a dependency in that group.
#[test]
fn index_source_rejects_package_missing_from_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["jinja2>=3"]

        [dependency-groups]
        cu118 = []

        [tool.uv.sources]
        jinja2 = [
            { index = "torch-cu118", group = "cu118" },
        ]

        [[tool.uv.index]]
        name = "torch-cu118"
        url = "https://example.invalid/simple"
        explicit = true
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `project @ file://[TEMP_DIR]/`
      cause: Source entry for `jinja2` only applies to dependency group `cu118`, but `jinja2` was not found under the `dependency-groups` section for that group. When a group is present on a source (e.g., `group = "cu118"`), the relevant package must be included in the `dependency-groups` section for that extra (e.g., `dependency-groups = { "cu118" = ["jinja2"] }`).
    "#);

    Ok(())
}
