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

fn write_noisy_project(context: &TestContext) -> Result<ChildPath> {
    let project = write_project(context)?;
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

    Ok(project)
}

fn assert_artifacts(context: &TestContext) {
    let dist = context.temp_dir.child("dist");
    dist.child(SDIST).assert(predicate::path::is_file());
    dist.child(WHEEL).assert(predicate::path::is_file());
}

#[test]
fn build_shows_backend_output() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = write_noisy_project(&context)?;

    uv_snapshot!(context.filters(), context.build()
        .arg(project.path())
        .args(["--offline", "--out-dir", "dist"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    backend: build_sdist
    Building wheel from source distribution...
    backend: build_wheel
    Successfully built dist/ok-1.0.0.tar.gz
    Successfully built dist/ok-1.0.0-py3-none-any.whl
    ");
    assert_artifacts(&context);
    Ok(())
}

#[test]
fn build_quiet() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = write_noisy_project(&context)?;

    uv_snapshot!(context.build()
        .arg(project.path())
        .args(["--offline", "--out-dir", "dist", "--quiet"]), @"exit_code: 0 (success)");
    assert_artifacts(&context);
    Ok(())
}

#[test]
fn build_no_build_logs() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = write_noisy_project(&context)?;

    uv_snapshot!(context.filters(), context.build()
        .arg(project.path())
        .args(["--offline", "--out-dir", "dist", "--no-build-logs"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    Successfully built dist/ok-1.0.0.tar.gz
    Successfully built dist/ok-1.0.0-py3-none-any.whl
    ");
    assert_artifacts(&context);
    Ok(())
}

#[test]
fn build_hide_build_output_env_var() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = write_noisy_project(&context)?;

    uv_snapshot!(context.filters(), context.build()
        .arg(project.path())
        .args(["--offline", "--out-dir", "dist"])
        .env(EnvVars::UV_HIDE_BUILD_OUTPUT, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    Successfully built dist/ok-1.0.0.tar.gz
    Successfully built dist/ok-1.0.0-py3-none-any.whl
    ");
    assert_artifacts(&context);
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
    context
        .build()
        .args(["project", "--offline"])
        .env("FOO", "bar")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("FOO=bar"));

    uv_snapshot!(context.filters(), context.build()
        .args(["project", "--offline"])
        .env("FOO", "bar")
        .env(EnvVars::UV_HIDE_BUILD_OUTPUT, "1"), @"
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
