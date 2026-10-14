use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use predicates::prelude::*;

use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

const SDIST: &str = "ok-1.0.0.tar.gz";
const WHEEL: &str = "ok-1.0.0-py3-none-any.whl";

fn write_project(context: &TestContext) -> Result<ChildPath> {
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "ok"
        version = "1.0.0"
        requires-python = ">=3.8"

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    Ok(project)
}

/// Output controls must suppress the intended messages without skipping either build stage.
#[test]
fn build_output_controls() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = write_project(&context)?;
    project
        .child(WHEEL)
        .write_binary(&context.read_bytes(context.workspace_root.join("test/links").join(WHEEL)))?;
    project
        .child("PKG-INFO")
        .write_str("Metadata-Version: 2.3\nName: ok\nVersion: 1.0.0\nRequires-Python: >=3.8\n")?;
    project.child("backend.py").write_str(&formatdoc! {r#"
        from pathlib import Path
        import shutil
        import sys
        import tarfile

        root = Path(__file__).parent
        wheel = "{WHEEL}"
        sdist = "{SDIST}"

        def build_sdist(sdist_directory, config_settings=None):
            print("backend: build_sdist", file=sys.stderr, flush=True)
            with tarfile.open(Path(sdist_directory) / sdist, "w:gz") as archive:
                for filename in ("pyproject.toml", "backend.py", "PKG-INFO", wheel):
                    archive.add(root / filename, arcname=sdist.removesuffix(".tar.gz") + "/" + filename)
            return sdist

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            print("backend: build_wheel", file=sys.stderr, flush=True)
            shutil.copyfile(root / wheel, Path(wheel_directory) / wheel)
            return wheel
    "#})?;

    let build = |output_dir: &str| {
        let mut command = context.build();
        command.args(["project", "--offline", "--out-dir", output_dir]);
        command
    };
    let assert_artifacts = |output_dir: &str| {
        for filename in [SDIST, WHEEL] {
            context
                .temp_dir
                .child(output_dir)
                .child(filename)
                .assert(predicate::path::is_file());
        }
    };

    build("visible").assert().success().stdout("").stderr(
        predicate::str::contains("backend: build_sdist")
            .and(predicate::str::contains("backend: build_wheel")),
    );
    assert_artifacts("visible");

    build("quiet")
        .arg("--quiet")
        .assert()
        .success()
        .stdout("")
        .stderr("");
    assert_artifacts("quiet");

    uv_snapshot!(context.filters(), build("flag").arg("--no-build-logs"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    Successfully built flag/ok-1.0.0.tar.gz
    Successfully built flag/ok-1.0.0-py3-none-any.whl
    ");
    assert_artifacts("flag");

    uv_snapshot!(context.filters(), build("env").env(EnvVars::UV_HIDE_BUILD_OUTPUT, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    Successfully built env/ok-1.0.0.tar.gz
    Successfully built env/ok-1.0.0-py3-none-any.whl
    ");
    assert_artifacts("env");

    Ok(())
}

#[test]
fn hide_build_output_on_failure() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = write_project(&context)?;
    project.child("backend.py").write_str(indoc! {r#"
        import os
        import sys

        def get_requires_for_build_sdist(config_settings=None):
            print("FOO=" + os.environ["FOO"], file=sys.stderr, flush=True)
            raise RuntimeError("Build failed intentionally!")
    "#})?;
    let build = || {
        let mut command = context.build();
        command.args(["project", "--offline"]).env("FOO", "bar");
        command
    };

    build()
        .assert()
        .code(2)
        .stderr(predicate::str::contains("FOO=bar"));

    uv_snapshot!(context.filters(), build().env(EnvVars::UV_HIDE_BUILD_OUTPUT, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Building source distribution...
    error: Failed to build `[TEMP_DIR]/project`
      cause: The build backend returned an error
      cause: Call to `backend.get_requires_for_build_sdist` failed (exit status: 1)

    hint: Build failures usually indicate a problem with the package or the build environment
    ");

    Ok(())
}
