#[cfg(unix)]
use fs_err::os::unix::fs::symlink;
#[cfg(windows)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteBin, PathChild, PathCreateDir};
use url::Url;

use uv_static::EnvVars;

#[cfg(unix)]
use uv_test::ReadOnlyDirectoryGuard;
#[cfg(windows)]
use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

#[test]
fn tool_uninstall() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 executables: black, blackd
    ");

    // After uninstalling the tool, it shouldn't be listed.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");

    // After uninstalling the tool, we should be able to reinstall it.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.2.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.2.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");
}

#[test]
fn tool_uninstall_preserves_replaced_executable() {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let launcher = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");
    let app = context
        .workspace_root
        .join("test/links/basic_app-0.1.0-py3-none-any.whl");
    let launcher_requirement = format!(
        "simple-launcher @ {}",
        Url::from_file_path(&launcher).expect("Failed to convert launcher path to file URL")
    );

    context.tool_install().arg(&launcher).assert().success();

    context
        .tool_install()
        .arg(&app)
        .arg("--with-executables-from")
        .arg(&launcher_requirement)
        .arg("--force")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed environment for `simple-launcher`
    ");

    assert!(
        bin_dir
            .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX))
            .exists()
    );

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("basic-app"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 executables: basic-app, simple_launcher
    ");
}

/// A stale original receipt must not retain a launcher owned by the replacement being removed.
#[test]
fn tool_uninstall_removes_replacement_before_stale_owner() {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let launcher = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");
    let app = context
        .workspace_root
        .join("test/links/basic_app-0.1.0-py3-none-any.whl");
    let launcher_requirement = format!(
        "simple-launcher @ {}",
        Url::from_file_path(&launcher).expect("Failed to convert launcher path to file URL")
    );

    context.tool_install().arg(&launcher).assert().success();

    context
        .tool_install()
        .arg(&app)
        .arg("--with-executables-from")
        .arg(&launcher_requirement)
        .arg("--force")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("basic-app"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 executables: basic-app, simple_launcher
    ");

    assert!(
        !bin_dir
            .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX))
            .exists()
    );
}

#[test]
fn tool_uninstall_validates_other_tools_before_removing_environment() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let launcher = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");

    context.tool_install().arg(&launcher).assert().success();

    tool_dir.child("babel").create_dir_all()?;
    tool_dir
        .child("babel/uv-receipt.toml")
        .write_binary(&[0xff])?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: failed to read from file `[TEMP_DIR]/tools/babel/uv-receipt.toml`: stream did not contain valid UTF-8
    "#);

    assert!(tool_dir.child("simple-launcher").exists());
    assert!(
        bin_dir
            .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX))
            .exists()
    );

    Ok(())
}

#[test]
fn tool_uninstall_all_with_dangling_environment() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tools = context.temp_dir.child("tools");
    context
        .tool_install()
        .arg(
            context
                .workspace_root
                .join("test/links/simple_launcher-0.1.0-py3-none-any.whl"),
        )
        .assert()
        .success();
    tools.child("dangling").create_dir_all()?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `dangling`
    Uninstalled 1 executable: simple_launcher
    ");
    assert!(!tools.child("dangling").exists());
    assert!(!tools.child("simple-launcher").exists());
    assert!(
        !context
            .temp_dir
            .child("bin")
            .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX))
            .exists()
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn tool_uninstall_preserves_replacement_with_symlinked_tool_directory() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_filtered_exe_suffix();
    let real_tools = context.temp_dir.child("real-tools");
    real_tools.create_dir_all()?;
    let tools = context.temp_dir.child("linked-tools");
    symlink(real_tools.path(), tools.path())?;
    let bin = context.temp_dir.child("bin");
    let launcher = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");
    let app = context
        .workspace_root
        .join("test/links/basic_app-0.1.0-py3-none-any.whl");
    let requirement = format!(
        "simple-launcher @ {}",
        Url::from_file_path(&launcher).expect("launcher file URL")
    );
    context
        .tool_install()
        .arg(&launcher)
        .env(EnvVars::UV_TOOL_DIR, tools.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin.as_os_str())
        .assert()
        .success();
    context
        .tool_install()
        .arg(&app)
        .arg("--with-executables-from")
        .arg(requirement)
        .arg("--force")
        .env(EnvVars::UV_TOOL_DIR, tools.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher")
        .env(EnvVars::UV_TOOL_DIR, tools.as_os_str()).env(EnvVars::XDG_BIN_HOME, bin.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed environment for `simple-launcher`
    ");
    assert!(tools.child("basic-app").exists());
    uv_snapshot!(context.filters(), Command::new(bin.child("simple_launcher").path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");
    Ok(())
}

/// Bin-directory aliases identify the same destination without conflating different commands.
#[test]
#[cfg(unix)]
fn tool_uninstall_preserves_replacement_with_symlinked_bin_directory() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_filtered_exe_suffix();
    let tools = context.temp_dir.child("tools");
    let bin = context.temp_dir.child("bin");
    bin.create_dir_all()?;
    let alias = context.temp_dir.child("linked-bin");
    symlink(bin.path(), alias.path())?;
    let launcher = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");
    let app = context
        .workspace_root
        .join("test/links/basic_app-0.1.0-py3-none-any.whl");
    let requirement = format!(
        "simple-launcher @ {}",
        Url::from_file_path(&launcher).expect("launcher file URL")
    );
    context
        .tool_install()
        .arg(&launcher)
        .env(EnvVars::UV_TOOL_DIR, tools.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin.as_os_str())
        .assert()
        .success();
    context
        .tool_install()
        .arg(&app)
        .arg("--with-executables-from")
        .arg(requirement)
        .arg("--force")
        .env(EnvVars::UV_TOOL_DIR, tools.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, alias.as_os_str())
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher")
        .env(EnvVars::UV_TOOL_DIR, tools.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed environment for `simple-launcher`
    ");
    assert!(tools.child("basic-app").exists());
    uv_snapshot!(context.filters(), Command::new(bin.child("simple_launcher").path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");
    Ok(())
}

/// Permission errors must not turn a retained executable into an unclaimed destination.
#[test]
#[cfg(unix)]
fn tool_uninstall_preserves_replacement_when_ownership_is_unreadable() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tools = context.temp_dir.child("tools");
    let bin = context.temp_dir.child("bin");
    let launcher = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");
    let app = context
        .workspace_root
        .join("test/links/basic_app-0.1.0-py3-none-any.whl");
    let requirement = format!(
        "simple-launcher @ {}",
        Url::from_file_path(&launcher).expect("launcher file URL")
    );
    context.tool_install().arg(&launcher).assert().success();
    context
        .tool_install()
        .arg(&app)
        .arg("--with-executables-from")
        .arg(requirement)
        .arg("--force")
        .assert()
        .success();
    let protected = tools.child("basic-app/bin");
    {
        let _restore = ReadOnlyDirectoryGuard::new(protected.path())?;
        let mut permissions = fs_err::metadata(&protected)?.permissions();
        permissions.set_mode(0o000);
        fs_err::set_permissions(&protected, permissions)?;
        uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher"), @"
        exit_code: 2 (failure)
        ----- stderr -----
        error: failed to canonicalize path `[TEMP_DIR]/bin/simple_launcher`: Permission denied (os error 13)
        ");
        assert!(tools.child("simple-launcher").exists());
        assert!(tools.child("basic-app").exists());
    }
    uv_snapshot!(context.filters(), Command::new(bin.child("simple_launcher").path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");
    Ok(())
}

/// Different Windows filename casing can identify the same copied executable.
#[test]
#[cfg(windows)]
fn tool_uninstall_preserves_replacement_with_different_filename_case() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin = context.temp_dir.child("bin");
    let (filename, wheel) = generate_wheel(
        &"first".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &["shared-tool".to_owned()],
    );
    let first = context.temp_dir.child(filename);
    first.write_binary(&wheel)?;
    let (filename, wheel) = generate_wheel(
        &"second".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &["Shared-Tool".to_owned()],
    );
    let second = context.temp_dir.child(filename);
    second.write_binary(&wheel)?;
    context.tool_install().arg(first.path()).assert().success();
    context
        .tool_install()
        .arg(second.path())
        .arg("--force")
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.tool_uninstall().arg("first"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed environment for `first`
    ");
    assert!(context.temp_dir.child("tools/second").exists());
    uv_snapshot!(context.filters(), Command::new(bin.child("shared-tool.exe").path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from second!
    ");
    Ok(())
}

#[test]
fn tool_uninstall_multiple_names() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    context.tool_install().arg("ruff==0.3.4").assert().success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black").arg("ruff"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 3 executables: black, blackd, ruff
    ");

    // After uninstalling the tool, it shouldn't be listed.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
}

#[test]
fn tool_uninstall_not_installed() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `black` is not installed
    ");
}

#[test]
fn tool_uninstall_missing_receipt() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    fs_err::remove_file(tool_dir.join("black").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `black`
    ");
}

#[test]
fn tool_uninstall_multiple_names_with_missing_receipt() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    context.tool_install().arg("ruff==0.3.4").assert().success();

    fs_err::remove_file(tool_dir.join("black").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black").arg("ruff"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `black`
    Uninstalled 1 executable: ruff
    ");

    // After uninstalling both tools, neither should be listed.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
}

#[test]
fn tool_uninstall_all_missing_receipt() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    fs_err::remove_file(tool_dir.join("black").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `black`
    ");
}
