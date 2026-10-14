use anyhow::Result;
use assert_fs::fixture::{FileWriteStr, PathChild};

use uv_static::EnvVars;
use uv_test::uv_snapshot;

/// Uninstalling all tools must preserve parent files when no tools are installed.
#[test]
fn all_preserves_parent_files() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let tool_dir = context.temp_dir.child("data/tools");
    context
        .temp_dir
        .child("data/uv.toml")
        .write_str("# User configuration\n")?;

    uv_snapshot!(context.filters(), context.tool_uninstall()
        .arg("--all")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to uninstall
    ");

    assert_eq!(context.read("data/uv.toml"), "# User configuration\n");
    assert!(tool_dir.child(".lock").is_file());

    Ok(())
}

#[test]
fn named_tool() {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `black` is not installed
    ");
}
