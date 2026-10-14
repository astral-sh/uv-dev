use uv_test::{assert_path_missing, uv_snapshot};

#[test]
fn tool_dir() {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_dir(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/tools
    ");
    assert_path_missing(context.temp_dir.join("tools"));
}

#[test]
fn tool_dir_bin() {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_dir().arg("--bin"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/bin
    ");
    assert_path_missing(context.temp_dir.join("bin"));
}
