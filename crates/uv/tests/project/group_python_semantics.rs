use anyhow::Result;
use assert_fs::prelude::*;
use indoc::formatdoc;

use uv_test::{TestContext, uv_snapshot};

fn write_project_root(
    context: &TestContext,
    base: &str,
    combined: &str,
    include_empty: bool,
) -> Result<()> {
    let empty = if include_empty { "empty = []" } else { "" };
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.12"

            [tool.uv]
            default-groups = []

            [dependency-groups]
            base = []
            combined = [{{ include-group = "base" }}]
            {empty}

            [tool.uv.dependency-groups]
            base = {{ requires-python = "{base}" }}
            combined = {{ requires-python = "{combined}" }}
        "#})?;
    Ok(())
}

fn write_workspace_root(
    context: &TestContext,
    base: &str,
    combined: &str,
    include_empty: bool,
) -> Result<()> {
    context
        .temp_dir
        .child("member/pyproject.toml")
        .write_str(&formatdoc! {r#"
            [project]
            name = "member"
            version = "0.1.0"
            requires-python = ">=3.12"
        "#})?;
    let empty = if include_empty { "empty = []" } else { "" };
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
            [tool.uv.workspace]
            members = ["member"]

            [tool.uv]
            default-groups = []

            [dependency-groups]
            base = []
            combined = [{{ include-group = "base" }}]
            {empty}

            [tool.uv.dependency-groups]
            base = {{ requires-python = "{base}" }}
            combined = {{ requires-python = "{combined}" }}
        "#})?;
    Ok(())
}

/// Project groups compare effective Python requirements without reformatting the lockfile.
#[test]
fn project_group_python_requirements_are_semantic() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_project_root(&context, ">=3.12", ">=3.11,<3.15", true)?;
    uv_snapshot!(
        context.filters(),
        context.lock().arg("--offline"),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );
    let original = context.read("uv.lock");

    // The included base already supplies the stronger lower bound.
    write_project_root(&context, ">=3.12", "<3.15", true)?;
    uv_snapshot!(
        context.filters(),
        context.lock().args(["--offline", "--locked"]),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );
    uv_snapshot!(
        context.filters(),
        context.lock().arg("--offline"),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );
    assert_eq!(original, context.read("uv.lock"));

    // Declaration metadata still records the existence of unrestricted groups.
    write_project_root(&context, ">=3.12", "<3.15", false)?;
    uv_snapshot!(
        context.filters(),
        context.lock().args(["--offline", "--locked"]),
        @r"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    "
    );
    assert_eq!(original, context.read("uv.lock"));

    write_project_root(&context, ">=3.12,!=3.13.*", "<3.15", true)?;
    let previous = context.read("uv.lock");
    uv_snapshot!(
        context.filters(),
        context.lock().args(["--offline", "--locked"]),
        @r"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    "
    );
    assert_eq!(previous, context.read("uv.lock"));
    uv_snapshot!(
        context.filters(),
        context.lock().arg("--offline"),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    write_project_root(&context, ">=3.12,!=3.13.0.*", "<3.15", true)?;
    let previous = context.read("uv.lock");
    uv_snapshot!(
        context.filters(),
        context.lock().args(["--offline", "--locked"]),
        @r"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    "
    );
    assert_eq!(previous, context.read("uv.lock"));
    uv_snapshot!(
        context.filters(),
        context.lock().arg("--offline"),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    write_project_root(&context, ">=3.13", "<3.15", true)?;
    let previous = context.read("uv.lock");
    uv_snapshot!(
        context.filters(),
        context.lock().args(["--offline", "--locked"]),
        @r"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    "
    );
    assert_eq!(previous, context.read("uv.lock"));
    uv_snapshot!(
        context.filters(),
        context.lock().arg("--offline"),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    Ok(())
}

/// Non-project root groups compare effective Python requirements without package metadata.
#[test]
fn workspace_group_python_requirements_are_semantic_without_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_workspace_root(&context, ">=3.12", ">=3.11,<3.15", true)?;
    uv_snapshot!(
        context.filters(),
        context
            .lock()
            .args(["--offline", "--preview-features", "lock-without-metadata"]),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );
    let original = context.read("uv.lock");

    // The included base already supplies the stronger lower bound.
    write_workspace_root(&context, ">=3.12", "<3.15", true)?;
    uv_snapshot!(
        context.filters(),
        context.lock().args([
            "--offline",
            "--preview-features",
            "lock-without-metadata",
            "--locked",
        ]),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );
    uv_snapshot!(
        context.filters(),
        context
            .lock()
            .args(["--offline", "--preview-features", "lock-without-metadata"]),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );
    assert_eq!(original, context.read("uv.lock"));

    write_workspace_root(&context, ">=3.12,!=3.13.*", "<3.15", true)?;
    let previous = context.read("uv.lock");
    uv_snapshot!(
        context.filters(),
        context.lock().args([
            "--offline",
            "--preview-features",
            "lock-without-metadata",
            "--locked",
        ]),
        @r"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    "
    );
    assert_eq!(previous, context.read("uv.lock"));
    uv_snapshot!(
        context.filters(),
        context
            .lock()
            .args(["--offline", "--preview-features", "lock-without-metadata"]),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    write_workspace_root(&context, ">=3.12,!=3.13.0.*", "<3.15", true)?;
    let previous = context.read("uv.lock");
    uv_snapshot!(
        context.filters(),
        context.lock().args([
            "--offline",
            "--preview-features",
            "lock-without-metadata",
            "--locked",
        ]),
        @r"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    "
    );
    assert_eq!(previous, context.read("uv.lock"));
    uv_snapshot!(
        context.filters(),
        context
            .lock()
            .args(["--offline", "--preview-features", "lock-without-metadata"]),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    write_workspace_root(&context, ">=3.13", "<3.15", true)?;
    let previous = context.read("uv.lock");
    uv_snapshot!(
        context.filters(),
        context.lock().args([
            "--offline",
            "--preview-features",
            "lock-without-metadata",
            "--locked",
        ]),
        @r"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    "
    );
    assert_eq!(previous, context.read("uv.lock"));
    uv_snapshot!(
        context.filters(),
        context
            .lock()
            .args(["--offline", "--preview-features", "lock-without-metadata"]),
        @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    Ok(())
}
