use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild};

use uv_static::EnvVars;
use uv_test::{assert_path_missing, uv_snapshot};

use crate::tool_fixture::local_tool_wheel;

#[test]
fn tool_uninstall() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let wheel = local_tool_wheel(&context, "formatter", &["formatter", "formatterd"])?;
    context
        .tool_install()
        .arg(wheel.path())
        .arg("--offline")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("formatter"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 executables: formatter, formatterd
    ");

    assert_path_missing(context.temp_dir.child("tools/formatter"));
    assert_path_missing(bin_dir.child(format!("formatter{}", std::env::consts::EXE_SUFFIX)));
    assert_path_missing(bin_dir.child(format!("formatterd{}", std::env::consts::EXE_SUFFIX)));
    assert!(context.temp_dir.child("tools/.lock").is_file());

    // After uninstalling the tool, it shouldn't be listed.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");

    // After uninstalling the tool, we should be able to reinstall it.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg(wheel.path())
        .arg("--offline")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + formatter==1.0.0 (from file://[TEMP_DIR]/formatter-1.0.0-py3-none-any.whl)
    Installed 2 executables: formatter, formatterd
    ");
    Ok(())
}

/// Files beside the tools directory must survive removal of the last tool, even if no other
/// directories remain in the parent.
#[test]
fn tool_uninstall_preserves_parent_files() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("data/tools");
    context
        .temp_dir
        .child("data/uv.toml")
        .write_str("# User configuration\n")?;
    context
        .temp_dir
        .child("data/keep.txt")
        .write_str("Unrelated file\n")?;

    let wheel = local_tool_wheel(&context, "linter", &["linter"])?;
    context
        .tool_install()
        .arg(wheel.path())
        .arg("--offline")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall()
        .arg("linter")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 executable: linter
    ");

    assert_eq!(context.read("data/uv.toml"), "# User configuration\n");
    assert_eq!(context.read("data/keep.txt"), "Unrelated file\n");
    assert!(tool_dir.child(".lock").is_file());

    Ok(())
}

#[test]
fn tool_uninstall_multiple_names() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let formatter = local_tool_wheel(&context, "formatter", &["formatter", "formatterd"])?;
    context
        .tool_install()
        .arg(formatter.path())
        .arg("--offline")
        .assert()
        .success();
    let linter = local_tool_wheel(&context, "linter", &["linter"])?;
    context
        .tool_install()
        .arg(linter.path())
        .arg("--offline")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("formatter").arg("linter"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 3 executables: formatter, formatterd, linter
    ");
    assert_path_missing(tool_dir.child("formatter"));
    assert_path_missing(tool_dir.child("linter"));
    assert_path_missing(bin_dir.child(format!("formatter{}", std::env::consts::EXE_SUFFIX)));
    assert_path_missing(bin_dir.child(format!("formatterd{}", std::env::consts::EXE_SUFFIX)));
    assert_path_missing(bin_dir.child(format!("linter{}", std::env::consts::EXE_SUFFIX)));

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
    Ok(())
}

#[test]
fn tool_uninstall_missing_receipt() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let environment = context.temp_dir.child("tools/formatter");

    let wheel = local_tool_wheel(&context, "formatter", &["formatter", "formatterd"])?;
    context
        .tool_install()
        .arg(wheel.path())
        .arg("--offline")
        .assert()
        .success();

    fs_err::remove_file(environment.child("uv-receipt.toml"))?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("formatter"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `formatter`
    ");
    assert_path_missing(environment);
    Ok(())
}

#[test]
fn tool_uninstall_multiple_names_with_missing_receipt() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let environment = context.temp_dir.child("tools/formatter");

    let formatter = local_tool_wheel(&context, "formatter", &["formatter", "formatterd"])?;
    context
        .tool_install()
        .arg(formatter.path())
        .arg("--offline")
        .assert()
        .success();
    let linter = local_tool_wheel(&context, "linter", &["linter"])?;
    context
        .tool_install()
        .arg(linter.path())
        .arg("--offline")
        .assert()
        .success();

    fs_err::remove_file(environment.child("uv-receipt.toml"))?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("formatter").arg("linter"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `formatter`
    Uninstalled 1 executable: linter
    ");
    assert_path_missing(environment);

    // After uninstalling both tools, neither should be listed.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
    Ok(())
}

#[test]
fn tool_uninstall_all_missing_receipt() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let environment = context.temp_dir.child("tools/formatter");

    let wheel = local_tool_wheel(&context, "formatter", &["formatter", "formatterd"])?;
    context
        .tool_install()
        .arg(wheel.path())
        .arg("--offline")
        .assert()
        .success();

    fs_err::remove_file(environment.child("uv-receipt.toml"))?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `formatter`
    ");
    assert_path_missing(environment);
    Ok(())
}
