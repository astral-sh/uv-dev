use std::path::PathBuf;
use std::process::Command;

use assert_cmd::prelude::*;
use uv_test::{TestContext, uv_snapshot, venv_bin_path};

fn install_launcher(context: &TestContext) -> PathBuf {
    let wheel = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");
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
