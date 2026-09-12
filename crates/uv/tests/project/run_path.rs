use std::collections::BTreeMap;
use std::process::Command;

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::{TestContext, packse::generate_wheel, uv_snapshot};

fn project(context: &TestContext) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = []

            [tool.uv]
            package = false
        "#})?;
    Ok(())
}

fn run(context: &TestContext) -> Command {
    let mut command = context.run();
    command
        .arg("--no-config")
        .arg("--offline")
        .arg("--no-index")
        .arg("--no-build")
        .arg("--python")
        .arg("3.12")
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV);
    command
}

#[test]
fn run_path_rejects_unjoinable_project_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]).with_filtered_virtualenv_bin();
    project(&context)?;

    uv_snapshot!(context.filters(), run(&context)
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "environment:base")
        .arg("python")
        .arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: environment:base
    error: Failed to construct `PATH` for command: cannot include directory `environment:base/[BIN]`
      Caused by: path segment contains separator `:`
    ");

    Ok(())
}

#[test]
fn run_path_accepts_joinable_project_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    project(&context)?;

    uv_snapshot!(context.filters(), run(&context)
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "environment with spaces")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: environment with spaces
    ");

    Ok(())
}

#[test]
fn run_path_rejects_unjoinable_ephemeral_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"])
        .with_cache_dir("cache:ephemeral")
        .with_filtered_virtualenv_bin();
    project(&context)?;

    let (filename, contents) = generate_wheel(
        &"path-dep".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
    );
    let wheel = context.temp_dir.child(filename);
    wheel.write_binary(&contents)?;

    // The base environment is valid; the additional wheel needs a cached environment.
    uv_snapshot!(context.filters(), run(&context)
        .arg("--with")
        .arg(wheel.path())
        .arg("python")
        .arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Installed 1 package in [TIME]
    error: Failed to construct `PATH` for command: cannot include directory `cache:ephemeral/builds-v0/[TMP]/[BIN]`
      Caused by: path segment contains separator `:`
    ");

    Ok(())
}
