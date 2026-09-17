use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::PathChild;

use uv_static::EnvVars;

use uv_test::uv_snapshot;

#[test]
fn tool_uninstall() {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-list.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("list-tool"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 executables: list-tool, list-tool-helper
    ");

    // After uninstalling the tool, it shouldn't be listed.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");

    // After uninstalling the tool, we should be able to reinstall it.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("list-tool==1.0.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + list-tool==1.0.0
    Installed 2 executables: list-tool, list-tool-helper
    ");
}

#[test]
fn tool_uninstall_multiple_names() {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-list.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();

    context
        .tool_install()
        .arg("list-other==0.3.4")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("list-tool").arg("list-other"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 3 executables: list-other, list-tool, list-tool-helper
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
        .with_packse_index("packages/tool-list.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("list-tool"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `list-tool` is not installed
    ");
}

#[test]
fn tool_uninstall_missing_receipt() {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-list.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();

    fs_err::remove_file(tool_dir.join("list-tool").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("list-tool"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `list-tool`
    ");
}

#[test]
fn tool_uninstall_multiple_names_with_missing_receipt() {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-list.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();

    context
        .tool_install()
        .arg("list-other==0.3.4")
        .assert()
        .success();

    fs_err::remove_file(tool_dir.join("list-tool").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("list-tool").arg("list-other"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `list-tool`
    Uninstalled 1 executable: list-other
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
        .with_packse_index("packages/tool-list.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();

    fs_err::remove_file(tool_dir.join("list-tool").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `list-tool`
    ");
}
