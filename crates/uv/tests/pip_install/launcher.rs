use std::path::PathBuf;
use std::process::Command;

use assert_cmd::prelude::*;
use uv_test::{TestContext, uv_snapshot, venv_bin_path};

use super::wheel_fixture::write_crlf_script_wheel;

/// Rewriting a CRLF shebang must leave the following encoding cookie usable.
#[test]
fn install_crlf_wheel_script_preserves_encoding_cookie() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = context
        .temp_dir
        .join("encoded_script-1.0.0-py3-none-any.whl");
    write_crlf_script_wheel(&wheel)?;
    context
        .pip_install()
        .arg(wheel)
        .arg("--no-index")
        .assert()
        .success();

    let script = venv_bin_path(&context.venv).join("encoded-script");
    let contents = fs_err::read(&script)?;
    assert!(
        contents.starts_with(b"#!") && !contents.starts_with(b"#!python"),
        "wheel script does not have a rewritten shebang: {}",
        script.display()
    );
    let mut command = if cfg!(unix) {
        Command::new(&script)
    } else {
        let mut command = context.python_command();
        command.arg(&script);
        command
    };
    command.env(uv_static::EnvVars::PYTHONUTF8, "1");
    uv_snapshot!(command, @"
    exit_code: 0 (success)
    ----- stdout -----
    café
    ");
    Ok(())
}

fn launcher_wheel(context: &TestContext) -> PathBuf {
    context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl")
}

fn install_launcher(context: &TestContext) -> PathBuf {
    let wheel = launcher_wheel(context);
    context
        .pip_install()
        .arg(format!("simple_launcher@{}", wheel.display()))
        .args(["--strict", "--no-index"])
        .assert()
        .success();
    venv_bin_path(&context.venv).join(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX))
}

#[test]
fn direct() {
    let context = uv_test::test_context!("3.12");
    let launcher = install_launcher(&context);
    uv_snapshot!(Command::new(launcher), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");
}

#[cfg(any(unix, windows))]
#[test]
fn symlinked() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12");
    let launcher = install_launcher(&context);
    let linked_launcher = context.temp_dir.join(launcher.file_name().unwrap());

    #[cfg(windows)]
    if let Err(error) = fs_err::os::windows::fs::symlink_file(&launcher, &linked_launcher) {
        // Creating symlinks requires Developer Mode or elevated privileges on Windows.
        if error.raw_os_error() == Some(1314) {
            return Ok(());
        }
        return Err(error.into());
    }
    #[cfg(unix)]
    fs_err::os::unix::fs::symlink(&launcher, &linked_launcher)?;

    uv_snapshot!(Command::new(linked_launcher), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");
    Ok(())
}

#[test]
fn relocated() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .venv()
        .arg(context.venv.as_os_str())
        .args(["--clear", "--python", "3.12", "--relocatable"])
        .assert()
        .success();

    let launcher = install_launcher(&context);
    uv_snapshot!(Command::new(&launcher), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");

    let relocated = context.venv.with_file_name("relocated");
    fs_err::rename(&context.venv, &relocated)?;
    let relocated_launcher = venv_bin_path(&relocated).join(launcher.file_name().unwrap());
    uv_snapshot!(Command::new(&relocated_launcher), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");

    #[cfg(unix)]
    {
        let linked_launcher = context.temp_dir.join(launcher.file_name().unwrap());
        fs_err::os::unix::fs::symlink(&relocated_launcher, &linked_launcher)?;
        uv_snapshot!(Command::new(linked_launcher), @"
        exit_code: 0 (success)
        ----- stdout -----
        Hi from the simple launcher!
        ");
    }
    Ok(())
}

/// A symlinked `lib` directory currently causes the launcher to be installed outside `bin`.
/// See astral-sh/uv#21255.
#[cfg(unix)]
#[test]
fn symlinked_lib() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12");
    let lib = context.venv.join("lib");
    let usr = context.venv.join("usr");
    fs_err::create_dir_all(&usr)?;
    fs_err::rename(&lib, usr.join("lib"))?;
    fs_err::os::unix::fs::symlink("usr/lib", &lib)?;
    fs_err::create_dir_all(usr.join("bin"))?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg(launcher_wheel(&context))
        .arg("--no-index"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: simple_launcher-0.1.0-py3-none-any.whl (simple-launcher==0.1.0 (from file://[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl))
      cause: failed to query metadata of file `[VENV]/bin/simple_launcher`: No such file or directory (os error 2)
    ");

    let expected = context.venv.join("bin/simple_launcher");
    let misplaced = usr.join("bin/simple_launcher");
    assert!(
        !expected.exists(),
        "unexpected launcher at {}",
        expected.display()
    );
    assert!(
        misplaced.is_file(),
        "expected misplaced launcher at {}",
        misplaced.display()
    );
    Ok(())
}
