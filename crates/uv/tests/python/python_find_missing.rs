//! Interpreter discovery when no Python installation is available.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::Result;
use assert_fs::prelude::{FileTouch, FileWriteStr, PathChild, PathCreateDir};
use indoc::indoc;
use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
fn python_find_script_python_not_found() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_env(EnvVars::UV_PYTHON_PREFERENCE, "only-managed")
        .with_env(EnvVars::UV_INTERNAL__TEST_PYTHON_MANAGED, "");
    context.temp_dir.child(".python-version").touch().unwrap();

    let script = context.temp_dir.child("foo.py");

    script
        .write_str(indoc! {r#"
            # /// script
            # requires-python = ">=3.12"
            # dependencies = []
            # ///
        "#})
        .unwrap();

    uv_snapshot!(context.filters(), context.python_find().arg("--script").arg("foo.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No interpreter found for Python >=3.12 in virtual environments or managed installations

    hint: A managed Python download is available for Python >=3.12, but Python downloads are set to 'never'
    ");
}

#[cfg(unix)]
#[test]
fn python_find_script_interpreter_error_chain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context.temp_dir.child("foo.py").write_str(indoc! {r"
        # /// script
        # dependencies = []
        # ///
    "})?;
    let python = context.temp_dir.child("broken-python");
    python.write_str("#!/bin/sh\nprintf 'interpreter probe failed\\n' >&2\nexit 42\n")?;
    fs_err::set_permissions(&python, std::fs::Permissions::from_mode(0o755))?;
    context
        .temp_dir
        .child(".python-version")
        .write_str(python.path().to_str().expect("UTF-8 fixture path"))?;
    uv_snapshot!(context.filters(), context.python_find().arg("--script").arg("foo.py").arg("-q"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from provided path at `broken-python`
      cause: Querying Python at `[TEMP_DIR]/broken-python` failed with exit status exit status: 42

             [stderr]
             interpreter probe failed
    ");
    Ok(())
}

#[test]
fn python_find_path() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_not_executable();

    context.temp_dir.child("foo").create_dir_all().unwrap();
    context.temp_dir.child("bar").touch().unwrap();

    // No interpreter in a directory
    uv_snapshot!(context.filters(), context.python_find().arg(context.temp_dir.child("foo").as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found in directory `foo`
    ");

    // No interpreter at a file
    uv_snapshot!(context.filters(), context.python_find().arg(context.temp_dir.child("bar").as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from provided path at `bar`
      cause: Failed to query Python interpreter at `[TEMP_DIR]/bar`
      cause: [PERMISSION DENIED]
    ");

    // No interpreter at a file that does not exist
    uv_snapshot!(context.filters(), context.python_find().arg(context.temp_dir.child("foobar").as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found at path `foobar`
    ");
}
