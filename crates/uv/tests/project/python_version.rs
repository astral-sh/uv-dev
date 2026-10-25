use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_python_discovery::PYTHON_VERSION_FILENAME;
use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
fn run_project_python_pin_transition() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []

        [tool.uv]
        package = false
    "#})?;
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
    uv_snapshot!(context.filters(), context.run()
        .args(["--offline", "--no-index", "--no-build"])
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV).arg("--no-sync").arg("python").arg("--version"), @"
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

    uv_snapshot!(context.filters(), context.run()
        .args(["--offline", "--no-index", "--no-build"])
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV).arg("--python").arg("3.11").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]
    ");

    // An ordinary project invocation replaces the incompatible environment.
    uv_snapshot!(context.filters(), context.run()
        .args(["--offline", "--no-index", "--no-build"])
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV).arg("python").arg("--version"), @"
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
    uv_snapshot!(context.filters(), context.run()
        .args(["--offline", "--no-index", "--no-build"])
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV).arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    Ok(())
}

#[test]
fn run_project_python_pin_missing_and_explicit() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []

        [tool.uv]
        package = false
    "#})?;
    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.12")?;

    // Creating a missing environment is not a rejection.
    uv_snapshot!(context.filters(), context.run()
        .args(["--offline", "--no-index", "--no-build"])
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV).arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    ");

    // A rejection caused by an explicit request is not attributed to the pin.
    uv_snapshot!(context.filters(), context.run()
        .args(["--offline", "--no-index", "--no-build"])
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV).arg("--python").arg("3.11").arg("python").arg("--version"), @"
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
    context
        .venv()
        .arg("--python")
        .arg("3.11")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.run()
        .args(["--offline", "--no-index", "--no-build"])
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV).arg("python").arg("--version"), @"
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
fn run_project_python_pin_centralized() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_centralized_environment_hashes();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []

        [tool.uv]
        package = false
    "#})?;
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
    uv_snapshot!(context.filters(), context.run()
        .args(["--offline", "--no-index", "--no-build"])
        .env_remove(EnvVars::UV_SHOW_RESOLUTION)
        .env_remove(EnvVars::VIRTUAL_ENV)
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
