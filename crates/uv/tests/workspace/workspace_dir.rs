use anyhow::Result;
use assert_fs::fixture::{FileWriteStr, PathChild};
use indoc::indoc;

use uv_test::uv_snapshot;

/// Test basic output for a simple workspace with one member.
#[test]
fn workspace_dir_simple() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let workspace = context.temp_dir.child("foo");
    workspace
        .child("pyproject.toml")
        .write_str("[project]\nname = \"foo\"\nversion = \"0.1.0\"\n")?;

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&workspace), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/foo
    "
    );
    Ok(())
}

/// Workspace dir output when run with `--package`.
#[test]
fn workspace_dir_specific_package() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let workspace = context.temp_dir.child("foo");
    workspace.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "foo"
        version = "0.1.0"

        [tool.uv.workspace]
        members = ["bar"]
    "#})?;
    workspace
        .child("bar/pyproject.toml")
        .write_str("[project]\nname = \"bar\"\nversion = \"0.1.0\"\n")?;

    // root workspace
    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&workspace), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/foo
    "
    );

    // with --package bar
    uv_snapshot!(context.filters(), context.workspace_dir().arg("--package").arg("bar").current_dir(&workspace), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/foo/bar
    "
    );
    Ok(())
}

/// Test output when run from a workspace member directory.
#[test]
fn workspace_dir_from_member() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let workspace = context.temp_dir.child("workspace");
    let member = workspace.child("member");
    workspace.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "root"
        version = "0.1.0"

        [tool.uv.workspace]
        members = ["member"]
        "#,
    )?;
    member.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "member"
        version = "0.1.0"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&member), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/workspace
    "
    );

    Ok(())
}

/// Test workspace discovery from a nested project that is ignored by a shallower member glob.
#[test]
fn workspace_dir_nested_gitignored_project() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let workspace = context.temp_dir.child("workspace");
    let project = workspace.child("packages").child("foo").child("nested");

    workspace.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["packages/*"]
        "#,
    )?;
    workspace
        .child(".gitignore")
        .write_str("packages/foo/nested/\n")?;
    project.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "nested"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&project), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/workspace/packages/foo/nested
    "#);

    Ok(())
}

/// Test workspace discovery from a nested project matched by a recursive member glob.
#[test]
fn workspace_dir_nested_recursive_member() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let workspace = context.temp_dir.child("workspace");
    let project = workspace.child("packages").child("foo").child("nested");

    workspace.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["packages/**/nested"]
        "#,
    )?;
    project.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "nested"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&project), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/workspace
    "#);

    Ok(())
}

/// Test that a project inside the configured cache directory is rejected before workspace
/// discovery.
#[test]
fn workspace_dir_rejects_project_inside_cache() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_cache_dir("workspace/cache");
    let workspace = context.temp_dir.child("workspace");
    let cached_project = context.cache_dir.child("cached-project");

    fs_err::create_dir_all(&cached_project)?;
    workspace.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["cache/cached-project"]
        "#,
    )?;
    cached_project.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "cached-project"
        version = "0.1.0"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&cached_project), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The project directory `.` is inside the cache directory `[CACHE_DIR]/`
    "
    );

    Ok(())
}

/// Test workspace dir error output for a non-existent package.
#[test]
fn workspace_dir_package_doesnt_exist() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let workspace = context.temp_dir.child("foo");
    workspace
        .child("pyproject.toml")
        .write_str("[project]\nname = \"foo\"\nversion = \"0.1.0\"\n")?;

    uv_snapshot!(context.filters(), context.workspace_dir().arg("--package").arg("bar").current_dir(&workspace), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `bar` not found in workspace.
    "
    );
    Ok(())
}

/// Test workspace dir error output when not in a project.
#[test]
fn workspace_dir_no_project() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.workspace_dir(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No `pyproject.toml` found in current directory or any parent directory
    "
    );
}
