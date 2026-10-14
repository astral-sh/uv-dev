use std::path::Path;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::{TestContext, site_packages_path, uv_snapshot};

fn assert_ok_installed_at(context: &TestContext, target: &Path, version: &'static str) {
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
        .arg(target)
        .assert()
        .success()
        .stdout(version);
}

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
    assert_ok_installed_at(&context, target.path(), "1.0.0");
    Ok(())
}

/// Sync replaces a distribution in the prefix without installing into the active environment.
#[test]
fn prefix_replaces_distribution() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin();
    let prefix = context.temp_dir.child("prefix");
    let site_packages = ChildPath::new(site_packages_path(prefix.path(), "python3.12"));
    let requirements = context.temp_dir.child("requirements.in");
    let sync = || {
        let mut command = context.pip_sync();
        command
            .args(["requirements.in", "--prefix"])
            .arg(prefix.path())
            .args(["--no-index", "--find-links"])
            .arg(context.workspace_root.join("test/links"));
        command
    };

    requirements.write_str("ok==2.0.0")?;
    uv_snapshot!(context.filters(), sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==2.0.0
    ");
    assert_ok_installed_at(&context, site_packages.path(), "2.0.0");
    context.assert_not_installed("ok");

    requirements.write_str("ok==1.0.0")?;
    uv_snapshot!(context.filters(), sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - ok==2.0.0
     + ok==1.0.0
    ");
    assert_ok_installed_at(&context, site_packages.path(), "1.0.0");
    site_packages
        .child("ok-2.0.0.dist-info")
        .assert(predicates::path::missing());
    Ok(())
}
