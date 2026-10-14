use assert_cmd::assert::OutputAssertExt;
use uv_test::{TestContext, uv_snapshot};

fn context_with_global_python_pin() -> TestContext {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]).with_filtered_counts();
    context
        .python_pin()
        .args(["3.11", "--global"])
        .assert()
        .success();
    context
}

#[test]
fn tool_run_python_from_global_version_file() {
    let context = context_with_global_python_pin();

    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "python", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");
}

#[test]
fn tool_run_python_version_overrides_global_pin() {
    let context = context_with_global_python_pin();

    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "python3.12", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");
}

#[test]
fn tool_run_python_with_explicit_default_bypasses_global_pin() {
    let context = context_with_global_python_pin();

    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "--python", "default", "python", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");
}
