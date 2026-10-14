#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::{FileTouch, PathChild};
use assert_fs::{fixture::FileWriteStr, prelude::PathCreateDir};
use indoc::indoc;

use uv_platform::{Arch, Os};
use uv_static::EnvVars;

use uv_test::{uv_snapshot, venv_bin_path};

#[test]
fn python_find_default_arch() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]).with_filtered_python_sources();
    let arch = Arch::from_env().to_string();
    let os = Os::from_env();
    context
        .temp_dir
        .child(".python-version")
        .write_str("3.12\n")?;

    // A version pin uses the architecture specified by the environment variable.
    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::UV_PYTHON_ARCH, "wasm32"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for any-3.12-any-wasm32-any in [PYTHON SOURCES]
    ");

    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::UV_PYTHON_ARCH, &arch), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");
    // An empty value leaves the architecture unrestricted.
    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::UV_PYTHON_ARCH, ""), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");
    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::UV_PYTHON_ARCH, "invalid"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse environment variable `UV_PYTHON_ARCH` with invalid value `invalid`: Unknown architecture: invalid
    ");

    // An architecture-qualified request and an explicit interpreter path take precedence.
    uv_snapshot!(context.filters(), context.python_find()
        .env(EnvVars::UV_PYTHON_ARCH, "wasm32")
        .arg(format!("cpython-3.12-{os}-{arch}")), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");
    let (_, python) = context.python_versions.first().unwrap();
    uv_snapshot!(context.filters(), context.python_find()
        .env(EnvVars::UV_PYTHON_ARCH, "wasm32")
        .arg(python), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    Ok(())
}

#[test]
fn python_default_arch_existing_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]).with_filtered_python_sources();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "example"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    context
        .temp_dir
        .child(".python-version")
        .write_str("3.12\n")?;

    context
        .venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync()
        .arg("--offline")
        .env(EnvVars::UV_PYTHON_ARCH, "wasm32"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for any-3.12-any-wasm32-any in [PYTHON SOURCES]
    ");

    Ok(())
}

/// Workspace discovery warnings should retain the parse diagnostic, unless warnings are disabled.
#[test]
fn python_find_warning_chain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = 42
            version = "0.1.0"
        "#})?;

    // Bypass settings discovery so this exercises the workspace discovery warning.
    uv_snapshot!(context.filters(), context.python_find().arg("--no-config"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]

    ----- stderr -----
    warning: Failed to parse: pyproject.toml
      cause: TOML parse error at line 2, column 8
               |
             2 | name = 42
               |        ^^
             invalid type: integer `42`, expected a string
    ");
    uv_snapshot!(context.filters(), context.python_find().arg("--no-config").arg("--quiet"), @"exit_code: 0 (success)");
    Ok(())
}

#[test]
fn python_find() {
    let mut context =
        uv_test::test_context_with_versions!(&["3.11", "3.12"]).with_filtered_python_sources();

    // No interpreters on the path
    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::UV_PYTHON_SEARCH_PATH, ""), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found in [PYTHON SOURCES]
    ");

    // We find the first interpreter on the path
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    // Request Python 3.12
    uv_snapshot!(context.filters(), context.python_find().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Request Python 3.12
    uv_snapshot!(context.filters(), context.python_find().arg("==3.12.*"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Request Python 3.11
    uv_snapshot!(context.filters(), context.python_find().arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    // Request CPython
    uv_snapshot!(context.filters(), context.python_find().arg("cpython"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    // Request CPython 3.12
    uv_snapshot!(context.filters(), context.python_find().arg("cpython@3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Request CPython 3.12 via partial key syntax
    uv_snapshot!(context.filters(), context.python_find().arg("cpython-3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Request Python 3.12 via partial key syntax with placeholders
    uv_snapshot!(context.filters(), context.python_find().arg("any-3.12-any"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Request CPython 3.12 for the current platform
    let os = Os::from_env();
    let arch = Arch::from_env();

    uv_snapshot!(context.filters(), context.python_find()
        .arg(format!("cpython-3.12-{os}-{arch}")), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Request PyPy (which should be missing)
    uv_snapshot!(context.filters(), context.python_find().arg("pypy"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for PyPy in [PYTHON SOURCES]
    ");

    // Swap the order of the Python versions
    context.python_versions.reverse();

    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Request Python 3.11
    uv_snapshot!(context.filters(), context.python_find().arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");
}

#[test]
fn python_find_skips_download_metadata_when_python_is_found() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let missing_downloads = context.temp_dir.child("missing-downloads.json");

    uv_snapshot!(context.filters(), context
        .python_find()
        .arg("--python-downloads-json-url")
        .arg(missing_downloads.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");
}

#[test]
#[cfg(unix)]
fn python_find_cached_launcher_override() {
    let context = uv_test::test_context!("3.12");
    let other_venv = context.temp_dir.child("other");

    context
        .venv()
        .arg("--python")
        .arg("3.12")
        .arg(other_venv.path())
        .assert()
        .success();

    let requested_python = venv_bin_path(&other_venv).join("python3");
    let override_python = venv_bin_path(&context.venv).join("python3");

    // `PYTHONEXECUTABLE` makes the requested `other` interpreter report `.venv`.
    uv_snapshot!(context.filters(), context.python_find()
        .arg(&requested_python)
        .env(EnvVars::PYTHONEXECUTABLE, &override_python), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/bin/python3
    ");

    // Without `PYTHONEXECUTABLE`, this should return `other`, not the cached `.venv`.
    uv_snapshot!(context.filters(), context.python_find()
        .arg(&requested_python)
        .env_remove(EnvVars::PYTHONEXECUTABLE), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/other/bin/python3
    ");
}

#[test]
fn python_find_pin() {
    let mut context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    // Pin to a version
    uv_snapshot!(context.filters(), context.python_pin().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.12`
    ");

    // We should find the pinned version, not the first on the path
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Unless explicitly requested
    uv_snapshot!(context.filters(), context.python_find().arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    // Or `--no-config` is used
    uv_snapshot!(context.filters(), context.python_find().arg("--no-config"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    let child_dir = context.temp_dir.child("child");
    child_dir.create_dir_all().unwrap();

    // We should also find pinned versions in the parent directory
    uv_snapshot!(context.filters(), context.python_find().current_dir(&child_dir), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    uv_snapshot!(context.filters(), context.python_pin().arg("3.11").current_dir(&child_dir), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.11`
    ");

    // Make both the search path and parent pin prefer 3.12, so only the child pin selects 3.11.
    context.python_versions.reverse();
    uv_snapshot!(context.filters(), context.python_find().current_dir(&child_dir), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");
}

#[test]
fn python_find_pin_arbitrary_name() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    // Try to pin to an arbitrary name
    uv_snapshot!(context.filters(), context.python_pin().arg("foo"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Requests for arbitrary names (e.g., `foo`) are not supported in version files
    ");

    // Pin to an arbitrary name, bypassing uv
    context
        .temp_dir
        .child(".python-version")
        .write_str("foo")
        .unwrap();

    // The arbitrary name should be ignored
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]

    ----- stderr -----
    warning: Ignoring unsupported Python request `foo` in version file: [TEMP_DIR]/.python-version
    ");

    // The pin should be updatable
    uv_snapshot!(context.filters(), context.python_pin().arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.11`

    ----- stderr -----
    warning: Ignoring unsupported Python request `foo` in version file: [TEMP_DIR]/.python-version
    ");

    // Warnings shouldn't appear afterwards...
    uv_snapshot!(context.filters(), context.python_pin().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `3.11` -> `3.12`
    ");

    // Pin in a sub-directory
    context.temp_dir.child("foo").create_dir_all().unwrap();
    context
        .temp_dir
        .child("foo")
        .child(".python-version")
        .write_str("foo")
        .unwrap();

    // The arbitrary name should be ignored, but we won't walk up to the parent `.python-version`
    // file (which contains 3.12); this behavior is a little questionable but we probably want to
    // ignore all empty version files if we want to change this?
    uv_snapshot!(context.filters(), context.python_find().current_dir(context.temp_dir.child("foo").path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]

    ----- stderr -----
    warning: Ignoring unsupported Python request `foo` in version file: [TEMP_DIR]/foo/.python-version
    ");
}

#[test]
fn python_find_project() {
    let context = uv_test::test_context_with_versions!(&["3.10", "3.11", "3.12"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = ["anyio==3.7.0"]
    "#})
        .unwrap();

    // We should respect the project's required version, not the first on the path
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    // Unless explicitly requested
    uv_snapshot!(context.filters(), context.python_find().arg("3.10"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.10]

    ----- stderr -----
    warning: The requested interpreter resolved to Python 3.10.[X], which is incompatible with the project's Python requirement: `>=3.11` (from `project.requires-python`)
    ");

    // Or `--no-project` is used
    uv_snapshot!(context.filters(), context.python_find().arg("--no-project"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.10]
    ");

    // But a pin should take precedence
    uv_snapshot!(context.filters(), context.python_pin().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.12`
    ");
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Create a pin that's incompatible with the project
    uv_snapshot!(context.filters(), context.python_pin().arg("3.10").arg("--no-workspace"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `3.12` -> `3.10`
    ");

    // We should warn on subsequent uses, but respect the pinned version?
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.10]

    ----- stderr -----
    warning: The Python request from `.python-version` resolved to Python 3.10.[X], which is incompatible with the project's Python requirement: `>=3.11` (from `project.requires-python`)
    Use `uv python pin` to update the `.python-version` file to a compatible version
    ");

    // Unless the pin file is outside the project, in which case we should just ignore it
    let child_dir = context.temp_dir.child("child");
    child_dir.create_dir_all().unwrap();

    let pyproject_toml = child_dir.child("pyproject.toml");
    pyproject_toml
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = ["anyio==3.7.0"]
    "#})
        .unwrap();

    uv_snapshot!(context.filters(), context.python_find().current_dir(&child_dir), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");
}

#[test]
fn virtual_empty() -> Result<()> {
    assert_virtual_project_discovery(indoc! {r#"
        [tool.mycooltool]
        wow = "someconfig"
    "#})
}

#[test]
fn virtual_dependency_group() -> Result<()> {
    assert_virtual_project_discovery(indoc! {r#"
        [dependency-groups]
        foo = ["sortedcontainers"]
        bar = ["iniconfig"]
        dev = ["sniffio"]
    "#})
}

/// Exercise default, explicit, and pinned interpreter selection for a virtual project.
fn assert_virtual_project_discovery(pyproject: &str) -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.10", "3.11", "3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(pyproject)?;

    insta::allow_duplicates! {
        uv_snapshot!(context.filters(), context.python_find(), @"
        exit_code: 0 (success)
        ----- stdout -----
        [PYTHON-3.10]
        ");

        uv_snapshot!(context.filters(), context.python_find()
            .arg("--no-project"), @"
        exit_code: 0 (success)
        ----- stdout -----
        [PYTHON-3.10]
        ");

        uv_snapshot!(context.filters(), context.python_find().arg("3.11"), @"
        exit_code: 0 (success)
        ----- stdout -----
        [PYTHON-3.11]
        ");

        // The pin takes precedence over default discovery, but not an explicit request.
        uv_snapshot!(context.filters(), context.python_pin().arg("3.12"), @"
        exit_code: 0 (success)
        ----- stdout -----
        Pinned `.python-version` to `3.12`
        ");

        uv_snapshot!(context.filters(), context.python_find(), @"
        exit_code: 0 (success)
        ----- stdout -----
        [PYTHON-3.12]
        ");

        uv_snapshot!(context.filters(), context.python_find().arg("3.11"), @"
        exit_code: 0 (success)
        ----- stdout -----
        [PYTHON-3.11]
        ");

        // The version pin is still respected with project discovery disabled.
        uv_snapshot!(context.filters(), context.python_find().arg("--no-project"), @"
        exit_code: 0 (success)
        ----- stdout -----
        [PYTHON-3.12]
        ");
    }
    Ok(())
}

/// Read the single absolute interpreter path printed by `uv python find`.
fn interpreter_path_from_output(output: &Output) -> Result<&Path> {
    let stdout = std::str::from_utf8(&output.stdout)?;
    let path = Path::new(
        stdout
            .strip_suffix("\r\n")
            .or_else(|| stdout.strip_suffix('\n'))
            .expect("the interpreter path should end with a newline"),
    );
    assert!(path.is_absolute(), "expected an absolute path: {path:?}");
    Ok(path)
}

/// Assert that discovery returned the Python executable inside the expected environment.
fn assert_venv_python(command: &mut Command, environment: &Path) -> Result<()> {
    let output = command.assert().success().stderr("");
    let actual = interpreter_path_from_output(output.get_output())?;
    let expected =
        venv_bin_path(environment).join(format!("python{}", std::env::consts::EXE_SUFFIX));
    assert!(actual.is_file(), "interpreter does not exist: {actual:?}");
    assert_eq!(actual.file_name(), expected.file_name());
    // Resolve directory aliases without following the Python executable, which may point to a
    // system interpreter shared by multiple environments.
    assert_eq!(
        dunce::canonicalize(actual.parent().expect("the interpreter has a parent"))?,
        dunce::canonicalize(expected.parent().expect("the interpreter has a parent"))?
    );
    Ok(())
}

#[test]
fn python_find_venv() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    // Create a virtual environment
    uv_snapshot!(context.filters(), context.venv().arg("--python").arg("3.12").arg("-q"), @"
    exit_code: 0 (success)
    ");

    // We should find it first.
    assert_venv_python(&mut context.python_find(), context.venv.path())?;

    let child_dir = context.temp_dir.child("child");
    child_dir.create_dir_all()?;

    // Unless the system flag is passed
    uv_snapshot!(context.filters(), context.python_find().arg("--system"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    // Or, `UV_SYSTEM_PYTHON` is set
    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::UV_SYSTEM_PYTHON, "1"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    // Unless, `--no-system` is included
    // TODO(zanieb): Report this as a bug upstream — this should be allowed.
    uv_snapshot!(context.filters(), context.python_find().arg("--no-system").env(EnvVars::UV_SYSTEM_PYTHON, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--no-system' cannot be used with '--system'

    Usage: uv python find --cache-dir [CACHE_DIR] [REQUEST]

    For more information, try '--help'.
    ");

    // We should find virtual environments from a child directory
    assert_venv_python(
        context.python_find().current_dir(&child_dir),
        context.venv.path(),
    )?;

    // A virtual environment in the child directory takes precedence over the parent
    uv_snapshot!(context.filters(), context.venv().arg("--python").arg("3.11").arg("-q").current_dir(&child_dir), @"
    exit_code: 0 (success)
    ");

    let child_venv = child_dir.child(".venv");
    assert_venv_python(
        context.python_find().current_dir(&child_dir),
        child_venv.path(),
    )?;

    // But if we delete the parent virtual environment
    fs_err::remove_dir_all(context.venv.path())?;

    // And query from there... we should not find the child virtual environment
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    // Unless, it is requested by path
    assert_venv_python(context.python_find().arg("child/.venv"), child_venv.path())?;

    // Or activated via `VIRTUAL_ENV`
    assert_venv_python(
        context
            .python_find()
            .env(EnvVars::VIRTUAL_ENV, child_venv.path()),
        child_venv.path(),
    )?;

    // Or at the front of the PATH
    let child_bin = venv_bin_path(child_venv.path());
    assert_venv_python(
        context
            .python_find()
            .env(EnvVars::UV_PYTHON_SEARCH_PATH, &child_bin),
        child_venv.path(),
    )?;

    // This holds even if there are other directories before it in the path, as long as they do
    // not contain a Python executable
    let path = std::env::join_paths([context.temp_dir.path(), child_bin.as_path()])?;
    assert_venv_python(
        context
            .python_find()
            .env(EnvVars::UV_PYTHON_SEARCH_PATH, path),
        child_venv.path(),
    )?;

    // But, if there's an executable _before_ the virtual environment — we prefer that
    let path = std::env::join_paths(
        std::env::split_paths(&context.python_path()).chain(std::iter::once(child_bin)),
    )?;
    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::UV_PYTHON_SEARCH_PATH, path), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    Ok(())
}

#[test]
#[cfg(unix)]
fn python_find_venv_executable_precedence() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let python = context.interpreter();
    let python3 = python.with_file_name("python3");

    // Prefer `python` when discovering an environment or requesting it by directory.
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/bin/python
    ");

    uv_snapshot!(context.filters(), context.python_find().arg(context.venv.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/bin/python
    ");

    uv_snapshot!(context.filters(), context.python_find()
        .env(EnvVars::VIRTUAL_ENV, context.venv.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/bin/python
    ");

    // An explicit executable path is still used as given.
    uv_snapshot!(context.filters(), context.python_find().arg(&python3), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/bin/python3
    ");

    // Discover environments containing only `python`.
    fs_err::remove_file(&python3)?;
    fs_err::remove_file(python.with_file_name("python3.12"))?;
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/bin/python
    ");
    uv_snapshot!(context.filters(), context.python_find().arg(context.venv.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/bin/python
    ");

    // Discover environments containing only `python3`.
    fs_err::os::unix::fs::symlink(fs_err::canonicalize(&python)?, &python3)?;
    fs_err::remove_file(&python)?;
    uv_snapshot!(context.filters(), context.python_find().arg(context.venv.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/bin/python3
    ");
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/bin/python3
    ");

    Ok(())
}

#[test]
fn python_find_unsupported_version() {
    let context = uv_test::test_context_with_versions!(&["3.12"]).with_filtered_python_sources();

    // Request a low version
    uv_snapshot!(context.filters(), context.python_find().arg("3.5"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid version request: Python <3.6 is not supported but 3.5 was requested.
    ");

    // Request a low version with a patch
    uv_snapshot!(context.filters(), context.python_find().arg("3.5.9"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid version request: Python <3.6 is not supported but 3.5.9 was requested.
    ");

    // Request a really low version
    uv_snapshot!(context.filters(), context.python_find().arg("2.6"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid version request: Python <3.6 is not supported but 2.6 was requested.
    ");

    // Request a really low version with a patch
    uv_snapshot!(context.filters(), context.python_find().arg("2.6.8"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid version request: Python <3.6 is not supported but 2.6.8 was requested.
    ");

    // Request a future version
    uv_snapshot!(context.filters(), context.python_find().arg("4.2"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 4.2 in [PYTHON SOURCES]
    ");

    // Request a low version with a range
    uv_snapshot!(context.filters(), context.python_find().arg("<3.0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python <3.0 in [PYTHON SOURCES]
    ");

    // Request free-threaded Python on unsupported version
    uv_snapshot!(context.filters(), context.python_find().arg("3.12t"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid version request: Python <3.13 does not support free-threading but 3.12+freethreaded was requested.
    ");
}

#[test]
fn python_find_venv_invalid() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    // We find the virtual environment
    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::VIRTUAL_ENV, context.venv.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/[PYTHON]
    ");

    // If the binaries are missing from a virtual environment, we fail
    fs_err::remove_dir_all(venv_bin_path(&context.venv)).unwrap();

    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::VIRTUAL_ENV, context.venv.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from active virtual environment at `.venv/[BIN]/[PYTHON]`
      cause: Python interpreter not found at `[VENV]/[BIN]/[PYTHON]`
    ");

    // Unless the virtual environment is not active
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // If there's not a `pyvenv.cfg` file, it's also non-fatal, we ignore the environment
    fs_err::remove_file(context.venv.join("pyvenv.cfg")).unwrap();

    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::VIRTUAL_ENV, context.venv.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");
}

#[test]
fn python_find_managed() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_python_sources()
        .with_versions_as_managed(&["3.12"]);

    // We find the managed interpreter
    uv_snapshot!(context.filters(), context.python_find().arg("--managed-python"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Request an interpreter that cannot be satisfied
    uv_snapshot!(context.filters(), context.python_find().arg("--managed-python").arg("3.11"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.11 in virtual environments or managed installations
    ");

    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_python_sources()
        .with_versions_as_managed(&["3.11"]);

    // We find the unmanaged interpreter with managed Python disabled
    uv_snapshot!(context.filters(), context.python_find().arg("--no-managed-python"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // Request an interpreter that cannot be satisfied
    uv_snapshot!(context.filters(), context.python_find().arg("--no-managed-python").arg("3.11"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.11 in [PYTHON SOURCES]
    ");

    // We find the unmanaged interpreter with system Python preferred
    uv_snapshot!(context.filters(), context.python_find().arg("--python-preference").arg("system"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // But, if no system Python meets the request, we'll use the managed interpreter
    uv_snapshot!(context.filters(), context.python_find().arg("--python-preference").arg("system").arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");
}

/// See: <https://github.com/astral-sh/uv/issues/11825>
#[test]
#[cfg(unix)]
#[cfg(feature = "test-python-managed")]
fn python_required_python_major_minor() -> Result<()> {
    let context = context_with_only_versioned_python("3.11")?;
    let path = &context.python_versions[0].1;

    assert_discovered_interpreter(context.python_find().arg(">=3.11.4, <3.12"), path)?.stderr("");
    assert_discovered_interpreter(context.python_find().arg(">3.11.4, <3.12"), path)?.stderr("");

    // Fail to find any matching Python interpreter.
    uv_snapshot!(context.filters(), context.python_find().arg(">3.11.255, <3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python >3.11.[X], <3.12 in virtual environments, managed installations, or search path
    ");

    Ok(())
}

#[test]
fn python_find_script() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names()
        .with_filtered_exe_suffix();

    uv_snapshot!(context.filters(), context.init().arg("--script").arg("foo.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Initialized script at `foo.py`
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--script").arg("foo.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Creating script environment at: [CACHE_DIR]/environments-v2/foo-[HASH]
    Resolved in [TIME]
    Checked in [TIME]
    ");

    let output = uv_snapshot!(context.filters(), context.python_find().arg("--script").arg("foo.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/foo-[HASH]/[BIN]/[PYTHON]
    ");

    context
        .external_command(interpreter_path_from_output(&output)?)
        .args([
            "-I",
            "-c",
            "import sys; assert sys.prefix != sys.base_prefix; print(f'{sys.version_info.major}.{sys.version_info.minor}', end='')",
        ])
        .assert()
        .success()
        .stdout("3.13");
    Ok(())
}

#[test]
fn python_find_script_no_environment() {
    let context = uv_test::test_context!("3.13")
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names()
        .with_filtered_exe_suffix();

    let script = context.temp_dir.child("foo.py");

    script
        .write_str(indoc! {r"
            # /// script
            # dependencies = []
            # ///
        "})
        .unwrap();

    uv_snapshot!(context.filters(), context.python_find().arg("--script").arg("foo.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/[PYTHON]
    ");
}

#[test]
fn python_find_script_python_not_found() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_env(EnvVars::UV_PYTHON_PREFERENCE, "only-managed")
        .with_env(EnvVars::UV_INTERNAL__TEST_PYTHON_MANAGED, "");
    context.temp_dir.child(".python-version").touch().unwrap();

    let script = context.temp_dir.child("foo.py");

    script
        .write_str(indoc! {r"
            # /// script
            # dependencies = []
            # ///
        "})
        .unwrap();

    uv_snapshot!(context.filters(), context.python_find().arg("--script").arg("foo.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No interpreter found in virtual environments or managed installations

    hint: A managed Python download is available, but Python downloads are set to 'never'
    ");
}

#[cfg(unix)]
#[test]
fn python_find_script_interpreter_error_chain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context.temp_dir.child("foo.py").write_str(indoc! {r"
        # /// script
        # dependencies = []
        # ///
    "})?;
    let python = context.temp_dir.child("broken-python");
    python.write_str(indoc! {r"
        #!/bin/sh
        printf 'interpreter probe failed\n' >&2
        exit 42
    "})?;
    fs_err::set_permissions(&python, std::fs::Permissions::from_mode(0o755))?;
    context
        .temp_dir
        .child(".python-version")
        .write_str(python.path().to_str().expect("UTF-8 fixture path"))?;
    uv_snapshot!(context.filters(), context.python_find().arg("--script").arg("foo.py").arg("-q"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from provided path at `broken-python`
      cause: Querying Python at `[TEMP_DIR]/broken-python` failed with exit status exit status: 42

             [stderr]
             interpreter probe failed
    ");
    Ok(())
}

#[test]
fn python_find_script_no_such_version() {
    let context = uv_test::test_context!("3.13")
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names()
        .with_filtered_exe_suffix()
        .with_filtered_python_sources();
    let script = context.temp_dir.child("foo.py");
    script
        .write_str(indoc! {r#"
            # /// script
            # requires-python = ">=3.13"
            # dependencies = []
            # ///
        "#})
        .unwrap();

    uv_snapshot!(context.filters(), context.sync().arg("--script").arg("foo.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Creating script environment at: [CACHE_DIR]/environments-v2/foo-[HASH]
    Resolved in [TIME]
    Checked in [TIME]
    ");

    script
        .write_str(indoc! {r#"
            # /// script
            # requires-python = ">=3.16"
            # dependencies = []
            # ///
        "#})
        .unwrap();

    uv_snapshot!(context.filters(), context.python_find().arg("--script").arg("foo.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No interpreter found for Python >=3.16 in [PYTHON SOURCES]
    ");
}

#[test]
fn python_find_show_version() {
    let context =
        uv_test::test_context_with_versions!(&["3.11", "3.12"]).with_filtered_python_sources();

    // No interpreters found
    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::UV_PYTHON_SEARCH_PATH, "").arg("--show-version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found in [PYTHON SOURCES]
    ");

    // Show the first version found
    uv_snapshot!(context.filters(), context.python_find().arg("--show-version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.11.[X]
    ");

    // Request Python 3.12
    uv_snapshot!(context.filters(), context.python_find().arg("--show-version").arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12.[X]
    ");

    // Request Python 3.11
    uv_snapshot!(context.filters(), context.python_find().arg("--show-version").arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.11.[X]
    ");
}

#[test]
fn python_find_path() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_not_executable();

    context.temp_dir.child("foo").create_dir_all().unwrap();
    context.temp_dir.child("bar").touch().unwrap();

    // No interpreter in a directory
    uv_snapshot!(context.filters(), context.python_find().arg(context.temp_dir.child("foo").as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found in directory `foo`
    ");

    // No interpreter at a file
    uv_snapshot!(context.filters(), context.python_find().arg(context.temp_dir.child("bar").as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from provided path at `bar`
      cause: Failed to query Python interpreter at `[TEMP_DIR]/bar`
      cause: [PERMISSION DENIED]
    ");

    // No interpreter at a file that does not exist
    uv_snapshot!(context.filters(), context.python_find().arg(context.temp_dir.child("foobar").as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found at path `foobar`
    ");
}

#[test]
#[cfg(feature = "test-python-managed")]
fn python_find_freethreaded_313() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_python_sources()
        .with_managed_python_dirs()
        .with_filtered_python_install_bin()
        .with_filtered_python_names()
        .with_filtered_exe_suffix();

    context
        .python_install()
        .arg("--preview")
        .arg("3.13t")
        .assert()
        .success();

    // Request Python 3.13 (without opt-in)
    uv_snapshot!(context.filters(), context.python_find().arg("3.13"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.13 in [PYTHON SOURCES]
    ");

    // Request Python 3.13t (with explicit opt-in)
    uv_snapshot!(context.filters(), context.python_find().arg("3.13t"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13+freethreaded-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");
}

#[test]
#[cfg(feature = "test-python-managed")]
fn python_find_freethreaded_314() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_python_sources()
        .with_managed_python_dirs()
        .with_filtered_python_install_bin()
        .with_filtered_python_names()
        .with_filtered_exe_suffix();

    context
        .python_install()
        .arg("--preview")
        .arg("3.14t")
        .assert()
        .success();

    // Request Python 3.14 (without opt-in)
    uv_snapshot!(context.filters(), context.python_find().arg("3.14"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.14+freethreaded-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // Request Python 3.14t (with explicit opt-in)
    uv_snapshot!(context.filters(), context.python_find().arg("3.14t"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.14+freethreaded-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // Request Python 3.14+gil
    uv_snapshot!(context.filters(), context.python_find().arg("3.14+gil"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.14+gil in [PYTHON SOURCES]
    ");

    // Install the non-freethreaded version
    context
        .python_install()
        .arg("--preview")
        .arg("3.14")
        .assert()
        .success();

    // Request Python 3.14
    uv_snapshot!(context.filters(), context.python_find().arg("3.14"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.14-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // Request Python 3.14+gil
    uv_snapshot!(context.filters(), context.python_find().arg("3.14+gil"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.14-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");
}

#[test]
#[cfg(feature = "test-python-managed")]
#[cfg(unix)]
fn python_find_version_range_installation_key_order() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_latest_python_versions()
        .with_filtered_python_sources()
        .with_managed_python_dirs()
        .with_filtered_python_install_bin()
        .with_filtered_exe_suffix();

    context.python_install().arg("3.15t").assert().success();
    context.python_install().arg("3.15").assert().success();

    // Managed installations have their own deterministic ordering.
    uv_snapshot!(context.filters(), context.python_find().arg("==3.15.*"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.15-[PLATFORM]/[INSTALL-BIN]/python3.15
    ");

    // Equally preferred search-path executables use the ordering of their queried installation
    // keys, rather than the order returned by the filesystem.
    uv_snapshot!(context.filters(), context.python_find()
        .arg("==3.15.*")
        .arg("--python-preference")
        .arg("system")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, context.bin_dir.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [BIN]/python3.15
    ");

    // Listing installed interpreters exercises the parallel query strategy within the same group.
    uv_snapshot!(context.filters(), context.python_list()
        .arg("==3.15.*")
        .arg("--only-installed")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, context.bin_dir.path())
        .env(EnvVars::UV_PYTHON_INSTALL_DIR, context.temp_dir.child("missing-managed").path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.15.[LATEST]-[PLATFORM]                 [BIN]/python3.15 -> managed/cpython-3.15-[PLATFORM]/[INSTALL-BIN]/python3.15
    cpython-3.15.[LATEST]+freethreaded-[PLATFORM]    [BIN]/python3.15t -> managed/cpython-3.15+freethreaded-[PLATFORM]/[INSTALL-BIN]/python3.15t
    ");

    // Interpreter metadata, rather than the executable name, determines whether a build is
    // free-threaded.
    let misleading_names = context.temp_dir.child("misleading-names");
    misleading_names.create_dir_all().unwrap();
    fs_err::os::unix::fs::symlink(
        context.bin_dir.path().join("python3.15t"),
        misleading_names.join("python3.15"),
    )
    .unwrap();
    fs_err::os::unix::fs::symlink(
        context.bin_dir.path().join("python3.15"),
        misleading_names.join("python3.15t"),
    )
    .unwrap();

    uv_snapshot!(context.filters(), context.python_find()
        .arg("==3.15.*")
        .arg("--python-preference")
        .arg("system")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, misleading_names.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/misleading-names/python3.15t
    ");

    context.python_install().arg("3.14").assert().success();

    // Preferred executable names must remain ahead of newer minor-version fallback candidates.
    let preferred_names = context.temp_dir.child("preferred-names");
    preferred_names.create_dir_all().unwrap();
    fs_err::os::unix::fs::symlink(
        context.bin_dir.path().join("python3.14"),
        preferred_names.join("python3"),
    )
    .unwrap();
    fs_err::os::unix::fs::symlink(
        context.bin_dir.path().join("python3.15"),
        preferred_names.join("python3.15"),
    )
    .unwrap();
    fs_err::os::unix::fs::symlink(
        context.bin_dir.path().join("python3.15t"),
        preferred_names.join("python3.15t"),
    )
    .unwrap();

    uv_snapshot!(context.filters(), context.python_find()
        .arg(">=3.14,<3.16")
        .arg("--python-preference")
        .arg("system")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, preferred_names.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/preferred-names/python3
    ");

    // Installation-key ordering must not override the order of directories on the search path.
    let first_directory = context.temp_dir.child("first");
    first_directory.create_dir_all().unwrap();
    fs_err::os::unix::fs::symlink(
        context.bin_dir.path().join("python3.15t"),
        first_directory.join("python3.15t"),
    )
    .unwrap();

    let second_directory = context.temp_dir.child("second");
    second_directory.create_dir_all().unwrap();
    fs_err::os::unix::fs::symlink(
        context.bin_dir.path().join("python3.15"),
        second_directory.join("python3.15"),
    )
    .unwrap();

    let search_path = std::env::join_paths([first_directory.path(), second_directory.path()])
        .expect("The test directories should form a valid search path");

    uv_snapshot!(context.filters(), context.python_find()
        .arg("==3.15.*")
        .arg("--python-preference")
        .arg("system")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, search_path), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/first/python3.15t
    ");
}

#[test]
#[cfg(feature = "test-python-managed")]
fn python_find_prerelease_version_specifiers() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_python_sources()
        .with_managed_python_dirs()
        .with_filtered_python_install_bin()
        .with_filtered_python_names()
        .with_filtered_exe_suffix();

    context.python_install().arg("3.13.0rc2").assert().success();
    context.python_install().arg("3.13.0rc3").assert().success();

    // `>=3.13` should allow pre-release versions
    uv_snapshot!(context.filters(), context.python_find().arg(">=3.13").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0rc3-[PLATFORM]/[INSTALL-BIN]/[PYTHON]

    ----- stderr -----
    warning: You're using a pre-release version of Python (3.13.0rc3) but a stable version is available. Use `uv python upgrade 3.13` to upgrade.
    ");

    // `>3.13.0rc2` should not match rc2
    uv_snapshot!(context.filters(), context.python_find().arg(">3.13.0rc2").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0rc3-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // `>3.13.0rc3` should not match rc3
    uv_snapshot!(context.filters(), context.python_find().arg(">3.13.0rc3"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python >3.13.0rc3 in [PYTHON SOURCES]
    ");

    // `>=3.13.0rc3` should match rc3
    uv_snapshot!(context.filters(), context.python_find().arg(">=3.13.0rc3").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0rc3-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // `<3.13.0rc3` should match rc2
    uv_snapshot!(context.filters(), context.python_find().arg("<3.13.0rc3").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0rc2-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // `<=3.13.0rc3` should match rc3
    uv_snapshot!(context.filters(), context.python_find().arg("<=3.13.0rc3").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0rc3-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // Install the stable version
    context.python_install().arg("3.13.0").assert().success();

    // `>=3.13` should prefer stable
    uv_snapshot!(context.filters(), context.python_find().arg(">=3.13").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // `>3.13.0rc2` should prefer stable
    uv_snapshot!(context.filters(), context.python_find().arg(">3.13.0rc2").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");
}

#[test]
#[cfg(feature = "test-python-managed")]
fn python_find_prerelease_with_patch_request() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_python_sources()
        .with_managed_python_dirs()
        .with_filtered_python_install_bin()
        .with_filtered_python_names()
        .with_filtered_exe_suffix();

    // Install 3.13.0rc3
    context.python_install().arg("3.13.0rc3").assert().success();

    // When no `.0` patch version is included, we'll allow selection of a pre-release
    uv_snapshot!(context.filters(), context.python_find().arg("3.13"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13-[PLATFORM]/[INSTALL-BIN]/[PYTHON]

    ----- stderr -----
    warning: You're using a pre-release version of Python (3.13.0rc3) but a stable version is available. Use `uv python upgrade 3.13` to upgrade.
    ");

    // When `.0` is explicitly included, we will require a stable release
    uv_snapshot!(context.filters(), context.python_find().arg("3.13.0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.13.0 in [PYTHON SOURCES]
    ");

    // Install 3.13.0 stable
    context.python_install().arg("3.13.0").assert().success();

    uv_snapshot!(context.filters(), context.python_find().arg("3.13.0"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");
}

#[test]
fn python_find_equal() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    uv_snapshot!(context.filters(), context.python_find().arg("==3.11"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    "###);

    uv_snapshot!(context.filters(), context.python_find().arg("==3.12"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    "###);
}

/// Test that `UV_PYTHON_SEARCH_PATH` overrides `PATH` for Python discovery.
#[test]
fn python_find_search_path() {
    let context =
        uv_test::test_context_with_versions!(&["3.11", "3.12"]).with_filtered_python_sources();
    let search_path = context.python_path();
    let mut directories = std::env::split_paths(&search_path);
    let python_3_11 = directories.next().expect("Python 3.11 search directory");
    let python_3_12 = directories.next().expect("Python 3.12 search directory");

    // An empty search path suppresses the interpreter on `PATH`.
    uv_snapshot!(context.filters(), context.python_find()
        .env(EnvVars::PATH, &python_3_11)
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, ""), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found in [PYTHON SOURCES]
    ");

    // The default search path follows the context's interpreter order.
    uv_snapshot!(context.filters(), context.python_find(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.11]
    ");

    // An explicit search path takes precedence over `PATH`.
    uv_snapshot!(context.filters(), context.python_find()
        .env(EnvVars::PATH, &python_3_11)
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, &python_3_12), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");

    // We can use `UV_PYTHON_SEARCH_PATH` to control the order of Python versions
    let reversed_path =
        std::env::join_paths([python_3_12, python_3_11]).expect("valid Python search path");
    uv_snapshot!(context.filters(), context.python_find().env(EnvVars::UV_PYTHON_SEARCH_PATH, reversed_path.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");
}

#[cfg(unix)]
fn context_with_only_versioned_python(version: &str) -> Result<uv_test::TestContext> {
    let context = uv_test::test_context_with_versions!(&[version]);
    let search_dir = context.temp_dir.child("child");
    search_dir.create_dir_all()?;
    fs_err::os::unix::fs::symlink(
        &context.python_versions[0].1,
        search_dir.child(format!("python{version}")),
    )?;
    Ok(context
        .with_env(EnvVars::PATH, search_dir.as_os_str())
        .with_env(EnvVars::UV_PYTHON_SEARCH_PATH, search_dir.as_os_str()))
}

/// Create a project whose search path contains only a versioned Python executable.
///
/// Regression fixture for <https://github.com/astral-sh/uv/issues/9695>.
#[cfg(unix)]
fn project_with_only_versioned_python(requires_python: &str) -> Result<uv_test::TestContext> {
    let context = context_with_only_versioned_python("3.12")?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&indoc::formatdoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = "{requires_python}"
        "#})?;
    Ok(context)
}

/// Compare interpreter identity because some Python installations report a canonical
/// `sys.executable` instead of the discovered symlink.
#[cfg(unix)]
fn assert_discovered_interpreter(
    command: &mut Command,
    expected: &Path,
) -> Result<assert_cmd::assert::Assert> {
    let output = command.assert().success();
    let found = interpreter_path_from_output(output.get_output())?;
    assert_eq!(
        fs_err::canonicalize(found)?,
        fs_err::canonicalize(expected)?
    );
    Ok(output)
}

#[test]
#[cfg(unix)]
fn python_find_versioned_executable_exact_minor() -> Result<()> {
    let context = project_with_only_versioned_python("==3.12")?;

    let output =
        assert_discovered_interpreter(&mut context.python_find(), &context.python_versions[0].1)?;
    let stderr = std::str::from_utf8(&output.get_output().stderr)?;
    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_snapshot!(stderr, @"warning: The resolved Python interpreter (Python 3.12.[X]) is incompatible with the project's Python requirement: `==3.12` (from `project.requires-python`)");
    });
    Ok(())
}

#[test]
#[cfg(unix)]
fn python_find_versioned_executable_wildcard_minor() -> Result<()> {
    let context = project_with_only_versioned_python("==3.12.*")?;

    assert_discovered_interpreter(&mut context.python_find(), &context.python_versions[0].1)?
        .stderr("");
    Ok(())
}

#[test]
#[cfg(unix)]
fn python_find_versioned_executable_compatible_release() -> Result<()> {
    let context = project_with_only_versioned_python("~=3.12.0")?;

    assert_discovered_interpreter(&mut context.python_find(), &context.python_versions[0].1)?
        .stderr("");
    Ok(())
}
