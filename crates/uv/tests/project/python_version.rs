use std::process::Command;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::formatdoc;

use uv_python::PYTHON_VERSION_FILENAME;
use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

fn project(context: &TestContext, requires_python: &str) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = "{requires_python}"
            dependencies = []

            [tool.uv]
            package = false
        "#})?;
    Ok(())
}

fn run(context: &TestContext) -> Command {
    let mut command = context.run();
    command
        .arg("--offline")
        .arg("--no-index")
        .arg("--no-build")
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV);
    command
}

#[test]
fn run_project_python_pin_transition() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    project(&context, ">=3.11")?;
    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.12")?;

    // An explicit `uv venv` request still overrides the project's pin.
    uv_snapshot!(context.filters(), context.venv().arg("--python").arg("3.11"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    // `--no-sync` preserves the existing environment and its original warning.
    uv_snapshot!(context.filters(), run(&context).arg("--no-sync").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    warning: Using incompatible environment (`.venv`) due to `--no-sync` (The project environment's Python version does not satisfy the request: `Python 3.12`)
    ");

    // Locking can select a different interpreter without replacing the environment.
    uv_snapshot!(context.filters(), context.lock().arg("--offline").arg("--no-index").arg("--no-build"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 1 package in [TIME]
    ");

    uv_snapshot!(context.filters(), run(&context).arg("--python").arg("3.11").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]
    ");

    // An ordinary project invocation replaces the incompatible environment.
    uv_snapshot!(context.filters(), run(&context).arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    warning: The project environment's Python version does not satisfy the request: `Python 3.12` (from version file at `.python-version`)
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    ");

    // Once it matches the pin, there is no rejection to explain.
    uv_snapshot!(context.filters(), run(&context).arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    Ok(())
}

#[test]
fn run_project_python_pin_missing_and_explicit() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    project(&context, ">=3.11")?;
    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.12")?;

    // Creating a missing environment is not a rejection.
    uv_snapshot!(context.filters(), run(&context).arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    ");

    // A rejection caused by an explicit request is not attributed to the pin.
    uv_snapshot!(context.filters(), run(&context).arg("--python").arg("3.11").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    ");

    Ok(())
}

#[test]
fn run_project_python_requires_python_only() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    project(&context, ">=3.12")?;
    context
        .venv()
        .arg("--python")
        .arg("3.11")
        .assert()
        .success();

    uv_snapshot!(context.filters(), run(&context).arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    ");

    Ok(())
}

#[test]
fn run_project_python_pin_incompatible_requirement() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    project(&context, ">=3.12")?;
    context
        .venv()
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();
    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.11")?;

    // Explain the rejected environment even if the pin cannot satisfy the project.
    uv_snapshot!(context.filters(), run(&context).arg("python").arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: The project environment's Python version does not satisfy the request: `Python 3.11` (from version file at `.python-version`)
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    error: The Python request from `.python-version` resolved to Python 3.11.[X], which is incompatible with the project's Python requirement: `>=3.12` (from `project.requires-python`)
    Use `uv python pin` to update the `.python-version` file to a compatible version
    ");

    Ok(())
}

#[test]
fn run_project_python_pin_centralized() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_centralized_environment_hashes();
    project(&context, ">=3.11")?;
    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.12")?;
    context
        .venv()
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .arg("--python")
        .arg("3.11")
        .assert()
        .success();

    // A centralized environment can be kept in the cache for later reuse.
    uv_snapshot!(context.filters(), run(&context)
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment `project-cp3.12.[X]-[HASH]`
    ");

    Ok(())
}
