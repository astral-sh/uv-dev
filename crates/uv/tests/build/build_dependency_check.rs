//! Build dependency checks using local backends and offline resolution.

use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use predicates::prelude::predicate;
use uv_test::{TestContext, assert_path_missing, uv_snapshot};

fn project_with_missing_backend(context: &TestContext) -> Result<ChildPath> {
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["missing-backend>=1"]
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    project.child("backend.py").write_str(indoc! {r#"
        raise RuntimeError("backend must not be imported")
    "#})?;
    Ok(project)
}

#[test]
fn build_dependency_check_missing_declared_requirement() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = project_with_missing_backend(&context)?;
    uv_snapshot!(context.filters(), context.build().args([
        "--preview-features", "build-dependency-check", "--no-build-isolation",
    ]).arg("--offline").current_dir(&project), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Building source distribution...
    error: Failed to build `[TEMP_DIR]/project`
      cause: Build requirement is not satisfied: `missing-backend>=1`
    ");
    assert_path_missing(project.child("dist/project-0.1.0.tar.gz"));
    assert_path_missing(project.child("dist/project-0.1.0-py3-none-any.whl"));
    Ok(())
}

#[test]
fn build_dependency_check_bundled_backend() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build=={}"]
        build-backend = "uv_build"
    "#, uv_version::version()})?;
    project.child("src/project/__init__.py").touch()?;
    uv_snapshot!(context.filters(), context.build().args([
        "--preview-features", "build-dependency-check", "--no-build-isolation",
    ]).arg("--offline").current_dir(&project), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    Successfully built dist/project-0.1.0.tar.gz
    Successfully built dist/project-0.1.0-py3-none-any.whl
    ");
    project
        .child("dist/project-0.1.0.tar.gz")
        .assert(predicate::path::exists());
    project
        .child("dist/project-0.1.0-py3-none-any.whl")
        .assert(predicate::path::exists());
    Ok(())
}

#[test]
fn build_dependency_check_preview_and_skip_dependency_check() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["missing-backend>=1"]
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    project.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path
        import zipfile

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            filename = "project-0.1.0-py3-none-any.whl"
            with zipfile.ZipFile(Path(wheel_directory) / filename, "w") as wheel:
                wheel.writestr("project-0.1.0.dist-info/METADATA", "Metadata-Version: 2.3\nName: project\nVersion: 0.1.0\n")
                wheel.writestr("project-0.1.0.dist-info/WHEEL", "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n")
                wheel.writestr("project-0.1.0.dist-info/RECORD", "")
            return filename

        def get_requires_for_build_wheel(config_settings=None):
            raise RuntimeError("dependency hook must not be called")
    "#})?;
    // Dependency checks require the preview feature.
    uv_snapshot!(context.filters(), context.build().args(["--wheel", "--no-build-isolation", "--offline"])
        .current_dir(&project), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building wheel...
    Successfully built dist/project-0.1.0-py3-none-any.whl
    ");
    // Skip static and dynamic checks even when the preview is enabled.
    uv_snapshot!(context.filters(), context.build().args([
        "--preview-features", "build-dependency-check", "--no-build-isolation",
    ]).args(["--wheel", "--offline", "--skip-dependency-check"])
        .current_dir(&project), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building wheel...
    Successfully built dist/project-0.1.0-py3-none-any.whl
    ");
    Ok(())
}

#[test]
fn build_dependency_check_checks_extracted_sdist() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    project.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path
        import tarfile

        def get_requires_for_build_sdist(config_settings=None):
            return []

        def get_requires_for_build_wheel(config_settings=None):
            # Only the extracted sdist has a backend that reports this requirement.
            return ["missing-in-sdist>=1"] if Path("PKG-INFO").exists() else []

        def build_sdist(sdist_directory, config_settings=None):
            import io
            filename = "project-0.1.0.tar.gz"
            with tarfile.open(Path(sdist_directory) / filename, "w:gz") as sdist:
                for name in ["pyproject.toml", "backend.py"]:
                    sdist.add(name, arcname=f"project-0.1.0/{name}")
                content = b"Metadata-Version: 2.3\nName: project\nVersion: 0.1.0\n"
                info = tarfile.TarInfo("project-0.1.0/PKG-INFO")
                info.size = len(content)
                sdist.addfile(info, io.BytesIO(content))
            return filename
    "#})?;
    uv_snapshot!(context.filters(), context.build().args([
        "--preview-features", "build-dependency-check", "--no-build-isolation",
    ]).arg("--offline").current_dir(&project), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    error: Failed to build `[TEMP_DIR]/project`
      cause: Build requirement is not satisfied: `missing-in-sdist>=1`
    ");
    project
        .child("dist/project-0.1.0.tar.gz")
        .assert(predicate::path::exists());
    project
        .child("dist/project-0.1.0-py3-none-any.whl")
        .assert(predicate::path::missing());
    Ok(())
}

#[test]
fn build_dependency_check_package_specific_isolation() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = project_with_missing_backend(&context)?;
    uv_snapshot!(context.filters(), context.build().args([
        "--wheel", "--offline", "--preview-features", "build-dependency-check",
        "--no-build-isolation-package", "project",
    ]).current_dir(&project), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Building wheel...
    error: Failed to build `[TEMP_DIR]/project`
      cause: Build requirement is not satisfied: `missing-backend>=1`
    ");
    assert_path_missing(project.child("dist/project-0.1.0-py3-none-any.whl"));
    Ok(())
}

#[test]
fn build_dependency_check_isolated_build_calls_dependency_hook_once() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    project.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path
        import zipfile

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            filename = "project-0.1.0-py3-none-any.whl"
            with zipfile.ZipFile(Path(wheel_directory) / filename, "w") as wheel:
                wheel.writestr("project-0.1.0.dist-info/METADATA", "Metadata-Version: 2.3\nName: project\nVersion: 0.1.0\n")
                wheel.writestr("project-0.1.0.dist-info/WHEEL", "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n")
                wheel.writestr("project-0.1.0.dist-info/RECORD", "")
            return filename

        def get_requires_for_build_wheel(config_settings=None):
            import sys
            if sys.prefix == sys.base_prefix:
                raise RuntimeError("build hook did not run in an isolated environment")
            with Path("hook-called").open("x"):
                pass
            return []
    "#})?;
    uv_snapshot!(context.filters(), context.build().args([
        "--wheel", "--offline", "--preview-features", "build-dependency-check",
    ]).current_dir(&project), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building wheel...
    Successfully built dist/project-0.1.0-py3-none-any.whl
    ");
    // A package-specific exemption for another package must still use an isolated build.
    fs_err::remove_file(project.child("hook-called"))?;
    uv_snapshot!(context.filters(), context.build().args([
        "--wheel", "--offline", "--preview-features", "build-dependency-check",
        "--no-build-isolation-package", "other-project",
    ]).current_dir(&project), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building wheel...
    Successfully built dist/project-0.1.0-py3-none-any.whl
    ");
    project
        .child("hook-called")
        .assert(predicate::path::is_file());
    Ok(())
}
