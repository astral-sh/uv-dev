use anyhow::Result;
use assert_fs::assert::PathAssert;
use assert_fs::fixture::{ChildPath, PathChild};
use predicates::prelude::predicate;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

use crate::tool_fixture::local_tool_wheel;

fn assert_formatter_launcher(directory: &ChildPath) {
    directory
        .child(format!("formatter{}", std::env::consts::EXE_SUFFIX))
        .assert(predicate::path::is_file().follow_links(true));
}

#[cfg(unix)]
#[test]
fn tool_install_home_fallback() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = local_tool_wheel(&context, "formatter", &["formatter"])?;

    uv_snapshot!(context.tool_install().arg(wheel.path()).args(["--offline", "--quiet"])
        .env(EnvVars::UV_TOOL_DIR, context.temp_dir.child("tools").path())
        .env_remove(EnvVars::XDG_DATA_HOME), @"exit_code: 0 (success)");
    assert_formatter_launcher(&context.home_dir.child(".local/bin"));
    Ok(())
}

#[test]
fn tool_install_xdg_data_home() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = local_tool_wheel(&context, "formatter", &["formatter"])?;

    uv_snapshot!(context.tool_install().arg(wheel.path()).args(["--offline", "--quiet"])
        .env(EnvVars::UV_TOOL_DIR, context.temp_dir.child("tools").path())
        .env(EnvVars::XDG_DATA_HOME, context.temp_dir.child("data/home").path()), @"exit_code: 0 (success)");
    assert_formatter_launcher(&context.temp_dir.child("data/bin"));
    context.assert_path_missing(context.home_dir.child(".local/bin"));
    Ok(())
}

#[test]
fn tool_install_xdg_bin_home_overrides_data_home() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = local_tool_wheel(&context, "formatter", &["formatter"])?;

    uv_snapshot!(context.tool_install().arg(wheel.path()).args(["--offline", "--quiet"])
        .env(EnvVars::UV_TOOL_DIR, context.temp_dir.child("tools").path())
        .env(EnvVars::XDG_DATA_HOME, context.temp_dir.child("data/home").path())
        .env(EnvVars::XDG_BIN_HOME, context.temp_dir.child("xdg-bin").path()), @"exit_code: 0 (success)");
    assert_formatter_launcher(&context.temp_dir.child("xdg-bin"));
    context.assert_path_missing("data/bin");
    Ok(())
}

#[test]
fn tool_install_tool_bin_dir_overrides_xdg_bin_home() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = local_tool_wheel(&context, "formatter", &["formatter"])?;

    uv_snapshot!(context.tool_install().arg(wheel.path()).args(["--offline", "--quiet"])
        .env(EnvVars::UV_TOOL_DIR, context.temp_dir.child("tools").path())
        .env(EnvVars::XDG_BIN_HOME, context.temp_dir.child("xdg-bin").path())
        .env(EnvVars::UV_TOOL_BIN_DIR, context.temp_dir.child("tool-bin").path()), @"exit_code: 0 (success)");
    assert_formatter_launcher(&context.temp_dir.child("tool-bin"));
    context.assert_path_missing("xdg-bin");
    Ok(())
}
