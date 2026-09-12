use std::fmt::Write;
use std::process::Command;

use anyhow::{Result, anyhow};
use assert_fs::{fixture::ChildPath, prelude::*};
use indoc::{formatdoc, indoc};
use insta::allow_duplicates;
use url::Url;

use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

const BACKEND: &str = indoc! {r#"
    import base64
    import csv
    import hashlib
    import io
    import os
    import tomllib
    import zipfile
    from pathlib import Path

    ROOT = Path(__file__).parent
    PROJECT = tomllib.loads((ROOT / "pyproject.toml").read_text())["project"]
    NAME = PROJECT["name"].replace("-", "_")
    VERSION = PROJECT.get("version", "0.1.0")
    DIST_INFO = f"{NAME}-{VERSION}.dist-info"
    METADATA = f"Metadata-Version: 2.3\nName: {PROJECT['name']}\nVersion: {VERSION}\nRequires-Python: >=3.12\n".encode()
    WHEEL = b"Wheel-Version: 1.0\nGenerator: uv-test\nRoot-Is-Purelib: true\nTag: py3-none-any\n"

    def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
        directory = Path(metadata_directory) / DIST_INFO
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "METADATA").write_bytes(METADATA)
        (directory / "WHEEL").write_bytes(WHEEL)
        return DIST_INFO

    prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel

    def get_requires_for_build_wheel(config_settings=None):
        return []

    get_requires_for_build_editable = get_requires_for_build_wheel

    def build(wheel_directory, editable):
        if marker := os.environ.get("SOURCE_PREPARATION_FAIL_ONCE"):
            try:
                descriptor = os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            except FileExistsError:
                pass
            else:
                os.close(descriptor)
                raise RuntimeError("intentional source preparation failure")
        files = {
            f"{DIST_INFO}/METADATA": METADATA,
            f"{DIST_INFO}/WHEEL": WHEEL,
        }
        if editable:
            files[f"{NAME}.pth"] = (ROOT.as_posix() + "\n").encode()
        else:
            files[f"{NAME}.py"] = (ROOT / f"{NAME}.py").read_bytes()
        record = io.StringIO()
        writer = csv.writer(record, lineterminator="\n")
        for name, data in files.items():
            digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
            writer.writerow([name, "sha256=" + digest, len(data)])
        writer.writerow([f"{DIST_INFO}/RECORD", "", ""])
        files[f"{DIST_INFO}/RECORD"] = record.getvalue().encode()
        filename = f"{NAME}-{VERSION}-py3-none-any.whl"
        with zipfile.ZipFile(Path(wheel_directory) / filename, "w", zipfile.ZIP_DEFLATED) as wheel:
            for name, data in files.items():
                wheel.writestr(name, data)
        return filename

    def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
        return build(wheel_directory, False)

    def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
        return build(wheel_directory, True)
"#};

fn write_workspace(context: &TestContext) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "source-preparation-root"
            version = "0.1.0"
            requires-python = ">=3.12"

            [tool.uv]
            package = false

            [tool.uv.workspace]
            members = ["packages/*"]
        "#})?;
    Ok(())
}

fn write_package(
    path: &ChildPath,
    name: &str,
    requirements: &[(&str, &ChildPath)],
    dynamic_version: bool,
) -> Result<()> {
    let version = if dynamic_version {
        r#"dynamic = ["version"]"#
    } else {
        r#"version = "0.1.0""#
    };
    let requires = requirements
        .iter()
        .map(|(name, path)| {
            let url = Url::from_directory_path(path.path())
                .map_err(|()| anyhow!("invalid source directory: {}", path.display()))?;
            Ok(format!("{name} @ {url}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let requires = serde_json::to_string(&requires)?;
    path.child("pyproject.toml").write_str(&formatdoc! {r#"
        [project]
        name = "{name}"
        {version}
        requires-python = ">=3.12"

        [build-system]
        requires = {requires}
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    let mut imports = String::new();
    for (name, _) in requirements {
        let module = name.replace('-', "_");
        writeln!(imports, "import {module}\nassert {module}.VALUE == 42")?;
    }
    path.child("backend.py")
        .write_str(&format!("{imports}{BACKEND}"))?;
    path.child(format!("{}.py", name.replace('-', "_")))
        .write_str("VALUE = 42\n")?;
    Ok(())
}

/// Bound liveness regressions without changing the test runner's resource limits.
fn bounded_command(
    context: &TestContext,
    inner: &Command,
    descriptor_limit: Option<u32>,
) -> Command {
    let mut command = context.external_command(&context.python_versions[0].1);
    command
        .arg("-I")
        .arg("-c")
        .arg(indoc! {r"
            import subprocess
            import sys

            if sys.argv[1]:
                import resource
                limit = int(sys.argv[1])
                resource.setrlimit(resource.RLIMIT_NOFILE, (limit, limit))
            result = subprocess.run(sys.argv[2:], timeout=60, check=False)
            raise SystemExit(result.returncode)
        "})
        .arg(descriptor_limit.map_or_else(String::new, |limit| limit.to_string()))
        .arg(inner.get_program())
        .args(inner.get_args());
    for (key, value) in inner.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    if let Some(directory) = inner.get_current_dir() {
        command.current_dir(directory);
    }
    command
}

fn sync(context: &TestContext, builds: usize) -> Command {
    let mut command = context.sync();
    command
        .arg("--offline")
        .arg("--quiet")
        .arg("--all-packages")
        .arg("--no-default-groups")
        .env(EnvVars::UV_CONCURRENT_BUILDS, builds.to_string())
        .env(EnvVars::UV_CONCURRENT_DOWNLOADS, "50");
    command
}

#[cfg(all(unix, feature = "test-slow"))]
#[test]
fn local_source_preparation_low_file_limit() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_workspace(&context)?;
    for number in 0..192 {
        let name = format!("source-preparation-{number:03}");
        write_package(
            &context.temp_dir.child("packages").child(&name),
            &name,
            &[],
            false,
        )?;
    }

    uv_snapshot!(context.filters(), bounded_command(&context, &sync(&context, 1), Some(128)), @"
    exit_code: 0 (success)
    ");
    context
        .assert_command(indoc! {r#"
            import importlib
            for number in range(192):
                assert importlib.import_module(f"source_preparation_{number:03}").VALUE == 42
        "#})
        .success();
    Ok(())
}

#[test]
fn local_source_preparation_nested_builds() -> Result<()> {
    for builds in [1, 2] {
        let context = uv_test::test_context!("3.12");
        write_workspace(&context)?;
        let dependency = context.temp_dir.child("build-dependency");
        write_package(&dependency, "source-preparation-dependency", &[], false)?;
        write_package(
            &context.temp_dir.child("packages").child("outer"),
            "source-preparation-outer",
            &[("source-preparation-dependency", &dependency)],
            false,
        )?;

        allow_duplicates! {
            uv_snapshot!(context.filters(), bounded_command(&context, &sync(&context, builds), None), @"
            exit_code: 0 (success)
            ");
        }
        context
            .assert_command(indoc! {r#"
                import importlib.util
                import source_preparation_outer
                assert source_preparation_outer.VALUE == 42
                assert importlib.util.find_spec("source_preparation_dependency") is None
            "#})
            .success();
    }
    Ok(())
}

#[test]
fn local_source_preparation_unnamed_build() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let dependency = context.temp_dir.child("build-dependency");
    write_package(&dependency, "source-preparation-dependency", &[], false)?;
    let outer = context.temp_dir.child("outer");
    write_package(
        &outer,
        "source-preparation-outer",
        &[("source-preparation-dependency", &dependency)],
        true,
    )?;
    let url = Url::from_directory_path(outer.path())
        .map_err(|()| anyhow!("invalid source directory: {}", outer.display()))?;
    let mut command = context.pip_install();
    command
        .arg("--offline")
        .arg("--quiet")
        .arg("--no-deps")
        .arg(url.as_str())
        .env(EnvVars::UV_CONCURRENT_BUILDS, "1");

    uv_snapshot!(context.filters(), bounded_command(&context, &command, None), @"
    exit_code: 0 (success)
    ");
    context
        .assert_command(
            "import source_preparation_outer; assert source_preparation_outer.VALUE == 42",
        )
        .success();
    Ok(())
}

#[test]
fn local_source_preparation_cycle() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_workspace(&context)?;
    let outer = context.temp_dir.child("packages").child("outer");
    let dependency = context.temp_dir.child("build-dependency");
    write_package(
        &dependency,
        "source-preparation-dependency",
        &[("source-preparation-outer", &outer)],
        false,
    )?;
    write_package(
        &outer,
        "source-preparation-outer",
        &[("source-preparation-dependency", &dependency)],
        false,
    )?;

    uv_snapshot!(context.filters(), bounded_command(&context, &sync(&context, 1), None), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `source-preparation-outer @ file://[TEMP_DIR]/packages/outer`
      cause: Failed to install requirements from `build-system.requires`
      cause: Failed to build `source-preparation-dependency @ file://[TEMP_DIR]/build-dependency`
      cause: Failed to install requirements from `build-system.requires`
      cause: Cyclic build dependency detected for `source-preparation-outer`
    ");
    Ok(())
}

#[test]
fn local_source_preparation_recovers_after_backend_failure() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_workspace(&context)?;
    write_package(
        &context.temp_dir.child("packages").child("outer"),
        "source-preparation-outer",
        &[],
        false,
    )?;
    let marker = context.temp_dir.child("failed-once");
    let mut command = sync(&context, 1);
    command.env("SOURCE_PREPARATION_FAIL_ONCE", marker.path());
    uv_snapshot!(context.filters(), bounded_command(&context, &command, None), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `source-preparation-outer @ file://[TEMP_DIR]/packages/outer`
      cause: The build backend returned an error
      cause: Call to `backend.build_editable` failed (exit status: 1)

             [stderr]
             Traceback (most recent call last):
               File "<string>", line 11, in <module>
               File "[TEMP_DIR]/packages/outer/backend.py", line 66, in build_editable
                 return build(wheel_directory, True)
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^
               File "[TEMP_DIR]/packages/outer/backend.py", line 40, in build
                 raise RuntimeError("intentional source preparation failure")
             RuntimeError: intentional source preparation failure

    hint: Build failures usually indicate a problem with the package or the build environment
    "#);

    uv_snapshot!(context.filters(), bounded_command(&context, &sync(&context, 1), None), @"
    exit_code: 0 (success)
    ");
    context
        .assert_command(
            "import source_preparation_outer; assert source_preparation_outer.VALUE == 42",
        )
        .success();
    Ok(())
}
