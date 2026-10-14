use std::process::Command;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::FileWriteStr;
use assert_fs::prelude::PathChild;
use insta::assert_snapshot;
use uv_python_managed::platform_key_from_env;
use uv_test::{TestContext, assert_path_missing, uv_snapshot, venv_bin_path};

use crate::build_upgrade::OutdatedPythonBuild;
use crate::managed_venv::{create_stdlib_venv, venv_version};

#[test]
fn python_upgrade() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    // Install an earlier patch version
    uv_snapshot!(context.filters(), context.python_install().arg("3.10.17"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.17 in [TIME]
     + cpython-3.10.17-[PLATFORM] (python3.10)
    ");

    // Don't accept patch version as argument to upgrade command
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10.17"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: `uv python upgrade` only accepts minor versions, got: 3.10.17
    ");

    // Upgrade an outdated patch even when a reinstall is requested.
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10").arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.[LATEST] in [TIME]
     + cpython-3.10.[LATEST]-[PLATFORM] (python3.10)
    ");

    let installation = dunce::canonicalize(
        context
            .temp_dir
            .join("managed")
            .join(format!("cpython-3.10-{}", platform_key_from_env()?)),
    )?;
    let marker = installation.join("before-reinstall");
    fs_err::write(&marker, "original installation\n")?;

    // Should be a no-op when already upgraded
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Python 3.10 is already on the latest supported patch release
    ");
    assert_eq!(context.read(&marker), "original installation\n");

    // Should reinstall on `--reinstall`
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10").arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.[LATEST] in [TIME]
     ~ cpython-3.10.[LATEST]-[PLATFORM] (python3.10)
    ");
    assert!(
        installation.is_dir(),
        "reinstalled Python is missing: {}",
        installation.display()
    );
    assert_path_missing(marker);

    // Install an earlier pre-release version
    uv_snapshot!(context.filters(), context.python_install().arg("3.14.0rc2"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.14.0rc2 in [TIME]
     + cpython-3.14.0rc2-[PLATFORM] (python3.14)
    ");

    // Upgrade the pre-release version
    uv_snapshot!(context.filters(), context.python_upgrade(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.14.[LATEST] in [TIME]
     + cpython-3.14.[LATEST]-[PLATFORM] (python3.14)
    ");
    Ok(())
}

#[test]
fn python_upgrade_without_version() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    // Should be a no-op when no versions have been installed
    uv_snapshot!(context.filters(), context.python_upgrade(), @"
    exit_code: 0 (success)
    ----- stderr -----
    There are no installed versions to upgrade
    ");

    // Install earlier patch versions for different minor versions
    uv_snapshot!(context.filters(), context.python_install().arg("3.11.8").arg("3.12.8").arg("3.13.1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 3 versions in [TIME]
     + cpython-3.11.8-[PLATFORM] (python3.11)
     + cpython-3.12.8-[PLATFORM] (python3.12)
     + cpython-3.13.1-[PLATFORM] (python3.13)
    ");

    // Upgrade one patch version
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.13"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.[LATEST] in [TIME]
     + cpython-3.13.[LATEST]-[PLATFORM] (python3.13)
    ");

    // Providing no minor version to `uv python upgrade` should upgrade the rest
    // of the patch versions
    uv_snapshot!(context.filters(), context.python_upgrade(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 2 versions in [TIME]
     + cpython-3.11.[LATEST]-[PLATFORM] (python3.11)
     + cpython-3.12.[LATEST]-[PLATFORM] (python3.12)
    ");

    // Should be a no-op when every version is already upgraded
    uv_snapshot!(context.filters(), context.python_upgrade(), @"
    exit_code: 0 (success)
    ----- stderr -----
    All versions already on latest supported patch release
    ");
}

#[test]
fn python_upgrade_transparent_from_venv() {
    let context = uv_test::test_context_with_versions!(&["3.13"])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    // Install an earlier patch version
    uv_snapshot!(context.filters(), context.python_install().arg("3.10.17"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.17 in [TIME]
     + cpython-3.10.17-[PLATFORM] (python3.10)
    ");

    // Create a virtual environment
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.17
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    uv_snapshot!(context.filters(), venv_version(&context, context.interpreter()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    "
    );

    let second_venv = ".venv2";

    // Create a second virtual environment with minor version request
    uv_snapshot!(context.filters(), context.venv().arg(second_venv).arg("-p").arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.17
    Creating virtual environment at: .venv2
    Activate with: source .venv2/[BIN]/activate
    ");

    let second_interpreter = venv_bin_path(context.temp_dir.join(second_venv))
        .join(format!("python{}", std::env::consts::EXE_SUFFIX));
    uv_snapshot!(context.filters(), venv_version(&context, &second_interpreter), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    "
    );

    // Upgrade patch version
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.[LATEST] in [TIME]
     + cpython-3.10.[LATEST]-[PLATFORM] (python3.10)
    ");

    // First virtual environment should reflect upgraded patch
    uv_snapshot!(context.filters(), venv_version(&context, context.interpreter()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.[LATEST]
    "
    );

    // Second virtual environment should reflect upgraded patch
    uv_snapshot!(context.filters(), venv_version(&context, &second_interpreter), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.[LATEST]
    "
    );
}

#[test]
fn python_run_honors_patch_pin_after_transparent_upgrade() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    // Install an earlier patch version
    uv_snapshot!(context.filters(), context.python_install().arg("3.10.17"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.17 in [TIME]
     + cpython-3.10.17-[PLATFORM] (python3.10)
    ");

    // Create a virtual environment
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.17
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    // Pin to older patch version
    uv_snapshot!(context.filters(), context.python_pin().arg("3.10.17"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.10.17`
    ");

    // Upgrade patch version
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.[LATEST] in [TIME]
     + cpython-3.10.[LATEST]-[PLATFORM] (python3.10)
    ");

    // The environment was created before the pin and follows the upgraded minor version.
    uv_snapshot!(context.filters(), venv_version(&context, context.interpreter()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.[LATEST]
    ");

    // `uv run` must still honor the older patch pin instead of selecting that environment.
    uv_snapshot!(context.filters(), context.run().arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    "
    );
}

// Virtual environments record patch versions. `uv venv -p 3.x.y` will
// prevent transparent upgrades.
#[test]
fn python_no_transparent_upgrade_with_venv_patch_specification() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    // Install an earlier patch version
    uv_snapshot!(context.filters(), context.python_install().arg("3.10.17"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.17 in [TIME]
     + cpython-3.10.17-[PLATFORM] (python3.10)
    ");

    // Create a virtual environment with a patch version
    uv_snapshot!(context.filters(), context.venv().arg("-p").arg("3.10.17"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.17
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    uv_snapshot!(context.filters(), venv_version(&context, context.interpreter()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    "
    );

    // Upgrade patch version
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.[LATEST] in [TIME]
     + cpython-3.10.[LATEST]-[PLATFORM] (python3.10)
    ");

    // The virtual environment Python version remains the same.
    uv_snapshot!(context.filters(), venv_version(&context, context.interpreter()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    "
    );
}

// Transparent upgrades should work for virtual environments created within
// virtual environments.
#[test]
fn python_transparent_upgrade_venv_venv() {
    let context = uv_test::test_context_with_versions!(&["3.13"])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_filtered_virtualenv_bin()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    // Install an earlier patch version
    uv_snapshot!(context.filters(), context.python_install().arg("3.10.17"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.17 in [TIME]
     + cpython-3.10.17-[PLATFORM] (python3.10)
    ");

    // Create an initial virtual environment
    uv_snapshot!(context.filters(), context.venv().arg("-p").arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.17
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    let venv_python = if cfg!(windows) {
        context.venv.child("Scripts/python.exe")
    } else {
        context.venv.child("bin/python")
    };

    let second_venv = ".venv2";

    // Create a new virtual environment from within a virtual environment
    uv_snapshot!(context.filters(), context.venv()
        .arg(second_venv)
        .arg("-p").arg(venv_python.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.17 interpreter at: .venv/[BIN]/python
    Creating virtual environment at: .venv2
    Activate with: source .venv2/[BIN]/activate
    ");

    let second_interpreter = venv_bin_path(context.temp_dir.join(second_venv))
        .join(format!("python{}", std::env::consts::EXE_SUFFIX));
    uv_snapshot!(context.filters(), venv_version(&context, &second_interpreter), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    "
    );

    // Upgrade patch version
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.[LATEST] in [TIME]
     + cpython-3.10.[LATEST]-[PLATFORM] (python3.10)
    ");

    // Should have transparently upgraded in second virtual environment
    uv_snapshot!(context.filters(), venv_version(&context, &second_interpreter), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.[LATEST]
    "
    );
}

// Transparent upgrades should work for virtual environments created using
// the `venv` module.
#[test]
fn python_upgrade_transparent_from_venv_module() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_python_install_bin()
        .with_filtered_latest_python_versions();

    // Install earlier patch version
    uv_snapshot!(context.filters(), context.python_install().arg("3.12.9"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.12.9 in [TIME]
     + cpython-3.12.9-[PLATFORM] (python3.12)
    ");

    uv_snapshot!(context.filters(), create_stdlib_venv(&context, "3.12"), @"
    exit_code: 0 (success)
    ");

    uv_snapshot!(context.filters(), venv_version(&context, context.interpreter()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.9
    "
    );

    // Upgrade patch version
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.12.[LATEST] in [TIME]
     + cpython-3.12.[LATEST]-[PLATFORM] (python3.12)
    "
    );

    // Virtual environment should reflect upgraded patch
    uv_snapshot!(context.filters(), venv_version(&context, context.interpreter()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[LATEST]
    "
    );
}

// Transparent Python upgrades should work in environments created using
// the `venv` module within an existing virtual environment.
#[test]
fn python_upgrade_transparent_from_venv_module_in_venv() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_python_install_bin()
        .with_filtered_latest_python_versions();

    // Install earlier patch version
    uv_snapshot!(context.filters(), context.python_install().arg("3.10.17"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.17 in [TIME]
     + cpython-3.10.17-[PLATFORM] (python3.10)
    ");

    // Create first virtual environment
    uv_snapshot!(context.filters(), context.venv().arg("-p").arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.17
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    let second_venv = ".venv2";

    uv_snapshot!(context.filters(), context.python_command()
        .arg("-m").arg("venv").arg(second_venv).arg("--without-pip"), @"
    exit_code: 0 (success)
    ");

    let second_interpreter = venv_bin_path(context.temp_dir.join(second_venv))
        .join(format!("python{}", std::env::consts::EXE_SUFFIX));
    uv_snapshot!(context.filters(), venv_version(&context, &second_interpreter), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    "
    );

    // Upgrade patch version
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.[LATEST] in [TIME]
     + cpython-3.10.[LATEST]-[PLATFORM] (python3.10)
    "
    );

    // Second virtual environment should reflect upgraded patch.
    uv_snapshot!(context.filters(), venv_version(&context, &second_interpreter), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.[LATEST]
    "
    );
}

// Tests that `uv python upgrade 3.12` will warn if trying to install over non-managed
// interpreter.
#[test]
fn python_upgrade_force_install() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_empty_python_install_mirror()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    let executable = context
        .bin_dir
        .child(format!("python3.12{}", std::env::consts::EXE_SUFFIX));
    executable.write_str("unmanaged executable\n")?;

    // Try to upgrade with a non-managed interpreter installed in `bin`.
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Executable already exists at `[BIN]/python3.12` but is not managed by uv; use `uv python install 3.12 --force` to replace it
    Installed Python 3.12.[LATEST] in [TIME]
     + cpython-3.12.[LATEST]-[PLATFORM]
    ");
    assert_eq!(context.read(&executable), "unmanaged executable\n");

    // Force the `bin` install.
    uv_snapshot!(context.filters(), context.python_install().arg("3.12").arg("--force"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.12.[LATEST] in [TIME]
     + cpython-3.12.[LATEST]-[PLATFORM] (python3.12)
    ");
    context
        .external_command(&executable)
        .args(["-c", "import sys; print(sys.version_info[:2], end='')"])
        .assert()
        .success()
        .stdout("(3, 12)");

    Ok(())
}

#[test]
fn python_upgrade_implementation() {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_empty_python_install_mirror()
        .with_managed_python_dirs();

    // Install pypy
    context.python_install().arg("pypy@3.11").assert().success();

    // Upgrading PyPy should not install CPython.
    uv_snapshot!(context.filters(), context.python_upgrade(), @"
    exit_code: 0 (success)
    ----- stderr -----
    All versions already on latest supported patch release
    ");

    context
        .python_list()
        .args(["cpython", "--only-installed", "--managed-python"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn python_upgrade_build_version() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    // Install Python 3.12
    uv_snapshot!(context.filters(), context.python_install().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.12.[LATEST] in [TIME]
     + cpython-3.12.[LATEST]-[PLATFORM] (python3.12)
    ");

    // Should be a no-op when already installed at latest version
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Python 3.12 is already on the latest supported patch release
    ");

    let outdated = OutdatedPythonBuild::python_3_12(&context)?;

    // Now upgrade should detect the outdated build version and reinstall
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.12.[LATEST] in [TIME]
     ~ cpython-3.12.[LATEST]-[PLATFORM]
    ");

    outdated.assert_replaced()?;

    // Should be a no-op again after upgrade
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Python 3.12 is already on the latest supported patch release
    ");

    Ok(())
}

// A project environment should survive a transparent Python patch upgrade.
#[test]
fn python_sync_transparent_patch_upgrade_reuses_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions()
        .with_pyvenv_cfg_filters();

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.10"
        "#,
    )?;

    context.python_install().arg("3.10.17").assert().success();
    context
        .sync()
        .arg("--python")
        .arg("3.10")
        .assert()
        .success();
    let marker = context.venv.child("reuse-marker");
    marker.write_str("pre-upgrade environment\n")?;

    // The minor-only `version_info` allows compatibility checks to accept a newer patch.
    let pyvenv_cfg = context.read(".venv/pyvenv.cfg");
    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(pyvenv_cfg, @r"
        home = [PYTHON_HOME]
        implementation = CPython
        uv = [UV_VERSION]
        version_info = 3.10
        include-system-site-packages = false
        prompt = project
        ");
    });

    context.python_upgrade().arg("3.10").assert().success();

    // The sync should not report removing or creating the environment.
    uv_snapshot!(context.filters(), context.sync().arg("--python").arg("3.10"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");
    assert_eq!(context.read(marker), "pre-upgrade environment\n");

    // The reused environment should run the upgraded Python patch.
    uv_snapshot!(context.filters(), venv_version(&context, context.interpreter()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.[LATEST]
    ");

    Ok(())
}

// Regression test for <https://github.com/astral-sh/uv/issues/19100>.
//
// A virtual environment created by `uv sync` should pin to a patch version
// recorded in `.python-version` rather than symlinking to the mutable
// minor-version directory, which would be silently upgraded by a later
// `uv python install` of a newer patch release.
#[test]
fn python_sync_honors_pinned_patch_version() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions()
        .with_pyvenv_cfg_filters();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.10"
        "#,
    )?;

    // Install an earlier patch version
    uv_snapshot!(context.filters(), context.python_install().arg("3.10.17"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.17 in [TIME]
     + cpython-3.10.17-[PLATFORM] (python3.10)
    ");

    // Pin the project to the older patch version before the environment exists
    uv_snapshot!(context.filters(), context.python_pin().arg("3.10.17"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    Pinned `.python-version` to `3.10.17`
    ");

    // Create the environment via `uv sync`
    uv_snapshot!(context.filters(), context.sync(), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.17
    Creating virtual environment at: .venv
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    // An exact-patch environment must retain the patch in `version_info`.
    let pyvenv_cfg = context.read(".venv/pyvenv.cfg");
    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(pyvenv_cfg, @r"
        home = [PYTHON_HOME]
        implementation = CPython
        uv = [UV_VERSION]
        version_info = 3.10.17
        include-system-site-packages = false
        prompt = project
        ");
    });

    // Install a newer patch version, which moves the mutable `cpython-3.10`
    // minor-version link to the newer release.
    uv_snapshot!(context.filters(), context.python_upgrade().arg("3.10"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.10.[LATEST] in [TIME]
     + cpython-3.10.[LATEST]-[PLATFORM] (python3.10)
    ");

    // Query the synced environment directly so a project command cannot repair it first.
    uv_snapshot!(context.filters(), venv_version(&context, context.interpreter()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    ");

    Ok(())
}

// A script environment must retain a patch pin discovered from a version file.
#[test]
fn python_script_honors_patch_version_file() -> Result<()> {
    let (context, probe) = script_interpreter_after_upgrade(Some("3.10.17"), ">=3.10")?;

    uv_snapshot!(context.filters(), probe, @r"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    ");

    Ok(())
}

// A script environment must retain an exact patch requirement from its inline metadata.
#[test]
fn python_script_honors_patch_requires_python() -> Result<()> {
    let (context, probe) = script_interpreter_after_upgrade(None, "==3.10.17")?;

    uv_snapshot!(context.filters(), probe, @r"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.17
    ");

    Ok(())
}

// A minor-version request from a version file permits transparent patch upgrades.
#[test]
fn python_script_upgrades_minor_version_file() -> Result<()> {
    let (context, probe) = script_interpreter_after_upgrade(Some("3.10"), ">=3.10,<3.11")?;

    uv_snapshot!(context.filters(), probe, @r"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.[LATEST]
    ");

    Ok(())
}

// A script's inline metadata can permit transparent patch upgrades without a version file.
#[test]
fn python_script_upgrades_requires_python() -> Result<()> {
    let (context, probe) = script_interpreter_after_upgrade(None, ">=3.10,<3.11")?;

    uv_snapshot!(context.filters(), probe, @r"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.10.[LATEST]
    ");

    Ok(())
}

/// Create a script environment, then upgrade its managed Python installation.
///
/// Return a command that verifies the cached interpreter still initializes as a virtual
/// environment and prints its version, without rediscovering Python or recreating the environment.
fn script_interpreter_after_upgrade(
    pin: Option<&str>,
    requires_python: &str,
) -> Result<(TestContext, Command)> {
    let initial_version = "3.10.17";
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();
    context
        .python_install()
        .arg(initial_version)
        .assert()
        .success();
    if let Some(pin) = pin {
        context.temp_dir.child(".python-version").write_str(pin)?;
    }
    context.temp_dir.child("script.py").write_str(&format!(
        r#"# /// script
# requires-python = "{requires_python}"
# dependencies = []
# ///
import sys
if sys.prefix == sys.base_prefix:
    raise RuntimeError("the script did not run in a virtual environment")
if sys.version.split()[0] != "{initial_version}":
    raise RuntimeError("unexpected initial Python: " + sys.version)
print(sys.executable)
"#,
    ))?;
    let output = context.run().arg("script.py").assert().success();
    let python = std::str::from_utf8(&output.get_output().stdout)?.trim();

    context.python_upgrade().arg("3.10").assert().success();

    let probe = venv_version(&context, python);
    Ok((context, probe))
}
