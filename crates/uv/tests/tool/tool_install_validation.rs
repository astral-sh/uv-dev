use anyhow::Result;
use assert_fs::fixture::PathChild;
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::packse::{PackseServer, scenario::Scenario};
use uv_test::{assert_path_missing, uv_snapshot};

#[test]
fn tool_install_from_selects_requested_version() -> Result<()> {
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "tool-from-version"
        [root]
        [expected]
        satisfiable = true
        [packages.formatter.versions."1.0.0"]
        sdist = false
        entry_points = ["formatter"]
        [packages.formatter.versions."2.0.0"]
        sdist = false
        entry_points = ["new-formatter"]
    "#})?;
    let index = PackseServer::from_scenario(&scenario);
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["formatter", "--from", "formatter==1.0.0", "--default-index"])
        .arg(index.index_url())
        .env(EnvVars::PATH, bin_dir.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + formatter==1.0.0
    Installed 1 executable: formatter
    ");
    uv_snapshot!(context.external_command(bin_dir.child(format!("formatter{}", std::env::consts::EXE_SUFFIX))), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from formatter!
    ");
    assert_path_missing(bin_dir.child(format!("new-formatter{}", std::env::consts::EXE_SUFFIX)));
    Ok(())
}

#[test]
fn tool_install_python() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_install().args(["python", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot install Python with `uv tool install`. Did you mean to use `uv python install`?
    ");
}

#[test]
fn tool_install_python_version() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_install().args(["python@3.12", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot install Python with `uv tool install`. Did you mean to use `uv python install`?
    ");
}

#[test]
fn tool_install_mismatched_wheel_name() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_install().args([
        "black",
        "--from",
        "https://example.invalid/flask-3.1.0-py3-none-any.whl",
        "--offline",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package name (`flask`) provided with `--from` does not match install request (`black`)
    ");
}

#[test]
fn tool_install_mismatched_requirement_name() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_install().args([
        "black",
        "--from",
        "flask @ https://example.invalid/flask-3.1.0-py3-none-any.whl",
        "--offline",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package name (`flask`) provided with `--from` does not match install request (`black`)
    ");
}

#[test]
fn tool_install_mismatched_registry_requirement_name() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.tool_install().args(["black", "--from", "flask==24.2.0", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package name (`flask`) provided with `--from` does not match install request (`black`)
    ");
    assert_path_missing(context.temp_dir.join("tools/black"));
}

#[test]
fn tool_install_conflicting_registry_requirement_version() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.tool_install().args(["black==24.2.0", "--from", "black==24.3.0", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package requirement (`black==24.3.0`) provided with `--from` conflicts with install request (`black==24.2.0`)
    ");
    assert_path_missing(context.temp_dir.join("tools/black"));
}

#[test]
fn tool_install_requirement_name_precedes_wheel_name() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_install().args([
        "flask",
        "--from",
        "black @ https://example.invalid/flask-3.1.0-py3-none-any.whl",
        "--offline",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package name (`black`) provided with `--from` does not match install request (`flask`)
    ");
}
