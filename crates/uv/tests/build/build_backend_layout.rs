use anyhow::Result;
use assert_fs::fixture::{ChildPath, FileWriteStr, PathChild, PathCreateDir};
use indoc::indoc;
use uv_test::{TestContext, assert_path_missing, uv_snapshot};

fn write_project(context: &TestContext) -> Result<ChildPath> {
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "foo"
        version = "1.0.0"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    Ok(project)
}

#[test]
fn sdist_requires_source_directory() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let project = write_project(&context)?;

    uv_snapshot!(context.build_backend()
        .arg("build-sdist")
        .arg(context.temp_dir.path())
        .current_dir(project.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Expected a Python module at: src/foo/__init__.py
    ");
    assert_path_missing(context.temp_dir.child("foo-1.0.0.tar.gz"));
    Ok(())
}

#[test]
fn sdist_requires_module_in_source_directory() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let project = write_project(&context)?;
    project.child("src").create_dir_all()?;

    uv_snapshot!(context.build_backend()
        .arg("build-sdist")
        .arg(context.temp_dir.path())
        .current_dir(project.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Expected a Python module at: src/foo/__init__.py
    ");
    assert_path_missing(context.temp_dir.child("foo-1.0.0.tar.gz"));
    Ok(())
}
