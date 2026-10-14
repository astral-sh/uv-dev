#[cfg(unix)]
use std::fs::Permissions;
#[cfg(all(unix, feature = "test-python"))]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(all(unix, feature = "test-python"))]
use std::os::unix::net::UnixListener;
#[cfg(all(unix, feature = "test-python"))]
use std::path::PathBuf;

use anyhow::Result;
#[cfg(all(unix, feature = "test-python"))]
use assert_fs::fixture::SymlinkToFile;
use assert_fs::fixture::{FileWriteStr, PathChild, PathCreateDir};
#[cfg(unix)]
use indoc::formatdoc;
use indoc::indoc;
use insta::assert_snapshot;
use uv_python_discovery::{PYTHON_VERSION_FILENAME, PYTHON_VERSIONS_FILENAME};
use uv_test::{assert_path_missing, uv_snapshot};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[cfg(feature = "test-python")]
use super::fixtures::interpreter_platform;

/// Pinning through a symlink must not replace a special file at its target.
#[cfg(all(unix, feature = "test-python"))]
#[test]
fn python_pin_symlink_to_special_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let target = context.temp_dir.child("socket");
    let _listener = UnixListener::bind(target.path())?;
    let version_file = context.temp_dir.child(PYTHON_VERSION_FILENAME);
    version_file.symlink_to_file("socket")?;

    uv_snapshot!(context.filters(), context.python_pin().arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot atomically replace `[TEMP_DIR]/socket`: not a regular file
    ");

    assert_eq!(
        fs_err::read_link(version_file.path())?,
        PathBuf::from("socket")
    );
    assert!(fs_err::metadata(target.path())?.file_type().is_socket());
    Ok(())
}

/// Repeated pins should warn once while retaining the failed interpreter's status and stderr.
#[cfg(unix)]
#[test]
fn python_pin_warning_chain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.12"
        "#})?;
    let python = context.temp_dir.child("broken-python");
    python.write_str(indoc! {r"
        #!/bin/sh
        printf 'interpreter query failed\n' >&2
        exit 42
    "})?;
    fs_err::set_permissions(&python, Permissions::from_mode(0o755))?;
    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str(&formatdoc! {"
            {0}
            {0}
        ", python.path().display()})?;

    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/broken-python
    [TEMP_DIR]/broken-python

    ----- stderr -----
    warning: Failed to resolve pinned Python version `[TEMP_DIR]/broken-python`
      cause: Failed to inspect Python interpreter from provided path at `broken-python`
      cause: Querying Python at `[TEMP_DIR]/broken-python` failed with exit status exit status: 42

             [stderr]
             interpreter query failed
    ");
    Ok(())
}

#[test]
#[cfg(feature = "test-python")]
fn python_pin() {
    let context =
        uv_test::test_context_with_versions!(&["3.11", "3.12"]).with_filtered_python_sources();

    // Without arguments, we attempt to read the current pin (which does not exist yet)
    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No Python version file found; specify a version to create one
    ");

    // Given an argument, we pin to that version
    uv_snapshot!(context.filters(), context.python_pin().arg("any"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `any`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    assert_snapshot!(python_version, @"any");

    // Without arguments, we read the current pin
    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 0 (success)
    ----- stdout -----
    any
    ");

    // Request Python 3.12
    uv_snapshot!(context.filters(), context.python_pin().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `any` -> `3.12`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    assert_snapshot!(python_version, @"3.12");

    // Request Python 3.11
    uv_snapshot!(context.filters(), context.python_pin().arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `3.12` -> `3.11`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    assert_snapshot!(python_version, @"3.11");

    // Request CPython
    uv_snapshot!(context.filters(), context.python_pin().arg("cpython"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `3.11` -> `cpython`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    assert_snapshot!(python_version, @"cpython");

    // Request CPython 3.12
    uv_snapshot!(context.filters(), context.python_pin().arg("cpython@3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `cpython` -> `cpython@3.12`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    assert_snapshot!(python_version, @"cpython@3.12");

    // Request CPython 3.12 via non-canonical syntax
    uv_snapshot!(context.filters(), context.python_pin().arg("cp3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `cpython@3.12`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    assert_snapshot!(python_version, @"cpython@3.12");

    // Request CPython 3.12 via partial key syntax
    uv_snapshot!(context.filters(), context.python_pin().arg("cpython-3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `cpython@3.12` -> `cpython-3.12-any-any-any`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    assert_snapshot!(python_version, @"cpython-3.12-any-any-any");

    // Request a specific path
    uv_snapshot!(context.filters(), context.python_pin().arg(&context.python_versions.first().unwrap().1), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `cpython-3.12-any-any-any` -> `[PYTHON-3.11]`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"[PYTHON-3.11]");
    });

    // A missing implementation can still be pinned without resolving it.
    uv_snapshot!(context.filters(), context.python_pin().arg("pypy"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `[PYTHON-3.11]` -> `pypy`

    ----- stderr -----
    warning: No interpreter found for PyPy in [PYTHON SOURCES]
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    assert_snapshot!(python_version, @"pypy");

    // Request a version that is not installed.
    uv_snapshot!(context.filters(), context.python_pin().arg("3.7"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `pypy` -> `3.7`

    ----- stderr -----
    warning: No interpreter found for Python 3.7 in [PYTHON SOURCES]
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    assert_snapshot!(python_version, @"3.7");
}

async fn empty_download_catalog_requested_once() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn python_pin_uses_python_downloads_json_url() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();
    let server = empty_download_catalog_requested_once().await;

    uv_snapshot!(context.filters(), context
        .python_pin()
        .arg("--resolved")
        .arg("3.12")
        .arg("--python-downloads-json-url")
        .arg(server.uri()), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]
    ");
}

#[tokio::test]
#[cfg(feature = "test-python")]
async fn python_pin_downloads_metadata_once_for_multiple_pins() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []
        "#,
    )?;

    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str(indoc! {"
            3.11
            3.12
        "})?;

    let server = empty_download_catalog_requested_once().await;

    uv_snapshot!(context.filters(), context
        .python_pin()
        .arg("--python-downloads-json-url")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.11
    3.12
    ");

    Ok(())
}

/// A global pin is readable without a local pin or an installed interpreter.
#[test]
fn python_pin_global_if_no_local() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .user_config_dir
        .child("uv")
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.11\n")?;

    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.11
    ");
    assert_path_missing(context.temp_dir.child(PYTHON_VERSION_FILENAME));

    Ok(())
}

// If there is a project-level `.python-version` file, it takes precedence over
// the global pin.
#[test]
#[cfg(feature = "test-python")]
fn python_pin_global_use_local_if_available() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    let uv = context.user_config_dir.child("uv");
    uv.create_dir_all()?;

    // Given an argument, we globally pin to that version
    uv_snapshot!(context.filters(), context.python_pin().arg("3.12").arg("--global"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `[UV_USER_CONFIG_DIR]/.python-version` to `3.12`
    ");

    let global_version_file = uv.child(PYTHON_VERSION_FILENAME);
    assert_eq!(context.read(&global_version_file), "3.12\n");

    // Request Python 3.11 for local .python-version
    uv_snapshot!(context.filters(), context.python_pin().arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.11`
    ");

    // Local should override global
    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.11
    ");

    // We should still be able to check global pin
    uv_snapshot!(context.filters(), context.python_pin().arg("--global"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12
    ");

    assert_eq!(context.read(PYTHON_VERSION_FILENAME), "3.11\n");
    assert_eq!(context.read(&global_version_file), "3.12\n");

    Ok(())
}

#[test]
#[cfg(feature = "test-python")]
fn python_pin_global_creates_parent_dirs() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let uv_global_config_dir = context.user_config_dir.child("uv");

    assert_path_missing(&uv_global_config_dir);

    uv_snapshot!(context.filters(), context.python_pin().arg("3.12").arg("--global"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `[UV_USER_CONFIG_DIR]/.python-version` to `3.12`
    ");

    assert_eq!(
        context.read(uv_global_config_dir.child(PYTHON_VERSION_FILENAME)),
        "3.12\n"
    );
    assert_path_missing(context.temp_dir.child(PYTHON_VERSION_FILENAME));
}

/// We do not need a Python interpreter to pin without `--resolved`.
#[test]
fn python_pin_no_python() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    uv_snapshot!(context.filters(), context.python_pin().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.12`

    ----- stderr -----
    warning: No interpreter found for Python 3.12 in [PYTHON SOURCES]
    ");
    assert_eq!(context.read(PYTHON_VERSION_FILENAME), "3.12\n");
}

#[test]
#[cfg(feature = "test-python")]
fn python_pin_compatible_with_requires_python() -> Result<()> {
    let context =
        uv_test::test_context_with_versions!(&["3.10", "3.11"]).with_filtered_python_sources();
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = ["iniconfig"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.python_pin().arg("3.10"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The requested Python version `3.10` is incompatible with the project `requires-python` value of `>=3.11`.
    ");
    let version_file = context.temp_dir.child(PYTHON_VERSION_FILENAME);
    assert_path_missing(&version_file);

    // Rejecting an incompatible implementation request must also preserve an existing pin.
    version_file.write_str("3.11\n")?;
    uv_snapshot!(context.filters(), context.python_pin().arg("cpython@3.10"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The requested Python version `cpython@3.10` is incompatible with the project `requires-python` value of `>=3.11`.
    ");
    assert_eq!(context.read(&version_file), "3.11\n");

    // Request an incompatible version with project discovery turned off
    uv_snapshot!(context.filters(), context.python_pin().arg("cpython@3.10").arg("--no-project"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `3.11` -> `cpython@3.10`
    ");
    assert_eq!(context.read(&version_file), "cpython@3.10\n");

    // Request a complex version range that resolves to an incompatible version
    uv_snapshot!(context.filters(), context.python_pin().arg(">3.8,<3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `cpython@3.10` -> `>3.8, <3.11`

    ----- stderr -----
    warning: The requested Python version `>3.8, <3.11` resolves to `3.10.[X]` which  is incompatible with the project `requires-python` value of `>=3.11`.
    ");

    // Request a version that is compatible
    uv_snapshot!(context.filters(), context.python_pin().arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `>3.8, <3.11` -> `3.11`
    ");

    // Request a version that is compatible and uses a Python variant
    uv_snapshot!(context.filters(), context.python_pin().arg("3.13t"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `3.11` -> `3.13+freethreaded`

    ----- stderr -----
    warning: No interpreter found for Python 3.13+freethreaded in [PYTHON SOURCES]
    ");

    // Request a implementation version that is compatible
    uv_snapshot!(context.filters(), context.python_pin().arg("cpython@3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `3.13+freethreaded` -> `cpython@3.11`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"cpython@3.11");
    });

    // Updating `requires-python` should affect `uv python pin` compatibilities.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython@3.11

    ----- stderr -----
    warning: The pinned Python version `cpython@3.11` is incompatible with the project `requires-python` value of `>=3.12`.
    ");

    // Request a implementation that resolves to a compatible version
    uv_snapshot!(context.filters(), context.python_pin().arg("cpython"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `cpython@3.11` -> `cpython`

    ----- stderr -----
    warning: The requested Python version `cpython` resolves to `3.10.[X]` which  is incompatible with the project `requires-python` value of `>=3.12`.
    ");

    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython

    ----- stderr -----
    warning: The pinned Python version `cpython` resolves to `3.10.[X]` which  is incompatible with the project `requires-python` value of `>=3.12`.
    ");

    // Request a complex version range that resolves to a compatible version
    uv_snapshot!(context.filters(), context.python_pin().arg(">3.8,<3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `cpython` -> `>3.8, <3.12`

    ----- stderr -----
    warning: The requested Python version `>3.8, <3.12` resolves to `3.10.[X]` which  is incompatible with the project `requires-python` value of `>=3.12`.
    ");

    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 0 (success)
    ----- stdout -----
    >3.8, <3.12

    ----- stderr -----
    warning: The pinned Python version `>3.8, <3.12` resolves to `3.10.[X]` which  is incompatible with the project `requires-python` value of `>=3.12`.
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-python")]
fn warning_pinned_python_version_not_installed() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.10", "3.11"]);
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = ["iniconfig"]
        "#,
    )?;

    let python_version_file = context.temp_dir.child(PYTHON_VERSION_FILENAME);
    python_version_file.write_str(r"3.12")?;
    if cfg!(windows) {
        uv_snapshot!(context.filters(), context.python_pin(), @r###"
        exit_code: 0 (success)
        ----- stdout -----
        3.12

        ----- stderr -----
        warning: Failed to resolve pinned Python version `3.12`
          cause: No interpreter found for Python 3.12 in managed installations, search path, or registry
        "###);
    } else {
        uv_snapshot!(context.filters(), context.python_pin(), @"
        exit_code: 0 (success)
        ----- stdout -----
        3.12

        ----- stderr -----
        warning: Failed to resolve pinned Python version `3.12`
          cause: No interpreter found for Python 3.12 in managed installations or search path
        ");
    }

    Ok(())
}

/// We do need a Python interpreter for `--resolved` pins
#[test]
fn python_pin_resolve_no_python() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();
    uv_snapshot!(context.filters(), context.python_pin().arg("--resolved").arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]

    hint: A managed Python download is available for Python 3.12, but Python downloads are set to 'never'
    ");
    assert_path_missing(context.temp_dir.child(PYTHON_VERSION_FILENAME));
}

#[test]
#[cfg(feature = "test-python")]
fn python_pin_resolve() -> Result<()> {
    let context =
        uv_test::test_context_with_versions!(&["3.12", "3.13"]).with_filtered_python_sources();

    // We pin the first interpreter on the path
    uv_snapshot!(context.filters(), context.python_pin().arg("--resolved").arg("any"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `[PYTHON-3.12]`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"[PYTHON-3.12]");
    });

    // Request Python 3.13
    uv_snapshot!(context.filters(), context.python_pin().arg("--resolved").arg("3.13"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `[PYTHON-3.12]` -> `[PYTHON-3.13]`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"[PYTHON-3.13]");
    });

    // Request Python 3.13
    uv_snapshot!(context.filters(), context.python_pin().arg("--resolved").arg("3.13"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `[PYTHON-3.13]`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"[PYTHON-3.13]");
    });

    // Request CPython
    uv_snapshot!(context.filters(), context.python_pin().arg("--resolved").arg("cpython"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `[PYTHON-3.13]` -> `[PYTHON-3.12]`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"[PYTHON-3.12]");
    });

    // Request CPython 3.13
    uv_snapshot!(context.filters(), context.python_pin().arg("--resolved").arg("cpython@3.13"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Updated `.python-version` from `[PYTHON-3.12]` -> `[PYTHON-3.13]`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"[PYTHON-3.13]");
    });

    // Request CPython 3.13 via partial key syntax
    uv_snapshot!(context.filters(), context.python_pin().arg("--resolved").arg("cpython-3.13"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `[PYTHON-3.13]`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"[PYTHON-3.13]");
    });

    // Request CPython 3.13 for the fixture interpreter's platform.
    let (_, python) = &context.python_versions[1];
    let (os, arch) = interpreter_platform(&context, python)?;
    uv_snapshot!(context.filters(), context.python_pin().arg("--resolved")
    .arg(format!("cpython-3.13-{os}-{arch}"))
    , @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `[PYTHON-3.13]`
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"[PYTHON-3.13]");
    });

    // Missing PyPy also reports a download hint on platforms where one is available.
    #[cfg(unix)]
    {
        uv_snapshot!(context.filters(), context.python_pin().arg("--resolved").arg("pypy"), @"
        exit_code: 2 (failure)
        ----- stderr -----
        error: No interpreter found for PyPy in [PYTHON SOURCES]

        hint: A managed Python download is available for PyPy, but Python downloads are set to 'never'
        ");

        let python_version = context.read(PYTHON_VERSION_FILENAME);
        insta::with_settings!({
            filters => context.filters(),
        }, {
            assert_snapshot!(python_version, @"[PYTHON-3.13]");
        });
    }

    // A failed resolved request must leave the existing pin unchanged.
    uv_snapshot!(context.filters(), context.python_pin().arg("--resolved").arg("3.7"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.7 in [PYTHON SOURCES]
    ");

    let python_version = context.read(PYTHON_VERSION_FILENAME);
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(python_version, @"[PYTHON-3.13]");
    });
    Ok(())
}

#[test]
fn python_pin_with_comments_and_whitespace() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let content = "  python3.12  \n \t\n  # 3.11\n\tpython3.10\t\n";

    let version_file = context.temp_dir.child(PYTHON_VERSION_FILENAME);
    version_file.write_str(content)?;
    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12
    3.10
    ");
    assert_eq!(context.read(&version_file), content);
    fs_err::remove_file(version_file)?;

    let versions_file = context.temp_dir.child(PYTHON_VERSIONS_FILENAME);
    versions_file.write_str(content)?;
    uv_snapshot!(context.filters(), context.python_pin(), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12
    3.10
    ");
    assert_eq!(context.read(&versions_file), content);

    Ok(())
}

#[test]
#[cfg(feature = "test-python-managed")]
fn python_pin_install() {
    use assert_cmd::assert::OutputAssertExt;
    use uv_static::EnvVars;

    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    // Should not install 3.12 when downloads are not automatic
    uv_snapshot!(context.filters(), context.python_pin().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.12`

    ----- stderr -----
    warning: No interpreter found for Python 3.12 in [PYTHON SOURCES]
    ");

    uv_snapshot!(context.filters(), context.python_pin().arg("3.12").env(EnvVars::UV_PYTHON_DOWNLOADS, "auto"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.12`
    ");

    // Pinning can succeed after a download error, so verify the interpreter was installed.
    context
        .python_find()
        .args(["3.12", "--managed-python"])
        .assert()
        .success();
}

#[test]
fn python_pin_rm() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let local_version = context.temp_dir.child(PYTHON_VERSION_FILENAME);
    let global_version = context
        .user_config_dir
        .child("uv")
        .child(PYTHON_VERSION_FILENAME);

    uv_snapshot!(context.filters(), context.python_pin().arg("--rm"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No Python version file found
    ");

    local_version.write_str("3.11\n")?;
    uv_snapshot!(context.filters(), context.python_pin().arg("--rm"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Removed Python version file at `.python-version`
    ");
    assert_path_missing(&local_version);

    uv_snapshot!(context.filters(), context.python_pin().arg("--rm").arg("--global"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No global Python pin found
    ");

    // Global removal does not detect a local pin.
    local_version.write_str("3.11\n")?;
    uv_snapshot!(context.filters(), context.python_pin().arg("--rm").arg("--global"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No global Python pin found
    ");

    global_version.write_str("3.12\n")?;

    uv_snapshot!(context.filters(), context.python_pin().arg("--rm").arg("--global"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Removed global Python pin at `[UV_USER_CONFIG_DIR]/.python-version`
    ");
    assert_path_missing(&global_version);
    assert_eq!(context.read(&local_version), "3.11\n");

    global_version.write_str("3.12\n")?;

    // Remove the local pin
    uv_snapshot!(context.filters(), context.python_pin().arg("--rm"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Removed Python version file at `.python-version`
    ");
    assert_path_missing(&local_version);
    assert_eq!(context.read(&global_version), "3.12\n");

    // The global pin should not be removed without `--global`
    uv_snapshot!(context.filters(), context.python_pin().arg("--rm"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No Python version file found; use `--rm --global` to remove the global pin
    ");
    Ok(())
}

#[test]
fn python_pin_rm_versions() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let global = context.user_config_dir.child("uv");
    global.create_dir_all()?;
    let global_versions = global.child(PYTHON_VERSIONS_FILENAME);
    global_versions.write_str(indoc! {"
        3.11
        3.12
    "})?;

    let local_versions = context.temp_dir.child(PYTHON_VERSIONS_FILENAME);
    local_versions.write_str("3.12\n")?;

    uv_snapshot!(context.filters(), context.python_pin().arg("--rm"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Removed Python version file at `.python-versions`
    ");
    assert_path_missing(&local_versions);

    uv_snapshot!(context.filters(), context.python_pin().arg("--rm"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No Python version file found; use `--rm --global` to remove the global pin
    ");
    assert_snapshot!(context.read(&global_versions), @"
    3.11
    3.12
    ");

    uv_snapshot!(context.filters(), context.python_pin().arg("--rm").arg("--global"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Removed global Python pin at `[UV_USER_CONFIG_DIR]/.python-versions`
    ");
    assert_path_missing(&global_versions);

    Ok(())
}
