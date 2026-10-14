use uv_test::uv_snapshot;

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
