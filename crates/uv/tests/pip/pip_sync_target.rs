use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::uv_snapshot;

/// Install into the requested target using a system interpreter and no virtual environment.
#[test]
fn target_system() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .temp_dir
        .child("requirements.in")
        .write_str("ok==1.0.0")?;
    let target = context.temp_dir.child("target");

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.in", "--target", "target", "--no-index", "--find-links"])
        .arg(context.workspace_root.join("test/links")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    ");

    context.venv.assert(predicates::path::missing());
    context
        .python_command()
        .args(["-I", "-c"])
        .arg(indoc! {r#"
            from pathlib import Path
            import sys

            target = Path(sys.argv[1]).resolve()
            sys.path.insert(0, str(target))
            import ok
            assert Path(ok.__file__).samefile(target / "ok" / "__init__.py")
            print(ok.__version__, end="")
        "#})
        .arg(target.path())
        .assert()
        .success()
        .stdout("1.0.0");
    Ok(())
}
