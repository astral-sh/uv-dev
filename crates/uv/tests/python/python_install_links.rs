use std::process::Command;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;

use uv_fs::Simplified;
use uv_python::managed::{
    ManagedPythonInstallation, ManagedPythonInstallations, PythonMinorVersionLink,
    platform_key_from_env,
};
use uv_test::{TestContext, capture_uv_snapshot, uv_snapshot};

use super::python_install::read_link;

fn find_installation(context: &TestContext, version: &str) -> Result<ManagedPythonInstallation> {
    let key = format!("cpython-{version}-{}", platform_key_from_env()?);
    let managed = context.temp_dir.join("managed");
    let installation_path = managed.join(key);
    ManagedPythonInstallations::from_settings(Some(managed))?
        .find_all()?
        .find(|installation| installation.path() == installation_path)
        .context("missing test installation")
}

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

    let installation = find_installation(context, "3.13.1")?;
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

#[test]
fn minor_link_retarget_keeps_executable_ownership() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_managed_python_dirs()
        .with_filtered_python_keys()
        .with_filtered_exe_suffix();
    context
        .python_install()
        .args(["--no-config", "--no-bin", "3.13.1"])
        .assert()
        .success();
    let older = find_installation(&context, "3.13.1")?;
    let minor_link = PythonMinorVersionLink::from_installation(&older)
        .context("CPython must have a minor-version link")?;
    let bin_python = context
        .bin_dir
        .child(format!("python3.13{}", std::env::consts::EXE_SUFFIX));

    uv_snapshot!(context.filters(), context.python_install()
        .args(["--no-config", "--offline", "3.13"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.1 in [TIME]
     + cpython-3.13.1-[PLATFORM] (python3.13)
    ");
    assert_eq!(
        read_link(bin_python.path()),
        minor_link
            .symlink_executable
            .simplified_display()
            .to_string()
    );
    assert!(older.is_bin_link(bin_python.path()));

    // The minor link moves before the entry point is considered for replacement. The previous
    // owner still determines whether a patch request replaces the upgradeable executable.
    uv_snapshot!(context.filters(), context.python_install()
        .args(["--no-config", "3.13.2"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.2 in [TIME]
     + cpython-3.13.2-[PLATFORM] (python3.13)
    ");
    let newer = find_installation(&context, "3.13.2")?;
    assert_eq!(
        read_link(bin_python.path()),
        newer.executable(false).simplified_display().to_string()
    );
    assert!(!older.is_bin_link(bin_python.path()));
    assert!(newer.is_bin_link(bin_python.path()));

    uv_snapshot!(context.filters(), context.python_install()
        .args(["--no-config", "--offline", "3.13.2"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Python 3.13.2 is already installed
    ");
    uv_snapshot!(context.filters(), context.python_install()
        .args(["--no-config", "3.13.2", "--reinstall"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.2 in [TIME]
     ~ cpython-3.13.2-[PLATFORM] (python3.13)
    ");
    assert_eq!(
        read_link(bin_python.path()),
        newer.executable(false).simplified_display().to_string()
    );
    uv_snapshot!(context.filters(), Command::new(bin_python.path())
        .args(["-I", "-c", "import sys; print(sys.version.split()[0])"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13.2
    ");
    Ok(())
}

#[test]
fn python_install_repairs_a_dangling_minor_link() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_managed_python_dirs()
        .with_filtered_python_keys()
        .with_filtered_exe_suffix();
    context
        .python_install()
        .args(["--no-config", "--no-bin", "3.13.1"])
        .assert()
        .success();
    context
        .python_install()
        .args(["--no-config", "--offline", "3.13"])
        .assert()
        .success();
    let older = find_installation(&context, "3.13.1")?;
    let minor_link = PythonMinorVersionLink::from_installation(&older)
        .context("CPython must have a minor-version link")?;
    let bin_python = context
        .bin_dir
        .child(format!("python3.13{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(
        read_link(bin_python.path()),
        minor_link
            .symlink_executable
            .simplified_display()
            .to_string()
    );
    assert!(older.is_bin_link(bin_python.path()));

    fs_err::remove_dir_all(older.path())?;
    assert!(fs_err::symlink_metadata(&minor_link.symlink_directory).is_ok());
    assert_eq!(
        fs_err::metadata(&minor_link.symlink_directory)
            .expect_err("the minor-version link must be dangling")
            .kind(),
        std::io::ErrorKind::NotFound,
    );

    // `--force` also permits replacing a Windows launcher whose former target is no longer
    // available for managed-owner identification.
    uv_snapshot!(context.filters(), context.python_install()
        .args(["--no-config", "3.13.2", "--force"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.2 in [TIME]
     + cpython-3.13.2-[PLATFORM] (python3.13)
    ");
    let newer = find_installation(&context, "3.13.2")?;
    let repaired_link = PythonMinorVersionLink::from_installation(&newer)
        .context("CPython must have a minor-version link")?;
    assert!(repaired_link.exists());
    assert_eq!(
        dunce::canonicalize(&repaired_link.symlink_directory)?,
        dunce::canonicalize(newer.path())?
    );
    assert_eq!(
        read_link(bin_python.path()),
        newer.executable(false).simplified_display().to_string()
    );
    assert!(newer.is_bin_link(bin_python.path()));
    uv_snapshot!(context.filters(), Command::new(bin_python.path())
        .args(["-I", "-c", "import sys; print(sys.version.split()[0])"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13.2
    ");

    uv_snapshot!(context.filters(), context.python_install()
        .args(["--no-config", "--offline", "3.13.2"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Python 3.13.2 is already installed
    ");
    assert!(repaired_link.exists());
    Ok(())
}
