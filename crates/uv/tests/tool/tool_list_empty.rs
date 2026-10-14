use anyhow::Result;
use assert_fs::fixture::{PathChild, PathCreateDir};

use uv_test::uv_snapshot;

#[test]
fn missing_tool_directory() {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
    context.assert_path_missing("tools");
}

#[test]
fn outdated_in_empty_tool_directory() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    context.temp_dir.child("tools").create_dir_all()?;

    uv_snapshot!(context.filters(), context.tool_list().arg("--outdated").arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");

    Ok(())
}
