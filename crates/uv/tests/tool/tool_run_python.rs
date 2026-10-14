use assert_cmd::assert::OutputAssertExt;
use indoc::indoc;
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
fn tool_run_short_implementation_name_is_a_package() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.tool_run().args(["--offline", "--no-index", "cp311", "--version"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving tool dependencies
      cause: Because cp311 was not found in the provided package locations and you require cp311, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    ");
}

#[test]
fn tool_run_bare_version_is_a_package() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.tool_run().args(["--offline", "--no-index", "311", "--version"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving tool dependencies
      cause: Because 311 was not found in the provided package locations and you require 311, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    ");
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

#[test]
fn tool_run_python() {
    let context = uv_test::test_context!("3.12").with_filtered_counts();
    uv_snapshot!(context.filters(), context.tool_run().arg("--offline")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run().arg("--offline")
        .arg("python")
        .arg("-c")
        .arg("print('Hello, world!')"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved in [TIME]
    ");
}

#[test]
fn tool_run_python_at_version() {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]).with_filtered_counts();

    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "python@3.12", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "python@3.11", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    // The @ is optional.
    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "python3.11", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // Dotless syntax also works.
    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "python311", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // The implementation can be explicit.
    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "cpython311", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // Request a version via `-p`
    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "-p", "3.11", "python", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // @ syntax is also allowed here.
    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "-p", "python@311", "python", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");
}

#[test]
fn tool_run_python_from() {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]).with_filtered_counts();

    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "--from", "python", "python", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "--from", "python@3.11", "python", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "--from", "python311", "python", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run().args(["--offline", "--from", "python>3.11,<3.13", "python", "--version"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");
}

#[test]
fn tool_run_python_from_exposes_selected_interpreter_to_child() {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]).with_filtered_counts();
    let outer_python = context.python_command();
    let script = indoc! {r#"
        import subprocess
        import sys

        child = subprocess.check_output(["python", "--version"], text=True).strip()
        print(f"outer={sys.version_info.major}.{sys.version_info.minor}; child={child}")
    "#};

    uv_snapshot!(context.filters(), context.tool_run()
        .args(["--offline", "--from", "python@3.11"])
        .arg(outer_python.get_program())
        .args(["-c", script]), @"
    exit_code: 0 (success)
    ----- stdout -----
    outer=3.12; child=Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");
}
