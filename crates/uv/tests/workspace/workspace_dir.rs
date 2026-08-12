use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild};
use indoc::indoc;

use uv_test::{copy_dir_ignore, uv_snapshot};

/// Test basic output for a simple workspace with one member.
#[test]
fn workspace_dir_simple() {
    let context = uv_test::test_context!("3.12");

    // Initialize a workspace with one member
    context.init().arg("foo").assert().success();

    let workspace = context.temp_dir.child("foo");

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&workspace), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/foo
    "
    );
}

/// Workspace dir output when run with `--package`.
#[test]
fn workspace_dir_specific_package() {
    let context = uv_test::test_context!("3.12");
    context.init().arg("foo").assert().success();
    context.init().arg("foo/bar").assert().success();
    let workspace = context.temp_dir.child("foo");

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
}

/// Test output when run from a workspace member directory.
#[test]
fn workspace_metadata_from_member() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let workspace = context.temp_dir.child("workspace");

    let albatross_workspace = context
        .workspace_root
        .join("test/workspaces/albatross-root-workspace");

    copy_dir_ignore(albatross_workspace, &workspace)?;

    let member_dir = workspace.join("packages").join("bird-feeder");

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&member_dir), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/workspace
    "
    );

    Ok(())
}

/// Test workspace discovery from a member nested inside another workspace member.
#[test]
fn workspace_dir_nested_member() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let workspace = context.temp_dir.child("workspace");
    let member = workspace.child("a");
    let nested = member.child("b");
    let standalone = member.child("standalone");
    let excluded = member.child("excluded");

    for project in [&nested, &standalone, &excluded] {
        fs_err::create_dir_all(project)?;
    }

    workspace.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv.workspace]
        members = ["a", "a/b", "a/excluded"]
        exclude = ["a/excluded"]
    "#})?;
    member.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "member"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    nested.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "nested"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    standalone.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "standalone"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    excluded.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "excluded"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&member), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/workspace
    ");

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&nested), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/workspace/a/b
    ");

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&standalone), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/workspace/a/standalone
    ");

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&excluded), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/workspace/a/excluded
    ");

    uv_snapshot!(context.filters(), context.lock().arg("--offline").current_dir(&nested), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 1 package in [TIME]
    ");

    assert!(nested.child("uv.lock").exists());
    assert!(!workspace.child("uv.lock").exists());

    Ok(())
}

/// Test that a project inside the configured cache directory is rejected before workspace
/// discovery.
#[test]
fn workspace_dir_rejects_project_inside_cache() -> Result<()> {
    let mut context = uv_test::test_context!("3.12");
    let workspace = context.temp_dir.child("workspace");
    let cache_dir = workspace.child("cache");
    let cached_project = cache_dir.child("cached-project");

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

    context.cache_dir = cache_dir;

    uv_snapshot!(context.filters(), context.workspace_dir().current_dir(&cached_project), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The project directory `.` is inside the cache directory `[TEMP_DIR]/workspace/cache`
    "
    );

    Ok(())
}

/// Test workspace dir error output for a non-existent package.
#[test]
fn workspace_dir_package_doesnt_exist() {
    let context = uv_test::test_context!("3.12");

    // Initialize a workspace with one member
    context.init().arg("foo").assert().success();

    let workspace = context.temp_dir.child("foo");

    uv_snapshot!(context.filters(), context.workspace_dir().arg("--package").arg("bar").current_dir(&workspace), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `bar` not found in workspace.
    "
    );
}

/// Test workspace dir error output when not in a project.
#[test]
fn workspace_metadata_no_project() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.workspace_dir(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No `pyproject.toml` found in current directory or any parent directory
    "
    );
}
