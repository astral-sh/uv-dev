//! Synchronizing an existing custom project environment path.

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;
use predicates::prelude::predicate;
use uv_static::EnvVars;
use uv_test::{uv_snapshot, venv_bin_path};

#[test]
fn reject_non_environment_without_deleting_contents() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    let user_data = context.temp_dir.child("foo/user-data");
    user_data.write_str("user data")?;

    uv_snapshot!(context.filters(), context.sync().arg("--offline").env(EnvVars::UV_PROJECT_ENVIRONMENT, "foo"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project virtual environment directory `[TEMP_DIR]/foo` cannot be used because it is not a valid Python environment (no Python executable was found)
    ");
    user_data.assert("user data");

    Ok(())
}

#[test]
fn replace_incompatible_environment() -> Result<()> {
    let context =
        uv_test::test_context_with_versions!(&["3.11", "3.12"]).with_filtered_virtualenv_bin();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;

    uv_snapshot!(context.filters(), context.venv().args(["foo", "--python", "3.11"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    warning: The requested interpreter resolved to Python 3.11.[X], which is incompatible with the project's Python requirement: `>=3.12` (from `project.requires-python`)
    Creating virtual environment at: foo
    Activate with: source foo/[BIN]/activate
    ");

    let target = context.temp_dir.child("foo");
    let old_environment_file = target.child("old-environment-file");
    old_environment_file.touch()?;
    uv_snapshot!(context.filters(), context.sync().arg("--offline").env(EnvVars::UV_PROJECT_ENVIRONMENT, "foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Removed virtual environment at: foo
    Creating virtual environment at: foo
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");
    old_environment_file.assert(predicate::path::missing());
    let python =
        venv_bin_path(target.path()).join(format!("python{}", std::env::consts::EXE_SUFFIX));
    uv_snapshot!(context.filters(), context.external_command(python).arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    Ok(())
}
