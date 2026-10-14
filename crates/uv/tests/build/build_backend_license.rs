use anyhow::Result;
use assert_fs::fixture::{FileTouch, FileWriteBin, FileWriteStr, PathChild};
use indoc::indoc;
use uv_test::uv_snapshot;

#[test]
fn license_glob_without_matches_errors() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let project = context.temp_dir.child("missing-license");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "missing-license"
        version = "1.0.0"
        license-files = ["abc", "LICENSE.txt"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    project.child("src/missing_license/__init__.py").touch()?;
    project
        .child("LICENSE.txt")
        .write_str("permissive license")?;

    uv_snapshot!(context
        .build_backend()
        .arg("build-wheel")
        .arg(context.temp_dir.path())
        .current_dir(project.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid project metadata
      cause: `project.license-files` glob `abc` did not match any files
    ");
    context.assert_path_missing("missing_license-1.0.0-py3-none-any.whl");
    Ok(())
}

#[test]
fn license_file_must_be_utf8() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let project = context.temp_dir.child("license-utf8");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "license-utf8"
        version = "1.0.0"
        license-files = ["LICENSE.bin"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    project.child("src/license_utf8/__init__.py").touch()?;
    project.child("LICENSE.bin").write_binary(&[0xff])?;

    uv_snapshot!(context
        .build_backend()
        .arg("build-wheel")
        .arg(context.temp_dir.path())
        .current_dir(project.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid project metadata
      cause: License file `LICENSE.bin` must be UTF-8 encoded
    ");
    context.assert_path_missing("license_utf8-1.0.0-py3-none-any.whl");
    Ok(())
}
