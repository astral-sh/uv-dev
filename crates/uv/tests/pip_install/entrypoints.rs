//! Wheel entry point path validation and normalization.

use std::collections::BTreeMap;
use std::env::consts::EXE_SUFFIX;
use std::path::PathBuf;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use fs_err as fs;
use indoc::indoc;

use uv_fs::Simplified;
use uv_test::packse::generate_wheel_with_files;
use uv_test::{TestContext, uv_snapshot, venv_bin_path};

/// Create entry point metadata directly so the installer receives names a build backend may reject.
fn wheel_with_entrypoint(
    context: &TestContext,
    section: &str,
    entrypoint_name: &str,
) -> Result<PathBuf> {
    wheel_with_entrypoint_source(
        context,
        section,
        entrypoint_name,
        "def main():\n    print('entry point executed', end='')\n",
    )
}

fn wheel_with_entrypoint_source(
    context: &TestContext,
    section: &str,
    entrypoint_name: &str,
    source: &str,
) -> Result<PathBuf> {
    let entry_points = format!("[{section}]\n{entrypoint_name} = foo.cli:main\n");
    let (filename, wheel) = generate_wheel_with_files(
        &"foo".parse()?,
        &"0.1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("foo-0.1.0.dist-info/entry_points.txt", &entry_points),
            ("foo/cli.py", source),
        ],
    );
    let path = context.temp_dir.join(filename);
    fs::write(&path, wheel)?;
    Ok(path)
}

#[test]
fn reject_wheel_entrypoint_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let escaped_entrypoint = context.temp_dir.child("escaped-entrypoint");
    let wheel = wheel_with_entrypoint(
        &context,
        "console_scripts",
        &escaped_entrypoint.path().portable_display().to_string(),
    )?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: The wheel is invalid: Script path must resolve to a file within the scripts directory: [TEMP_DIR]/escaped-entrypoint
    "
    );

    escaped_entrypoint.assert(predicates::path::missing());

    Ok(())
}

#[test]
fn reject_normalized_reserved_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "nested/../python")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `python`, got: `nested/../python`
    "
    );

    Ok(())
}

#[test]
fn reject_case_variant_reserved_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "Python")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `python`, got: `Python`
    ");

    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "Python.PY")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `python`, got: `Python.PY`
    ");

    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "python.Py")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `python`, got: `python.Py`
    ");

    Ok(())
}

#[test]
fn reject_normalized_reserved_gui_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "gui_scripts", "nested/../python")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `python`, got: `nested/../python`
    "
    );

    Ok(())
}

#[test]
fn reject_free_threaded_python_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "python3.13t")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `python3.13t`, got: `python3.13t`
    "
    );

    Ok(())
}

#[test]
fn reject_windowed_free_threaded_python_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "pythonw3.13t")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `pythonw3.13t`, got: `pythonw3.13t`
    "
    );

    Ok(())
}

#[test]
fn reject_windows_rewritten_python_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "python.py")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `python`, got: `python.py`
    "
    );

    Ok(())
}

#[test]
fn reject_windows_rewritten_free_threaded_python_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "pythonw3.13t.py")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `pythonw3.13t`, got: `pythonw3.13t.py`
    "
    );

    Ok(())
}

#[test]
fn reject_pypy_major_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "pypy3")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `pypy3`, got: `pypy3`
    "
    );

    Ok(())
}

#[test]
fn reject_windows_rewritten_pypy_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "pypy.py")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: foo-0.1.0-py3-none-any.whl (foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl))
      cause: Scripts must not use the reserved name `pypy`, got: `pypy.py`
    "
    );

    Ok(())
}

#[test]
fn warn_normalized_activation_wheel_entrypoint_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "nested/../activate.bash")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    warning: The script name `activate.bash` is reserved for virtual environment activation scripts.
    Installed 1 package in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl)
    "
    );

    Ok(())
}

#[test]
fn accept_normalized_gui_wheel_entrypoint_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint_source(
        &context,
        "gui_scripts",
        "nested/../normalized-gui",
        indoc! {r#"
            from pathlib import Path
            import sys

            def main():
                Path(sys.argv[1]).write_text("entry point executed", encoding="utf-8")
        "#},
    )?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl)
    "
    );

    let normalized_script =
        venv_bin_path(&context.venv).join(format!("normalized-gui{EXE_SUFFIX}"));
    let marker = context.temp_dir.child("entrypoint-ran");
    context
        .external_command(normalized_script)
        .arg(marker.path())
        .assert()
        .success();
    marker.assert("entry point executed");

    Ok(())
}

#[test]
fn accept_normalized_wheel_entrypoint_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = wheel_with_entrypoint(&context, "console_scripts", "nested/../normalized-script")?;

    uv_snapshot!(context.filters(), context.pip_install().arg(&wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo-0.1.0-py3-none-any.whl)
    "
    );

    let normalized_script =
        venv_bin_path(&context.venv).join(format!("normalized-script{EXE_SUFFIX}"));
    context
        .external_command(normalized_script)
        .assert()
        .success()
        .stdout("entry point executed");

    Ok(())
}
