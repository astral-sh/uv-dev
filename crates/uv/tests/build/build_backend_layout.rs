use anyhow::Result;
#[cfg(feature = "test-python")]
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{ChildPath, FileWriteStr, PathChild, PathCreateDir};
use indoc::indoc;
use uv_test::{TestContext, uv_snapshot};

const EXPLICIT_MODULE_WHEEL: &str = "django_plugin-1.0.0-py3-none-any.whl";

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
    context.assert_path_missing("foo-1.0.0.tar.gz");
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
    context.assert_path_missing("foo-1.0.0.tar.gz");
    Ok(())
}

fn write_project_with_explicit_module(context: &TestContext) -> Result<ChildPath> {
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "django-plugin"
        version = "1.0.0"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.build-backend]
        module-name = "Django_plugin"
    "#})?;
    Ok(project)
}

#[test]
fn wheel_requires_explicit_module() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let project = write_project_with_explicit_module(&context)?;
    project.child("src").create_dir_all()?;

    uv_snapshot!(context.build_backend()
        .arg("build-wheel")
        .arg(context.temp_dir.path())
        .current_dir(project.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Expected a Python module at: src/Django_plugin/__init__.py
    ");
    context.assert_path_missing(EXPLICIT_MODULE_WHEEL);
    Ok(())
}

#[test]
fn wheel_requires_explicit_module_initializer() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let project = write_project_with_explicit_module(&context)?;
    project.child("src/Django_plugin").create_dir_all()?;

    uv_snapshot!(context.build_backend()
        .arg("build-wheel")
        .arg(context.temp_dir.path())
        .current_dir(project.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Expected a Python module at: src/Django_plugin/__init__.py
    ");
    context.assert_path_missing(EXPLICIT_MODULE_WHEEL);
    Ok(())
}

#[test]
#[cfg(feature = "test-python")]
fn wheel_selects_explicit_module_case() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = write_project_with_explicit_module(&context)?;
    project
        .child("src/Django_plugin/__init__.py")
        .write_str("VALUE = 'selected module'\n")?;
    // On a case-sensitive filesystem, the normalized project name can coexist with the explicit module.
    let normalized_module = project.child("src/django_plugin/__init__.py");
    if !normalized_module.path().exists() {
        normalized_module.write_str("VALUE = 'normalized project name'\n")?;
    }

    uv_snapshot!(context.build_backend()
        .arg("build-wheel")
        .arg(context.temp_dir.path())
        .current_dir(project.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    django_plugin-1.0.0-py3-none-any.whl
    ");
    context
        .pip_install()
        .arg(context.temp_dir.child(EXPLICIT_MODULE_WHEEL).path())
        .args(["--no-deps", "--no-index"])
        .assert()
        .success();
    uv_snapshot!(context.python_command()
        .args(["-I", "-c", "import Django_plugin; print(Django_plugin.VALUE)"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    selected module
    ");
    Ok(())
}
