use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

/// An isolated script does not need to warn about an unrelated active environment.
#[test]
fn add_script_ignores_active_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # ///
    "#})?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("# No dependencies are needed for interpreter discovery.\n")?;

    uv_snapshot!(context.filters(), context.add()
        .args(["--script", "script.py", "-r", "requirements.txt"])
        .args(["--offline", "--no-index", "--no-python-downloads"])
        .env(EnvVars::VIRTUAL_ENV, context.venv.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    Resolved in [TIME]
    ");

    Ok(())
}
