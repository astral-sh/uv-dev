//! Sync failures before resolving or changing the target environment.

use anyhow::Result;
use assert_fs::prelude::*;

#[cfg(feature = "test-python")]
use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
fn missing_requirements_txt() {
    let context = uv_test::test_context_with_versions!(&[]);
    let requirements_txt = context.temp_dir.child("requirements.txt");

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: File not found: requirements.txt
    ");
    requirements_txt.assert(predicates::path::missing());
}

/// `--cert` is forwarded to the HTTP client rather than silently ignored.
#[test]
#[cfg(feature = "test-python")]
fn cert() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_missing_file_error();
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--cert")
        .arg("ca-bundle.pem"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to read certificate file `ca-bundle.pem`
      cause: [OS ERROR 2]
    ");
    Ok(())
}

/// A broken active virtual environment must not fall back to an available system interpreter.
#[test]
#[cfg(feature = "test-python")]
fn missing_active_venv() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"])
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names();
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("anyio")?;

    uv_snapshot!(context.filters(), context.pip_sync().arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from active virtual environment at `.venv/[BIN]/[PYTHON]`
      cause: Python interpreter not found at `[VENV]/[BIN]/[PYTHON]`
    ");
    context.venv.assert(predicates::path::missing());
    Ok(())
}

/// An available system interpreter must not be used without `--system`.
#[test]
#[cfg(feature = "test-python")]
fn missing_venv() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("anyio")?;

    uv_snapshot!(context.filters(), context.pip_sync().arg("requirements.txt").env_remove(EnvVars::VIRTUAL_ENV), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No virtual environment found; run `uv venv` to create an environment, or pass `--system` to install into a non-virtual environment
    ");
    context.venv.assert(predicates::path::missing());
    Ok(())
}

#[test]
fn missing_system() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("anyio")?;

    uv_snapshot!(context.filters(), context.pip_sync().arg("requirements.txt").arg("--system"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No system Python installation found
    ");
    Ok(())
}
