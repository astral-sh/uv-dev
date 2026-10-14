//! Running compiled Python scripts.

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use predicates::prelude::predicate;
use uv_test::uv_snapshot;

#[test]
fn execute_bytecode() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = context.temp_dir.child("main.py");
    source.write_str("print('Hello, world!')\n")?;
    context
        .python_command()
        .args(["-m", "compileall", "-b"])
        .arg(source.path())
        .assert()
        .success();
    let bytecode = context.temp_dir.child("main.pyc");
    bytecode.assert(predicate::path::is_file());

    uv_snapshot!(context.filters(), context.run().arg("--offline").arg(bytecode.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!
    ");

    Ok(())
}

#[test]
fn inline_metadata_is_unavailable_in_bytecode() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.workspace_root.join("test/links");
    let source = context.temp_dir.child("script.py");
    source.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = ["ok==1.0.0"]
        # ///
        import ok
    "#})?;

    uv_snapshot!(context.filters(), context.run().args(["--offline", "--no-index", "--find-links"]).arg(&links).arg(source.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    ");

    context
        .python_command()
        .args(["-m", "compileall", "-b"])
        .arg(source.path())
        .assert()
        .success();
    let bytecode = context.temp_dir.child("script.pyc");
    bytecode.assert(predicate::path::is_file());
    uv_snapshot!(context.filters(), context.run().args(["--offline", "--no-index", "--find-links"]).arg(&links).arg(bytecode.path()), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Traceback (most recent call last):
      File "[TEMP_DIR]/script.py", line 5, in <module>
        import ok
    ModuleNotFoundError: No module named 'ok'
    "#);

    Ok(())
}
