#[cfg(any(feature = "test-git", feature = "test-git-lfs"))]
use std::collections::BTreeSet;
#[cfg(any(windows, feature = "test-git"))]
use std::ffi::OsString;
#[cfg(windows)]
use std::io::{Read, Seek};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
#[cfg(feature = "test-git")]
use assert_fs::fixture::ChildPath;
use assert_fs::{
    assert::PathAssert,
    fixture::{FileTouch, FileWriteStr, PathChild, PathCreateDir},
};
use indoc::indoc;
use insta::assert_snapshot;
use predicates::prelude::predicate;
#[cfg(windows)]
use sha2::{Digest, Sha256};
use url::Url;
use uv_extract::dirhash::dirhash_path;
#[cfg(windows)]
use uv_fs::Simplified;
use uv_fs::copy_dir_all;
use uv_static::EnvVars;

#[cfg(windows)]
use uv_test::packse::generate_wheel_with_binary_files;
use uv_test::packse::generate_wheel_with_files;
use uv_test::{site_packages_path, uv_snapshot, venv_bin_path};

#[cfg(feature = "test-git")]
fn tool_install_git_path(bin_dir: &ChildPath) -> OsString {
    let mut paths = BTreeSet::new();
    paths.insert(bin_dir.to_path_buf());
    paths.insert(
        which::which("git")
            .expect("Failed to find `git` executable.")
            .parent()
            .expect("Failed to find `git` executable directory.")
            .to_path_buf(),
    );

    // Git Submodule in macOS seems to rely on `sed`.
    if cfg!(target_os = "macos") {
        paths.insert(
            which::which("sed")
                .expect("Failed to find `sed` executable.")
                .parent()
                .expect("Failed to find `sed` executable directory.")
                .to_path_buf(),
        );
    }

    std::env::join_paths(paths).unwrap()
}

#[test]
fn tool_install() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/black/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from black import patched_main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(patched_main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("black").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    black, 24.3.0 (compiled: yes)
    Python (CPython) 3.12.[X]
    ");

    // Install another tool
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + blinker==1.7.0
     + click==8.1.7
     + flask==3.0.2
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + werkzeug==3.0.1
    Installed 1 executable: flask
    ");

    tool_dir.child("flask").assert(predicate::path::is_dir());
    assert!(
        bin_dir
            .child(format!("flask{}", std::env::consts::EXE_SUFFIX))
            .exists()
    );

    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(bin_dir.join("flask")).unwrap(), @r#"
        #![TEMP_DIR]/tools/flask/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from flask.cli import main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(main())
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("flask").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    Flask 3.0.2
    Werkzeug 3.0.1
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("flask").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "flask" }]
        entrypoints = [
            { name = "flask", install-path = "[TEMP_DIR]/bin/flask", from = "flask" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });
}

#[test]
fn tool_install_relative_exclude_newer_receipt_preserves_span() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    context
        .tool_install()
        .arg("black==24.2.0")
        .arg("--exclude-newer")
        .arg("3 weeks")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, "2024-05-01T00:00:00Z")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", specifier = "==24.2.0" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-04-10T00:00:00Z"
        exclude-newer-span = "P3W"
        "#);
    });
}

/// Package-specific pre-release policies are persisted and reused when upgrading a tool.
#[test]
fn tool_install_prerelease_package_receipt_preserves_policy() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    context
        .tool_install()
        .arg("black")
        .arg("--prerelease-package")
        .arg("black=allow")
        .arg("--prerelease-package")
        .arg("click=disallow")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        prerelease-package = { black = "allow", click = "disallow" }
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    context
        .tool_upgrade()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        prerelease-package = { black = "allow", click = "disallow" }
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });
}

#[test]
fn tool_install_from_directory_ignores_global_pin_outside_requires_python_range() {
    let context = uv_test::test_context_with_versions!(&["3.13", "3.12", "3.11"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let foo_dir = context.temp_dir.child("foo");
    foo_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "foo"
        version = "0.1.0"
        requires-python = ">=3.11,<3.13"
        dependencies = []

        [project.scripts]
        foo = "foo.main:run"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
        })
        .unwrap();
    foo_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()
        .unwrap();
    foo_dir
        .child("src")
        .child("foo")
        .child("main.py")
        .write_str(indoc! {r#"
        import sys

        def run():
            print(f"{sys.version_info.major}.{sys.version_info.minor}")
        "#
        })
        .unwrap();

    context
        .python_pin()
        .arg("3.13")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_install()
        .arg(foo_dir.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo)
    Installed 1 executable: foo
    ");

    uv_snapshot!(context.filters(), Command::new("foo")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12
    ");
}

#[test]
fn tool_install_from_directory_uses_global_pin_within_requires_python_range() {
    let context = uv_test::test_context_with_versions!(&["3.13", "3.12", "3.11"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let foo_dir = context.temp_dir.child("foo");
    foo_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "foo"
        version = "0.1.0"
        requires-python = ">=3.11,<3.13"
        dependencies = []

        [project.scripts]
        foo = "foo.main:run"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
        })
        .unwrap();
    foo_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()
        .unwrap();
    foo_dir
        .child("src")
        .child("foo")
        .child("main.py")
        .write_str(indoc! {r#"
        import sys

        def run():
            print(f"{sys.version_info.major}.{sys.version_info.minor}")
        "#
        })
        .unwrap();

    context
        .python_pin()
        .arg("3.11")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_install()
        .arg(foo_dir.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo)
    Installed 1 executable: foo
    ");

    uv_snapshot!(context.filters(), Command::new("foo")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.11
    ");
}

#[test]
fn tool_install_python_from_global_version_file() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12", "3.13"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Pin to 3.12
    context
        .python_pin()
        .arg("3.12")
        .arg("--global")
        .assert()
        .success();

    // Install a tool
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + blinker==1.7.0
     + click==8.1.7
     + flask==3.0.2
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + werkzeug==3.0.1
    Installed 1 executable: flask
    ");

    // It should use the version from the global file
    uv_snapshot!(context.filters(), Command::new("flask").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    Flask 3.0.2
    Werkzeug 3.0.1
    ");

    // Change global version
    context
        .python_pin()
        .arg("3.13")
        .arg("--global")
        .assert()
        .success();

    // Installing flask again should be a no-op, even though the global pin changed
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    `flask` is already installed
    ");

    uv_snapshot!(context.filters(), Command::new("flask").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    Flask 3.0.2
    Werkzeug 3.0.1
    ");

    // Using `--upgrade` forces us to check the environment
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .arg("--upgrade")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Checked [N] packages in [TIME]
    Installed 1 executable: flask
    ");

    // This will not change to the new global pin, since there was not a reinstall request
    uv_snapshot!(context.filters(), Command::new("flask").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    Flask 3.0.2
    Werkzeug 3.0.1
    ");

    // Using `--reinstall` forces us to install flask again
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .arg("--reinstall")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Ignoring existing environment for `flask`: the Python interpreter does not match the environment interpreter
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + blinker==1.7.0
     + click==8.1.7
     + flask==3.0.2
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + werkzeug==3.0.1
    Installed 1 executable: flask
    ");

    // This will change to the new global pin, since there was not an explicit request recorded in
    // the receipt
    uv_snapshot!(context.filters(), Command::new("flask").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.13.[X]
    Flask 3.0.2
    Werkzeug 3.0.1
    ");

    // If we request a specific Python version, it takes precedence over the pin
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .arg("--python")
        .arg("3.11")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Ignoring existing environment for `flask`: the requested Python interpreter does not match the environment interpreter
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + blinker==1.7.0
     + click==8.1.7
     + flask==3.0.2
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + werkzeug==3.0.1
    Installed 1 executable: flask
    ");

    uv_snapshot!(context.filters(), Command::new("flask").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]
    Flask 3.0.2
    Werkzeug 3.0.1
    ");

    // Use `--reinstall` to install flask again
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .arg("--reinstall")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     ~ blinker==1.7.0
     ~ click==8.1.7
     ~ flask==3.0.2
     ~ itsdangerous==2.1.2
     ~ jinja2==3.1.3
     ~ markupsafe==2.1.5
     ~ werkzeug==3.0.1
    Installed 1 executable: flask
    ");

    // We should continue to use the version from the install, not the global pin
    uv_snapshot!(context.filters(), Command::new("flask").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]
    Flask 3.0.2
    Werkzeug 3.0.1
    ");
}

#[test]
fn tool_install_force_respects_global_python_change() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12", "3.13"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    context
        .python_pin()
        .arg("3.12")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + blinker==1.7.0
     + click==8.1.7
     + flask==3.0.2
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + werkzeug==3.0.1
    Installed 1 executable: flask
    ");

    uv_snapshot!(context.filters(), Command::new("flask").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    Flask 3.0.2
    Werkzeug 3.0.1
    ");

    context
        .python_pin()
        .arg("3.13")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .arg("--force")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + blinker==1.7.0
     + click==8.1.7
     + flask==3.0.2
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + werkzeug==3.0.1
    Installed 1 executable: flask
    ");

    uv_snapshot!(context.filters(), Command::new("flask").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.13.[X]
    Flask 3.0.2
    Werkzeug 3.0.1
    ");
}

#[test]
fn tool_install_with_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-18T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let anyio_local = context.temp_dir.child("src").child("anyio_local");
    copy_dir_all(
        context.workspace_root.join("test/packages/anyio_local"),
        &anyio_local,
    )?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--with-editable")
        .arg("./src/anyio_local")
        .arg("--with")
        .arg("iniconfig")
        .arg("executable-application")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + anyio==4.3.0+foo (from file://[TEMP_DIR]/src/anyio_local)
     + executable-application==0.3.0
     + iniconfig==2.0.0
    Installed 1 executable: app
    ");

    Ok(())
}

#[test]
fn tool_install_workspace_members_do_not_override_explicit_with_requirements() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_exe_suffix();
    let with_editable_tool_dir = context.temp_dir.child("tools-with-editable");
    let with_editable_bin_dir = context.temp_dir.child("bin-with-editable");
    let with_tool_dir = context.temp_dir.child("tools-with");
    let with_bin_dir = context.temp_dir.child("bin-with");

    let root_pyproject = context.temp_dir.child("pyproject.toml");
    root_pyproject.write_str(indoc! {
        r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.scripts]
        root_cli = "root:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.workspace]
        members = ["child"]
        "#
    })?;

    let root_src = context.temp_dir.child("src").child("root");
    root_src.create_dir_all()?;
    root_src.child("__init__.py").write_str(indoc! {
        r"
        def main():
            import child
            print(child.MESSAGE)
        "
    })?;

    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;

    let child_src = child.child("src").child("child");
    child_src.create_dir_all()?;
    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    let status = context
        .tool_install()
        .arg("--with-editable")
        .arg(child.path())
        .arg(context.temp_dir.path())
        .env(EnvVars::UV_TOOL_DIR, with_editable_tool_dir.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, with_editable_bin_dir.as_os_str())
        .env(EnvVars::PATH, with_editable_bin_dir.as_os_str())
        .status()
        .expect("failed to run uv tool install with --with-editable");
    assert!(status.success());

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, with_editable_bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    OK
    ");

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'CHANGED'\n")?;

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, with_editable_bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    CHANGED
    ");

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    let status = context
        .tool_install()
        .arg("--editable")
        .arg("--with")
        .arg(child.path())
        .arg(context.temp_dir.path())
        .env(EnvVars::UV_TOOL_DIR, with_tool_dir.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, with_bin_dir.as_os_str())
        .env(EnvVars::PATH, with_bin_dir.as_os_str())
        .status()
        .expect("failed to run uv tool install with --with");
    assert!(status.success());

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, with_bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    OK
    ");

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'CHANGED'\n")?;

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, with_bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    OK
    ");

    Ok(())
}

#[test]
fn tool_install_preserves_mixed_workspace_member_editability() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let tool_root = context.temp_dir.child("tool-root");
    tool_root.create_dir_all()?;
    tool_root.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "tool-root"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.scripts]
        root_cli = "tool_root:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    let tool_root_src = tool_root.child("src").child("tool_root");
    tool_root_src.create_dir_all()?;
    tool_root_src.child("__init__.py").write_str(indoc! {
        r#"
        def main():
            import importlib.metadata
            import other_child

            print(f"{importlib.metadata.version('tool-root')} {other_child.MESSAGE}")
        "#
    })?;

    let other_workspace = context.temp_dir.child("other-workspace");
    other_workspace.create_dir_all()?;
    other_workspace
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "other-workspace"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["other-child"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.sources]
        other-child = { workspace = true }

        [tool.uv.workspace]
        members = ["packages/*"]
    "#})?;
    let other_workspace_src = other_workspace.child("src").child("other_workspace");
    other_workspace_src.create_dir_all()?;
    other_workspace_src.child("__init__.py").touch()?;

    let other_child = other_workspace.child("packages").child("other-child");
    other_child.create_dir_all()?;
    other_child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "other-child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    let other_child_src = other_child.child("src").child("other_child");
    other_child_src.create_dir_all()?;
    other_child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    let status = context
        .tool_install()
        .arg("--with-editable")
        .arg(other_workspace.path())
        .arg(tool_root.path())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .status()
        .expect("failed to run uv tool install with mixed workspace editability");
    assert!(status.success());

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    0.1.0 OK
    ");

    other_child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'CHANGED'\n")?;

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    0.1.0 CHANGED
    ");

    Ok(())
}

#[test]
fn tool_install_preserves_mixed_workspace_member_non_editability() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let tool_root = context.temp_dir.child("tool-root");
    tool_root.create_dir_all()?;
    tool_root.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "tool-root"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.scripts]
        root_cli = "tool_root:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    let tool_root_src = tool_root.child("src").child("tool_root");
    tool_root_src.create_dir_all()?;
    tool_root_src.child("__init__.py").write_str(indoc! {
        r#"
        def main():
            import importlib.metadata
            import other_child

            print(f"{importlib.metadata.version('tool-root')} {other_child.MESSAGE}")
        "#
    })?;

    let other_workspace = context.temp_dir.child("other-workspace");
    other_workspace.create_dir_all()?;
    other_workspace
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "other-workspace"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["other-child"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.sources]
        other-child = { workspace = true }

        [tool.uv.workspace]
        members = ["packages/*"]
    "#})?;
    let other_workspace_src = other_workspace.child("src").child("other_workspace");
    other_workspace_src.create_dir_all()?;
    other_workspace_src.child("__init__.py").touch()?;

    let other_child = other_workspace.child("packages").child("other-child");
    other_child.create_dir_all()?;
    other_child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "other-child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    let other_child_src = other_child.child("src").child("other_child");
    other_child_src.create_dir_all()?;
    other_child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    let status = context
        .tool_install()
        .arg("--editable")
        .arg("--with")
        .arg(other_workspace.path())
        .arg(tool_root.path())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .status()
        .expect("failed to run uv tool install with mixed workspace editability");
    assert!(status.success());

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    0.1.0 OK
    ");

    other_child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'CHANGED'\n")?;

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    0.1.0 OK
    ");

    Ok(())
}

#[test]
fn tool_install_reinstall_converts_workspace_members_to_non_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let root_pyproject = context.temp_dir.child("pyproject.toml");
    root_pyproject.write_str(indoc! {
        r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["child"]

        [project.scripts]
        root_cli = "root:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.sources]
        child = { workspace = true }

        [tool.uv.workspace]
        members = ["child"]
        "#
    })?;

    let root_src = context.temp_dir.child("src").child("root");
    root_src.create_dir_all()?;
    root_src.child("__init__.py").write_str(indoc! {
        r"
        def main():
            import child
            print(child.MESSAGE)
        "
    })?;

    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;

    let child_src = child.child("src").child("child");
    child_src.create_dir_all()?;
    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--editable")
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/child)
     + root==0.1.0 (from file://[TEMP_DIR]/)
    Installed 1 executable: root_cli
    ");

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    OK
    ");

    let status = context
        .tool_install()
        .arg("--reinstall")
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .status()
        .expect("failed to run uv tool install --reinstall");
    assert!(status.success());

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'CHANGED'\n")?;

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    OK
    ");

    Ok(())
}

#[test]
fn tool_install_workspace_members_are_non_editable_by_default() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let root_pyproject = context.temp_dir.child("pyproject.toml");
    root_pyproject.write_str(indoc! {
        r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["child"]

        [project.scripts]
        root_cli = "root:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.sources]
        child = { workspace = true }

        [tool.uv.workspace]
        members = ["child"]
        "#
    })?;

    let root_src = context.temp_dir.child("src").child("root");
    root_src.create_dir_all()?;
    root_src.child("__init__.py").write_str(indoc! {
        r"
        def main():
            import child
            print(child.MESSAGE)
        "
    })?;

    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;

    let child_src = child.child("src").child("child");
    child_src.create_dir_all()?;
    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/child)
     + root==0.1.0 (from file://[TEMP_DIR]/)
    Installed 1 executable: root_cli
    ");

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    OK
    ");

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'CHANGED'\n")?;

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    OK
    ");

    Ok(())
}

#[test]
fn tool_install_workspace_members_honor_editable_flag() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let root_pyproject = context.temp_dir.child("pyproject.toml");
    root_pyproject.write_str(indoc! {
        r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["child"]

        [project.scripts]
        root_cli = "root:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.sources]
        child = { workspace = true }

        [tool.uv.workspace]
        members = ["child"]
        "#
    })?;

    let root_src = context.temp_dir.child("src").child("root");
    root_src.create_dir_all()?;
    root_src.child("__init__.py").write_str(indoc! {
        r"
        def main():
            import child
            print(child.MESSAGE)
        "
    })?;

    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;

    let child_src = child.child("src").child("child");
    child_src.create_dir_all()?;
    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--editable")
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/child)
     + root==0.1.0 (from file://[TEMP_DIR]/)
    Installed 1 executable: root_cli
    ");

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    OK
    ");

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'CHANGED'\n")?;

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    CHANGED
    ");

    Ok(())
}

#[test]
fn tool_install_workspace_members_honor_source_editable_flag() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let root_pyproject = context.temp_dir.child("pyproject.toml");
    root_pyproject.write_str(indoc! {
        r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["child"]

        [project.scripts]
        root_cli = "root:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.sources]
        child = { workspace = true, editable = true }

        [tool.uv.workspace]
        members = ["child"]
        "#
    })?;

    let root_src = context.temp_dir.child("src").child("root");
    root_src.create_dir_all()?;
    root_src.child("__init__.py").write_str(indoc! {
        r"
        ROOT_MESSAGE = 'ROOT'

        def main():
            import child
            print(f'{ROOT_MESSAGE} {child.MESSAGE}')
        "
    })?;

    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;

    let child_src = child.child("src").child("child");
    child_src.create_dir_all()?;
    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/child)
     + root==0.1.0 (from file://[TEMP_DIR]/)
    Installed 1 executable: root_cli
    ");

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    ROOT OK
    ");

    root_src.child("__init__.py").write_str(indoc! {
        r"
        ROOT_MESSAGE = 'CHANGED'

        def main():
            import child
            print(f'{ROOT_MESSAGE} {child.MESSAGE}')
        "
    })?;

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    ROOT OK
    ");

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'CHANGED'\n")?;

    uv_snapshot!(context.filters(), Command::new("root_cli").env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    ROOT CHANGED
    ");

    Ok(())
}

#[test]
fn tool_install_with_compatible_build_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.9")
        .with_exclude_newer("2024-05-04T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let constraints_txt = context.temp_dir.child("build_constraints.txt");
    constraints_txt.write_str("setuptools>=40")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--with")
        .arg("requests==1.2")
        .arg("--build-constraints")
        .arg("build_constraints.txt")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.4.2
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.1
     + requests==1.2.0
     + tomli==2.0.1
     + typing-extensions==4.11.0
    Installed 2 executables: black, blackd
    ");

    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [
            { name = "black" },
            { name = "requests", specifier = "==1.2" },
        ]
        build-constraint-dependencies = [{ name = "setuptools", specifier = ">=40" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-05-04T00:00:00Z"
        "#);
    });

    Ok(())
}

#[test]
fn tool_install_with_incompatible_build_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.9")
        .with_exclude_newer("2024-05-04T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let constraints_txt = context.temp_dir.child("build_constraints.txt");
    constraints_txt.write_str("setuptools==2")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--with")
        .arg("requests==1.2")
        .arg("--build-constraints")
        .arg("build_constraints.txt")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `requests==1.2.0`
      cause: Failed to resolve requirements from `setup.py` build
      cause: No solution found when resolving: `setuptools>=40.8.0`
      cause: Because you require setuptools>=40.8.0 and setuptools==2, we can conclude that your requirements are unsatisfiable.
    ");

    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::missing());

    Ok(())
}

#[test]
fn tool_install_suggest_other_packages_with_executable() {
    // FastAPI 0.111 is only available from this date onwards.
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2024-05-04T00:00:00Z")
        .with_filtered_exe_suffix()
        .with_filter(("\\+ uvloop(.+)\n ", ""))
        .with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("fastapi==0.111.0"), @"
    exit_code: 2 (failure)
    ----- stdout -----
    No executables are provided by package `fastapi`; removing tool

    ----- stderr -----
    Resolved 35 packages in [TIME]
    Prepared 35 packages in [TIME]
    Installed 35 packages in [TIME]
     + annotated-types==0.6.0
     + anyio==4.3.0
     + certifi==2024.2.2
     + click==8.1.7
     + dnspython==2.6.1
     + email-validator==2.1.1
     + fastapi==0.111.0
     + fastapi-cli==0.0.2
     + h11==0.14.0
     + httpcore==1.0.5
     + httptools==0.6.1
     + httpx==0.27.0
     + idna==3.7
     + jinja2==3.1.3
     + markdown-it-py==3.0.0
     + markupsafe==2.1.5
     + mdurl==0.1.2
     + orjson==3.10.3
     + pydantic==2.7.1
     + pydantic-core==2.18.2
     + pygments==2.17.2
     + python-dotenv==1.0.1
     + python-multipart==0.0.9
     + pyyaml==6.0.1
     + rich==13.7.1
     + shellingham==1.5.4
     + sniffio==1.3.1
     + starlette==0.37.2
     + typer==0.12.3
     + typing-extensions==4.11.0
     + ujson==5.9.0
     + uvicorn==0.29.0
     + watchfiles==0.21.0
     + websockets==12.0
    error: Failed to install entrypoints for `fastapi`

    hint: An executable with the name `fastapi` is available via dependency `fastapi-cli`.
          Did you mean `uv tool install fastapi-cli`?
    ");
}

/// Test installing a tool at a version
#[test]
fn tool_install_version() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.2.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.2.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/black/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from black import patched_main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(patched_main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", specifier = "==24.2.0" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("black").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    black, 24.2.0 (compiled: yes)
    Python (CPython) 3.12.[X]
    ");
}

/// Test an editable installation of a tool.
#[test]
fn tool_install_editable() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black` as an editable package.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/black_editable"))
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + black==0.1.0 (from file://[WORKSPACE]/test/packages/black_editable)
    Installed 1 executable: black
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(&executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/black/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from black import main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", editable = "[WORKSPACE]/test/packages/black_editable" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("black").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello world!
    ");

    // Request `black`. It should reinstall from the registry.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    Installed 1 executable: black
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Request `black` at a different version. It should install a new version.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--from")
        .arg("black==24.2.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 6 packages in [TIME]
     - black==0.1.0 (from file://[WORKSPACE]/test/packages/black_editable)
     + black==24.2.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", specifier = "==24.2.0" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });
}

/// An explicit local tool should be rebuilt after its dynamic metadata changes, including when
/// switching between editable and non-editable installs.
#[test]
fn tool_install_editable_rebuilds_explicit_local_directory() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let project = context.temp_dir.child("dynamic_tool");

    project.child("pyproject.toml").write_str(indoc! {r#"
        [build-system]
        requires = ["setuptools>=61"]
        build-backend = "setuptools.build_meta"

        [project]
        name = "dynamic-tool-demo"
        dynamic = ["version", "scripts"]
        requires-python = ">=3.11"
        "#
    })?;

    project.child("setup.py").write_str(indoc! {r#"
        from pathlib import Path
        from setuptools import setup

        root = Path(__file__).parent
        version = (root / "VERSION").read_text().strip()
        commands = [
            line.strip()
            for line in (root / "commands.txt").read_text().splitlines()
            if line.strip()
        ]

        setup(
            version=version,
            entry_points={
                "console_scripts": [
                    f"dynamic-tool-{command}=dynamic_tool.commands.{command}:main"
                    for command in commands
                ]
            },
        )
        "#
    })?;

    project.child("VERSION").write_str("0.1.0")?;
    project.child("commands.txt").write_str("alpha")?;
    project
        .child("src")
        .child("dynamic_tool")
        .child("__init__.py")
        .touch()?;
    project
        .child("src")
        .child("dynamic_tool")
        .child("commands")
        .child("__init__.py")
        .touch()?;
    project
        .child("src")
        .child("dynamic_tool")
        .child("commands")
        .child("alpha.py")
        .write_str(indoc! {r#"
            def main():
                print("alpha")
            "#
        })?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-e")
        .arg(project.path())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + dynamic-tool-demo==0.1.0 (from file://[TEMP_DIR]/dynamic_tool)
    Installed 1 executable: dynamic-tool-alpha
    ");

    project.child("VERSION").write_str("0.2.0")?;
    project.child("commands.txt").write_str("alpha\nbeta")?;
    project
        .child("src")
        .child("dynamic_tool")
        .child("commands")
        .child("beta.py")
        .write_str(indoc! {r#"
            def main():
                print("beta")
            "#
        })?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg(project.path())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - dynamic-tool-demo==0.1.0 (from file://[TEMP_DIR]/dynamic_tool)
     + dynamic-tool-demo==0.2.0 (from file://[TEMP_DIR]/dynamic_tool)
    Installed 2 executables: dynamic-tool-alpha, dynamic-tool-beta
    ");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-e")
        .arg(project.path())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ dynamic-tool-demo==0.2.0 (from file://[TEMP_DIR]/dynamic_tool)
    Installed 2 executables: dynamic-tool-alpha, dynamic-tool-beta
    ");

    Ok(())
}

/// Reinstalling an explicit local tool should use a newly selected global Python even when the
/// tool's source is unchanged.
#[test]
fn tool_install_explicit_local_directory_respects_global_python_change() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"])
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let project = context.temp_dir.child("project");

    project.child("pyproject.toml").write_str(indoc! {r#"
        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [project]
        name = "local-tool"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.scripts]
        local-tool = "local_tool:main"
        "#
    })?;
    project
        .child("src")
        .child("local_tool")
        .child("__init__.py")
        .write_str(indoc! {r#"
            import sys

            def main():
                print(f"{sys.version_info.major}.{sys.version_info.minor}")
            "#
        })?;

    context
        .python_pin()
        .arg("3.12")
        .arg("--global")
        .assert()
        .success();

    context
        .tool_install()
        .arg(project.path())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), Command::new("local-tool")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12
    ");

    context
        .python_pin()
        .arg("3.13")
        .arg("--global")
        .assert()
        .success();

    context
        .tool_install()
        .arg(project.path())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), Command::new("local-tool")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13
    ");

    Ok(())
}

/// A direct local `--with` requirement should be rebuilt on every invocation, while a local
/// requirement discovered through `--with-requirements` should retain its normal cache behavior.
#[test]
fn tool_install_rebuilds_explicit_local_with_requirement() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let project = context.temp_dir.child("project");
    let helper = context.temp_dir.child("helper");

    project.child("pyproject.toml").write_str(indoc! {r#"
        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [project]
        name = "local-tool"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.scripts]
        local-tool = "local_tool:main"
        "#
    })?;
    project
        .child("src")
        .child("local_tool")
        .child("__init__.py")
        .write_str(indoc! {r#"
            from importlib.metadata import version

            def main():
                print(version("local-helper"))
            "#
        })?;

    helper.child("pyproject.toml").write_str(indoc! {r#"
        [build-system]
        requires = ["setuptools>=61"]
        build-backend = "setuptools.build_meta"

        [project]
        name = "local-helper"
        dynamic = ["version"]
        requires-python = ">=3.12"
        "#
    })?;
    helper.child("setup.py").write_str(indoc! {r#"
        from pathlib import Path
        from setuptools import setup

        setup(version=(Path(__file__).parent / "VERSION").read_text().strip())
        "#
    })?;
    helper.child("VERSION").write_str("0.1.0")?;
    helper
        .child("src")
        .child("local_helper")
        .child("__init__.py")
        .touch()?;

    context
        .tool_install()
        .arg("--with")
        .arg(helper.path())
        .arg(project.path())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), Command::new("local-tool")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    0.1.0
    ");

    helper.child("VERSION").write_str("0.2.0")?;

    context
        .tool_install()
        .arg("--with")
        .arg(helper.path())
        .arg(project.path())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), Command::new("local-tool")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    0.2.0
    ");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!("{}\n", helper.path().display()))?;
    helper.child("VERSION").write_str("0.3.0")?;

    context
        .tool_install()
        .arg("--with-requirements")
        .arg(requirements_txt.path())
        .arg(project.path())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), Command::new("local-tool")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    0.2.0
    ");

    Ok(())
}

/// Ensure that we remove any existing entrypoints upon error.
#[test]
fn tool_install_remove_on_empty() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Request `black`. It should reinstall from the registry.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Install `black` as an editable package, but without any entrypoints.
    let black = context.temp_dir.child("black");
    fs_err::create_dir_all(black.path())?;

    let pyproject_toml = black.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "black"
        version = "0.1.0"
        description = "Black without any entrypoints"
        authors = []
        dependencies = []
        requires-python = ">=3.11,<3.13"

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#
    })?;

    let src = black.child("src").child("black");
    fs_err::create_dir_all(src.path())?;

    let init = src.child("__init__.py");
    init.touch()?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-e")
        .arg(black.path())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stdout -----
    No executables are provided by package `black`; removing tool

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 6 packages in [TIME]
    Installed 1 package in [TIME]
     - black==24.3.0
     + black==0.1.0 (from file://[TEMP_DIR]/black)
     - click==8.1.7
     - mypy-extensions==1.0.0
     - packaging==24.0
     - pathspec==0.12.1
     - platformdirs==4.2.0
    error: Failed to install entrypoints for `black`
    ");

    // Re-request `black`. It should reinstall, without requiring `--force`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    Ok(())
}

/// Test an editable installation of a tool using `--from`.
#[test]
fn tool_install_editable_from() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black` as an editable package.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("-e")
        .arg("--from")
        .arg(context.workspace_root.join("test/packages/black_editable"))
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + black==0.1.0 (from file://[WORKSPACE]/test/packages/black_editable)
    Installed 1 executable: black
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(&executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/black/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from black import main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", editable = "[WORKSPACE]/test/packages/black_editable" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("black").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello world!
    ");
}

/// Test installing a tool with `uv tool install --from`
#[test]
fn tool_install_from() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black` using `--from` to specify the version
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--from")
        .arg("black==24.2.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.2.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    // Attempt to install `black` using `--from` with a different package name
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--from")
        .arg("flask==24.2.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package name (`flask`) provided with `--from` does not match install request (`black`)
    ");

    // Attempt to install `black` using `--from` with a different version
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.2.0")
        .arg("--from")
        .arg("black==24.3.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package requirement (`black==24.3.0`) provided with `--from` conflicts with install request (`black==24.2.0`)
    ");
}

/// Test installing and reinstalling an already installed tool
#[test]
fn tool_install_already_installed() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/black/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from black import patched_main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(patched_main())
        "#);
    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Install `black` again
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    `black` is already installed
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    bin_dir
        .child(format!("black{}", std::env::consts::EXE_SUFFIX))
        .assert(predicate::path::exists());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should not have an additional tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Install `black` again with the `--reinstall` flag
    // We should recreate the entire environment and reinstall the entry points
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--reinstall")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     ~ black==24.3.0
     ~ click==8.1.7
     ~ mypy-extensions==1.0.0
     ~ packaging==24.0
     ~ pathspec==0.12.1
     ~ platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    // Install `black` again with `--reinstall-package` for `black`
    // We should reinstall `black` in the environment and reinstall the entry points
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--reinstall-package")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     ~ black==24.3.0
    Installed 2 executables: black, blackd
    ");

    // Install `black` again with `--reinstall-package` for a dependency
    // We should reinstall `click` in the environment but not reinstall `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--reinstall-package")
        .arg("click")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     ~ click==8.1.7
    Installed 2 executables: black, blackd
    ");
}

#[test]
fn tool_install_restores_missing_executables() -> Result<()> {
    for preview in [None, Some("tool-install-locks")] {
        let context = uv_test::test_context!("3.13").with_filtered_exe_suffix();
        let context = if let Some(preview) = preview {
            context.with_env(EnvVars::UV_PREVIEW_FEATURES, preview)
        } else {
            context
        };
        let tool_dir = context.temp_dir.child("tools");
        let first_bin_dir = context.temp_dir.child("first-bin");
        let second_bin_dir = context.temp_dir.child("second-bin");
        let third_bin_dir = context.temp_dir.child("third-bin");
        let launcher = context
            .workspace_root
            .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");
        let app = context
            .workspace_root
            .join("test/links/basic_app-0.1.0-py3-none-any.whl");
        let app_requirement = format!(
            "basic-app @ {}",
            Url::from_file_path(&app).expect("Failed to convert app path to file URL")
        );

        context
            .tool_install()
            .arg(&launcher)
            .arg("--with-executables-from")
            .arg(&app_requirement)
            .arg("--offline")
            .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
            .env(EnvVars::UV_TOOL_BIN_DIR, first_bin_dir.as_os_str())
            .env(EnvVars::PATH, first_bin_dir.as_os_str())
            .assert()
            .success();

        let environment = tool_dir.child("simple-launcher");
        let receipt = environment.child("uv-receipt.toml");
        let original_receipt = fs_err::read(receipt.path())?;
        let original_config = fs_err::read(environment.child("pyvenv.cfg"))?;
        let site_packages = site_packages_path(environment.path(), "python3.13");
        let original_packages = dirhash_path(&site_packages)?;
        let source_paths = ["simple_launcher", "basic-app"].map(|name| {
            venv_bin_path(environment.path())
                .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
        });
        let source_contents = source_paths
            .iter()
            .map(fs_err::read)
            .collect::<std::io::Result<Vec<_>>>()?;
        let source_modified = source_paths
            .iter()
            .map(|path| fs_err::metadata(path)?.modified())
            .collect::<std::io::Result<Vec<_>>>()?;
        #[cfg(unix)]
        let source_identity = source_paths
            .iter()
            .map(|path| fs_err::metadata(path).map(|metadata| (metadata.dev(), metadata.ino())))
            .collect::<std::io::Result<Vec<_>>>()?;
        environment
            .child("recovery-sentinel")
            .write_str("existing environment")?;
        let check = |bin: &Path| -> Result<()> {
            for ((name, expected), source) in [
                ("simple_launcher", "Hi from the simple launcher!\n"),
                ("basic-app", "Hello from basic-app!\n"),
            ]
            .into_iter()
            .zip(&source_paths)
            {
                let executable = bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
                Command::new(&executable)
                    .env("PYTHONDONTWRITEBYTECODE", "1")
                    .assert()
                    .success()
                    .stdout(expected);
                assert_eq!(fs_err::read(&executable)?, fs_err::read(source)?);
                #[cfg(unix)]
                {
                    assert!(fs_err::symlink_metadata(&executable)?.is_symlink());
                    assert_eq!(
                        fs_err::canonicalize(&executable)?,
                        fs_err::canonicalize(source)?
                    );
                }
                #[cfg(windows)]
                assert!(!fs_err::symlink_metadata(&executable)?.is_symlink());
            }
            assert_eq!(
                fs_err::read(environment.child("pyvenv.cfg"))?,
                original_config
            );
            assert_eq!(dirhash_path(&site_packages)?, original_packages);
            assert_eq!(
                fs_err::read_to_string(environment.child("recovery-sentinel"))?,
                "existing environment"
            );
            for ((source, contents), modified) in source_paths
                .iter()
                .zip(&source_contents)
                .zip(&source_modified)
            {
                assert_eq!(fs_err::read(source)?, *contents);
                assert_eq!(fs_err::metadata(source)?.modified()?, *modified);
            }
            #[cfg(unix)]
            for (source, identity) in source_paths.iter().zip(&source_identity) {
                let metadata = fs_err::metadata(source)?;
                assert_eq!((metadata.dev(), metadata.ino()), *identity);
            }
            let mut current: toml::Value =
                toml::from_str(&fs_err::read_to_string(receipt.path())?)?;
            let mut original: toml::Value =
                toml::from_str(std::str::from_utf8(&original_receipt)?)?;
            let current_entrypoints = current["tool"]
                .as_table_mut()
                .expect("tool table")
                .remove("entrypoints")
                .expect("entrypoints");
            let original_entrypoints = original["tool"]
                .as_table_mut()
                .expect("tool table")
                .remove("entrypoints")
                .expect("entrypoints");
            assert_eq!(current, original);
            assert_eq!(
                current_entrypoints
                    .as_array()
                    .expect("entrypoint array")
                    .len(),
                2
            );
            assert_eq!(
                original_entrypoints
                    .as_array()
                    .expect("entrypoint array")
                    .len(),
                2
            );
            for (current, original) in current_entrypoints
                .as_array()
                .expect("entrypoint array")
                .iter()
                .zip(original_entrypoints.as_array().expect("entrypoint array"))
            {
                let mut current = current.as_table().expect("entrypoint table").clone();
                let mut original = original.as_table().expect("entrypoint table").clone();
                let path = current.remove("install-path").expect("install path");
                original.remove("install-path");
                assert_eq!(
                    Path::new(path.as_str().expect("path string")).parent(),
                    Some(bin)
                );
                assert_eq!(current, original);
            }
            Ok(())
        };
        check(first_bin_dir.path())?;

        let launcher_executable =
            first_bin_dir.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
        let app_executable =
            first_bin_dir.child(format!("basic-app{}", std::env::consts::EXE_SUFFIX));
        fs_err::remove_file(&launcher_executable)?;
        fs_err::remove_file(&app_executable)?;

        context
            .tool_install()
            .arg(&launcher)
            .arg("--with-executables-from")
            .arg(&app_requirement)
            .arg("--offline")
            .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
            .env(EnvVars::UV_TOOL_BIN_DIR, first_bin_dir.as_os_str())
            .env(EnvVars::PATH, first_bin_dir.as_os_str())
            .assert()
            .success()
            .stderr(predicate::str::contains(
                "Restored 2 executables: basic-app, simple_launcher",
            ));

        check(first_bin_dir.path())?;
        assert_eq!(fs_err::read(receipt.path())?, original_receipt);
        fs_err::remove_dir_all(first_bin_dir.path())?;

        context
            .tool_upgrade()
            .arg("simple-launcher")
            .arg("--offline")
            .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
            .env(EnvVars::UV_TOOL_BIN_DIR, first_bin_dir.as_os_str())
            .env(EnvVars::PATH, first_bin_dir.as_os_str())
            .assert()
            .success()
            .stderr(predicate::str::contains(
                "Restored 2 executables: basic-app, simple_launcher",
            ));

        check(first_bin_dir.path())?;

        context
            .tool_install()
            .arg(&launcher)
            .arg("--with-executables-from")
            .arg(&app_requirement)
            .arg("--offline")
            .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
            .env(EnvVars::UV_TOOL_BIN_DIR, second_bin_dir.as_os_str())
            .env(EnvVars::PATH, second_bin_dir.as_os_str())
            .assert()
            .success()
            .stderr(predicate::str::contains(
                "Restored 2 executables: basic-app, simple_launcher",
            ));

        check(second_bin_dir.path())?;
        launcher_executable.assert(predicate::path::missing());
        app_executable.assert(predicate::path::missing());

        context
            .tool_upgrade()
            .arg("simple-launcher")
            .arg("--offline")
            .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
            .env(EnvVars::UV_TOOL_BIN_DIR, third_bin_dir.as_os_str())
            .env(EnvVars::PATH, third_bin_dir.as_os_str())
            .assert()
            .success()
            .stderr(predicate::str::contains(
                "Restored 2 executables: basic-app, simple_launcher",
            ));
        check(third_bin_dir.path())?;
        for name in ["simple_launcher", "basic-app"] {
            second_bin_dir
                .child(format!("{name}{}", std::env::consts::EXE_SUFFIX))
                .assert(predicate::path::missing());
        }
    }

    Ok(())
}

/// Recovery must check all destinations before replacing an unrelated executable.
#[test]
fn tool_install_recovery_preflights_existing_executables() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let links = context.workspace_root.join("test/links");
    let install = || {
        let mut command = context.tool_install();
        command
            .arg("simple-launcher==0.1.0")
            .arg("--with-executables-from")
            .arg("basic-app==0.1.0")
            .arg("--no-index")
            .arg("--find-links")
            .arg(&links)
            .env(EnvVars::PATH, bin_dir.as_os_str());
        command
    };
    install().assert().success();
    let environment = tool_dir.child("simple-launcher");
    let receipt = environment.child("uv-receipt.toml");
    let receipt_contents = fs_err::read(receipt.path())?;
    let site_packages = site_packages_path(environment.path(), "python3.13");
    let installed_contents = dirhash_path(&site_packages)?;
    let app = bin_dir.child(format!("basic-app{}", std::env::consts::EXE_SUFFIX));
    let launcher = bin_dir.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    fs_err::remove_file(app.path())?;
    fs_err::remove_file(launcher.path())?;
    launcher.write_str("unrelated executable bytes")?;
    install()
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Executable already exists:"));
    app.assert(predicate::path::missing());
    assert_eq!(
        fs_err::read_to_string(launcher.path())?,
        "unrelated executable bytes"
    );
    assert_eq!(fs_err::read(receipt.path())?, receipt_contents);
    assert_eq!(dirhash_path(&site_packages)?, installed_contents);

    install().arg("--force").assert().success();
    Command::new(launcher.path())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .assert()
        .success()
        .stdout("Hi from the simple launcher!\n");
    Command::new(app.path())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .assert()
        .success()
        .stdout("Hello from basic-app!\n");
    assert_eq!(fs_err::read(receipt.path())?, receipt_contents);
    assert_eq!(dirhash_path(&site_packages)?, installed_contents);
    Ok(())
}

/// A stale receipt does not give recovery authority over another tool's exported command.
#[test]
fn tool_install_recovery_preserves_transferred_executables() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let second_bin_dir = context.temp_dir.child("second-bin");
    let links = context.workspace_root.join("test/links");
    let install = || {
        let mut command = context.tool_install();
        command
            .arg("simple-launcher==0.1.0")
            .arg("--with-executables-from")
            .arg("basic-app==0.1.0")
            .arg("--no-index")
            .arg("--find-links")
            .arg(&links)
            .env(EnvVars::PATH, bin_dir.as_os_str());
        command
    };
    install().assert().success();
    context
        .tool_install()
        .arg("basic-app==0.1.0")
        .arg("--with-executables-from")
        .arg("simple-launcher==0.1.0")
        .arg("--force")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links)
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();
    let launcher = bin_dir.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    let app = bin_dir.child(format!("basic-app{}", std::env::consts::EXE_SUFFIX));
    let owner_source = venv_bin_path(tool_dir.child("basic-app").path())
        .join(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(fs_err::read(launcher.path())?, fs_err::read(&owner_source)?);
    #[cfg(unix)]
    assert_eq!(
        fs_err::canonicalize(launcher.path())?,
        fs_err::canonicalize(&owner_source)?
    );
    let launcher_contents = fs_err::read(launcher.path())?;
    let receipts =
        ["simple-launcher", "basic-app"].map(|name| tool_dir.child(name).child("uv-receipt.toml"));
    let receipt_contents = receipts
        .iter()
        .map(|path| fs_err::read(path.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let package_paths = ["simple-launcher", "basic-app"]
        .map(|name| site_packages_path(tool_dir.child(name).path(), "python3.13"));
    let packages = package_paths
        .iter()
        .map(|path| dirhash_path(path))
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // An otherwise-fresh install does not reclaim the command from its current owner.
    install().assert().success();
    assert_eq!(fs_err::read(launcher.path())?, launcher_contents);
    fs_err::remove_file(app.path())?;
    for force in [false, true] {
        let mut command = install();
        if force {
            command.arg("--force");
        }
        command.assert().code(2).stderr(predicate::str::contains(
            "because it is also recorded for `basic-app`",
        ));
    }
    context
        .tool_upgrade()
        .arg("simple-launcher")
        .arg("--offline")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "because it is also recorded for `basic-app`",
        ));
    app.assert(predicate::path::missing());
    assert_eq!(fs_err::read(launcher.path())?, launcher_contents);
    for (receipt, contents) in receipts.iter().zip(&receipt_contents) {
        assert_eq!(fs_err::read(receipt.path())?, *contents);
    }
    for (path, contents) in package_paths.iter().zip(&packages) {
        assert_eq!(dirhash_path(path)?, *contents);
    }

    // A distinct bin directory is a new destination; the old current owner is left alone.
    install()
        .env(EnvVars::UV_TOOL_BIN_DIR, second_bin_dir.as_os_str())
        .env(EnvVars::PATH, second_bin_dir.as_os_str())
        .assert()
        .success();
    assert_eq!(fs_err::read(launcher.path())?, launcher_contents);
    app.assert(predicate::path::missing());
    assert_eq!(fs_err::read(receipts[1].path())?, receipt_contents[1]);
    for (name, expected) in [
        ("simple_launcher", "Hi from the simple launcher!\n"),
        ("basic-app", "Hello from basic-app!\n"),
    ] {
        let executable = second_bin_dir.child(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        Command::new(executable.path())
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .assert()
            .success()
            .stdout(expected);
    }
    Command::new(launcher.path())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .assert()
        .success()
        .stdout("Hi from the simple launcher!\n");
    for (path, contents) in package_paths.iter().zip(&packages) {
        assert_eq!(dirhash_path(path)?, *contents);
    }
    Ok(())
}

/// A tool without recorded exports must not reacquire commands during no-op recovery.
#[test]
fn tool_install_recovery_preserves_empty_entrypoints() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let second_bin_dir = context.temp_dir.child("second-bin");
    let environment = context.temp_dir.child("tools").child("simple-launcher");
    let wheel = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");
    context
        .tool_install()
        .arg(&wheel)
        .arg("--offline")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();
    let receipt = environment.child("uv-receipt.toml");
    let mut document = fs_err::read_to_string(receipt.path())?.parse::<toml_edit::DocumentMut>()?;
    document["tool"]["entrypoints"] = toml_edit::value(toml_edit::Array::new());
    receipt.write_str(&document.to_string())?;
    let contents = fs_err::read(receipt.path())?;
    fs_err::remove_file(bin_dir.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX)))?;
    let site_packages = site_packages_path(environment.path(), "python3.13");
    let installed = dirhash_path(&site_packages)?;
    context
        .tool_install()
        .arg(&wheel)
        .arg("--offline")
        .env(EnvVars::UV_TOOL_BIN_DIR, second_bin_dir.as_os_str())
        .assert()
        .success();
    context
        .tool_upgrade()
        .arg("simple-launcher")
        .arg("--offline")
        .env(EnvVars::UV_TOOL_BIN_DIR, second_bin_dir.as_os_str())
        .assert()
        .success();
    second_bin_dir.assert(predicate::path::missing());
    assert_eq!(fs_err::read(receipt.path())?, contents);
    let source = venv_bin_path(environment.path())
        .join(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    Command::new(source)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .assert()
        .success()
        .stdout("Hi from the simple launcher!\n");
    assert_eq!(dirhash_path(&site_packages)?, installed);
    Ok(())
}

#[cfg(windows)]
#[test]
fn tool_install_recovery_handles_bin_directory_aliases() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let alias = context.temp_dir.child("BIN");
    let links = context.workspace_root.join("test/links");
    let install = || {
        let mut command = context.tool_install();
        command
            .arg("simple-launcher==0.1.0")
            .arg("--no-index")
            .arg("--find-links")
            .arg(&links)
            .env(EnvVars::PATH, bin_dir.as_os_str());
        command
    };
    install().assert().success();
    assert_eq!(
        uv_fs::is_same_file_allow_missing(bin_dir.path(), alias.path()),
        Some(true)
    );
    let executable = bin_dir.child("simple_launcher.exe");
    let contents = fs_err::read(executable.path())?;
    install()
        .env(EnvVars::UV_TOOL_BIN_DIR, alias.as_os_str())
        .assert()
        .success();
    assert_eq!(fs_err::read(executable.path())?, contents);
    Command::new(executable.path())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .assert()
        .success()
        .stdout("Hi from the simple launcher!\n");

    context
        .tool_install()
        .arg("basic-app==0.1.0")
        .arg("--with-executables-from")
        .arg("simple-launcher==0.1.0")
        .arg("--force")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links)
        .env(EnvVars::UV_TOOL_BIN_DIR, alias.as_os_str())
        .assert()
        .success();
    let receipts = ["simple-launcher", "basic-app"].map(|name| {
        context
            .temp_dir
            .child("tools")
            .child(name)
            .child("uv-receipt.toml")
    });
    let receipt_contents = receipts
        .iter()
        .map(|path| fs_err::read(path.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    fs_err::remove_dir_all(bin_dir.path())?;
    install().assert().code(2).stderr(predicate::str::contains(
        "Cannot compare missing executable directories",
    ));
    bin_dir.assert(predicate::path::missing());
    for (receipt, contents) in receipts.iter().zip(&receipt_contents) {
        assert_eq!(fs_err::read(receipt.path())?, *contents);
    }
    Ok(())
}

fn write_recovery_wheel(
    directory: &Path,
    name: &str,
    version: &str,
    requirements: &[&str],
    commands: &[(&str, &str)],
) -> Result<PathBuf> {
    let normalized = name.replace('-', "_");
    let entrypoints_path = format!("{normalized}-{version}.dist-info/entry_points.txt");
    let module_path = format!("{normalized}/commands.py");
    let mut entrypoints = String::from("[console_scripts]\n");
    let mut module = String::new();
    for (index, (command, output)) in commands.iter().enumerate() {
        entrypoints.push_str(&format!(
            "{command} = {normalized}.commands:command_{index}\n"
        ));
        module.push_str(&format!("def command_{index}():\n    print({output:?})\n"));
    }
    let requirements = requirements
        .iter()
        .map(|requirement| requirement.parse())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let (filename, wheel) = generate_wheel_with_files(
        &name.parse()?,
        &version.parse()?,
        &requirements,
        &Default::default(),
        None,
        "py3-none-any",
        &[(&entrypoints_path, &entrypoints), (&module_path, &module)],
    );
    let path = directory.join(filename);
    fs_err::write(&path, wheel)?;
    Ok(path)
}

/// Changed dependencies, root versions, and interpreters all use the original export authority.
#[test]
fn tool_install_recovery_survives_environment_updates() -> Result<()> {
    for preview in [None, Some("tool-install-locks")] {
        let context = uv_test::test_context_with_versions!(&["3.13", "3.12"]).with_tool_dirs();
        let context = if let Some(preview) = preview {
            context.with_env(EnvVars::UV_PREVIEW_FEATURES, preview)
        } else {
            context
        };
        let links = context.temp_dir.child("links");
        links.create_dir_all()?;
        let bins = (0..6)
            .map(|index| context.temp_dir.child(format!("bin-{index}")))
            .collect::<Vec<_>>();
        let environment = context.temp_dir.child("tools").child("recovery-root");
        let receipt = environment.child("uv-receipt.toml");
        write_recovery_wheel(
            links.path(),
            "recovery-dep",
            "1.0.0",
            &[],
            &[("recovery-dep", "dep-1")],
        )?;
        write_recovery_wheel(
            links.path(),
            "recovery-root",
            "1.0.0",
            &["recovery-dep>=1"],
            &[("recovery-root", "root-1"), ("recovery-old", "old-1")],
        )?;
        let install = |bin: &Path| {
            let mut command = context.tool_install();
            command
                .arg("recovery-root")
                .args([
                    "--with-executables-from",
                    "recovery-dep",
                    "--no-index",
                    "--find-links",
                ])
                .arg(links.path())
                .env(EnvVars::UV_TOOL_BIN_DIR, bin)
                .env(EnvVars::PATH, bin);
            command
        };
        let upgrade = |bin: &Path| {
            let mut command = context.tool_upgrade();
            command
                .arg("recovery-root")
                .args(["--no-index", "--find-links"])
                .arg(links.path())
                .env(EnvVars::UV_TOOL_BIN_DIR, bin)
                .env(EnvVars::PATH, bin);
            command
        };
        let exported =
            |bin: &Path, name: &str| bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let run = |bin: &Path, name: &str, output: &str| {
            Command::new(exported(bin, name))
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .assert()
                .success()
                .stdout(format!("{output}\n"));
        };
        let assert_receipt = |bin: &Path, names: &[&str]| -> Result<()> {
            let document =
                fs_err::read_to_string(receipt.path())?.parse::<toml_edit::DocumentMut>()?;
            let entries = document["tool"]["entrypoints"]
                .as_array()
                .expect("entrypoint array");
            assert_eq!(entries.len(), names.len());
            for name in names {
                let target = exported(bin, name);
                assert!(
                    entries
                        .iter()
                        .any(|entry| entry.as_inline_table().is_some_and(|entry| {
                            entry.get("install-path").and_then(toml_edit::Value::as_str)
                                == target.to_str()
                        }))
                );
            }
            Ok(())
        };

        install(bins[0].path())
            .args(["--python", "3.13"])
            .assert()
            .success();
        let sentinel = environment.child("preserve-unless-replaced");
        sentinel.write_str("retained environment")?;
        let root_metadata = site_packages_path(environment.path(), "python3.13")
            .join("recovery_root-1.0.0.dist-info/METADATA");
        let root_metadata_before = fs_err::read(&root_metadata)?;
        fs_err::remove_file(exported(bins[0].path(), "recovery-dep"))?;
        write_recovery_wheel(
            links.path(),
            "recovery-dep",
            "2.0.0",
            &[],
            &[("recovery-dep", "dep-2")],
        )?;
        upgrade(bins[1].path()).assert().success();
        assert_eq!(fs_err::read(&root_metadata)?, root_metadata_before);
        sentinel.assert("retained environment");
        run(bins[1].path(), "recovery-dep", "dep-2");
        run(bins[1].path(), "recovery-root", "root-1");
        assert!(!exported(bins[0].path(), "recovery-root").exists());
        assert_receipt(
            bins[1].path(),
            &["recovery-dep", "recovery-root", "recovery-old"],
        )?;

        write_recovery_wheel(
            links.path(),
            "recovery-root",
            "2.0.0",
            &["recovery-dep>=1"],
            &[("recovery-root", "root-2"), ("recovery-new", "new-2")],
        )?;
        upgrade(bins[2].path()).assert().success();
        sentinel.assert("retained environment");
        run(bins[2].path(), "recovery-root", "root-2");
        run(bins[2].path(), "recovery-new", "new-2");
        assert!(!exported(bins[1].path(), "recovery-old").exists());
        assert!(!exported(bins[2].path(), "recovery-old").exists());
        assert_receipt(
            bins[2].path(),
            &["recovery-dep", "recovery-root", "recovery-new"],
        )?;

        install(bins[3].path())
            .arg("--reinstall")
            .assert()
            .success();
        sentinel.assert("retained environment");
        run(bins[3].path(), "recovery-root", "root-2");
        assert!(!exported(bins[2].path(), "recovery-root").exists());
        assert_receipt(
            bins[3].path(),
            &["recovery-dep", "recovery-root", "recovery-new"],
        )?;

        install(bins[4].path()).arg("--force").assert().success();
        sentinel.assert(predicate::path::missing());
        run(bins[4].path(), "recovery-root", "root-2");
        assert!(!exported(bins[3].path(), "recovery-root").exists());
        assert_receipt(
            bins[4].path(),
            &["recovery-dep", "recovery-root", "recovery-new"],
        )?;

        sentinel.write_str("replace interpreter")?;
        upgrade(bins[5].path())
            .args(["--python", "3.12"])
            .assert()
            .success();
        sentinel.assert(predicate::path::missing());
        run(bins[5].path(), "recovery-root", "root-2");
        run(bins[5].path(), "recovery-dep", "dep-2");
        assert!(!exported(bins[4].path(), "recovery-root").exists());
        assert_receipt(
            bins[5].path(),
            &["recovery-dep", "recovery-root", "recovery-new"],
        )?;
        Command::new(
            venv_bin_path(environment.path())
                .join(format!("python{}", std::env::consts::EXE_SUFFIX)),
        )
        .args([
            "-c",
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')",
        ])
        .assert()
        .success()
        .stdout("3.12\n");
    }
    Ok(())
}

#[cfg(windows)]
#[test]
fn tool_install_recovery_rejects_case_only_competing_claims() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let tools = context.temp_dir.child("tools");
    let bin = context.temp_dir.child("bin");
    let links = context.workspace_root.join("test/links");
    let install = || {
        let mut command = context.tool_install();
        command
            .args(["simple-launcher==0.1.0", "--no-index", "--find-links"])
            .arg(&links)
            .env(EnvVars::PATH, bin.as_os_str());
        command
    };
    install().assert().success();
    context
        .tool_install()
        .args([
            "basic-app==0.1.0",
            "--with-executables-from",
            "simple-launcher==0.1.0",
            "--force",
            "--no-index",
            "--find-links",
        ])
        .arg(&links)
        .env(EnvVars::PATH, bin.as_os_str())
        .assert()
        .success();
    let launcher = bin.child("simple_launcher.exe");
    let upper = bin.child("SIMPLE_LAUNCHER.exe");
    assert_eq!(
        uv_fs::is_same_file_allow_missing(launcher.path(), upper.path()),
        Some(true)
    );
    let owner_receipt = tools.child("basic-app").child("uv-receipt.toml");
    let mut document =
        fs_err::read_to_string(owner_receipt.path())?.parse::<toml_edit::DocumentMut>()?;
    let mut changed = 0;
    for entry in document["tool"]["entrypoints"]
        .as_array_mut()
        .expect("entrypoint array")
        .iter_mut()
    {
        let entry = entry.as_inline_table_mut().expect("entrypoint table");
        if entry.get("name").and_then(toml_edit::Value::as_str) == Some("simple_launcher") {
            changed += 1;
            entry.insert(
                "install-path",
                toml_edit::Value::from(upper.path().to_str().expect("UTF-8 test path")),
            );
        }
    }
    assert_eq!(changed, 1);
    owner_receipt.write_str(&document.to_string())?;
    let receipts = [
        tools.child("simple-launcher").child("uv-receipt.toml"),
        owner_receipt,
    ];
    let before = receipts
        .iter()
        .map(|receipt| fs_err::read(receipt.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let environments = [tools.child("simple-launcher"), tools.child("basic-app")];
    let package_paths = environments
        .iter()
        .map(|path| site_packages_path(path.path(), "python3.13"))
        .collect::<Vec<_>>();
    let packages = package_paths
        .iter()
        .map(|path| dirhash_path(path))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for present in [true, false] {
        if !present {
            fs_err::remove_file(launcher.path())?;
        }
        install()
            .arg("--force")
            .assert()
            .code(2)
            .stderr(predicate::str::contains(
                "because it is also recorded for `basic-app`",
            ));
        if !present {
            install().assert().code(2).stderr(predicate::str::contains(
                "because it is also recorded for `basic-app`",
            ));
            context
                .tool_upgrade()
                .args(["simple-launcher", "--offline"])
                .assert()
                .code(1)
                .stderr(predicate::str::contains(
                    "because it is also recorded for `basic-app`",
                ));
        }
        for (receipt, contents) in receipts.iter().zip(&before) {
            assert_eq!(fs_err::read(receipt.path())?, *contents);
        }
        for (path, contents) in package_paths.iter().zip(&packages) {
            assert_eq!(dirhash_path(path)?, *contents);
        }
    }
    launcher.assert(predicate::path::missing());
    Ok(())
}

/// A newly introduced command is checked before any export is changed.
#[test]
fn tool_install_recovery_preflights_new_commands() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let bin = context.temp_dir.child("bin");
    let tools = context.temp_dir.child("tools");
    let original_wheel = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");
    context
        .tool_install()
        .arg(original_wheel)
        .arg("--offline")
        .assert()
        .success();
    write_recovery_wheel(
        links.path(),
        "recovery-root",
        "1.0.0",
        &[],
        &[("recovery-root", "root-1")],
    )?;
    context
        .tool_install()
        .args(["recovery-root", "--no-index", "--find-links"])
        .arg(links.path())
        .assert()
        .success();
    let root_export = bin.child(format!("recovery-root{}", std::env::consts::EXE_SUFFIX));
    let foreign_export = bin.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    let root_bytes = fs_err::read(root_export.path())?;
    let foreign_bytes = fs_err::read(foreign_export.path())?;
    #[cfg(unix)]
    let root_identity = {
        let metadata = fs_err::symlink_metadata(root_export.path())?;
        (
            metadata.dev(),
            metadata.ino(),
            fs_err::read_link(root_export.path())?,
        )
    };
    let receipts =
        ["recovery-root", "simple-launcher"].map(|name| tools.child(name).child("uv-receipt.toml"));
    let receipt_bytes = receipts
        .iter()
        .map(|path| fs_err::read(path.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let foreign_packages = site_packages_path(tools.child("simple-launcher").path(), "python3.13");
    let foreign_package_bytes = dirhash_path(&foreign_packages)?;
    write_recovery_wheel(
        links.path(),
        "recovery-root",
        "2.0.0",
        &[],
        &[
            ("recovery-root", "root-2"),
            ("simple_launcher", "not the owner"),
        ],
    )?;
    context
        .tool_upgrade()
        .args(["recovery-root", "--no-index", "--find-links"])
        .arg(links.path())
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "because it is also recorded for `simple-launcher`",
        ));
    assert_eq!(fs_err::read(root_export.path())?, root_bytes);
    assert_eq!(fs_err::read(foreign_export.path())?, foreign_bytes);
    #[cfg(unix)]
    {
        let metadata = fs_err::symlink_metadata(root_export.path())?;
        assert_eq!(
            (
                metadata.dev(),
                metadata.ino(),
                fs_err::read_link(root_export.path())?
            ),
            root_identity
        );
    }
    for (receipt, bytes) in receipts.iter().zip(&receipt_bytes) {
        assert_eq!(fs_err::read(receipt.path())?, *bytes);
    }
    assert_eq!(dirhash_path(&foreign_packages)?, foreign_package_bytes);
    Command::new(foreign_export.path())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .assert()
        .success()
        .stdout("Hi from the simple launcher!\n");
    // Package mutation is not rolled back by the export preflight.
    assert!(
        site_packages_path(tools.child("recovery-root").path(), "python3.13")
            .join("recovery_root-2.0.0.dist-info/METADATA")
            .exists()
    );
    Ok(())
}

#[test]
fn tool_install_recovery_does_not_reacquire_pruned_commands() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let bin = context.temp_dir.child("bin");
    let environment = context.temp_dir.child("tools").child("recovery-root");
    let receipt = environment.child("uv-receipt.toml");
    write_recovery_wheel(
        links.path(),
        "recovery-root",
        "1.0.0",
        &[],
        &[("recovery-root", "root-1"), ("recovery-pruned", "pruned-1")],
    )?;
    let install = || {
        let mut command = context.tool_install();
        command
            .args(["recovery-root", "--no-index", "--find-links"])
            .arg(links.path());
        command
    };
    install().assert().success();
    let pruned = bin.child(format!("recovery-pruned{}", std::env::consts::EXE_SUFFIX));
    let mut document = fs_err::read_to_string(receipt.path())?.parse::<toml_edit::DocumentMut>()?;
    let entries = document["tool"]["entrypoints"]
        .as_array_mut()
        .expect("entrypoint array");
    let index = entries
        .iter()
        .position(|entry| {
            entry
                .as_inline_table()
                .and_then(|entry| entry.get("name"))
                .and_then(toml_edit::Value::as_str)
                == Some("recovery-pruned")
        })
        .expect("pruned command");
    entries.remove(index);
    receipt.write_str(&document.to_string())?;
    fs_err::remove_file(pruned.path())?;
    write_recovery_wheel(
        links.path(),
        "recovery-root",
        "2.0.0",
        &[],
        &[
            ("recovery-root", "root-2"),
            ("recovery-pruned", "pruned-2"),
            ("recovery-new", "new-2"),
        ],
    )?;
    context
        .tool_upgrade()
        .args(["recovery-root", "--no-index", "--find-links"])
        .arg(links.path())
        .assert()
        .success();
    for argument in ["--reinstall", "--force"] {
        install().arg(argument).assert().success();
    }
    pruned.assert(predicate::path::missing());
    let document = fs_err::read_to_string(receipt.path())?.parse::<toml_edit::DocumentMut>()?;
    let entries = document["tool"]["entrypoints"]
        .as_array()
        .expect("entrypoint array");
    assert_eq!(entries.len(), 2);
    assert!(!entries.iter().any(|entry| {
        entry
            .as_inline_table()
            .and_then(|entry| entry.get("name"))
            .and_then(toml_edit::Value::as_str)
            == Some("recovery-pruned")
    }));
    for (name, output) in [("recovery-root", "root-2\n"), ("recovery-new", "new-2\n")] {
        Command::new(
            bin.child(format!("{name}{}", std::env::consts::EXE_SUFFIX))
                .path(),
        )
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .assert()
        .success()
        .stdout(output);
    }
    Ok(())
}

#[cfg(windows)]
struct NativePeFixture {
    path: PathBuf,
    bytes: Vec<u8>,
    sha256: String,
    machine: u16,
}

#[cfg(windows)]
#[expect(unsafe_code)]
fn native_system_directory() -> Result<PathBuf> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "GetSystemDirectoryW"]
        fn get_system_directory(buffer: *mut u16, size: u32) -> u32;
    }
    let mut buffer = vec![0_u16; 32768];
    // SAFETY: The writable buffer contains `size` initialized UTF-16 elements and remains alive
    // throughout the synchronous Windows API call.
    let length = unsafe { get_system_directory(buffer.as_mut_ptr(), u32::try_from(buffer.len())?) };
    if length == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let length = usize::try_from(length)?;
    anyhow::ensure!(
        length < buffer.len(),
        "Native system directory exceeds the fixture bound"
    );
    Ok(PathBuf::from(OsString::from_wide(&buffer[..length])))
}

#[cfg(windows)]
#[expect(unsafe_code)]
fn observed_short_path(path: &Path, assigned_name: Option<&str>) -> Result<PathBuf> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "GetShortPathNameW"]
        fn get_short_path_name(path: *const u16, buffer: *mut u16, size: u32) -> u32;
    }
    let parent = path.parent().expect("test export parent");
    anyhow::ensure!(
        !uv_windows::directory_is_case_sensitive(&uv_windows::open_directory(parent)?)?,
        "The short-name fixture requires a case-insensitive directory"
    );
    // Provision only this owned file. A failed short-name precondition is a failed native gate.
    if let Some(assigned_name) = assigned_name {
        Command::new(native_system_directory()?.join("fsutil.exe"))
            .args(["file", "setshortname"])
            .arg(path)
            .arg(assigned_name)
            .assert()
            .success();
    }
    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    anyhow::ensure!(!wide.contains(&0), "NUL in short-name fixture path");
    wide.push(0);
    let mut buffer = vec![0_u16; 32768];
    // SAFETY: The input is NUL-terminated, and the output has the declared initialized length.
    let length = unsafe {
        get_short_path_name(
            wide.as_ptr(),
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len())?,
        )
    };
    if length == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let length = usize::try_from(length)?;
    anyhow::ensure!(
        length < buffer.len(),
        "Short-name fixture exceeds its bound"
    );
    let observed = PathBuf::from(OsString::from_wide(&buffer[..length]));
    let filename = observed.file_name().expect("observed short filename");
    let alias = parent.join(filename);
    anyhow::ensure!(
        filename != path.file_name().expect("long filename")
            && uv_windows::could_be_dos_short_name(filename)?,
        "The native fixture did not establish the requested distinct short spelling"
    );
    if let Some(assigned_name) = assigned_name {
        anyhow::ensure!(
            uv_windows::names_equal_ordinal(filename, std::ffi::OsStr::new(assigned_name))?,
            "The native fixture did not observe the assigned short spelling"
        );
    }
    anyhow::ensure!(
        uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(path)?)?
            == uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(&alias)?)?,
        "Observed short spelling does not identify the owned export"
    );
    let actual_names = fs_err::read_dir(parent)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    anyhow::ensure!(
        actual_names
            .iter()
            .any(|name| name == path.file_name().expect("long filename"))
            && !actual_names.iter().any(|name| name == filename),
        "The long and short spellings did not identify one enumerated entry"
    );
    Ok(alias)
}

/// A missing historical short spelling cannot release another receipt's export claim.
#[cfg(windows)]
#[test]
fn tool_install_recovery_rejects_missing_short_name_claims() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let links = context.temp_dir.child("links");
    let bin = context.temp_dir.child("bin");
    let tools = context.temp_dir.child("tools");
    links.create_dir_all()?;
    for (name, commands) in [
        (
            "short-alias-root",
            vec![("owned-long-recovery-command", "root")],
        ),
        (
            "short-alias-peer",
            vec![
                ("owned-long-recovery-command", "peer"),
                ("unrelated-long-peer-command", "unrelated"),
            ],
        ),
    ] {
        write_recovery_wheel(links.path(), name, "1.0.0", &[], &commands)?;
        context
            .tool_install()
            .arg(name)
            .args(["--no-index", "--find-links"])
            .arg(links.path())
            .arg("--force")
            .assert()
            .success();
    }
    let long = bin.child("owned-long-recovery-command.exe");
    let peer_export = bin.child("unrelated-long-peer-command.exe");
    let peer_export_bytes = fs_err::read(peer_export.path())?;
    let alias = observed_short_path(long.path(), Some("UVALIAS.EXE"))?;
    let peer_source = venv_bin_path(tools.child("short-alias-peer").path())
        .join("owned-long-recovery-command.exe");
    let peer_source_alias = observed_short_path(&peer_source, Some("UVALIAS.EXE"))?;
    assert_eq!(alias.file_name(), peer_source_alias.file_name());
    let peer_receipt = tools.child("short-alias-peer").child("uv-receipt.toml");
    let mut document =
        fs_err::read_to_string(peer_receipt.path())?.parse::<toml_edit::DocumentMut>()?;
    let entries = document["tool"]["entrypoints"]
        .as_array_mut()
        .expect("entrypoints");
    assert_eq!(entries.len(), 2);
    entries
        .iter_mut()
        .find(|entry| {
            entry
                .as_inline_table()
                .and_then(|entry| entry.get("name"))
                .and_then(toml_edit::Value::as_str)
                == Some("owned-long-recovery-command")
        })
        .expect("peer shared entry")
        .as_inline_table_mut()
        .expect("entry table")
        .insert(
            "install-path",
            toml_edit::Value::from(alias.to_str().expect("UTF-8 alias")),
        );
    peer_receipt.write_str(&document.to_string())?;
    let receipts = [
        tools.child("short-alias-root").child("uv-receipt.toml"),
        peer_receipt,
    ];
    let receipt_bytes = receipts
        .iter()
        .map(|path| fs_err::read(path.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let package_paths = ["short-alias-root", "short-alias-peer"]
        .map(|name| site_packages_path(tools.child(name).path(), "python3.13"));
    let packages = package_paths
        .iter()
        .map(|path| dirhash_path(path))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    fs_err::remove_file(long.path())?;
    anyhow::ensure!(!alias.exists(), "Observed alias survived removal");
    for force in [false, true] {
        let mut command = context.tool_install();
        command
            .args(["short-alias-root", "--no-index", "--find-links"])
            .arg(links.path());
        if force {
            command.arg("--force");
        }
        command.assert().code(2).stderr(predicate::str::contains(
            "possible short-name alias is missing",
        ));
    }
    context
        .tool_upgrade()
        .args(["short-alias-root", "--no-index", "--find-links"])
        .arg(links.path())
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "possible short-name alias is missing",
        ));
    long.assert(predicate::path::missing());
    assert!(!alias.exists());
    assert_eq!(fs_err::read(peer_export.path())?, peer_export_bytes);
    for (path, bytes) in receipts.iter().zip(&receipt_bytes) {
        assert_eq!(fs_err::read(path.path())?, *bytes);
    }
    for (path, bytes) in package_paths.iter().zip(&packages) {
        assert_eq!(dirhash_path(path)?, *bytes);
    }
    Ok(())
}

/// Two absent planned names cannot be admitted using a short spelling observed before deletion.
#[cfg(windows)]
#[test]
fn tool_install_recovery_rejects_planned_short_name_aliases() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let links = context.temp_dir.child("links");
    let bin = context.temp_dir.child("bin");
    let environment = context.temp_dir.child("tools").child("planned-alias-root");
    links.create_dir_all()?;
    write_recovery_wheel(
        links.path(),
        "planned-alias-root",
        "1.0.0",
        &[],
        &[("existing-long-recovery-command", "existing-1")],
    )?;
    context
        .tool_install()
        .args(["planned-alias-root", "--no-index", "--find-links"])
        .arg(links.path())
        .assert()
        .success();
    let existing = bin.child("existing-long-recovery-command.exe");
    let existing_bytes = fs_err::read(existing.path())?;
    let receipt = environment.child("uv-receipt.toml");
    let receipt_bytes = fs_err::read(receipt.path())?;
    let long = bin.child("planned-long-recovery-command.exe");
    long.write_str("owned alias calibration")?;
    let alias = observed_short_path(long.path(), None)?;
    let alias_command = alias
        .file_stem()
        .expect("short command")
        .to_str()
        .expect("UTF-8 command");
    fs_err::remove_file(long.path())?;
    assert!(!alias.exists());
    long.write_str("owned alias recreation")?;
    assert_eq!(observed_short_path(long.path(), None)?, alias);
    fs_err::remove_file(long.path())?;
    assert!(!alias.exists());
    write_recovery_wheel(
        links.path(),
        "planned-alias-root",
        "2.0.0",
        &[],
        &[
            ("planned-long-recovery-command", "long-2"),
            (alias_command, "short-2"),
        ],
    )?;
    context
        .tool_upgrade()
        .args(["planned-alias-root", "--no-index", "--find-links"])
        .arg(links.path())
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "possible short-name alias is missing",
        ));
    assert_eq!(fs_err::read(existing.path())?, existing_bytes);
    assert_eq!(fs_err::read(receipt.path())?, receipt_bytes);
    long.assert(predicate::path::missing());
    assert!(!alias.exists());
    // Package replacement precedes discovery of new names; export admission is not rollback.
    assert!(
        site_packages_path(environment.path(), "python3.13")
            .join("planned_alias_root-2.0.0.dist-info")
            .is_dir()
    );
    Ok(())
}

/// No-op repair admits all final destinations before restoring either potentially aliased name.
#[cfg(windows)]
#[test]
fn tool_install_recovery_rejects_noop_short_name_aliases() -> Result<()> {
    let fixture = native_pe_fixture("where.exe")?;
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let links = context.temp_dir.child("links");
    let markers = context.temp_dir.child("markers");
    let observation = context.temp_dir.child("alias-observation");
    let bin = context.temp_dir.child("bin");
    let empty_bin = context.temp_dir.child("empty-bin");
    let environment = context.temp_dir.child("tools").child("noop-alias-root");
    for directory in [&links, &markers, &observation, &empty_bin] {
        directory.create_dir_all()?;
    }
    markers
        .child("owned-native-marker.txt")
        .write_str("owned-native-marker\r\n")?;
    let long_name = "noop-long-recovery-command.exe";
    let observed_long = observation.child(long_name);
    observed_long.write_str("owned automatic alias")?;
    let observed = observed_short_path(observed_long.path(), None)?;
    let short_command = observed
        .file_stem()
        .expect("short command")
        .to_str()
        .expect("UTF-8 command");
    let short_name = format!("{short_command}.exe");
    fs_err::remove_file(observed_long.path())?;
    observed_long.write_str("owned automatic alias recreation")?;
    assert_eq!(observed_short_path(observed_long.path(), None)?, observed);
    fs_err::remove_file(observed_long.path())?;

    // Console launchers are installed before .data/scripts. Reserve the literal short name in
    // the export directory too, so both installed source and initial export are distinct entries.
    let entrypoints =
        format!("[console_scripts]\n{short_command} = noop_alias_root.commands:main\n");
    let module = b"def main():\n    print('literal short command')\n";
    let tag = match fixture.machine {
        0x014c => "py3-none-win32",
        0x8664 => "py3-none-win_amd64",
        0xaa64 => "py3-none-win_arm64",
        other => anyhow::bail!("Unsupported native PE machine: 0x{other:04x}"),
    };
    let script = format!("noop_alias_root-1.0.0.data/scripts/{long_name}");
    let (filename, wheel) = generate_wheel_with_binary_files(
        &"noop-alias-root".parse()?,
        &"1.0.0".parse()?,
        &[],
        &Default::default(),
        None,
        tag,
        &[
            (
                "noop_alias_root-1.0.0.dist-info/entry_points.txt",
                entrypoints.as_bytes(),
            ),
            ("noop_alias_root/commands.py", module.as_slice()),
            (script.as_str(), fixture.bytes.as_slice()),
        ],
    );
    fs_err::write(links.path().join(filename), wheel)?;
    bin.child(&short_name)
        .write_str("owned literal-name reservation")?;
    let install = |destination: &Path| {
        let mut command = context.tool_install();
        command
            .args(["noop-alias-root", "--no-index", "--find-links"])
            .arg(links.path())
            .env(EnvVars::UV_TOOL_BIN_DIR, destination);
        command
    };
    install(bin.path()).arg("--force").assert().success();
    let source_long = venv_bin_path(environment.path()).join(long_name);
    let source_short = venv_bin_path(environment.path()).join(&short_name);
    let exported_long = bin.child(long_name);
    let exported_short = bin.child(&short_name);
    for (long, short) in [
        (source_long.as_path(), source_short.as_path()),
        (exported_long.path(), exported_short.path()),
    ] {
        let actual = fs_err::read_dir(long.parent().expect("entry parent"))?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        assert!(
            actual
                .iter()
                .any(|name| name == long.file_name().expect("long name"))
        );
        assert!(
            actual
                .iter()
                .any(|name| name == short.file_name().expect("short name"))
        );
        assert_ne!(
            uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(long)?)?,
            uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(short)?)?
        );
    }
    assert_native_where(exported_long.path(), markers.path());
    Command::new(exported_short.path())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .assert()
        .success()
        .stdout("literal short command\n");
    let source_bytes = [fs_err::read(&source_long)?, fs_err::read(&source_short)?];
    let export_bytes = [
        fs_err::read(exported_long.path())?,
        fs_err::read(exported_short.path())?,
    ];
    let receipt = environment.child("uv-receipt.toml");
    let receipt_bytes = fs_err::read(receipt.path())?;
    let package_path = site_packages_path(environment.path(), "python3.13");
    let packages = dirhash_path(&package_path)?;
    let assert_refusal = |destination: &Path| {
        install(destination)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(
                "possible short-name alias is missing",
            ));
        context
            .tool_upgrade()
            .args(["noop-alias-root", "--no-index", "--find-links"])
            .arg(links.path())
            .env(EnvVars::UV_TOOL_BIN_DIR, destination)
            .assert()
            .code(1)
            .stderr(predicate::str::contains(
                "possible short-name alias is missing",
            ));
    };
    assert_refusal(empty_bin.path());
    assert_eq!(fs_err::read_dir(empty_bin.path())?.count(), 0);
    assert_eq!(fs_err::read(exported_long.path())?, export_bytes[0]);
    assert_eq!(fs_err::read(exported_short.path())?, export_bytes[1]);
    fs_err::remove_file(exported_long.path())?;
    fs_err::remove_file(exported_short.path())?;
    assert_refusal(bin.path());
    exported_long.assert(predicate::path::missing());
    exported_short.assert(predicate::path::missing());
    assert_eq!(fs_err::read(&source_long)?, source_bytes[0]);
    assert_eq!(fs_err::read(&source_short)?, source_bytes[1]);
    assert_eq!(fs_err::read(receipt.path())?, receipt_bytes);
    assert_eq!(dirhash_path(&package_path)?, packages);
    Ok(())
}

#[cfg(windows)]
fn native_pe_fixture(filename: &str) -> Result<NativePeFixture> {
    anyhow::ensure!(
        matches!(filename, "where.exe" | "findstr.exe"),
        "Unexpected native fixture name"
    );
    const MAX_BYTES: u64 = 2 * 1024 * 1024;
    let directory = native_system_directory()?;
    let path = directory.join(filename);
    let metadata = fs_err::symlink_metadata(&path)?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.is_symlink() && metadata.len() <= MAX_BYTES,
        "Native PE fixture is not a bounded regular file"
    );
    let mut file = uv_windows::open_file_entry(&path)?;
    let identity = uv_windows::FileIdentity::from_file(&file)?;
    let mut bytes = Vec::new();
    (&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? == metadata.len(),
        "Native PE fixture changed size"
    );
    file.rewind()?;
    let mut repeated = Vec::new();
    (&mut file).take(MAX_BYTES + 1).read_to_end(&mut repeated)?;
    anyhow::ensure!(repeated == bytes, "Native PE fixture changed while reading");
    anyhow::ensure!(
        uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(&path)?)? == identity,
        "Native PE fixture path changed identity"
    );
    anyhow::ensure!(
        bytes.get(..2) == Some(b"MZ") && bytes.len() >= 64,
        "Native fixture is not a DOS/PE image"
    );
    let offset = usize::try_from(u32::from_le_bytes(bytes[60..64].try_into()?))?;
    let end = offset
        .checked_add(6)
        .ok_or_else(|| anyhow::anyhow!("PE header offset overflow"))?;
    anyhow::ensure!(
        end <= bytes.len() && bytes.get(offset..offset + 4) == Some(b"PE\0\0"),
        "Native fixture has no bounded PE header"
    );
    let machine = u16::from_le_bytes(bytes[offset + 4..end].try_into()?);
    let expected_machine = match std::env::consts::ARCH {
        "x86" => 0x014c,
        "x86_64" => 0x8664,
        "aarch64" => 0xaa64,
        other => anyhow::bail!("Unsupported native PE fixture architecture: {other}"),
    };
    anyhow::ensure!(
        machine == expected_machine,
        "Native fixture architecture does not match the test process"
    );
    anyhow::ensure!(
        uv_trampoline_builder::Launcher::try_from_path(&path)?.is_none(),
        "Native fixture is a uv trampoline"
    );
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    eprintln!(
        "native-pe-fixture {}",
        serde_json::json!({
            "source": path, "bytes": bytes.len(), "sha256": sha256,
            "machine": format!("0x{machine:04x}"), "identity": format!("{identity:?}"),
        })
    );
    Ok(NativePeFixture {
        path,
        bytes,
        sha256,
        machine,
    })
}

#[cfg(windows)]
fn write_native_recovery_wheel(
    directory: &Path,
    name: &str,
    version: &str,
    command: &str,
    fixture: &NativePeFixture,
) -> Result<PathBuf> {
    let tag = match fixture.machine {
        0x014c => "py3-none-win32",
        0x8664 => "py3-none-win_amd64",
        0xaa64 => "py3-none-win_arm64",
        other => anyhow::bail!("Unsupported native PE machine: 0x{other:04x}"),
    };
    let script = format!(
        "{}-{version}.data/scripts/{command}.exe",
        name.replace('-', "_")
    );
    let (filename, bytes) = generate_wheel_with_binary_files(
        &name.parse()?,
        &version.parse()?,
        &[],
        &Default::default(),
        None,
        tag,
        &[(script.as_str(), fixture.bytes.as_slice())],
    );
    let path = directory.join(filename);
    fs_err::write(&path, bytes)?;
    Ok(path)
}

#[cfg(windows)]
fn assert_native_where(executable: &Path, markers: &Path) {
    Command::new(executable)
        .args(["/q", "/r"])
        .arg(markers)
        .arg("owned-native-marker.txt")
        .current_dir(markers)
        .env(EnvVars::PATH, markers)
        .assert()
        .success()
        .stdout("");
    Command::new(executable)
        .args(["/q", "/r"])
        .arg(markers)
        .arg("absent-native-marker.txt")
        .current_dir(markers)
        .env(EnvVars::PATH, markers)
        .assert()
        .code(1)
        .stdout("");
}

#[cfg(windows)]
fn assert_native_findstr(executable: &Path, markers: &Path) {
    Command::new(executable)
        .args(["/x", "/c:owned-native-marker"])
        .arg(markers.join("owned-native-marker.txt"))
        .current_dir(markers)
        .env(EnvVars::PATH, markers)
        .assert()
        .success()
        .stdout("owned-native-marker\r\n");
}

/// Generic native PE exports retain the captured owner when source bytes and environments change.
#[cfg(windows)]
#[test]
fn tool_install_recovery_native_pe_updates() -> Result<()> {
    let where_exe = native_pe_fixture("where.exe")?;
    let findstr_exe = native_pe_fixture("findstr.exe")?;
    assert_ne!(where_exe.sha256, findstr_exe.sha256);
    let context = uv_test::test_context_with_versions!(&["3.13", "3.12"]).with_tool_dirs();
    let links = context.temp_dir.child("links");
    let markers = context.temp_dir.child("markers");
    links.create_dir_all()?;
    markers.create_dir_all()?;
    markers
        .child("owned-native-marker.txt")
        .write_str("owned-native-marker\r\n")?;
    let bins = (0..3)
        .map(|index| context.temp_dir.child(format!("native-bin-{index}")))
        .collect::<Vec<_>>();
    let tools = context.temp_dir.child("tools");
    let environment = tools.child("native-recovery");
    let peer_environment = tools.child("native-peer");
    write_native_recovery_wheel(
        links.path(),
        "native-recovery",
        "1.0.0",
        "native-recovery",
        &where_exe,
    )?;
    write_native_recovery_wheel(
        links.path(),
        "native-peer",
        "1.0.0",
        "native-peer",
        &where_exe,
    )?;
    let install = |bin: &Path| {
        let mut command = context.tool_install();
        command
            .args(["native-recovery", "--no-index", "--find-links"])
            .arg(links.path())
            .env(EnvVars::UV_TOOL_BIN_DIR, bin)
            .env(EnvVars::PATH, bin);
        command
    };
    let upgrade = |bin: &Path| {
        let mut command = context.tool_upgrade();
        command
            .args(["native-recovery", "--no-index", "--find-links"])
            .arg(links.path())
            .env(EnvVars::UV_TOOL_BIN_DIR, bin)
            .env(EnvVars::PATH, bin);
        command
    };
    install(bins[0].path())
        .args(["--python", "3.13"])
        .assert()
        .success();
    context
        .tool_install()
        .args([
            "native-peer==1.0.0",
            "--python",
            "3.13",
            "--no-index",
            "--find-links",
        ])
        .arg(links.path())
        .env(EnvVars::UV_TOOL_BIN_DIR, bins[0].as_os_str())
        .assert()
        .success();
    let first = bins[0].child("native-recovery.exe");
    let peer = bins[0].child("native-peer.exe");
    let peer_receipt = peer_environment.child("uv-receipt.toml");
    let peer_receipt_bytes = fs_err::read(peer_receipt.path())?;
    let peer_packages = site_packages_path(peer_environment.path(), "python3.13");
    let peer_package_bytes = dirhash_path(&peer_packages)?;
    assert_eq!(fs_err::read(first.path())?, where_exe.bytes);
    assert_native_where(first.path(), markers.path());
    let sentinel = environment.child("preserve-unless-replaced");
    sentinel.write_str("native environment")?;
    fs_err::remove_file(first.path())?;
    install(bins[0].path()).assert().success();
    sentinel.assert("native environment");
    assert_native_where(first.path(), markers.path());

    // The two receipt paths are distinct entries even though their native files share an inode.
    fs_err::remove_file(peer.path())?;
    fs_err::hard_link(first.path(), peer.path())?;
    let peer_identity =
        uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(peer.path())?)?;
    assert_eq!(
        uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(first.path())?)?,
        peer_identity
    );
    write_native_recovery_wheel(
        links.path(),
        "native-recovery",
        "2.0.0",
        "native-recovery",
        &findstr_exe,
    )?;
    upgrade(bins[0].path()).assert().success();
    sentinel.assert("native environment");
    assert_eq!(fs_err::read(first.path())?, findstr_exe.bytes);
    assert_eq!(fs_err::read(peer.path())?, where_exe.bytes);
    assert_ne!(
        uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(first.path())?)?,
        peer_identity
    );
    assert_eq!(
        uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(peer.path())?)?,
        peer_identity
    );
    assert_native_findstr(first.path(), markers.path());
    assert_native_where(peer.path(), markers.path());

    install(bins[1].path()).arg("--force").assert().success();
    sentinel.assert(predicate::path::missing());
    first.assert(predicate::path::missing());
    assert_eq!(
        fs_err::read(bins[1].child("native-recovery.exe").path())?,
        findstr_exe.bytes
    );
    assert_native_findstr(bins[1].child("native-recovery.exe").path(), markers.path());
    sentinel.write_str("replace native interpreter")?;
    upgrade(bins[2].path())
        .args(["--python", "3.12"])
        .assert()
        .success();
    sentinel.assert(predicate::path::missing());
    bins[1]
        .child("native-recovery.exe")
        .assert(predicate::path::missing());
    assert_native_findstr(bins[2].child("native-recovery.exe").path(), markers.path());
    assert_native_where(peer.path(), markers.path());
    assert_eq!(fs_err::read(peer_receipt.path())?, peer_receipt_bytes);
    assert_eq!(dirhash_path(&peer_packages)?, peer_package_bytes);
    assert_eq!(fs_err::read(&where_exe.path)?, where_exe.bytes);
    assert_eq!(fs_err::read(&findstr_exe.path)?, findstr_exe.bytes);
    Ok(())
}

/// Identical native bytes cannot identify the last force winner when two receipts claim one path.
#[cfg(windows)]
#[test]
fn tool_install_recovery_native_pe_rejects_ambiguous_force() -> Result<()> {
    let fixture = native_pe_fixture("where.exe")?;
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let links = context.temp_dir.child("links");
    let markers = context.temp_dir.child("markers");
    links.create_dir_all()?;
    markers.create_dir_all()?;
    markers
        .child("owned-native-marker.txt")
        .write_str("owned-native-marker\r\n")?;
    for name in ["native-first", "native-second"] {
        write_native_recovery_wheel(links.path(), name, "1.0.0", "native-shared", &fixture)?;
    }
    let install = |name: &str| {
        let mut command = context.tool_install();
        command
            .arg(name)
            .args(["--no-index", "--find-links"])
            .arg(links.path());
        command
    };
    install("native-first").assert().success();
    install("native-second").arg("--force").assert().success();
    let exported = context.temp_dir.child("bin").child("native-shared.exe");
    assert_eq!(fs_err::read(exported.path())?, fixture.bytes);
    assert_native_where(exported.path(), markers.path());
    let environments =
        ["native-first", "native-second"].map(|name| context.temp_dir.child("tools").child(name));
    let receipts = environments
        .iter()
        .map(|environment| environment.child("uv-receipt.toml"))
        .collect::<Vec<_>>();
    let receipt_bytes = receipts
        .iter()
        .map(|path| fs_err::read(path.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let package_paths = environments
        .iter()
        .map(|environment| site_packages_path(environment.path(), "python3.13"))
        .collect::<Vec<_>>();
    let package_bytes = package_paths
        .iter()
        .map(|path| dirhash_path(path))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    install("native-first")
        .arg("--force")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "Cannot determine whether executable",
        ));
    context
        .tool_upgrade()
        .args(["native-first", "--offline"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "Cannot determine whether executable",
        ));
    assert_eq!(fs_err::read(exported.path())?, fixture.bytes);
    fs_err::remove_file(exported.path())?;
    install("native-first")
        .arg("--force")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "because it is also recorded for `native-second`",
        ));
    exported.assert(predicate::path::missing());
    for (receipt, bytes) in receipts.iter().zip(&receipt_bytes) {
        assert_eq!(fs_err::read(receipt.path())?, *bytes);
    }
    for (path, bytes) in package_paths.iter().zip(&package_bytes) {
        assert_eq!(dirhash_path(path)?, *bytes);
    }
    assert_eq!(fs_err::read(&fixture.path)?, fixture.bytes);
    Ok(())
}

/// Test installing a tool when its entry point already exists
#[test]
fn tool_install_force() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    executable.touch().unwrap();

    // Attempt to install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    error: Executable already exists: black (use `--force` to overwrite)
    ");

    // We should delete the virtual environment
    assert!(!tool_dir.child("black").exists());

    // We should not write a tools entry
    assert!(!tool_dir.join("black").join("uv-receipt.toml").exists());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Nor should we change the `black` entry point that exists
        assert_snapshot!(fs_err::read_to_string(&executable).unwrap(), @"");

    });

    // Attempt to install `black` with the `--reinstall` flag
    // Should have no effect
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--reinstall")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    error: Executable already exists: black (use `--force` to overwrite)
    ");

    // We should not create a virtual environment
    assert!(!tool_dir.child("black").exists());

    // We should not write a tools entry
    assert!(!tool_dir.join("tools.toml").exists());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Nor should we change the `black` entry point that exists
        assert_snapshot!(fs_err::read_to_string(&executable).unwrap(), @"");

    });

    // Test error message when multiple entry points exist
    bin_dir
        .child(format!("blackd{}", std::env::consts::EXE_SUFFIX))
        .touch()
        .unwrap();
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    error: Executables already exist: black, blackd (use `--force` to overwrite)
    ");

    // Install `black` with `--force`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--force")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());

    let marker = tool_dir.child("black").child("marker");
    fs_err::write(&marker, b"marker").unwrap();
    marker.assert(predicate::path::is_file());

    // Re-install `black` with `--force`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--force")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    marker.assert(predicate::path::missing());

    // Re-install `black` without `--force`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    `black` is already installed
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());

    // Re-install `black` with `--reinstall`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--reinstall")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     ~ black==24.3.0
     ~ click==8.1.7
     ~ mypy-extensions==1.0.0
     ~ packaging==24.0
     ~ pathspec==0.12.1
     ~ platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We write a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/black/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from black import patched_main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(patched_main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("black").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    black, 24.3.0 (compiled: yes)
    Python (CPython) 3.12.[X]
    ");
}

/// Test `uv tool install` when the bin directory is inferred from `$HOME`
///
/// Only tested on Linux right now because it's not clear how to change the %USERPROFILE% on Windows
#[cfg(unix)]
#[test]
fn tool_install_home() {
    let context = uv_test::test_context!("3.12").with_filtered_exe_suffix();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    let mut cmd = context.tool_install();
    cmd.arg("black")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .env(
            EnvVars::XDG_DATA_HOME,
            context.home_dir.child(".local").child("share").as_os_str(),
        )
        .env(
            EnvVars::PATH,
            context.home_dir.child(".local").child("bin").as_os_str(),
        );
    uv_snapshot!(context.filters(), cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    context
        .home_dir
        .child(format!(".local/bin/black{}", std::env::consts::EXE_SUFFIX))
        .assert(predicate::path::exists());
}

/// Test `uv tool install` when the bin directory is inferred from `$XDG_DATA_HOME`
#[test]
fn tool_install_xdg_data_home() {
    let context = uv_test::test_context!("3.12").with_filtered_exe_suffix();
    let tool_dir = context.temp_dir.child("tools");
    let data_home = context.temp_dir.child("data/home");
    let bin_dir = context.temp_dir.child("data/bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .env(EnvVars::XDG_DATA_HOME, data_home.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    context
        .temp_dir
        .child(format!("data/bin/black{}", std::env::consts::EXE_SUFFIX))
        .assert(predicate::path::exists());
}

/// Test `uv tool install` when the bin directory is set by `$XDG_BIN_HOME`
#[test]
fn tool_install_xdg_bin_home() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    bin_dir
        .child(format!("black{}", std::env::consts::EXE_SUFFIX))
        .assert(predicate::path::exists());
}

/// Test `uv tool install` when the bin directory is set by `$UV_TOOL_BIN_DIR`
#[test]
fn tool_install_tool_bin_dir() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::UV_TOOL_BIN_DIR, bin_dir.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    bin_dir
        .child(format!("black{}", std::env::consts::EXE_SUFFIX))
        .assert(predicate::path::exists());
}

/// Test installing a tool that lacks entrypoints
#[test]
fn tool_install_no_entrypoints() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("iniconfig")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stdout -----
    No executables are provided by package `iniconfig`; removing tool

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    error: Failed to install entrypoints for `iniconfig`
    ");

    // Ensure the tool environment is not created.
    tool_dir
        .child("iniconfig")
        .assert(predicate::path::missing());
    bin_dir
        .child("iniconfig")
        .assert(predicate::path::missing());
}

/// A failed forced installation must not remove another tool's existing executable.
#[test]
fn tool_install_failure_preserves_existing_additional_entrypoints() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let links = context.workspace_root.join("test/links");

    context
        .tool_install()
        .arg("simple-launcher==0.1.0")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links)
        .env(EnvVars::UV_TOOL_BIN_DIR, bin_dir.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    let executable = bin_dir.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    uv_snapshot!(context.filters(), Command::new(executable.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");

    let environment = tool_dir.child("simple-launcher");
    let receipt = environment.child("uv-receipt.toml");
    let receipt_contents = fs_err::read(receipt.path())?;
    let site_packages = site_packages_path(environment.path(), "python3.13");
    let installed_contents = dirhash_path(&site_packages)?;
    let executable_contents = fs_err::read(executable.path())?;
    let executable_metadata = fs_err::symlink_metadata(executable.path())?;
    let source_executable = venv_bin_path(environment.path())
        .join(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(fs_err::read(&source_executable)?, executable_contents);
    #[cfg(unix)]
    let executable_identity = {
        assert!(executable_metadata.is_symlink());
        assert_eq!(
            fs_err::canonicalize(executable.path())?,
            fs_err::canonicalize(&source_executable)?
        );
        (
            executable_metadata.dev(),
            executable_metadata.ino(),
            fs_err::read_link(executable.path())?,
        )
    };

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("basic-package==0.1.0")
        .arg("--with-executables-from")
        .arg("simple-launcher")
        .arg("--force")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links)
        .env(EnvVars::UV_TOOL_BIN_DIR, bin_dir.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stdout -----
    No executables are provided by package `basic-package`; removing tool

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + basic-package==0.1.0
     + simple-launcher==0.1.0
    error: Failed to install entrypoints for `basic-package`
    ");

    tool_dir
        .child("basic-package")
        .assert(predicate::path::missing());
    assert_eq!(fs_err::read(receipt.path())?, receipt_contents);
    assert_eq!(dirhash_path(&site_packages)?, installed_contents);
    assert_eq!(fs_err::read(executable.path())?, executable_contents);
    let current_metadata = fs_err::symlink_metadata(executable.path())?;
    assert_eq!(
        current_metadata.file_type(),
        executable_metadata.file_type()
    );
    #[cfg(unix)]
    {
        assert_eq!(
            (
                current_metadata.dev(),
                current_metadata.ino(),
                fs_err::read_link(executable.path())?,
            ),
            executable_identity
        );
        assert_eq!(
            fs_err::canonicalize(executable.path())?,
            fs_err::canonicalize(&source_executable)?
        );
    }

    uv_snapshot!(context.filters(), Command::new(executable.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");

    Ok(())
}

/// Test that a failed tool installation removes entrypoints installed from additional packages.
#[test]
fn tool_install_failure_removes_additional_entrypoints() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    context.temp_dir.child("uv.toml").write_str(
        r#"
        exclude-dependencies = ["iniconfig"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--with-executables-from")
        .arg("black")
        .arg("iniconfig")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stdout -----
    No executables are provided by package `iniconfig`; removing tool

    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + iniconfig==2.0.0
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    error: Failed to install entrypoints for `iniconfig`
    ");

    tool_dir
        .child("iniconfig")
        .assert(predicate::path::missing());
    bin_dir
        .child(format!("black{}", std::env::consts::EXE_SUFFIX))
        .assert(predicate::path::missing());
    bin_dir
        .child(format!("blackd{}", std::env::consts::EXE_SUFFIX))
        .assert(predicate::path::missing());

    Ok(())
}

#[test]
fn tool_install_no_binary_package_env_var() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("pytest")
        .env(EnvVars::UV_NO_BINARY_PACKAGE, "iniconfig")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + iniconfig==2.0.0
     + packaging==24.0
     + pluggy==1.4.0
     + pytest==8.1.1
    Installed 2 executables: py.test, pytest
    ");

    let receipt: toml::Value = toml::from_str(
        &fs_err::read_to_string(tool_dir.join("pytest").join("uv-receipt.toml")).unwrap(),
    )
    .unwrap();
    assert_snapshot!(
        receipt["tool"]["options"]["no-binary-package"].to_string(),
        @r#"["iniconfig"]"#
    );
}

/// Test installing a package that can't be installed.
#[test]
fn tool_install_uninstallable() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let filters = context
        .filters()
        .into_iter()
        .chain([
            (r"bdist\.[^/\\\s]+(-[^/\\\s]+)?", "bdist.linux-x86_64"),
            (r"\\\.", ""),
            (r"#+", "#"),
            (
                "Please read the installation instructions at:\n ",
                "Please read the installation instructions at:\n",
            ),
        ])
        .collect::<Vec<_>>();
    uv_snapshot!(filters, context.tool_install()
        .arg("pyenv")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to build `pyenv==0.0.1`
      cause: The build backend returned an error
      cause: Call to `setuptools.build_meta:__legacy__.build_wheel` failed (exit status: 1)

             [stdout]
             running bdist_wheel
             running build
             installing to build/bdist.linux-x86_64/wheel
             running install

             [stderr]
             # NOTE #
             We are sorry, but this package is not installable with pip.

             Please read the installation instructions at:

             https://github.com/pyenv/pyenv#installation
             #

    hint: Build failures usually indicate a problem with the package or the build environment
    ");

    // Ensure the tool environment is not created.
    tool_dir.child("pyenv").assert(predicate::path::missing());
    bin_dir.child("pyenv").assert(predicate::path::missing());
}

/// Test installing a tool with a bare URL requirement.
#[test]
fn tool_install_unnamed_package() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("https://files.pythonhosted.org/packages/0f/89/294c9a6b6c75a08da55e9d05321d0707e9418735e3062b12ef0f54c33474/black-24.4.2-py3-none-any.whl")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.4.2 (from https://files.pythonhosted.org/packages/0f/89/294c9a6b6c75a08da55e9d05321d0707e9418735e3062b12ef0f54c33474/black-24.4.2-py3-none-any.whl)
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/black/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from black import patched_main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(patched_main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", url = "https://files.pythonhosted.org/packages/0f/89/294c9a6b6c75a08da55e9d05321d0707e9418735e3062b12ef0f54c33474/black-24.4.2-py3-none-any.whl" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("black").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    black, 24.4.2 (compiled: no)
    Python (CPython) 3.12.[X]
    ");
}

/// Test installing a tool with a Git requirement.
#[test]
#[cfg(feature = "test-git")]
fn tool_install_git() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let path = tool_install_git_path(&bin_dir);

    // Unnamed Git Install
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("git+https://github.com/psf/black@24.2.0")
        .env(EnvVars::PATH, path.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.2.0 (from git+https://github.com/psf/black@6fdf8a4af28071ed1d079c01122b34c5d587207a)
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    fs_err::remove_dir_all(&bin_dir).expect("Failed to remove bin dir.");
    fs_err::remove_dir_all(&tool_dir).expect("Failed to remove tool dir.");

    // Named Git Install
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black @ git+https://github.com/psf/black@24.2.0")
        .env(EnvVars::PATH, path.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.2.0 (from git+https://github.com/psf/black@6fdf8a4af28071ed1d079c01122b34c5d587207a)
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());
}

/// Test that installing a tool from Git uses statically available `requires-python` metadata
/// before selecting a global Python pin.
#[test]
#[cfg(feature = "test-git")]
fn tool_install_git_infers_static_requires_python() {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let path = tool_install_git_path(&bin_dir);

    context
        .python_pin()
        .arg("3.11")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("git+https://github.com/astral-sh/uv-dynamic-requires-python-test@75a612dc87fc215e999a25a0efc376cbf9831afa#subdirectory=static")
        .env(EnvVars::PATH, path.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + static-requires-python-tool==0.1.0 (from git+https://github.com/astral-sh/uv-dynamic-requires-python-test@75a612dc87fc215e999a25a0efc376cbf9831afa#subdirectory=static)
    Installed 1 executable: static-requires-python-tool
    ");

    uv_snapshot!(context.filters(), Command::new("static-requires-python-tool")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12
    ");
}

/// Test that installing a tool from Git does not infer dynamic `requires-python` metadata.
#[test]
#[cfg(feature = "test-git")]
fn tool_install_git_does_not_infer_dynamic_requires_python() {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let path = tool_install_git_path(&bin_dir);

    context
        .python_pin()
        .arg("3.11")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("git+https://github.com/astral-sh/uv-dynamic-requires-python-test@75a612dc87fc215e999a25a0efc376cbf9831afa#subdirectory=dynamic")
        .env(EnvVars::PATH, path.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because the current Python version (3.11.[X]) does not satisfy Python>=3.12,<3.13 and dynamic-requires-python-tool==0.1.0 depends on Python>=3.12,<3.13, we can conclude that dynamic-requires-python-tool==0.1.0 cannot be used.
             And because only dynamic-requires-python-tool==0.1.0 is available and you require dynamic-requires-python-tool, we can conclude that your requirements are unsatisfiable.
    ");
}

/// Test installing a tool with a Git LFS enabled requirement.
#[test]
#[cfg(feature = "test-git-lfs")]
fn tool_install_git_lfs() {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_git_lfs_config()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let mut paths = BTreeSet::new();

    // Avoid removing `git` or `git-lfs` from PATH
    let git_path = which::which("git")
        .expect("Failed to find `git` executable.")
        .parent()
        .expect("Failed to find `git` executable directory.")
        .to_path_buf();
    let git_lfs_path = which::which("git-lfs")
        .expect("Failed to find `git-lfs` executable.")
        .parent()
        .expect("Failed to find `git-lfs` executable directory.")
        .to_path_buf();
    paths.insert(bin_dir.to_path_buf());
    paths.insert(git_path);
    paths.insert(git_lfs_path);
    // Git LFS filter-process in macos seems to rely on `sh`.
    // Git Submodule in macos seems to rely on `sed`.
    if cfg!(target_os = "macos") {
        for bin_path in ["sh", "sed"].into_iter().map(|name| {
            which::which(name)
                .unwrap_or_else(|_| panic!("Failed to find `{name}` executable."))
                .parent()
                .unwrap_or_else(|| panic!("Failed to find `{name}` executable directory."))
                .to_path_buf()
        }) {
            paths.insert(bin_path);
        }
    }
    let path = std::env::join_paths(paths).unwrap();

    // Verify a successful LFS request
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--lfs")
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa")
        .env(EnvVars::PATH, path.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa#lfs=true)
    Installed 2 executables: test-lfs-repo, test-lfs-repo-assets
    ");

    tool_dir
        .child("test-lfs-repo")
        .assert(predicate::path::is_dir());
    tool_dir
        .child("test-lfs-repo")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("test-lfs-repo{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("test-lfs-repo").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "test-lfs-repo", git = "https://github.com/astral-sh/test-lfs-repo?lfs=true&rev=e282f5be233e3f1d44934164895a043fc534b8aa" }]
        entrypoints = [
            { name = "test-lfs-repo", install-path = "[TEMP_DIR]/bin/test-lfs-repo", from = "test-lfs-repo" },
            { name = "test-lfs-repo-assets", install-path = "[TEMP_DIR]/bin/test-lfs-repo-assets", from = "test-lfs-repo" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("test-lfs-repo").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from test-lfs-repo!
    ");

    uv_snapshot!(context.filters(), Command::new("test-lfs-repo-assets").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from test-lfs-repo! LFS_TEST=True ANOTHER_LFS_TEST=True
    ");

    // Attempt to install when LFS artifacts are missing and LFS is requested.

    // The filters below will remove any boilerplate before what we actually want to match.
    // They help handle slightly different output in uv-distribution/src/source/mod.rs between
    // calls to `git` and `git_metadata` functions which don't have guaranteed execution order.
    // In addition, we can get different error codes depending on where the failure occurs,
    // although we know the error code cannot be 0.
    let context = context
        .with_filter((r"exit_code: -?[1-9]\d*", "exit_code: [ERROR_CODE]"))
        .with_filter((
            "(?s)(----- stderr -----).*?The source distribution `[^`]+` is missing Git LFS artifacts.*",
            "$1\n[PREFIX]The source distribution `[DISTRIBUTION]` is missing Git LFS artifacts",
        ));

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--reinstall")
        .arg("--lfs")
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa")
        .env(EnvVars::UV_INTERNAL__TEST_LFS_DISABLED, "1")
        .env(EnvVars::PATH, path.as_os_str()), @"
    exit_code: [ERROR_CODE] (failure)
    ----- stderr -----
    [PREFIX]The source distribution `[DISTRIBUTION]` is missing Git LFS artifacts
    ");

    // Attempt to install when LFS artifacts are missing but LFS was not requested.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa")
        .env(EnvVars::PATH, path.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa#lfs=true)
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa)
    Installed 2 executables: test-lfs-repo, test-lfs-repo-assets
    ");

    #[cfg(not(windows))]
    uv_snapshot!(context.filters(), Command::new("test-lfs-repo-assets").env(EnvVars::PATH, bin_dir.as_os_str()), @r#"
    exit_code: [ERROR_CODE] (failure)
    ----- stderr -----
    Traceback (most recent call last):
      File "[TEMP_DIR]/bin/test-lfs-repo-assets", line 10, in <module>
        sys.exit(main_lfs())
                 ~~~~~~~~^^
      File "[TEMP_DIR]/tools/test-lfs-repo/[PYTHON-LIB]/site-packages/test_lfs_repo/__init__.py", line 5, in main_lfs
        from .lfs_module import LFS_TEST
      File "[TEMP_DIR]/tools/test-lfs-repo/[PYTHON-LIB]/site-packages/test_lfs_repo/lfs_module.py", line 1
        version https://git-lfs.github.com/spec/v1
                ^^^^^
    SyntaxError: invalid syntax
    "#);

    #[cfg(windows)]
    uv_snapshot!(context.filters(), Command::new("test-lfs-repo-assets").env(EnvVars::PATH, bin_dir.as_os_str()), @r#"
    exit_code: [ERROR_CODE] (failure)
    ----- stderr -----
    Traceback (most recent call last):
      File "<frozen runpy>", line 198, in _run_module_as_main
      File "<frozen runpy>", line 88, in _run_code
      File "[TEMP_DIR]/bin/test-lfs-repo-assets/__main__.py", line 10, in <module>
        sys.exit(main_lfs())
                 ~~~~~~~~^^
      File "[TEMP_DIR]/tools/test-lfs-repo/[PYTHON-LIB]/site-packages/test_lfs_repo/__init__.py", line 5, in main_lfs
        from .lfs_module import LFS_TEST
      File "[TEMP_DIR]/tools/test-lfs-repo/[PYTHON-LIB]/site-packages/test_lfs_repo/lfs_module.py", line 1
        version https://git-lfs.github.com/spec/v1
                ^^^^^
    SyntaxError: invalid syntax
    "#);
}

/// Test installing a tool with a bare URL requirement using `--from`, where the URL and the package
/// name conflict.
#[test]
fn tool_install_unnamed_conflict() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--from")
        .arg("https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package name (`iniconfig`) provided with `--from` does not match install request (`black`)
    ");
}

/// Test installing a tool with a bare URL requirement using `--from`.
#[test]
fn tool_install_unnamed_from() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--from")
        .arg("https://files.pythonhosted.org/packages/0f/89/294c9a6b6c75a08da55e9d05321d0707e9418735e3062b12ef0f54c33474/black-24.4.2-py3-none-any.whl")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.4.2 (from https://files.pythonhosted.org/packages/0f/89/294c9a6b6c75a08da55e9d05321d0707e9418735e3062b12ef0f54c33474/black-24.4.2-py3-none-any.whl)
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/black/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from black import patched_main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(patched_main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", url = "https://files.pythonhosted.org/packages/0f/89/294c9a6b6c75a08da55e9d05321d0707e9418735e3062b12ef0f54c33474/black-24.4.2-py3-none-any.whl" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("black").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    black, 24.4.2 (compiled: no)
    Python (CPython) 3.12.[X]
    ");
}

/// Test installing a tool with a bare URL requirement using `--with`.
#[test]
fn tool_install_unnamed_with() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--with")
        .arg("https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + iniconfig==2.0.0 (from https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl)
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir.child("black").assert(predicate::path::is_dir());
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("black{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/black/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from black import patched_main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(patched_main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [
            { name = "black" },
            { name = "iniconfig", url = "https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl" },
        ]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), Command::new("black").arg("--version").env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    black, 24.3.0 (compiled: yes)
    Python (CPython) 3.12.[X]
    ");
}

#[test]
fn tool_install_with_dependencies_from_script() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio",
        # ]
        # ///

        import anyio
    "#})?;

    // script dependencies (anyio) are now installed.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--with-requirements")
        .arg("script.py")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + anyio==4.3.0
     + black==24.3.0
     + click==8.1.7
     + idna==3.6
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
     + sniffio==1.3.1
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [
            { name = "black" },
            { name = "anyio" },
        ]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Update the script file.
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio",
        #   "iniconfig",
        # ]
        # ///

        import anyio
    "#})?;

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--with-requirements")
        .arg("script.py")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + iniconfig==2.0.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [
            { name = "black" },
            { name = "anyio" },
            { name = "iniconfig" },
        ]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    Ok(())
}

/// Test installing a tool with additional requirements from a `requirements.txt` file.
#[test]
fn tool_install_requirements_txt() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig").unwrap();

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--with-requirements")
        .arg("requirements.txt")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + iniconfig==2.0.0
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [
            { name = "black" },
            { name = "iniconfig" },
        ]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Update the `requirements.txt` file.
    requirements_txt.write_str("idna").unwrap();

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--with-requirements")
        .arg("requirements.txt")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + idna==3.6
     - iniconfig==2.0.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [
            { name = "black" },
            { name = "idna" },
        ]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });
}

/// Ignore and warn when (e.g.) the `--index-url` argument is a provided `requirements.txt`.
#[test]
fn tool_install_requirements_txt_arguments() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(indoc! { r"
        --index-url https://test.pypi.org/simple
        idna
        "
        })
        .unwrap();

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--with-requirements")
        .arg("requirements.txt")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring `--index-url` from requirements file: `https://test.pypi.org/simple`. Instead, use the `--index-url` command-line argument, or set `index-url` in a `uv.toml` or `pyproject.toml` file.
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + idna==3.6
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [
            { name = "black" },
            { name = "idna" },
        ]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Don't warn, though, if the index URL is the same as the default or as settings.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(indoc! { r"
        --index-url https://pypi.org/simple
        idna
        "
        })
        .unwrap();

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--with-requirements")
        .arg("requirements.txt")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    `black` is already installed
    ");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(indoc! { r"
        --index-url https://test.pypi.org/simple
        idna
        "
        })
        .unwrap();

    // Install `flask`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .arg("--with-requirements")
        .arg("requirements.txt")
        .arg("--index-url")
        .arg("https://test.pypi.org/simple")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 8 packages in [TIME]
    Installed 8 packages in [TIME]
     + blinker==1.7.0
     + click==8.1.7
     + flask==3.0.2
     + idna==2.7
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + werkzeug==3.0.1
    Installed 1 executable: flask
    ");
}

/// Test upgrading an already installed tool.
#[test]
fn tool_install_upgrade() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.1.1")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.1.1
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", specifier = "==24.1.1" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Install without the constraint. It should be replaced, but the package shouldn't be installed
    // since it's already satisfied in the environment.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Checked [N] packages in [TIME]
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Install with a `with`. It should be added to the environment.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--with")
        .arg("iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + iniconfig==2.0.0 (from https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl)
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [
            { name = "black" },
            { name = "iniconfig", url = "https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl" },
        ]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Install with `--upgrade`. `black` should be reinstalled with a more recent version, and
    // `iniconfig` should be removed.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--upgrade")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     - black==24.1.1
     + black==24.3.0
     - iniconfig==2.0.0 (from https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl)
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });
}

/// Test reinstalling tools with varying `--python` requests.
#[test]
fn tool_install_python_requests() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-p")
        .arg("3.12")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    // Install with Python 3.12 (compatible).
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-p")
        .arg("3.12")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    `black` is already installed
    ");

    // // Install with Python 3.11 (incompatible).
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-p")
        .arg("3.11")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Ignoring existing environment for `black`: the requested Python interpreter does not match the environment interpreter
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");
}

/// Test reinstalling tools with varying `--python` and
/// `--python-preference` parameters.
#[ignore = "https://github.com/astral-sh/uv/issues/7473"]
#[test]
fn tool_install_python_preference() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-p")
        .arg("3.12")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    "###);

    // Install with Python 3.12 (compatible).
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-p")
        .arg("3.12")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    `black` is already installed
    "###);

    // Install with system Python 3.11 (different version, incompatible).
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-p")
        .arg("3.11")
        .arg("--python-preference")
        .arg("only-system")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    Ignoring existing environment for `black`: the requested Python interpreter does not match the environment interpreter
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    "###);

    // Install with system Python 3.11 (compatible).
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-p")
        .arg("3.11")
        .arg("--python-preference")
        .arg("only-system")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    `black` is already installed
    "###);

    // Install with managed Python 3.11 (different source, incompatible).
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-p")
        .arg("3.11")
        .arg("--python-preference")
        .arg("only-managed")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    Ignoring existing environment for `black`: the requested Python interpreter does not match the environment interpreter
    Resolved [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    "###);

    // Install with managed Python 3.11 (compatible).
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("-p")
        .arg("3.11")
        .arg("--python-preference")
        .arg("only-managed")
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    `black` is already installed
    "###);
}

/// Test preserving a tool environment when new but incompatible requirements are requested.
#[test]
fn tool_install_preserve_environment() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.1.1")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.1.1
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    // Install `black`, but with an incompatible requirement.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.1.1")
        .arg("--with")
        .arg("packaging==0.0.1")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because black==24.1.1 depends on packaging>=22.0 and you require black==24.1.1, we can conclude that you require packaging>=22.0.
             And because you require packaging==0.0.1, we can conclude that your requirements are unsatisfiable.
    ");

    // Install `black`. The tool should already be installed, since we didn't remove the environment.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.1.1")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    `black==24.1.1` is already installed
    ");
}

/// Test warning when the binary directory is not on the user's PATH.
#[test]
#[cfg(unix)]
fn tool_install_warn_path() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.1.1")
        .env_remove(EnvVars::PATH), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.1.1
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    warning: `[TEMP_DIR]/bin` is not on your PATH. To use installed tools, run `export PATH="[TEMP_DIR]/bin:$PATH"` or `uv tool update-shell`.
    "#);
}

/// Test installing and reinstalling with an invalid receipt.
#[test]
fn tool_install_bad_receipt() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    // Override the `uv-receipt.toml` file with an invalid receipt.
    tool_dir
        .child("black")
        .child("uv-receipt.toml")
        .write_str("invalid")?;

    // Reinstall `black`, which should remove the invalid receipt.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Removed existing `black` with invalid receipt
    Resolved [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    Ok(())
}

/// Test installing a tool with a malformed `.dist-info` directory (i.e., a `.dist-info` directory
/// that isn't properly normalized).
#[test]
fn tool_install_malformed_dist_info() {
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-18T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `executable-application`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("executable-application")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + executable-application==0.3.0
    Installed 1 executable: app
    ");

    tool_dir
        .child("executable-application")
        .assert(predicate::path::is_dir());
    tool_dir
        .child("executable-application")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("app{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/executable-application/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from executable_application import main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("executable-application").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "executable-application" }]
        entrypoints = [
            { name = "app", install-path = "[TEMP_DIR]/bin/app", from = "executable-application" },
        ]

        [tool.options]
        exclude-newer = "2025-01-18T00:00:00Z"
        "#);
    });
}

/// Test installing, then re-installing with different settings.
#[test]
fn tool_install_settings() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install the lowest `flask>=3` version and the latest compatible dependencies.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask>=3")
        .arg("--resolution=lowest-direct")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + blinker==1.7.0
     + click==8.1.7
     + flask==3.0.0
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + werkzeug==3.0.1
    Installed 1 executable: flask
    ");

    tool_dir.child("flask").assert(predicate::path::is_dir());
    tool_dir
        .child("flask")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("flask{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/flask/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from flask.cli import main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("flask").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "flask", specifier = ">=3" }]
        entrypoints = [
            { name = "flask", install-path = "[TEMP_DIR]/bin/flask", from = "flask" },
        ]

        [tool.options]
        resolution = "lowest-direct"
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Reinstall with `highest`. This is a no-op, since we _do_ have a compatible version installed.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask>=3")
        .arg("--resolution=highest")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    `flask>=3` is already installed
    ");

    // It should update the receipt though.
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("flask").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "flask", specifier = ">=3" }]
        entrypoints = [
            { name = "flask", install-path = "[TEMP_DIR]/bin/flask", from = "flask" },
        ]

        [tool.options]
        resolution = "highest"
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Reinstall with `highest` and `--upgrade`. This should change the setting and install a higher
    // version.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask>=3")
        .arg("--resolution=highest")
        .arg("--upgrade")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     - flask==3.0.0
     + flask==3.0.2
    Installed 1 executable: flask
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("flask").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "flask", specifier = ">=3" }]
        entrypoints = [
            { name = "flask", install-path = "[TEMP_DIR]/bin/flask", from = "flask" },
        ]

        [tool.options]
        resolution = "highest"
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });
}

/// Test installing a tool with `uv tool install {package}@{version}`.
#[test]
fn tool_install_at_version() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black` at `24.1.0`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black@24.1.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.1.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", specifier = "==24.1.0" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Combining `{package}@{version}` with a `--from` should fail (even if they're ultimately
    // compatible).
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black@24.1.0")
        .arg("--from")
        .arg("black==24.1.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package requirement (`black==24.1.0`) provided with `--from` conflicts with install request (`black@24.1.0`)
    ");
}

/// Test installing a tool with `uv tool install {package}@latest`.
#[test]
fn tool_install_at_latest() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black` at latest.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black@latest")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });
}

/// Test installing a tool with `uv tool install {package} --from {package}@latest`.
#[test]
fn tool_install_from_at_latest() {
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-18T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("app")
        .arg("--from")
        .arg("executable-application@latest")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + executable-application==0.3.0
    Installed 1 executable: app
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("executable-application").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "executable-application" }]
        entrypoints = [
            { name = "app", install-path = "[TEMP_DIR]/bin/app", from = "executable-application" },
        ]

        [tool.options]
        exclude-newer = "2025-01-18T00:00:00Z"
        "#);
    });
}

/// Test installing a tool with `uv tool install {package} --from {package}@{version}`.
#[test]
fn tool_install_from_at_version() {
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-18T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("app")
        .arg("--from")
        .arg("executable-application@0.2.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + executable-application==0.2.0
    Installed 1 executable: app
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("executable-application").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "executable-application", specifier = "==0.2.0" }]
        entrypoints = [
            { name = "app", install-path = "[TEMP_DIR]/bin/app", from = "executable-application" },
        ]

        [tool.options]
        exclude-newer = "2025-01-18T00:00:00Z"
        "#);
    });
}

/// Test upgrading an already installed tool via `{package}@{latest}`.
#[test]
fn tool_install_at_latest_upgrade() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.1.1")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.1.1
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", specifier = "==24.1.1" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Install without the constraint. It should be replaced, but the package shouldn't be installed
    // since it's already satisfied in the environment.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Checked [N] packages in [TIME]
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Install with `{package}@{latest}`. `black` should be reinstalled with a more recent version.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black@latest")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     - black==24.1.1
     + black==24.3.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });
}

/// Install a tool with `--constraints`.
#[test]
fn tool_install_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let constraints_txt = context.temp_dir.child("constraints.txt");
    constraints_txt.write_str(indoc::indoc! {r"
        mypy-extensions<1
        anyio>=3
    "})?;

    // Install `black`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--constraints")
        .arg(constraints_txt.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==0.4.4
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        constraints = [
            { name = "mypy-extensions", specifier = "<1" },
            { name = "anyio", specifier = ">=3" },
        ]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Installing with the same constraints should be a no-op.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--constraints")
        .arg(constraints_txt.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    `black` is already installed
    ");

    let constraints_txt = context.temp_dir.child("constraints.txt");
    constraints_txt.write_str(indoc::indoc! {r"
        platformdirs<4
    "})?;

    // Installing with revised constraints should reinstall the tool.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--constraints")
        .arg(constraints_txt.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     - platformdirs==4.2.0
     + platformdirs==3.11.0
    Installed 2 executables: black, blackd
    ");

    Ok(())
}

/// Install a tool with `--overrides`.
#[test]
fn tool_install_overrides() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    let overrides_txt = context.temp_dir.child("overrides.txt");
    overrides_txt.write_str(indoc::indoc! {r"
        click<8
        anyio>=3
    "})?;

    // Install `black`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--overrides")
        .arg(overrides_txt.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==7.1.2
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black" }]
        overrides = [
            { name = "click", specifier = "<8" },
            { name = "anyio", specifier = ">=3" },
        ]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    Ok(())
}

/// `uv tool install python` is not allowed
#[test]
fn tool_install_python() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `python`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("python")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot install Python with `uv tool install`. Did you mean to use `uv python install`?
    ");

    // Install `python@<version>`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("python@3.12")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot install Python with `uv tool install`. Did you mean to use `uv python install`?
    ");
}

#[test]
fn tool_install_mismatched_name() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--from")
        .arg("https://files.pythonhosted.org/packages/af/47/93213ee66ef8fae3b93b3e29206f6b251e65c97bd91d8e1c5596ef15af0a/flask-3.1.0-py3-none-any.whl")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package name (`flask`) provided with `--from` does not match install request (`black`)
    ");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--from")
        .arg("flask @ https://files.pythonhosted.org/packages/af/47/93213ee66ef8fae3b93b3e29206f6b251e65c97bd91d8e1c5596ef15af0a/flask-3.1.0-py3-none-any.whl")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package name (`flask`) provided with `--from` does not match install request (`black`)
    ");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("flask")
        .arg("--from")
        .arg("black @ https://files.pythonhosted.org/packages/af/47/93213ee66ef8fae3b93b3e29206f6b251e65c97bd91d8e1c5596ef15af0a/flask-3.1.0-py3-none-any.whl")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package name (`black`) provided with `--from` does not match install request (`flask`)
    ");
}

/// When installing from an authenticated index, the credentials should be omitted from the receipt.
#[tokio::test]
async fn tool_install_credentials() {
    let proxy = crate::pypi_proxy::start().await;
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-18T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `executable-application`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("executable-application")
         .arg("--index")
        .arg(proxy.authenticated_url("public", "heron", "/basic-auth/simple"))
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + executable-application==0.3.0
    Installed 1 executable: app
    ");

    tool_dir
        .child("executable-application")
        .assert(predicate::path::is_dir());
    tool_dir
        .child("executable-application")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("app{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/executable-application/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from executable_application import main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(main())
        "#);

    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("executable-application").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "executable-application" }]
        entrypoints = [
            { name = "app", install-path = "[TEMP_DIR]/bin/app", from = "executable-application" },
        ]

        [tool.options]
        index = [{ url = "http://[LOCALHOST]/basic-auth/simple", explicit = false, default = false, format = "simple", authenticate = "always" }]
        exclude-newer = "2025-01-18T00:00:00Z"
        "#);
    });
}

/// When installing from an authenticated index, the credentials should be omitted from the receipt.
#[tokio::test]
async fn tool_install_default_credentials() -> Result<()> {
    let proxy = crate::pypi_proxy::start().await;
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-18T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Write a `uv.toml` with a default index that has credentials.
    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml.write_str(&format!(
        indoc::indoc! {r#"
        [[index]]
        url = "{}"
        default = true
        authenticate = "always"
    "#},
        proxy.authenticated_url("public", "heron", "/basic-auth/simple")
    ))?;

    // Install `executable-application`
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("executable-application")
        .arg("--config-file")
        .arg(uv_toml.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + executable-application==0.3.0
    Installed 1 executable: app
    ");

    tool_dir
        .child("executable-application")
        .assert(predicate::path::is_dir());
    tool_dir
        .child("executable-application")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("app{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run black in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/executable-application/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from executable_application import main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(main())
        "#);
    });

    insta::with_settings!({
        filters => context.filters(),
    }, {
        // We should have a tool receipt
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("executable-application").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "executable-application" }]
        entrypoints = [
            { name = "app", install-path = "[TEMP_DIR]/bin/app", from = "executable-application" },
        ]

        [tool.options]
        index = [{ url = "http://[LOCALHOST]/basic-auth/simple", explicit = false, default = true, format = "simple", authenticate = "always" }]
        exclude-newer = "2025-01-18T00:00:00Z"
        "#);
    });

    // Attempt to upgrade without providing the credentials (from the config file).
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("executable-application")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to upgrade executable-application
      cause: Failed to fetch: `http://[LOCALHOST]/basic-auth/simple/executable-application/`
      cause: Missing credentials for http://[LOCALHOST]/basic-auth/simple/executable-application/
    ");

    // Attempt to upgrade.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("executable-application")
        .arg("--config-file")
        .arg(uv_toml.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to upgrade
    ");

    Ok(())
}

/// Test installing a tool with `--with-executables-from`.
#[test]
fn tool_install_with_executables_from() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--with-executables-from")
        .arg("ansible-core,black")
        .arg("ansible==9.3.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + ansible==9.3.0
     + ansible-core==2.16.4
     + black==24.3.0
     + cffi==1.16.0
     + click==8.1.7
     + cryptography==42.0.5
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
     + pycparser==2.21
     + pyyaml==6.0.1
     + resolvelib==1.0.1
    Installed 11 executables from `ansible-core`: ansible, ansible-config, ansible-connection, ansible-console, ansible-doc, ansible-galaxy, ansible-inventory, ansible-playbook, ansible-pull, ansible-test, ansible-vault
    Installed 2 executables from `black`: black, blackd
    Installed 1 executable: ansible-community
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("ansible").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [
            { name = "ansible", specifier = "==9.3.0" },
            { name = "ansible-core" },
            { name = "black" },
        ]
        entrypoints = [
            { name = "ansible", install-path = "[TEMP_DIR]/bin/ansible", from = "ansible-core" },
            { name = "ansible-community", install-path = "[TEMP_DIR]/bin/ansible-community", from = "ansible" },
            { name = "ansible-config", install-path = "[TEMP_DIR]/bin/ansible-config", from = "ansible-core" },
            { name = "ansible-connection", install-path = "[TEMP_DIR]/bin/ansible-connection", from = "ansible-core" },
            { name = "ansible-console", install-path = "[TEMP_DIR]/bin/ansible-console", from = "ansible-core" },
            { name = "ansible-doc", install-path = "[TEMP_DIR]/bin/ansible-doc", from = "ansible-core" },
            { name = "ansible-galaxy", install-path = "[TEMP_DIR]/bin/ansible-galaxy", from = "ansible-core" },
            { name = "ansible-inventory", install-path = "[TEMP_DIR]/bin/ansible-inventory", from = "ansible-core" },
            { name = "ansible-playbook", install-path = "[TEMP_DIR]/bin/ansible-playbook", from = "ansible-core" },
            { name = "ansible-pull", install-path = "[TEMP_DIR]/bin/ansible-pull", from = "ansible-core" },
            { name = "ansible-test", install-path = "[TEMP_DIR]/bin/ansible-test", from = "ansible-core" },
            { name = "ansible-vault", install-path = "[TEMP_DIR]/bin/ansible-vault", from = "ansible-core" },
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), context.tool_uninstall()
        .arg("ansible")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 14 executables: ansible, ansible-community, ansible-config, ansible-connection, ansible-console, ansible-doc, ansible-galaxy, ansible-inventory, ansible-playbook, ansible-pull, ansible-test, ansible-vault, black, blackd
    ");
}

/// Test installing a tool with `--with-executables-from`, but the package has no entrypoints.
#[test]
fn tool_install_with_executables_from_no_entrypoints() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Try to install flask with executables from requests (which has no executables)
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--with-executables-from")
        .arg("requests")
        .arg("flask")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    No executables are provided by package `requests`

    hint: Use `--with requests` to include `requests` as a dependency without installing its executables

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + blinker==1.7.0
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + click==8.1.7
     + flask==3.0.2
     + idna==3.6
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + requests==2.31.0
     + urllib3==2.2.1
     + werkzeug==3.0.1
    Installed 1 executable: flask
    ");
}

#[test]
fn tool_install_find_links() {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Run with `--find-links`.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/"))
        .arg("basic-app"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from basic-app!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-app==0.1.0
    ");

    // Install with `--find-links`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/"))
        .arg("basic-app")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-app==0.1.0
    Installed 1 executable: basic-app
    ");

    tool_dir
        .child("basic-app")
        .assert(predicate::path::is_dir());
    tool_dir
        .child("basic-app")
        .child("uv-receipt.toml")
        .assert(predicate::path::exists());

    let executable = bin_dir.child(format!("basic-app{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.exists());

    // On Windows, we can't snapshot an executable file.
    #[cfg(not(windows))]
    insta::with_settings!({
        filters => context.filters(),
    }, {
        // Should run basic-app in the virtual environment
        assert_snapshot!(fs_err::read_to_string(executable).unwrap(), @r#"
        #![TEMP_DIR]/tools/basic-app/bin/python
        # -*- coding: utf-8 -*-
        import sys
        from basic_app import main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(main())
        "#);
    });

    // Run the installed version with `--find-links` on the CLI again.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--offline")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/"))
        .arg("basic-app"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from basic-app!
    ");

    // Run the installed version without `--find-links`.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--offline")
        .arg("basic-app"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving tool dependencies
      cause: Because basic-app==0.1 needs to be downloaded from a registry and only basic-app==0.1 is available, we can conclude that all versions of basic-app cannot be used.
             And because you require basic-app, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because the network was disabled. When the network is disabled, registry packages may only be read from the cache.
    ");
}

#[test]
fn tool_install_python_platform() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black` for macos.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--python-platform")
        .arg("macos")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    // Install `black` for Linux.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--python-platform")
        .arg("linux")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     ~ black==24.3.0
    Installed 2 executables: black, blackd
    ");
}

/// Reinstalling a tool after the underlying Python has been removed.
///
/// Regression test for <https://github.com/astral-sh/uv/issues/16252>.
#[test]
fn tool_install_removed_python() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let (_, python_executable) = context.python_versions.first().unwrap();

    // Install `black` with an explicit Python request.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--python")
        .arg(python_executable)
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    let tool_root = tool_dir.child("black");

    // Simulate the tool's interpreter disappearing without copying an arbitrary system prefix
    // like `/usr` into the test directory.
    cfg_select! {
        unix => {
            let tool_python = tool_root.child("bin").child("python");
            fs_err::remove_file(&tool_python).unwrap();
            fs_err::os::unix::fs::symlink(context.temp_dir.join("missing-python"), &tool_python)
                .unwrap();
        },
        windows => {
            let pyvenv_cfg = tool_root.child("pyvenv.cfg");
            let broken_home = context.temp_dir.join("missing-python");
            let contents = fs_err::read_to_string(&pyvenv_cfg).unwrap();
            let contents = contents
                .lines()
                .map(|line| {
                    if line.starts_with("home = ") {
                        format!("home = {}", broken_home.simplified_display())
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            fs_err::write(&pyvenv_cfg, format!("{contents}\n")).unwrap();
        },
    }

    // Reinstalling should skip the broken Python install.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--reinstall")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");
}

#[test]
fn tool_install_locks_are_preview() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let links = context.workspace_root.join("test/links");

    context
        .tool_install()
        .arg("simple-launcher")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links)
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    let lock_path = tool_dir.child("simple-launcher").child("uv.lock");
    lock_path.assert(predicate::path::missing());

    context
        .tool_install()
        .arg("simple-launcher")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links)
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(context.read("tools/simple-launcher/uv.lock"), @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [{ name = "simple-launcher" }]

        [[package]]
        name = "simple-launcher"
        version = "0.1.0"
        source = { registry = "[WORKSPACE]/test/links" }
        wheels = [
            { path = "[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl" },
        ]
        "#);
    });
}

#[test]
fn tool_install_lock_supports_local_wheel() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let wheel = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");

    for _ in 0..2 {
        context
            .tool_install()
            .arg(&wheel)
            .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
            .env(EnvVars::PATH, bin_dir.as_os_str())
            .assert()
            .success();
    }

    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(context.read("tools/simple-launcher/uv.lock"), @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [{ name = "simple-launcher", path = "[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl" }]

        [[package]]
        name = "simple-launcher"
        version = "0.1.0"
        source = { path = "[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl" }
        wheels = [
            { filename = "simple_launcher-0.1.0-py3-none-any.whl", hash = "sha256:5327e0bb67cdb46800999de6dcf034bf0a5335702883494af0d8b7f6ca48cee4" },
        ]
        "#);
    });
}

#[test]
fn tool_install_lock_verifies_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let wheel = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");

    context
        .tool_install()
        .arg(&wheel)
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    let lock_path = tool_dir.child("simple-launcher").child("uv.lock");
    let lock = fs_err::read_to_string(&lock_path)?;
    lock_path.write_str(&lock.replace(
        "sha256:5327e0bb67cdb46800999de6dcf034bf0a5335702883494af0d8b7f6ca48cee4",
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    ))?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg(&wheel)
        .arg("--force")
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to read `simple-launcher @ file://[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl`
      cause: Hash mismatch for `simple-launcher @ file://[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl`

             Expected:
               sha256:0000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:5327e0bb67cdb46800999de6dcf034bf0a5335702883494af0d8b7f6ca48cee4
    "#);

    Ok(())
}

#[test]
fn tool_install_lock_refreshes_local_directory_constraint() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let local_package = context.temp_dir.child("simple-launcher");
    local_package.create_dir_all()?;
    local_package.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "simple-launcher"
        version = "1.0.0"
        requires-python = ">=3.12"

        [project.scripts]
        simple-launcher = "simple_launcher:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    let local_package_src = local_package.child("src").child("simple_launcher");
    local_package_src.create_dir_all()?;
    local_package_src
        .child("__init__.py")
        .write_str("def main(): pass\n")?;
    let constraints_txt = context.temp_dir.child("constraints.txt");
    constraints_txt.write_str(&format!(
        "simple-launcher @ {}\n",
        local_package.path().display()
    ))?;

    context
        .tool_install()
        .arg("simple-launcher")
        .arg("--constraints")
        .arg(constraints_txt.as_os_str())
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("simple-launcher")
        .arg("--constraints")
        .arg(constraints_txt.as_os_str())
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    `simple-launcher` is already installed
    ");

    local_package.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "simple-launcher"
        version = "2.0.0"
        requires-python = ">=3.12"

        [project.scripts]
        simple-launcher = "simple_launcher:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("simple-launcher")
        .arg("--constraints")
        .arg(constraints_txt.as_os_str())
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     - simple-launcher==1.0.0 (from file://[TEMP_DIR]/simple-launcher)
     + simple-launcher==2.0.0 (from file://[TEMP_DIR]/simple-launcher)
    Installed 1 executable: simple-launcher
    ");

    // A validation warning should retain the parse error that prevents reusing the tool lock.
    local_package.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "simple-launcher"
        version = 42
    "#})?;
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("simple-launcher")
        .arg("--constraints")
        .arg(constraints_txt.as_os_str())
        .arg("--offline")
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Failed to validate existing tool lock
      cause: Failed to parse `[TEMP_DIR]/simple-launcher/pyproject.toml`
      cause: TOML parse error at line 3, column 11
               |
             3 | version = 42
               |           ^^
             invalid type: integer `42`, expected a string
    error: Failed to build `simple-launcher @ file://[TEMP_DIR]/simple-launcher`
      cause: Failed to parse metadata from built wheel
      cause: TOML parse error at line 3, column 11
               |
             3 | version = 42
               |           ^^
             invalid type: integer `42`, expected a string
    ");

    Ok(())
}

/// Ensure that changing a constraint invalidates an otherwise reusable tool lock.
#[test]
fn tool_install_lock_revalidates_changed_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let constraints_txt = context.temp_dir.child("constraints.txt");
    constraints_txt.write_str("platformdirs>=4\n")?;

    context
        .tool_install()
        .arg("black")
        .arg("--constraints")
        .arg(constraints_txt.as_os_str())
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    constraints_txt.write_str("platformdirs<4\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black")
        .arg("--constraints")
        .arg(constraints_txt.as_os_str())
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared [N] packages in [TIME]
    Uninstalled [N] packages in [TIME]
    Installed [N] packages in [TIME]
     - platformdirs==4.2.0
     + platformdirs==3.11.0
    Installed 2 executables: black, blackd
    ");

    Ok(())
}
