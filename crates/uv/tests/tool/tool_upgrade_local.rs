use anyhow::Result;
use assert_fs::fixture::{FileWriteBin, FileWriteStr, PathChild};

use uv_static::EnvVars;
use uv_test::{assert_path_missing, uv_snapshot};

#[test]
fn tool_upgrade_empty() {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("--all")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to upgrade
    ");

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("--all")
        .arg("-p")
        .arg("3.13")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to upgrade
    ");
}

#[test]
fn tool_upgrade_all_ignores_invalid_tool_name() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let backup = context.temp_dir.child("tools/tool backup/user-data.txt");
    backup.write_str("user data")?;

    uv_snapshot!(context.filters(), context.tool_upgrade().arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring tool directory `tools/tool backup` with an invalid package name; move it outside the tool directory, or remove it if no longer needed
    Nothing to upgrade
    ");

    assert_eq!(context.read(&backup), "user data");
    Ok(())
}

#[test]
fn tool_upgrade_all_unreadable_receipt() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let receipt = context.temp_dir.child("tools/babel/uv-receipt.toml");
    receipt.write_binary(&[0xff])?;

    uv_snapshot!(context.filters(), context.tool_upgrade().arg("--all"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect installed tools in `tools`
      cause: failed to read from file `[TEMP_DIR]/tools/babel/uv-receipt.toml`: stream did not contain valid UTF-8
    ");

    assert_eq!(context.read_bytes(&receipt), [0xff]);
    Ok(())
}

#[test]
fn tool_upgrade_non_existing_package() {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to upgrade black
      cause: `black` is not installed; run `uv tool install black` to install
    ");

    assert_path_missing(context.temp_dir.child("tools/black"));
}
