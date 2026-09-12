use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{PathChild, PathCreateDir};
use url::Url;

use uv_static::EnvVars;

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
    let context = uv_test::test_context!("3.13").with_filtered_exe_suffix();
    let tool_dir = context.temp_dir.child("tools");
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

    context
        .tool_install()
        .arg(&launcher)
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str())
        .assert()
        .success();

    context
        .tool_install()
        .arg(&app)
        .arg("--with-executables-from")
        .arg(&launcher_requirement)
        .arg("--force")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str()), @"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    Removed environment for `simple-launcher`
    ");

    assert!(
        bin_dir
            .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX))
            .exists()
    );

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("basic-app")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str()), @"
    success: true
    exit_code: 0
    ----- stdout -----

    ----- stderr -----
    Uninstalled 2 executables: basic-app, simple_launcher
    ");

    #[cfg(unix)]
    {
        context
            .tool_install()
            .arg(&launcher)
            .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
            .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str())
            .assert()
            .success();

        context
            .tool_install()
            .arg(&app)
            .arg("--with-executables-from")
            .arg(&launcher_requirement)
            .arg("--force")
            .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
            .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str())
            .assert()
            .success();

        uv_snapshot!(context.filters(), context.tool_uninstall().arg("basic-app")
            .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
            .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str()), @"
        success: true
        exit_code: 0
        ----- stdout -----

        ----- stderr -----
        Uninstalled 2 executables: basic-app, simple_launcher
        ");

        assert!(
            !bin_dir
                .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX))
                .exists()
        );
    }
}

#[test]
fn tool_uninstall_validates_other_tools_before_removing_environment() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_filtered_exe_suffix();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let launcher = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");

    context
        .tool_install()
        .arg(&launcher)
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str())
        .assert()
        .success();

    tool_dir.child(".tmp-invalid").create_dir_all()?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str()), @r#"
    success: false
    exit_code: 2
    ----- stdout -----

    ----- stderr -----
    error: Not a valid package or extra name: ".tmp-invalid". Names must start and end with a letter or digit and may only contain -, _, ., and alphanumeric characters.
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
