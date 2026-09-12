use std::process::Command;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;

use uv_python::managed::{
    ManagedPythonInstallation, ManagedPythonInstallations, PythonMinorVersionLink,
    platform_key_from_env,
};
use uv_test::{TestContext, capture_uv_snapshot, uv_snapshot};

fn block_minor_version_link(
    context: &TestContext,
    install_bin: bool,
) -> Result<ManagedPythonInstallation> {
    let mut install = context.python_install();
    install.args(["--no-config", "3.13.1"]);
    if !install_bin {
        install.arg("--no-bin");
    }
    install.assert().success();

    let key = format!("cpython-3.13.1-{}", platform_key_from_env()?);
    let managed = context.temp_dir.join("managed");
    let installation_path = managed.join(key);
    let installation = ManagedPythonInstallations::from_settings(Some(managed))?
        .find_all()?
        .find(|installation| installation.path() == installation_path)
        .context("missing test installation")?;
    let minor_version_link = PythonMinorVersionLink::from_installation(&installation)
        .context("CPython must have a minor-version link")?;
    uv_fs::remove_symlink(&minor_version_link.symlink_directory)?;
    fs_err::create_dir(&minor_version_link.symlink_directory)?;
    fs_err::write(
        minor_version_link.symlink_directory.join("existing"),
        b"existing directory contents",
    )?;
    Ok(installation)
}

fn assert_blocked_minor_link(installation: &ManagedPythonInstallation) -> Result<()> {
    let minor_version_link = PythonMinorVersionLink::from_installation(installation)
        .context("CPython must have a minor-version link")?;
    assert_eq!(
        fs_err::read(minor_version_link.symlink_directory.join("existing"))?,
        b"existing directory contents"
    );
    Ok(())
}

fn assert_minor_link_failure(snapshot: &str) {
    insta::allow_duplicates! {
        #[cfg(unix)]
        insta::assert_snapshot!(snapshot, @"
        exit_code: 2 (failure)
        ----- stderr -----
        error: Failed to create Python minor version link directory
          cause: failed to rename file from [TEMP_DIR]/managed/[TMP] to [TEMP_DIR]/managed/cpython-3.13-[PLATFORM]: Is a directory (os error 21)
        ");

        #[cfg(windows)]
        insta::assert_snapshot!(snapshot, @"
        exit_code: 2 (failure)
        ----- stderr -----
        error: Failed to create Python minor version link directory
          cause: failed to remove directory `[TEMP_DIR]/managed/cpython-3.13-[PLATFORM]`: The directory is not empty. (os error 145)
        ");
    }
}

#[test]
fn failed_minor_link_keeps_bin_unpublished() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_managed_python_dirs()
        .with_filtered_python_keys();
    let installation = block_minor_version_link(&context, false)?;

    let snapshot = capture_uv_snapshot!(
        context.filters(),
        context
            .python_install()
            .args(["--no-config", "--offline", "3.13"])
    );
    let bin_python = context
        .bin_dir
        .child(format!("python3.13{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(
        fs_err::symlink_metadata(bin_python.path())
            .expect_err("a failed minor-version link must not publish an executable")
            .kind(),
        std::io::ErrorKind::NotFound,
        "{snapshot}",
    );
    assert_blocked_minor_link(&installation)?;
    assert_minor_link_failure(&snapshot);
    uv_snapshot!(context.filters(), Command::new(installation.executable(false))
        .args(["-I", "-c", "import sys; print(sys.version.split()[0])"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13.1
    ");
    Ok(())
}

#[test]
fn failed_minor_link_keeps_existing_executable() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_managed_python_dirs()
        .with_filtered_python_keys();
    let installation = block_minor_version_link(&context, true)?;
    let bin_python = context
        .bin_dir
        .child(format!("python3.13{}", std::env::consts::EXE_SUFFIX));
    assert!(installation.is_bin_link(bin_python.path()));
    uv_snapshot!(context.filters(), Command::new(bin_python.path())
        .args(["-I", "-c", "import sys; print(sys.version.split()[0])"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13.1
    ");

    let snapshot = capture_uv_snapshot!(
        context.filters(),
        context
            .python_install()
            .args(["--no-config", "--offline", "3.13", "--force"])
    );
    assert!(
        installation.is_bin_link(bin_python.path()),
        "a failed minor-version link must retain the previous executable: {snapshot}",
    );
    assert_blocked_minor_link(&installation)?;
    assert_minor_link_failure(&snapshot);
    uv_snapshot!(context.filters(), Command::new(bin_python.path())
        .args(["-I", "-c", "import sys; print(sys.version.split()[0])"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13.1
    ");
    Ok(())
}
