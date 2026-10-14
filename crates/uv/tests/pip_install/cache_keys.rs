//! Invalidation of local source builds through configured cache keys.

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;
use uv_test::uv_snapshot;

/// A recursive cache-key glob detects new matches at every depth and ignores other files.
#[test]
fn glob_invalidates_local_source_build() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        context.temp_dir.child("ok-1.0.0-py3-none-any.whl"),
    )?;
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "ok"
        version = "1.0.0"

        [build-system]
        requires = []
        backend-path = ["."]
        build-backend = "backend"

        [tool.uv]
        cache-keys = [{ file = "**/*.txt" }]
    "#})?;
    project.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path
        from shutil import copyfile

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            root = Path(__file__).resolve().parent.parent
            with (root / "builds.log").open("ab") as log:
                log.write(b"built\n")
            wheel = root / "ok-1.0.0-py3-none-any.whl"
            copyfile(wheel, Path(wheel_directory) / wheel.name)
            return wheel.name
    "#})?;
    // An empty match set has no cache fingerprint, so it cannot prove cache reuse.
    project.child("seed.txt").write_str("initial match")?;
    let builds = context.temp_dir.child("builds.log");

    uv_snapshot!(context.filters(), context.pip_install().args(["./project", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0 (from file://[TEMP_DIR]/project)
    ");
    builds.assert("built\n");

    project
        .child("resources/untracked.bin")
        .write_str("untracked")?;
    uv_snapshot!(context.filters(), context.pip_install().args(["./project", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");
    builds.assert("built\n");

    project
        .child("resources/data.txt")
        .write_str("nested match")?;
    uv_snapshot!(context.filters(), context.pip_install().args(["./project", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ ok==1.0.0 (from file://[TEMP_DIR]/project)
    ");
    builds.assert("built\nbuilt\n");

    project.child("data.txt").write_str("root match")?;
    uv_snapshot!(context.filters(), context.pip_install().args(["./project", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ ok==1.0.0 (from file://[TEMP_DIR]/project)
    ");
    builds.assert("built\nbuilt\nbuilt\n");
    context.assert_installed("ok", "1.0.0");
    Ok(())
}
