use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;

use uv_test::uv_snapshot;

use crate::tool_fixture::local_tool_wheel;

#[test]
fn tool_list() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let wheel = local_tool_wheel(&context, "formatter", &["formatter", "formatterd"])?;
    context
        .tool_install()
        .arg(wheel.path())
        .arg("--offline")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    formatter v1.0.0
    - formatter
    - formatterd
    ");
    Ok(())
}

#[test]
fn tool_list_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let wheel = local_tool_wheel(&context, "formatter", &["formatter", "formatterd"])?;
    context
        .tool_install()
        .arg(wheel.path())
        .arg("--offline")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_list().arg("--show-paths"), @"
    exit_code: 0 (success)
    ----- stdout -----
    formatter v1.0.0 ([TEMP_DIR]/tools/formatter)
    - formatter ([TEMP_DIR]/bin/formatter)
    - formatterd ([TEMP_DIR]/bin/formatterd)
    ");
    Ok(())
}

#[cfg(windows)]
#[test]
fn tool_list_paths_windows() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .clear_filters()
        .with_filtered_windows_temp_dir()
        .with_tool_dirs();
    let wheel = local_tool_wheel(&context, "formatter", &["formatter", "formatterd"])?;
    context
        .tool_install()
        .arg(wheel.path())
        .arg("--offline")
        .assert()
        .success();

    uv_snapshot!(context.filters_without_standard_filters(), context.tool_list().arg("--show-paths"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    formatter v1.0.0 ([TEMP_DIR]\tools\formatter)
    - formatter ([TEMP_DIR]\bin\formatter.exe)
    - formatterd ([TEMP_DIR]\bin\formatterd.exe)
    "###);
    Ok(())
}

#[test]
fn tool_list_show_python() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let wheel = local_tool_wheel(&context, "formatter", &["formatter", "formatterd"])?;
    context
        .tool_install()
        .arg(wheel.path())
        .arg("--offline")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_list().arg("--show-python"), @"
    exit_code: 0 (success)
    ----- stdout -----
    formatter v1.0.0 [CPython 3.12.[X]]
    - formatter
    - formatterd
    ");
    Ok(())
}
