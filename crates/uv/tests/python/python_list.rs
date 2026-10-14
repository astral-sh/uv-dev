#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use uv_python_managed::platform_key_from_env;
use uv_static::EnvVars;

use anyhow::Result;
#[cfg(any(feature = "test-python", feature = "test-python-managed"))]
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use uv_test::uv_snapshot;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[cfg(feature = "test-python")]
use super::fixtures::interpreter_platform;

#[test]
fn python_list_default_arch() {
    let context = uv_test::test_context_with_versions!(&[]).with_collapsed_whitespace();

    uv_snapshot!(context.filters(), context.python_list()
        .arg("cpython-3.14.0-windows-any-none")
        .arg("--only-downloads")
        .env(EnvVars::UV_PYTHON_ARCH, "x86_64"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.14.0-windows-x86_64-none <download available>
    ");

    uv_snapshot!(context.filters(), context.python_list()
        .arg("cpython-3.14.0-windows-aarch64-none")
        .arg("--only-downloads")
        .env(EnvVars::UV_PYTHON_ARCH, "x86_64"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.14.0-windows-aarch64-none <download available>
    ");

    // Windows PyPy has only an x86-64 build, so the output order is host-independent.
    uv_snapshot!(context.filters(), context.python_list()
        .arg("pypy-3.11.15-windows-any-none")
        .arg("--only-downloads")
        .env(EnvVars::UV_PYTHON_ARCH, "aarch64"), @"
    exit_code: 0 (success)
    ");

    uv_snapshot!(context.filters(), context.python_list()
        .arg("pypy-3.11.15-windows-any-none")
        .arg("--only-downloads")
        .arg("--all-arches")
        .env(EnvVars::UV_PYTHON_ARCH, "aarch64"), @"
    exit_code: 0 (success)
    ----- stdout -----
    pypy-3.11.15-windows-x86_64-none <download available>
    ");
}

#[test]
#[cfg(feature = "test-python")]
fn python_list() -> Result<()> {
    let mut context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_python_symlinks()
        .with_filtered_python_keys()
        .with_collapsed_whitespace();

    uv_snapshot!(context.filters(), context.python_list().env(EnvVars::UV_PYTHON_SEARCH_PATH, ""), @"
    exit_code: 0 (success)
    ");

    // We show all interpreters
    uv_snapshot!(context.filters(), context.python_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");

    // Request Python 3.12
    uv_snapshot!(context.filters(), context.python_list().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    ");

    // Request Python 3.11
    uv_snapshot!(context.filters(), context.python_list().arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");

    // Request CPython
    uv_snapshot!(context.filters(), context.python_list().arg("cpython"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");

    // Request CPython 3.12
    uv_snapshot!(context.filters(), context.python_list().arg("cpython@3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    ");

    // Request CPython 3.12 via partial key syntax
    uv_snapshot!(context.filters(), context.python_list().arg("cpython-3.12"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    ");

    // Request CPython 3.12 for the fixture interpreter's platform.
    let (_, python) = &context.python_versions[1];
    let (os, arch) = interpreter_platform(&context, python)?;
    uv_snapshot!(context.filters(), context.python_list().arg(format!("cpython-3.12-{os}-{arch}")), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    ");

    // Request PyPy (which should be missing)
    uv_snapshot!(context.filters(), context.python_list().arg("pypy"), @"
    exit_code: 0 (success)
    ");

    // Swap the order of the Python versions
    context.python_versions.reverse();

    uv_snapshot!(context.filters(), context.python_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");

    // Request Python 3.11
    uv_snapshot!(context.filters(), context.python_list().arg("3.11"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");
    Ok(())
}

#[test]
fn python_list_unknown_managed_implementation() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_managed_python_dirs()
        .with_env(EnvVars::RUST_LOG, "uv_python_managed=warn");
    let installation = context
        .temp_dir
        .join("managed")
        .join(format!("unknown-3.12.0-{}", platform_key_from_env()?));
    fs_err::create_dir_all(&installation)?;

    uv_snapshot!(context.filters(), context.python_list()
        .arg("--only-installed"), @"
    exit_code: 0 (success)
    ----- stderr -----
    WARN Ignoring malformed managed Python entry:
        Unknown Python implementation `unknown`
    ");

    uv_snapshot!(context.filters(), context.python_upgrade().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    WARN Ignoring malformed managed Python entry:
        Unknown Python implementation `unknown`
    There are no installed versions to upgrade
    ");

    Ok(())
}

#[cfg(unix)]
#[test]
fn python_list_warns_on_noncritical_explicit_path_errors() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let contents = r"#!/bin/sh
    echo 'error: intentionally broken python executable' >&2
    exit 1";

    let python = context.temp_dir.join("python");
    fs_err::write(&python, contents)?;
    let mut permissions = fs_err::metadata(&python)?.permissions();
    permissions.set_mode(0o755);
    fs_err::set_permissions(&python, permissions)?;

    uv_snapshot!(context.filters(), context.python_list()
        .arg(&python)
        .arg("--only-installed"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Failed to inspect Python interpreter from provided path at `python`
      cause: Querying Python at `[TEMP_DIR]/python` failed with exit status exit status: 1

             [stderr]
             error: intentionally broken python executable
    ");

    let environment = context.temp_dir.join("environment");
    let environment_bin = environment.join("bin");
    let environment_python = environment_bin.join("python");
    fs_err::create_dir_all(&environment_bin)?;
    fs_err::copy(&python, &environment_python)?;

    uv_snapshot!(context.filters(), context.python_list()
        .arg(&environment)
        .arg("--only-installed"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Failed to inspect Python interpreter from provided path at `environment`
      cause: Querying Python at `[TEMP_DIR]/environment/bin/python` failed with exit status exit status: 1

             [stderr]
             error: intentionally broken python executable
    ");

    Ok(())
}

#[cfg(all(target_os = "macos", feature = "test-python"))]
#[test]
fn python_list_warns_on_non_native_search_path_interpreters() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"])
        .with_filtered_python_symlinks()
        .with_filtered_python_keys()
        .with_collapsed_whitespace();

    let foreign_bin = context.temp_dir.join("foreign-bin");
    fs_err::create_dir_all(&foreign_bin)?;

    // A 64-bit PowerPC Mach-O cannot execute on supported macOS architectures.
    let foreign_python = foreign_bin.join("python");
    fs_err::write(
        &foreign_python,
        [
            0xcf, 0xfa, 0xed, 0xfe, 0x12, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ],
    )?;
    let mut permissions = fs_err::metadata(&foreign_python)?.permissions();
    permissions.set_mode(0o755);
    fs_err::set_permissions(&foreign_python, permissions)?;

    let python_search_path = std::env::join_paths(
        std::iter::once(foreign_bin).chain(std::env::split_paths(&context.python_path())),
    )?;

    uv_snapshot!(context.filters(), context.python_list()
        .arg("--only-installed")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, &python_search_path), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]

    ----- stderr -----
    warning: Failed to inspect Python interpreter from first executable in the search path at `foreign-bin/python`
     cause: Failed to query Python interpreter at `[TEMP_DIR]/foreign-bin/python`
     cause: Bad CPU type in executable (os error 86)
    ");

    uv_snapshot!(context.filters(), context.python_list()
        .arg("--only-installed")
        .arg("--quiet")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, &python_search_path), @"
    exit_code: 0 (success)
    ");

    uv_snapshot!(context.filters(), context.python_list()
        .arg(&foreign_python)
        .arg("--only-installed"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Failed to inspect Python interpreter from provided path at `foreign-bin/python`
     cause: Failed to query Python interpreter at `[TEMP_DIR]/foreign-bin/python`
     cause: Bad CPU type in executable (os error 86)
    ");

    uv_snapshot!(context.filters(), context.python_find()
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, &python_search_path), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from first executable in the search path at `foreign-bin/python`
     cause: Failed to query Python interpreter at `[TEMP_DIR]/foreign-bin/python`
     cause: Bad CPU type in executable (os error 86)
    ");

    uv_snapshot!(context.filters(), context.python_find()
        .arg(&foreign_python), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from provided path at `foreign-bin/python`
     cause: Failed to query Python interpreter at `[TEMP_DIR]/foreign-bin/python`
     cause: Bad CPU type in executable (os error 86)
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-python")]
fn python_list_pin() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_python_symlinks()
        .with_filtered_python_keys()
        .with_collapsed_whitespace();

    // Pin the older interpreter so accidentally prioritizing it would change the listing order.
    context
        .temp_dir
        .child(".python-version")
        .write_str("3.11\n")?;

    // The pin should not affect the listing
    uv_snapshot!(context.filters(), context.python_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");

    // So `--no-config` has no effect
    uv_snapshot!(context.filters(), context.python_list().arg("--no-config"), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-python")]
fn python_list_venv() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_python_symlinks()
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_collapsed_whitespace();

    context.venv().args(["--python", "3.12"]).assert().success();
    // Otherwise, both omission checks could pass because the environment is unusable.
    context
        .external_command(context.interpreter())
        .args([
            "-I",
            "-c",
            "import sys; print(sys.prefix != sys.base_prefix, end='')",
        ])
        .assert()
        .success()
        .stdout("True");

    // An active virtual environment is omitted from the listing.
    uv_snapshot!(context.filters(), context.python_list().env(EnvVars::VIRTUAL_ENV, context.venv.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");

    // A discovered but inactive virtual environment is also omitted.
    uv_snapshot!(context.filters(), context.python_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");
}

#[test]
#[cfg(feature = "test-python")]
fn python_list_no_matching_version() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    uv_snapshot!(context.filters(), context.python_list().arg("4.2"), @"exit_code: 0 (success)");
}

#[test]
#[cfg(feature = "test-python")]
fn python_list_range_below_supported_versions() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    uv_snapshot!(context.filters(), context.python_list().arg("<3.0"), @"exit_code: 0 (success)");
}

#[test]
#[cfg(feature = "test-python")]
fn python_list_duplicate_path_entries() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_python_symlinks()
        .with_filtered_python_keys()
        .with_collapsed_whitespace();

    let directories = std::env::split_paths(&context.python_path()).collect::<Vec<_>>();
    // Duplicate every interpreter search directory.
    let path = std::env::join_paths(directories.iter().chain(&directories)).unwrap();

    uv_snapshot!(context.filters(), context.python_list().env(EnvVars::UV_PYTHON_SEARCH_PATH, &path), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
    ");

    #[cfg(unix)]
    {
        let links = directories
            .iter()
            .map(|directory| {
                let mut link = directory.as_os_str().to_owned();
                link.push("-link");
                let link = std::path::PathBuf::from(link);
                fs_err::os::unix::fs::symlink(directory, &link).unwrap();
                link
            })
            .collect::<Vec<_>>();
        let path = std::env::join_paths(directories.iter().chain(&links)).unwrap();

        uv_snapshot!(context.filters(), context.python_list().env(EnvVars::UV_PYTHON_SEARCH_PATH, &path), @"
        exit_code: 0 (success)
        ----- stdout -----
        cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
        cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]
        ");

        // Put the aliases first without also changing the order of Python versions.
        let path = std::env::join_paths(links.iter().chain(&directories)).unwrap();

        uv_snapshot!(context.filters(), context.python_list().env(EnvVars::UV_PYTHON_SEARCH_PATH, &path), @"
        exit_code: 0 (success)
        ----- stdout -----
        cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]-link/python3
        cpython-3.11.[X]-[PLATFORM] [PYTHON-3.11]-link/python3
        ");
    }
}

#[test]
fn python_list_downloads() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_latest_python_versions();

    // These implementations are available on Windows x86-64 even when the host platform differs.
    uv_snapshot!(context.filters(), context.python_list()
        .arg("any-3.10-windows-x86_64-none")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.10.[LATEST]-windows-x86_64-none    <download available>
    pypy-3.10.16-windows-x86_64-none       <download available>
    graalpy-3.10.0-windows-x86_64-none     <download available>
    ");

    // Bound the request so newly released patches do not change the expected historical list.
    uv_snapshot!(context.filters(), context.python_list()
        .arg("any->=3.10.16,<=3.10.18-windows-x86_64-none")
        .arg("--all-versions")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.10.18-windows-x86_64-none    <download available>
    cpython-3.10.17-windows-x86_64-none    <download available>
    cpython-3.10.16-windows-x86_64-none    <download available>
    pypy-3.10.16-windows-x86_64-none       <download available>
    ");
}

#[test]
#[cfg(feature = "test-python-managed")]
fn python_list_downloads_installed() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_python_install_bin()
        .with_filtered_python_names()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    // Keep a second CPython minor uninstalled so unrelated downloads remain visible without
    // depending on PyPy or GraalPy availability for the host platform.
    let request = "cpython->=3.10,<3.12";

    // First, the download is shown as available
    uv_snapshot!(context.filters(), context.python_list().arg(request).env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.11.[LATEST]-[PLATFORM]    <download available>
    cpython-3.10.[LATEST]-[PLATFORM]    <download available>
    ");

    // TODO(zanieb): It'd be nice to test `--show-urls` here too but we need special filtering for
    // the URL

    // But not if `--only-installed` is used
    uv_snapshot!(context.filters(), context.python_list().arg(request).arg("--only-installed").env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ");

    // Install a Python version
    context.python_install().arg("3.10").assert().success();

    // Then, it should be listed as installed instead of available
    uv_snapshot!(context.filters(), context.python_list().arg(request).env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.11.[LATEST]-[PLATFORM]    <download available>
    cpython-3.10.[LATEST]-[PLATFORM]    managed/cpython-3.10-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // But, the display should be reverted if `--only-downloads` is used
    uv_snapshot!(context.filters(), context.python_list().arg(request).arg("--only-downloads").env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.11.[LATEST]-[PLATFORM]    <download available>
    cpython-3.10.[LATEST]-[PLATFORM]    <download available>
    ");

    // And should not be shown if `--no-managed-python` is used
    uv_snapshot!(context.filters(), context.python_list().arg(request).arg("--no-managed-python").env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ");

    // When `--managed-python` is used, managed installations should still be shown
    uv_snapshot!(context.filters(), context.python_list().arg(request).arg("--managed-python").env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.11.[LATEST]-[PLATFORM]    <download available>
    cpython-3.10.[LATEST]-[PLATFORM]    managed/cpython-3.10-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");
}

/// Test that symlinks installed by `python install` on the search path are correctly
/// filtered by `--managed-python` and `--no-managed-python`.
#[test]
#[cfg(all(unix, feature = "test-python-managed"))]
fn python_list_managed_symlinks() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_python_install_bin()
        .with_filtered_python_names()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    // Install a Python version; this creates a symlink in `bin_dir` (on the search path)
    context.python_install().arg("3.10").assert().success();

    // Include `bin_dir` in the test search path so the symlink is discoverable
    let bin_dir = context.bin_dir.to_path_buf();

    // With `--no-managed-python`, the symlink should be excluded since it points to a
    // managed installation
    uv_snapshot!(context.filters(), context.python_list()
        .arg("3.10")
        .arg("--only-installed")
        .arg("--no-managed-python")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, &bin_dir), @"
    exit_code: 0 (success)
    ");

    // With `--managed-python`, both the managed installation and the symlink are shown
    uv_snapshot!(context.filters(), context.python_list()
        .arg("3.10")
        .arg("--only-installed")
        .arg("--managed-python")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, &bin_dir), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.10.[LATEST]-[PLATFORM]    [BIN]/[PYTHON] -> managed/cpython-3.10-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    cpython-3.10.[LATEST]-[PLATFORM]    managed/cpython-3.10-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");
}

#[tokio::test]
#[cfg(feature = "test-python")]
async fn python_list_only_installed_skips_download_metadata() {
    let context = uv_test::test_context_with_versions!(&["3.12"])
        .with_filtered_python_symlinks()
        .with_filtered_python_keys()
        .with_collapsed_whitespace();
    let server = MockServer::start().await;

    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_raw("{", "application/json"))
        .expect(0)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.python_list()
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS)
        .arg("--only-installed")
        .arg("--python-downloads-json-url").arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.12.[X]-[PLATFORM] [PYTHON-3.12]
    ");
}

#[tokio::test]
async fn python_list_remote_python_downloads_json_url() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_http_retries();
    let server = MockServer::start().await;

    let remote_json = r#"
    {
        "cpython-3.14.0-darwin-aarch64-none": {
            "name": "cpython",
            "arch": {
                "family": "aarch64",
                "variant": null
            },
            "os": "darwin",
            "libc": "none",
            "major": 3,
            "minor": 14,
            "patch": 0,
            "prerelease": "",
            "url": "https://custom.com/cpython-3.14.0-darwin-aarch64-none.tar.gz",
            "sha256": "C3223D5924A0ED0EF5958A750377C362D0957587F896C0F6C635AE4B39E0F337",
            "variant": null,
            "build": "20251028"
        },
        "cpython-3.13.2+freethreaded-linux-powerpc64le-gnu": {
            "name": "cpython",
            "arch": {
                "family": "powerpc64le",
                "variant": null
            },
            "os": "linux",
            "libc": "gnu",
            "major": 3,
            "minor": 13,
            "patch": 2,
            "prerelease": "",
            "url": "https://custom.com/ccpython-3.13.2+freethreaded-linux-powerpc64le-gnu.tar.gz",
            "sha256": "6ae8fa44cb2edf4ab49cff1820b53c40c10349c0f39e11b8cd76ce7f3e7e1def",
            "variant": "freethreaded",
            "build": "20250317"
        }
    }
    "#;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(remote_json, "application/json"))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/invalid"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{", "application/json"))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/invalid-hash"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            remote_json.replace(
                "C3223D5924A0ED0EF5958A750377C362D0957587F896C0F6C635AE4B39E0F337",
                "short",
            ),
            "application/json",
        ))
        .mount(&server)
        .await;

    // Test showing all interpreters from the remote JSON URL
    uv_snapshot!(context
        .python_list()
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS)
        .arg("--all-versions")
        .arg("--all-platforms")
        .arg("--all-arches")
        .arg("--show-urls")
        .arg("--python-downloads-json-url").arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.14.0-macos-aarch64-none                    https://custom.com/cpython-3.14.0-darwin-aarch64-none.tar.gz
    cpython-3.13.2+freethreaded-linux-powerpc64le-gnu    https://custom.com/ccpython-3.13.2+freethreaded-linux-powerpc64le-gnu.tar.gz
    ");

    // test invalid URL path
    uv_snapshot!(context.filters(), context
        .python_list()
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS)
        .arg("--python-downloads-json-url").arg(format!("{}/404", server.uri())), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Error while fetching remote python downloads json from `http://[LOCALHOST]/404`
      cause: Failed to fetch: http://[LOCALHOST]/404
      cause: HTTP status client error (404 Not Found) for url (http://[LOCALHOST]/404)
    ");

    // test invalid json
    uv_snapshot!(context.filters(), context
        .python_list()
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS)
        .arg("--python-downloads-json-url").arg(format!("{}/invalid", server.uri())), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to parse the JSON Python download list at `http://[LOCALHOST]/invalid`
      cause: EOF while parsing an object at line 1 column 1
    ");

    // Test a syntactically valid JSON document containing an invalid SHA-256 digest.
    uv_snapshot!(context.filters(), context
        .python_list()
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS)
        .arg("--python-downloads-json-url").arg(format!("{}/invalid-hash", server.uri())), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to parse the JSON Python download list at `http://[LOCALHOST]/invalid-hash`
      cause: Invalid hash digest length (expected 64 hexadecimal characters, found 5) at line 16 column 29
    ");

    Ok(())
}

#[test]
fn python_list_with_mirrors() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_collapsed_whitespace()
        .with_filtered_latest_python_versions()
        // Normalize build-specific archive paths while preserving the mirror host and artifact family.
        .with_filter((
            r"/[^/\s]+/(cpython|graalpy|xbuildenv)-[^/\s]+\.(?:tar\.[a-z0-9]+|zip)\b",
            "/[RELEASE]/$1-[ARCHIVE]",
        ))
        .with_filter((r"/pypy[^/\s]+\.(?:tar\.[a-z0-9]+|zip)\b", "/pypy[ARCHIVE]"));

    // All three implementations have downloads for this target regardless of the host platform.
    let all_implementations = "any-3.10-windows-x86_64-none";

    // Test with UV_PYTHON_INSTALL_MIRROR environment variable - verify mirror URL is used
    uv_snapshot!(context.filters(), context.python_list()
        .arg("cpython-3.10.19-windows-x86_64-none")
        .arg("--show-urls")
        .env(EnvVars::UV_PYTHON_INSTALL_MIRROR, "https://mirror.example.com")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.10.19-windows-x86_64-none https://mirror.example.com/[RELEASE]/cpython-[ARCHIVE]
    ");

    // Test with UV_PYPY_INSTALL_MIRROR environment variable - verify PyPy mirror URL is used
    uv_snapshot!(context.filters(), context.python_list()
        .arg("pypy-3.10-windows-x86_64-none")
        .arg("--show-urls")
        .env(EnvVars::UV_PYPY_INSTALL_MIRROR, "https://pypy-mirror.example.com")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    pypy-3.10.16-windows-x86_64-none https://pypy-mirror.example.com/pypy[ARCHIVE]
    ");

    // Test with UV_GRAALPY_INSTALL_MIRROR environment variable.
    uv_snapshot!(context.filters(), context.python_list()
        .arg("graalpy-3.10-windows-x86_64-none")
        .arg("--show-urls")
        .env(EnvVars::UV_GRAALPY_INSTALL_MIRROR, "https://graalpy-mirror.example.com")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    graalpy-3.10.0-windows-x86_64-none https://graalpy-mirror.example.com/[RELEASE]/graalpy-[ARCHIVE]
    ");

    // Test with UV_PYODIDE_INSTALL_MIRROR environment variable.
    uv_snapshot!(context.filters(), context.python_list()
        .arg("pyodide@3.13")
        .arg("--show-urls")
        .env(EnvVars::UV_PYODIDE_INSTALL_MIRROR, "https://pyodide-mirror.example.com")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    pyodide-3.13.2-emscripten-wasm32-musl https://pyodide-mirror.example.com/[RELEASE]/xbuildenv-[ARCHIVE]
    ");

    // Test with all mirror environment variables set.
    uv_snapshot!(context.filters(), context.python_list()
        .arg(all_implementations)
        .arg("--show-urls")
        .env(EnvVars::UV_PYTHON_INSTALL_MIRROR, "https://python-mirror.example.com")
        .env(EnvVars::UV_PYPY_INSTALL_MIRROR, "https://pypy-mirror.example.com")
        .env(EnvVars::UV_GRAALPY_INSTALL_MIRROR, "https://graalpy-mirror.example.com")
        .env(EnvVars::UV_PYODIDE_INSTALL_MIRROR, "https://pyodide-mirror.example.com")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.10.[LATEST]-windows-x86_64-none https://python-mirror.example.com/[RELEASE]/cpython-[ARCHIVE]
    pypy-3.10.16-windows-x86_64-none https://pypy-mirror.example.com/pypy[ARCHIVE]
    graalpy-3.10.0-windows-x86_64-none https://graalpy-mirror.example.com/[RELEASE]/graalpy-[ARCHIVE]
    ");

    // Pyodide uses its own mirror when the CPython mirror is also set.
    uv_snapshot!(context.filters(), context.python_list()
        .arg("pyodide@3.13")
        .arg("--show-urls")
        .env(EnvVars::UV_PYTHON_INSTALL_MIRROR, "https://python-mirror.example.com")
        .env(EnvVars::UV_PYODIDE_INSTALL_MIRROR, "https://pyodide-mirror.example.com")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    pyodide-3.13.2-emscripten-wasm32-musl https://pyodide-mirror.example.com/[RELEASE]/xbuildenv-[ARCHIVE]
    ");

    // Test without mirrors - verify the default Astral mirror URL is used for CPython
    uv_snapshot!(context.filters(), context.python_list()
        .arg(all_implementations)
        .arg("--show-urls")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.10.[LATEST]-windows-x86_64-none https://releases.astral.sh/github/python-build-standalone/releases/download/[RELEASE]/cpython-[ARCHIVE]
    pypy-3.10.16-windows-x86_64-none https://downloads.python.org/pypy/pypy[ARCHIVE]
    graalpy-3.10.0-windows-x86_64-none https://github.com/oracle/graalpython/releases/download/[RELEASE]/graalpy-[ARCHIVE]
    ");

    // The CPython mirror does not change the default Pyodide URL.
    uv_snapshot!(context.filters(), context.python_list()
        .arg("pyodide@3.13")
        .arg("--show-urls")
        .env(EnvVars::UV_PYTHON_INSTALL_MIRROR, "https://python-mirror.example.com")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    pyodide-3.13.2-emscripten-wasm32-musl https://github.com/pyodide/pyodide/releases/download/[RELEASE]/xbuildenv-[ARCHIVE]
    ");

    context.temp_dir.child("uv.toml").write_str(indoc! {r#"
        python-install-mirror = "https://python-mirror.example.com"
        pypy-install-mirror = "https://pypy-mirror.example.com"
        graalpy-install-mirror = "https://graalpy-mirror.example.com"
        pyodide-install-mirror = "https://pyodide-mirror.example.com"
    "#})?;

    uv_snapshot!(context.filters(), context.python_list()
        .arg(all_implementations)
        .arg("--show-urls")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.10.[LATEST]-windows-x86_64-none https://python-mirror.example.com/[RELEASE]/cpython-[ARCHIVE]
    pypy-3.10.16-windows-x86_64-none https://pypy-mirror.example.com/pypy[ARCHIVE]
    graalpy-3.10.0-windows-x86_64-none https://graalpy-mirror.example.com/[RELEASE]/graalpy-[ARCHIVE]
    ");

    uv_snapshot!(context.filters(), context.python_list()
        .arg("pyodide@3.13")
        .arg("--show-urls")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    pyodide-3.13.2-emscripten-wasm32-musl https://pyodide-mirror.example.com/[RELEASE]/xbuildenv-[ARCHIVE]
    ");

    // Each environment variable overrides only its corresponding configured mirror.
    uv_snapshot!(context.filters(), context.python_list()
        .arg(all_implementations)
        .arg("--show-urls")
        .env(EnvVars::UV_PYPY_INSTALL_MIRROR, "https://mirror.example.com")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    cpython-3.10.[LATEST]-windows-x86_64-none https://python-mirror.example.com/[RELEASE]/cpython-[ARCHIVE]
    pypy-3.10.16-windows-x86_64-none https://mirror.example.com/pypy[ARCHIVE]
    graalpy-3.10.0-windows-x86_64-none https://graalpy-mirror.example.com/[RELEASE]/graalpy-[ARCHIVE]
    ");

    uv_snapshot!(context.filters(), context.python_list()
        .arg("pyodide@3.13")
        .arg("--show-urls")
        .env(EnvVars::UV_PYODIDE_INSTALL_MIRROR, "https://mirror.example.com")
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS), @"
    exit_code: 0 (success)
    ----- stdout -----
    pyodide-3.13.2-emscripten-wasm32-musl https://mirror.example.com/[RELEASE]/xbuildenv-[ARCHIVE]
    ");

    Ok(())
}
