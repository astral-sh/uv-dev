//! Python requests rejected without running or downloading an interpreter.

use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::{PathChild, PathCreateDir};
use uv_test::{assert_path_missing, uv_snapshot};

#[test]
fn python_install_invalid_request() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs();

    // Request something that is not a Python version
    uv_snapshot!(context.filters(), context.python_install().arg("foobar"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `foobar` is not a valid Python download request; see `uv help python` for supported formats and `uv python list --only-downloads` for available versions
    ");

    // Request a version we don't have a download for
    uv_snapshot!(context.filters(), context.python_install().arg("3.8.0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No download found for request: cpython-3.8.0-[PLATFORM]
    ");

    // Resolve every request before installing; an unavailable later request must not leave the
    // earlier valid request installed. The explicit platform makes the first request available
    // independently of the host.
    let available = "cpython-3.12.9-linux-x86_64-gnu";
    uv_snapshot!(context.filters(), context.python_install().arg(available).arg("3.8.0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No download found for request: cpython-3.8.0-[PLATFORM]
    ");
    context
        .python_list()
        .args(["3.12", "--only-installed", "--managed-python"])
        .assert()
        .success()
        .stdout("");
    assert_path_missing(context.temp_dir.child("managed").child(available));
    assert_path_missing(
        context
            .bin_dir
            .child(format!("python3.12{}", std::env::consts::EXE_SUFFIX)),
    );
}

#[test]
fn python_install_rejects_directory_request() {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();

    context.temp_dir.child("foo").create_dir_all().unwrap();

    uv_snapshot!(context.filters(), context.python_install().arg("./foo"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `./foo` is not a valid Python download request; see `uv help python` for supported formats and `uv python list --only-downloads` for available versions
    ");
}

#[test]
fn python_install_armv7_musl_unavailable() {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();

    // Explicitly request a musl build for armv7l
    uv_snapshot!(context.filters(), context.python_install().arg("cpython-3.12.12-linux-armv7-musl"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: uv does not yet provide musl Python distributions on armv7.
    ");
}

#[test]
fn python_uninstall_requires_target() {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();

    uv_snapshot!(context.filters(), context.python_uninstall(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      <TARGETS>...

    Usage: uv python uninstall --cache-dir [CACHE_DIR] --install-dir <INSTALL_DIR> <TARGETS>...

    For more information, try '--help'.
    ");
}

#[test]
fn python_uninstall_rejects_target_with_all() {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();

    uv_snapshot!(context.filters(), context.python_uninstall().arg("--all").arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--all' cannot be used with '<TARGETS>...'

    Usage: uv python uninstall --cache-dir [CACHE_DIR] --all --install-dir <INSTALL_DIR> <TARGETS>...

    For more information, try '--help'.
    ");
}

#[test]
fn python_upgrade_rejects_patch() {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();

    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.13.0"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: `uv python upgrade` only accepts minor versions, got: 3.13.0
    ");
}

#[test]
fn python_upgrade_rejects_prerelease() {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();

    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.14rc3"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: `uv python upgrade` only accepts minor versions, got: 3.14rc3
    ");
}

#[test]
fn python_install_upgrade_rejects_patch() {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();

    uv_snapshot!(context.filters(), context.python_install().arg("--upgrade").arg("3.11.4"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: `uv python install --upgrade` only accepts minor versions, got: 3.11.4
    ");
}
