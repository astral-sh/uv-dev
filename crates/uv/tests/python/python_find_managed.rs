//! Interpreter discovery against managed installations.

use assert_cmd::assert::OutputAssertExt;
#[cfg(unix)]
use assert_fs::prelude::{PathChild, PathCreateDir};
#[cfg(unix)]
use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
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

    // `>3.14rc2` should not match rc2
    uv_snapshot!(context.filters(), context.python_find().arg(">3.13.0rc2").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0rc3-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // `>3.14rc3` should not match rc3
    uv_snapshot!(context.filters(), context.python_find().arg(">3.13.0rc3"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python >3.13.0rc3 in [PYTHON SOURCES]
    ");

    // `>=3.14.0rc3` should match rc3
    uv_snapshot!(context.filters(), context.python_find().arg(">=3.13.0rc3").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0rc3-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // `<3.14.0rc3` should match rc2
    uv_snapshot!(context.filters(), context.python_find().arg("<3.13.0rc3").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0rc2-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");

    // `<=3.14.0rc3` should match rc3
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

    // `>3.14rc2` should prefer stable
    uv_snapshot!(context.filters(), context.python_find().arg(">3.13.0rc2").arg("--resolve-links"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.0-[PLATFORM]/[INSTALL-BIN]/[PYTHON]
    ");
}

#[test]
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
