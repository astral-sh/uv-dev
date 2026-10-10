#![expect(clippy::disallowed_types)]

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::{fixture::ChildPath, prelude::*};
use indoc::{formatdoc, indoc};
use insta::assert_snapshot;
use predicates::{prelude::predicate, str::contains};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use uv_fs::copy_dir_all;
use uv_python_discovery::PYTHON_VERSION_FILENAME;
use uv_static::EnvVars;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_test::packse::generate_wheel_with_files;
use uv_test::{TestContext, packse::PackseServer, site_packages_path, uv_snapshot, venv_bin_path};

#[test]
fn run_with_python_version() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11", "3.9"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = [
          "anyio==3.6.0 ; python_version == '3.11'",
          "anyio==3.7.0 ; python_version == '3.12'",
        ]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        import importlib.metadata
        import platform

        print(platform.python_version())
        print(importlib.metadata.version("anyio"))
       "#
    })?;

    // Our tests change files in <1s, so we must disable CPython bytecode caching with `-B` or we'll
    // get stale files, see https://github.com/python/cpython/issues/75953.
    let mut command = context.run();
    let command_with_args = command.arg("python").arg("-B").arg("main.py");
    uv_snapshot!(context.filters(), command_with_args, @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12.[X]
    3.7.0

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 5 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==3.7.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    // This is the same Python, no reinstallation.
    let mut command = context.run();
    let command_with_args = command
        .arg("-p")
        .arg("3.12")
        .arg("python")
        .arg("-B")
        .arg("main.py");
    uv_snapshot!(context.filters(), command_with_args, @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12.[X]
    3.7.0

    ----- stderr -----
    Resolved 5 packages in [TIME]
    Checked 4 packages in [TIME]
    ");

    // This time, we target Python 3.11 instead.
    let mut command = context.run();
    let command_with_args = command
        .arg("-p")
        .arg("3.11")
        .arg("python")
        .arg("-B")
        .arg("main.py")
        .env_remove(EnvVars::VIRTUAL_ENV);

    uv_snapshot!(context.filters(), command_with_args, @"
    exit_code: 0 (success)
    ----- stdout -----
    3.11.[X]
    3.6.0

    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 4 packages in [TIME]
     + anyio==3.6.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    // This time, we target Python 3.9 instead.
    let mut command = context.run();
    let command_with_args = command
        .arg("-p")
        .arg("3.9")
        .arg("python")
        .arg("-B")
        .arg("main.py")
        .env_remove(EnvVars::VIRTUAL_ENV);

    uv_snapshot!(context.filters(), command_with_args, @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    error: The requested interpreter resolved to Python 3.9.[X], which is incompatible with the project's Python requirement: `>=3.11, <4` (from `project.requires-python`)
    ");

    Ok(())
}

#[test]
fn run_args() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filter((
            r"Usage: uv(?:\.exe)? run \[OPTIONS\] (?s:.*?)(\n----- stderr -----|$)",
            "[UV RUN HELP]$1",
        ))
        .with_filter((
            r"usage: (?s:.*?)(\n----- stderr -----|$)",
            "usage: [PYTHON HELP]$1",
        ));

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = []

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    // We treat arguments before the command as uv arguments
    uv_snapshot!(context.filters(), context.run().arg("--help").arg("python"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Run a command or script

    [UV RUN HELP]");

    // We don't treat arguments after the command as uv arguments
    uv_snapshot!(context.filters(), context.run().arg("python").arg("--help"), @"
    exit_code: 0 (success)
    ----- stdout -----
    usage: [PYTHON HELP]
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
    ");

    // Can use `--` to separate uv arguments from the command arguments.
    uv_snapshot!(context.filters(), context.run().arg("--").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// Run without specifying any arguments.
///
/// This should list the available scripts.
#[test]
fn run_no_args() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = []

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    // Run without specifying any arguments.
    #[cfg(not(windows))]
    uv_snapshot!(context.filters(), context.run(), @"
    exit_code: 2 (failure)
    ----- stdout -----
    Provide a command or script to invoke with `uv run <command>` or `uv run <script>.py`.

    The following commands are available in the environment:

    - python
    - python3
    - python3.12

    See `uv run --help` for more information.

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
    ");

    #[cfg(windows)]
    uv_snapshot!(context.filters(), context.run(), @r###"
    exit_code: 2 (failure)
    ----- stdout -----
    Provide a command or script to invoke with `uv run <command>` or `uv run <script>.py`.

    The following commands are available in the environment:

    - pydoc.bat
    - python
    - pythonw

    See `uv run --help` for more information.

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
    "###);

    Ok(())
}

/// Run a PEP 723-compatible script. The script should take precedence over the workspace
/// dependencies.
#[test]
fn run_pep723_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    // If the script contains a PEP 723 tag, we should install its requirements.
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///

        import iniconfig
       "#
    })?;

    // Running the script should install the requirements.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Running again should use the existing environment.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ");

    // But neither invocation should create a lockfile.
    assert!(!context.temp_dir.child("main.py.lock").exists());

    // Otherwise, the script requirements should _not_ be available, but the project requirements
    // should.
    let test_non_script = context.temp_dir.child("main.py");
    test_non_script.write_str(indoc! { r"
        import iniconfig
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    Traceback (most recent call last):
      File "[TEMP_DIR]/main.py", line 1, in <module>
        import iniconfig
    ModuleNotFoundError: No module named 'iniconfig'
    "#);

    // But the script should be runnable.
    let test_non_script = context.temp_dir.child("main.py");
    test_non_script.write_str(indoc! { r#"
        import idna

        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    ");

    // If the script contains a PEP 723 tag, it can omit the dependencies field.
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # ///

        print("Hello, world!")
       "#
    })?;

    // Running the script should succeed without installing any requirements.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!
    ");

    // Running a script with `--locked` should error.
    uv_snapshot!(context.filters(), context.run().arg("--locked").arg("main.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to find lockfile for Python script, but `--locked` was provided. To create a lockfile, run `uv lock --script`.
    ");

    // Running a script with `UV_LOCKED` should warn (not error).
    uv_snapshot!(context.filters(), context.run().env("UV_LOCKED", "1").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    warning: No lockfile found for Python script (ignoring `UV_LOCKED=1`); run `uv lock --script` to generate a lockfile
    ");

    // If the script can't be resolved, we should reference the script.
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "add",
        # ]
        # ///
       "#
    })?;

    // Running a script with `--group` should warn.
    uv_snapshot!(context.filters(), context.run().arg("--group").arg("foo").arg("main.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving script dependencies
      cause: Because there are no versions of add and you require add, we can conclude that your requirements are unsatisfiable.
    ");

    // If the script can't be resolved, we should reference the script.
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "add",
        # ]
        # ///
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("main.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving script dependencies
      cause: Because there are no versions of add and you require add, we can conclude that your requirements are unsatisfiable.
    ");

    // If the script contains an unclosed PEP 723 tag, we should error.
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]

        # ///

        import iniconfig
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("main.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: An opening tag (`# /// script`) was found without a closing tag (`# ///`). Ensure that every line between the opening and closing tags (including empty lines) starts with a leading `#`.
    ");

    // Regression test for: <https://github.com/astral-sh/uv/issues/18617>
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # dependencies = []
        # ///

        print("Hello, world!")

        # /// script
        # dependencies = []
        # ///
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("main.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The script contains multiple PEP 723 metadata blocks
    ");

    Ok(())
}

#[test]
fn run_pep723_script_empty_dependency() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("script.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [""]
        # ///
       "#
    })?;

    // The invalid requirement is empty, so the PEP 508 error should not include an orphaned caret;
    // see astral-sh/uv#21089.
    uv_snapshot!(context.filters(), context.run().arg("--script").arg("script.py"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: TOML parse error at line 2, column 17
      |
    2 | dependencies = [""]
      |                 ^^
    Empty field is not allowed for PEP508
    "#);

    Ok(())
}

/// Equivalent scripts should retain their own writable environments while sharing their installed
/// dependencies when shared script environments are enabled.
#[test]
fn run_pep723_scripts_share_immutable_environment() -> Result<()> {
    fn shared_base(configuration: &str) -> Option<&str> {
        configuration.lines().find_map(|line| {
            line.split_once('=')
                .filter(|(key, _)| key.trim() == "extends-environment")
                .map(|(_, value)| value.trim())
        })
    }

    let context = uv_test::test_context!("3.12").with_pyvenv_cfg_filters();
    let script = indoc! { r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["iniconfig==2.0.0"]
        # ///

        import iniconfig
        import sys

        print(sys.prefix)
        print(iniconfig.__file__)
        "#
    };
    context.temp_dir.child("first.py").write_str(script)?;
    context.temp_dir.child("second.py").write_str(script)?;

    let first = uv_snapshot!(context.filters(), context.run()
        .arg("--preview-features")
        .arg("shared-script-environments")
        .arg("first.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-first-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/iniconfig/__init__.py

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");
    let second = uv_snapshot!(context.filters(), context.run()
        .arg("--preview-features")
        .arg("shared-script-environments")
        .arg("second.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-second-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/iniconfig/__init__.py

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    let first_stdout = std::str::from_utf8(&first.stdout)?;
    let second_stdout = std::str::from_utf8(&second.stdout)?;
    let mut first_lines = first_stdout.lines();
    let mut second_lines = second_stdout.lines();
    let first_root = first_lines
        .next()
        .context("first script did not report its virtual environment")?;
    let second_root = second_lines
        .next()
        .context("second script did not report its virtual environment")?;
    let first_import = first_lines
        .next()
        .context("first script did not report its installed dependency")?;
    let second_import = second_lines
        .next()
        .context("second script did not report its installed dependency")?;

    let first_configuration = context.read(Path::new(first_root).join("pyvenv.cfg"));
    let second_configuration = context.read(Path::new(second_root).join("pyvenv.cfg"));
    let first_base = shared_base(&first_configuration)
        .context("first script environment did not identify its shared base")?;
    let second_base = shared_base(&second_configuration)
        .context("second script environment did not identify its shared base")?;

    let shared_configuration = context.read(Path::new(first_base).join("pyvenv.cfg"));

    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(first_configuration, @"
        home = [PYTHON_HOME]
        implementation = CPython
        uv = [UV_VERSION]
        version_info = 3.12.[X]
        include-system-site-packages = false
        prompt = first.py
        extends-environment = [CACHE_DIR]/archive-v0/[HASH]
        ");

        assert_snapshot!(second_configuration, @"
        home = [PYTHON_HOME]
        implementation = CPython
        uv = [UV_VERSION]
        version_info = 3.12.[X]
        include-system-site-packages = false
        prompt = second.py
        extends-environment = [CACHE_DIR]/archive-v0/[HASH]
        ");

        assert_snapshot!(shared_configuration, @"
        home = [PYTHON_HOME]
        implementation = CPython
        uv = [UV_VERSION]
        version_info = 3.12.[X]
        include-system-site-packages = false
        relocatable = true
        immutable = true
        ");


    });

    assert_ne!(first_root, second_root);
    assert_eq!(first_base, second_base);
    assert_eq!(first_import, second_import);

    Ok(())
}

/// Reusing an unchanged script overlay does not rewrite its import-path configuration.
#[test]
fn run_pep723_script_overlay_preserves_pth_timestamp() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("first.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["iniconfig==2.0.0"]
        # ///

        import iniconfig
        import sys

        print(sys.prefix)
        print(iniconfig.__file__)
    "#})?;

    let output = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "first.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-first-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/iniconfig/__init__.py

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");
    let stdout = std::str::from_utf8(&output.stdout)?;
    let script_root = stdout
        .lines()
        .next()
        .context("script did not report its virtual environment")?;

    let overlay_path =
        site_packages_path(Path::new(script_root), "python3.12").join("_uv_ephemeral_overlay.pth");
    let original_contents = context.read(&overlay_path);
    let original_time = filetime::FileTime::from_unix_time(1_700_000_000, 0);
    filetime::set_file_mtime(&overlay_path, original_time)?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "first.py"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-first-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/iniconfig/__init__.py

    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);
    assert_eq!(context.read(&overlay_path), original_contents);
    assert_eq!(
        filetime::FileTime::from_last_modification_time(&fs_err::metadata(&overlay_path)?),
        original_time
    );

    Ok(())
}

/// Installed overlay packages satisfy `--with` before the immutable shared dependency set.
#[test]
fn run_pep723_script_overlay_with_precedence() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("first.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["iniconfig==2.0.0"]
        # ///

        import iniconfig
        import sys

        print(sys.prefix)
        print(iniconfig.__file__)
    "#})?;

    let output = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "first.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-first-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/iniconfig/__init__.py

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");
    let stdout = std::str::from_utf8(&output.stdout)?;
    let script_root = stdout
        .lines()
        .next()
        .context("script did not report its virtual environment")?;

    context
        .pip_install()
        .arg("--python")
        .arg(venv_bin_path(script_root).join(format!("python{}", std::env::consts::EXE_SUFFIX)))
        .arg("iniconfig==1.1.1")
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--with", "iniconfig<2", "first.py"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-first-[HASH]
    [CACHE_DIR]/environments-v2/shared-first-[HASH]/[PYTHON-LIB]/site-packages/iniconfig/__init__.py

    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);

    Ok(())
}

/// All content-addressed environments should advertise their immutability, even without opting in
/// to shared script environments.
#[test]
fn run_with_cached_environment_is_immutable() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_pyvenv_cfg_filters();
    context.temp_dir.child("main.py").write_str(indoc! { r#"
        import pathlib

        import iniconfig

        for parent in pathlib.Path(iniconfig.__file__).parents:
            configuration = parent / "pyvenv.cfg"
            if configuration.is_file():
                print(configuration.read_text(), end="")
                break
        "#
    })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--with")
        .arg("iniconfig==2.0.0")
        .arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    home = [PYTHON_HOME]
    implementation = CPython
    uv = [UV_VERSION]
    version_info = 3.12.[X]
    include-system-site-packages = false
    relocatable = true
    immutable = true

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

/// Mutating one script's overlay must not affect another script using the same cached base.
#[test]
fn run_pep723_script_overlays_isolate_mutations() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("first.py").write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["iniconfig==2.0.0"]
        # ///

        import pathlib
        import sysconfig

        pathlib.Path(sysconfig.get_path("purelib"), "only_first.py").write_text("VALUE = 1")
        "#
    })?;
    context.temp_dir.child("second.py").write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["iniconfig==2.0.0"]
        # ///

        import importlib.util

        print(importlib.util.find_spec("only_first"))
        "#
    })?;

    context
        .run()
        .arg("--preview-features")
        .arg("shared-script-environments")
        .arg("first.py")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.run()
        .arg("--preview-features")
        .arg("shared-script-environments")
        .arg("second.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    None

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    Ok(())
}

/// A PEP 723 script with a long file name should still succeed at creating a script environment on
/// Windows.
#[test]
fn run_pep723_script_long_filename() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // The cache environment entry path, which is derived from the script's name, would exceed many
    // common path component length limits if it was not truncated first.
    let script_name = format!("{}.py", "a".repeat(240));
    let test_script = context.temp_dir.child(&script_name);
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///

        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg(&script_name), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_pep723_script_requires_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    // If we have a `.python-version` that's incompatible with the script, we should use the
    // script's `requires-python` for Python discovery instead.
    let python_version = context.temp_dir.child(PYTHON_VERSION_FILENAME);
    python_version.write_str("3.11")?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.12"
        # ///

        import platform
        print(platform.python_version())
       "#
    })?;

    // The `.python-version` (3.11) is incompatible with the script's `requires-python` (>=3.12),
    // so uv should ignore it and discover a compatible Python (3.12) instead.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12.[X]
    ");

    // Deleting the `.python-version` file should not change the behavior.
    fs_err::remove_file(&python_version)?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12.[X]
    ");

    Ok(())
}

/// When a `.python-version` is compatible with a script's `requires-python`, the `.python-version`
/// should be used.
#[test]
fn run_pep723_script_requires_python_compatible() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    let python_version = context.temp_dir.child(PYTHON_VERSION_FILENAME);
    python_version.write_str("3.11")?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # ///

        import platform
        print(platform.python_version())
       "#
    })?;

    // The `.python-version` (3.11) is compatible with the script's `requires-python` (>=3.11),
    // so it should be used.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.11.[X]
    ");

    Ok(())
}

/// When `.python-version` specifies an incompatible range, script `requires-python` should be used
/// for discovery.
#[test]
fn run_pep723_script_requires_python_incompatible_range() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    let python_version = context.temp_dir.child(PYTHON_VERSION_FILENAME);
    python_version.write_str(">3.8,<3.12")?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.12"
        # ///

        import platform
        print(platform.python_version())
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12.[X]
    ");

    Ok(())
}

/// Run a `.pyw` script. The script should be executed with `pythonw.exe`.
#[test]
fn run_pythonw_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    let test_script = context.temp_dir.child("main.pyw");
    test_script.write_str(indoc! { r"
        import anyio
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.pyw"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    Ok(())
}

/// Run a PEP 723-compatible script with `tool.uv` metadata.
#[test]
#[cfg(feature = "test-git")]
fn run_pep723_script_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // If the script contains a PEP 723 tag, we should install its requirements.
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig>1",
        # ]
        #
        # [tool.uv]
        # resolution = "lowest-direct"
        # ///

        import iniconfig
       "#
    })?;

    // Running the script should honor its inline resolution setting.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==1.0.1
    ");

    // Respect `tool.uv.sources`.
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "uv-public-pypackage",
        # ]
        #
        # [tool.uv.sources]
        # uv-public-pypackage = { git = "https://github.com/astral-test/uv-public-pypackage", rev = "0dacfd662c64cb4ceb16e6cf65a157a8b715b979" }
        # ///

        import uv_public_pypackage
       "#
    })?;

    // The script should succeed with the specified source.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    ");

    Ok(())
}

/// Run a PEP 723-compatible script with a `[[tool.uv.index]]`.
#[test]
fn run_pep723_script_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "idna>=2",
        # ]
        #
        # [[tool.uv.index]]
        # name = "test"
        # url = "https://test.pypi.org/simple"
        # explicit = true
        #
        # [tool.uv.sources]
        # idna = { index = "test" }
        # ///

        import idna
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + idna==2.7
    ");

    Ok(())
}

/// Run a PEP 723-compatible script with a relative index and pinned and unpinned dependencies.
#[test]
fn run_pep723_script_relative_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let scripts = context.temp_dir.child("scripts");
    let links = scripts.child("links");
    links.create_dir_all()?;
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        links.child("ok-1.0.0-py3-none-any.whl"),
    )?;
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/validation-1.0.0-py3-none-any.whl"),
        links.child("validation-1.0.0-py3-none-any.whl"),
    )?;

    let test_script = scripts.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = ["ok", "validation"]
        #
        # [[tool.uv.index]]
        # name = "local"
        # url = "./links"
        # format = "flat"
        #
        # [tool.uv.sources]
        # ok = { index = "local" }
        # ///

        import ok
        import validation
        "#
    })?;

    let elsewhere = context.temp_dir.child("elsewhere");
    elsewhere.create_dir_all()?;

    uv_snapshot!(context.filters(), context.run().current_dir(elsewhere).arg("--offline").arg(test_script.path()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + ok==1.0.0
     + validation==1.0.0
    ");

    Ok(())
}

/// Package-scoped source disabling must not discard unrelated script sources or indexes.
#[test]
fn run_pep723_script_no_sources_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let explicit = PackseServer::new("simple/single-package.toml");
    let default = PackseServer::new("extras/missing-extra.toml");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(&formatdoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "a",
        # ]
        #
        # [[tool.uv.index]]
        # name = "test"
        # url = "{index}"
        # explicit = true
        #
        # [tool.uv.sources]
        # a = {{ index = "test" }}
        # ///

        import a
       "#,
        index = explicit.index_url(),
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--default-index").arg(default.index_url()).arg("--no-sources-package").arg("unrelated").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + a==2.0.0
    ");

    fs_err::remove_dir_all(&context.cache_dir)?;

    uv_snapshot!(context.filters(), context.run().arg("--default-index").arg(default.index_url()).arg("--no-sources-package").arg("a").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + a==1.0.0
    ");

    Ok(())
}

/// Run a PEP 723-compatible script with `tool.uv` constraints.
#[test]
fn run_pep723_script_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio>=3",
        # ]
        #
        # [tool.uv]
        # constraint-dependencies = ["idna<=3"]
        # ///

        import anyio
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.0
     + sniffio==1.3.1
    ");

    Ok(())
}

/// Run a PEP 723-compatible script with `tool.uv` overrides.
#[test]
fn run_pep723_script_overrides() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio>=3",
        # ]
        #
        # [tool.uv]
        # override-dependencies = ["idna<=2"]
        # ///

        import anyio
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==2.0
     + sniffio==1.3.1
    ");

    Ok(())
}

/// Run a PEP 723-compatible script with `tool.uv` excludes.
#[test]
fn run_pep723_script_excludes() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio>=3",
        # ]
        #
        # [tool.uv]
        # exclude-dependencies = ["idna"]
        # ///

        import anyio
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + anyio==4.3.0
     + sniffio==1.3.1
    ");

    Ok(())
}

/// Run a PEP 723-compatible script with `tool.uv` build constraints.
#[test]
fn run_pep723_script_build_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.9");

    let test_script = context.temp_dir.child("main.py");

    // Incompatible build constraints.
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.9"
        # dependencies = [
        #   "anyio>=3",
        #   "requests==1.2"
        # ]
        #
        # [tool.uv]
        # build-constraint-dependencies = ["setuptools==1"]
        # ///

        import anyio
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `requests==1.2.0`
      cause: Failed to resolve requirements from `setup.py` build
      cause: No solution found when resolving: `setuptools>=40.8.0`
      cause: Because you require setuptools>=40.8.0 and setuptools==1, we can conclude that your requirements are unsatisfiable.
    ");

    // Compatible build constraints.
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.9"
        # dependencies = [
        #   "anyio>=3",
        #   "requests==1.2"
        # ]
        #
        # [tool.uv]
        # build-constraint-dependencies = ["setuptools>=40"]
        # ///

        import anyio
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + anyio==4.3.0
     + exceptiongroup==1.2.0
     + idna==3.6
     + requests==1.2.0
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    Ok(())
}

/// Run a PEP 723-compatible script with a lockfile.
#[test]
fn run_pep723_script_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///

        import iniconfig

        print("Hello, world!")
       "#
    })?;

    // Without a lockfile, running with `--locked` should error.
    uv_snapshot!(context.filters(), context.run().arg("--locked").arg("main.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to find lockfile for Python script, but `--locked` was provided. To create a lockfile, run `uv lock --script`.
    ");

    // Explicitly lock the script.
    uv_snapshot!(context.filters(), context.lock().arg("--script").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    let lock = context.read("main.py.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 5
        requires-python = ">=3.11"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [{ name = "iniconfig" }]

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "https://pypi.org/simple" }
        sdist = { url = "https://files.pythonhosted.org/packages/d7/4b/cbd8e699e64a6f16ca3a8220661b5f83792b3017d0f79807cb8708d33913/iniconfig-2.0.0.tar.gz", hash = "sha256:2d91e135bf72d31a410b17c16da610a82cb55f6b0477d1a902134b24a455b8b3", size = 4646, upload-time = "2023-01-07T11:08:11.254Z" }
        wheels = [
            { url = "https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374", size = 5892, upload-time = "2023-01-07T11:08:09.864Z" },
        ]
        "#
        );
    });

    // Run the script.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // With a lockfile, running with `--locked` should not warn.
    uv_snapshot!(context.filters(), context.run().arg("--locked").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    // Modify the metadata.
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio",
        # ]
        # ///

        import anyio

        print("Hello, world!")
       "#
    })?;

    // Re-running the script with `--locked` should error.
    uv_snapshot!(context.filters(), context.run().arg("--locked").arg("main.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");

    // Re-running the script with `--frozen` should also error, but at runtime.
    uv_snapshot!(context.filters(), context.run().arg("--frozen").arg("main.py"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 1 package in [TIME]
    Traceback (most recent call last):
      File "[TEMP_DIR]/main.py", line 8, in <module>
        import anyio
    ModuleNotFoundError: No module named 'anyio'
    "#);

    // Re-running the script should update the lockfile.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let lock = context.read("main.py.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 5
        requires-python = ">=3.11"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [{ name = "anyio" }]

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "https://pypi.org/simple" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "https://files.pythonhosted.org/packages/db/4d/3970183622f0330d3c23d9b8a5f52e365e50381fd484d08e3285104333d3/anyio-4.3.0.tar.gz", hash = "sha256:f75253795a87df48568485fd18cdd2a3fa5c4f7c5be8e5e36637733fce06fed6", size = 159642, upload-time = "2024-02-19T08:36:28.641Z" }
        wheels = [
            { url = "https://files.pythonhosted.org/packages/14/fd/2f20c40b45e4fb4324834aea24bd4afdf1143390242c0b33774da0e2e34f/anyio-4.3.0-py3-none-any.whl", hash = "sha256:048e05d0f6caeed70d731f3db756d35dcc1f35747c8c403364a8332c630441b8", size = 85584, upload-time = "2024-02-19T08:36:26.842Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "https://pypi.org/simple" }
        sdist = { url = "https://files.pythonhosted.org/packages/bf/3f/ea4b9117521a1e9c50344b909be7886dd00a519552724809bb1f486986c2/idna-3.6.tar.gz", hash = "sha256:9ecdbbd083b06798ae1e86adcbfe8ab1479cf864e4ee30fe4e46a003d12491ca", size = 175426, upload-time = "2023-11-25T15:40:54.902Z" }
        wheels = [
            { url = "https://files.pythonhosted.org/packages/c2/e7/a82b05cf63a603df6e68d59ae6a68bf5064484a0718ea5033660af4b54a9/idna-3.6-py3-none-any.whl", hash = "sha256:c05567e9c24a6b9faaa835c4821bad0590fbb9d5779e7caa6e1cc4978e7eb24f", size = 61567, upload-time = "2023-11-25T15:40:52.604Z" },
        ]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "https://pypi.org/simple" }
        sdist = { url = "https://files.pythonhosted.org/packages/a2/87/a6771e1546d97e7e041b6ae58d80074f81b7d5121207425c964ddf5cfdbd/sniffio-1.3.1.tar.gz", hash = "sha256:f4324edc670a0f49750a81b895f35c3adb843cca46f0530f79fc1babb23789dc", size = 20372, upload-time = "2024-02-25T23:20:04.057Z" }
        wheels = [
            { url = "https://files.pythonhosted.org/packages/e9/44/75a9c9421471a6c4805dbf2356f7c181a29c1879239abab1ea2cc8f38b40/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2f6da418d1f1e0fddd844478f41680e794e6051915791a034ff65e5f100525a2", size = 10235, upload-time = "2024-02-25T23:20:01.196Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// With `managed = false`, we should avoid installing the project itself.
#[test]
fn run_managed_false() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv]
        managed = false
        "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    Ok(())
}

#[test]
fn run_exact() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["iniconfig"]
        "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("python").arg("-c").arg("import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Remove `iniconfig`.
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio"]
        "#
    })?;

    // By default, `uv run` uses inexact semantics, so both `iniconfig` and `anyio` should still be available.
    uv_snapshot!(context.filters(), context.run().arg("python").arg("-c").arg("import iniconfig; import anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    // But under `--exact`, `iniconfig` should not be available.
    uv_snapshot!(context.filters(), context.run().arg("--exact").arg("python").arg("-c").arg("import iniconfig"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Uninstalled 1 package in [TIME]
     - iniconfig==2.0.0
    Traceback (most recent call last):
      File "<string>", line 1, in <module>
    ModuleNotFoundError: No module named 'iniconfig'
    "#);

    Ok(())
}

#[test]
fn run_with() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["sniffio==1.3.0"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import sniffio

        print(sniffio.__version__)
       "
    })?;

    // Requesting an unsatisfied requirement should install it.
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("iniconfig").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.3.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + sniffio==1.3.0
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Requesting a satisfied requirement should use the base environment.
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("sniffio").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.3.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 2 packages in [TIME]
    ");

    // Unless the user requests a different version.
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("sniffio<1.3.0").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.2.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 2 packages in [TIME]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.2.0
    ");

    // If we request a dependency that isn't in the base environment, we should still respect any
    // other dependencies. In this case, `sniffio==1.3.0` is not the latest-compatible version, but
    // we should use it anyway.
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("anyio").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.3.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 2 packages in [TIME]
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.0
    ");

    // Even if we run with` --no-sync`.
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("anyio==4.2.0").arg("--no-sync").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.3.0

    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.2.0
     + idna==3.6
     + sniffio==1.3.0
    ");

    // If the dependencies can't be resolved, we should reference `--with`.
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("add").arg("main.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 2 packages in [TIME]
    error: No solution found when resolving `--with` dependencies
      cause: Because there are no versions of add and you require add, we can conclude that your requirements are unsatisfiable.
    ");

    Ok(())
}

#[test]
fn run_with_local_wheel_refreshes_rebuilt_wheel() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    let package = context.temp_dir.child("foo");
    package.child("pyproject.toml").write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    let init = package.child("src").child("foo").child("__init__.py");
    init.write_str(indoc! { r#"
        def hello() -> str:
            return "Hello from foo!"
        "#
    })?;

    context
        .build()
        .arg("--wheel")
        .current_dir(package.path())
        .assert()
        .success();

    let wheel = package.child("dist").child("foo-0.1.0-py3-none-any.whl");
    filetime::set_file_mtime(
        wheel.path(),
        filetime::FileTime::from_unix_time(1_700_000_000, 0),
    )
    .unwrap();

    // First run: install the original wheel.
    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--refresh")
        .arg("--with")
        .arg(wheel.as_os_str())
        .arg("python")
        .arg("-c")
        .arg("import foo; print(foo.hello())")
        .env_remove(EnvVars::VIRTUAL_ENV), @r"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from foo!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo/dist/foo-0.1.0-py3-none-any.whl)
    ");

    init.write_str(indoc! { r#"
        def hello() -> str:
            return "Updated code!"
        "#
    })?;
    fs_err::remove_file(wheel.path())?;

    context
        .build()
        .arg("--wheel")
        .current_dir(package.path())
        .assert()
        .success();

    filetime::set_file_mtime(
        wheel.path(),
        filetime::FileTime::from_unix_time(1_700_000_001, 0),
    )
    .unwrap();

    // Second run: should pick up the rebuilt wheel due to `--refresh`.
    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--refresh")
        .arg("--with")
        .arg(wheel.as_os_str())
        .arg("python")
        .arg("-c")
        .arg("import foo; print(foo.hello())")
        .env_remove(EnvVars::VIRTUAL_ENV), @r"
    exit_code: 0 (success)
    ----- stdout -----
    Updated code!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo/dist/foo-0.1.0-py3-none-any.whl)
    ");

    context.prune().assert().success();

    // Third run: after cache prune, should still see the updated code.
    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--refresh")
        .arg("--with")
        .arg(wheel.as_os_str())
        .arg("python")
        .arg("-c")
        .arg("import foo; print(foo.hello())")
        .env_remove(EnvVars::VIRTUAL_ENV), @r"
    exit_code: 0 (success)
    ----- stdout -----
    Updated code!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo/dist/foo-0.1.0-py3-none-any.whl)
    ");

    Ok(())
}

/// Test that an ephemeral environment writes the path of its parent environment to the `extends-environment` key
/// of its `pyvenv.cfg` file. This feature makes it easier for static-analysis tools like ty to resolve which import
/// search paths are available in these ephemeral environments.
#[test]
fn run_with_pyvenv_cfg_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]).with_pyvenv_cfg_filters();

    // This sets up to test for a regression where we escaped double quotes and backslashes.
    // Windows paths don't allow double quotes and use backslash as a path separator so the path has
    // to differ.
    let parent_environment = context.temp_dir.child(if cfg!(windows) {
        ".\\parent-environment"
    } else {
        "parent\"\\environment"
    });
    context
        .venv()
        .arg(parent_environment.path())
        .assert()
        .success();
    let context = context.with_filtered_path(&parent_environment, "PARENT_VENV");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        import os

        with open(f'{os.getenv("VIRTUAL_ENV")}/pyvenv.cfg') as f:
            print(f.read())
       "#
    })?;

    uv_snapshot!(context.filters(), context.run()
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, parent_environment.path())
        .env(EnvVars::VIRTUAL_ENV, parent_environment.path())
        .arg("--with")
        .arg("iniconfig")
        .arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    home = [PYTHON_HOME]
    implementation = CPython
    uv = [UV_VERSION]
    version_info = 3.12.[X]
    include-system-site-packages = false
    extends-environment = [PARENT_VENV]/


    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_with_overlay_interpreter() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [project.scripts]
        main = "foo:main"

        [project.gui-scripts]
        main_gui = "foo:main_gui"
        "#
    })?;

    let foo = context.temp_dir.child("src").child("foo");
    foo.create_dir_all()?;
    let init_py = foo.child("__init__.py");
    init_py.write_str(indoc! { r#"
        import sys
        import shutil
        from pathlib import Path

        def show_python():
            print(sys.executable)

        def copy_entrypoint():
            base = Path(sys.executable)
            shutil.copyfile(base.with_name("main").with_suffix(base.suffix), sys.argv[1])

        def copy_gui_entrypoint():
            base = Path(sys.executable)
            shutil.copyfile(base.with_name("main_gui").with_suffix(base.suffix), sys.argv[1])

        def main():
            show_python()
            if len(sys.argv) > 1:
                copy_entrypoint()

        def main_gui():
            show_python()
            if len(sys.argv) > 1:
                copy_gui_entrypoint()
       "#
    })?;

    // The project's entrypoint should be rewritten to use the overlay interpreter.
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("iniconfig").arg("main").arg(context.temp_dir.child("main").as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/builds-v0/[TMP]/[BIN]/python

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // The project's gui entrypoint should be rewritten to use the overlay interpreter.
    #[cfg(windows)]
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("iniconfig").arg("main_gui").arg(context.temp_dir.child("main_gui").as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/builds-v0/[TMP]/[BIN]/pythonw

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    Resolved 1 package in [TIME]
    ");

    #[cfg(unix)]
    insta::with_settings!({
        filters => context.filters(),
    }, {
            assert_snapshot!(
                context.read("main"), @r#"
            #![CACHE_DIR]/builds-v0/[TMP]/[BIN]/python
            # -*- coding: utf-8 -*-
            import sys
            from foo import main
            if __name__ == "__main__":
                if sys.argv[0].endswith("-script.pyw"):
                    sys.argv[0] = sys.argv[0][:-11]
                elif sys.argv[0].endswith(".exe"):
                    sys.argv[0] = sys.argv[0][:-4]
                sys.exit(main())
            "#
            );
        }
    );

    // The package, its dependencies, and the overlay dependencies should be available.
    context
        .run()
        .arg("--with")
        .arg("iniconfig")
        .arg("python")
        .arg("-c")
        .arg("import foo; import anyio; import iniconfig")
        .assert()
        .success();

    // When layering the project on top (via `--with`), the overlay interpreter also should be used.
    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("--with").arg(".").arg("main"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/builds-v0/[TMP]/[BIN]/python

    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    // When layering the project on top (via `--with`), the overlay gui interpreter also should be used.
    #[cfg(windows)]
    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("--gui-script").arg("--with").arg(".").arg("main_gui"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/builds-v0/[TMP]/[BIN]/pythonw

    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    // Switch to a relocatable virtual environment using the same interpreter.
    context
        .venv()
        .arg("--allow-existing")
        .arg("--relocatable")
        .arg("--python")
        .arg(context.venv.path())
        .assert()
        .success();

    // Cleanup previous shutil
    fs_err::remove_file(context.temp_dir.child("main"))?;
    #[cfg(windows)]
    fs_err::remove_file(context.temp_dir.child("main_gui"))?;

    // The project's entrypoint should be rewritten to use the overlay interpreter.
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("iniconfig").arg("main").arg(context.temp_dir.child("main").as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/builds-v0/[TMP]/[BIN]/python

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    Resolved 1 package in [TIME]
    ");

    // The project's gui entrypoint should be rewritten to use the overlay interpreter.
    #[cfg(windows)]
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("iniconfig").arg("main_gui").arg(context.temp_dir.child("main_gui").as_os_str()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/builds-v0/[TMP]/[BIN]/pythonw

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    Resolved 1 package in [TIME]
    ");

    // The package, its dependencies, and the overlay dependencies should be available.
    context
        .run()
        .arg("--with")
        .arg("iniconfig")
        .arg("python")
        .arg("-c")
        .arg("import foo; import anyio; import iniconfig")
        .assert()
        .success();

    #[cfg(unix)]
    insta::with_settings!({
        filters => context.filters(),
    }, {
            assert_snapshot!(
                context.read("main"), @r#"
            #![CACHE_DIR]/builds-v0/[TMP]/[BIN]/python
            # -*- coding: utf-8 -*-
            import sys
            from foo import main
            if __name__ == "__main__":
                if sys.argv[0].endswith("-script.pyw"):
                    sys.argv[0] = sys.argv[0][:-11]
                elif sys.argv[0].endswith(".exe"):
                    sys.argv[0] = sys.argv[0][:-4]
                sys.exit(main())
            "#
            );
        }
    );

    // When layering the project on top (via `--with`), the overlay interpreter also should be used.
    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("--with").arg(".").arg("main"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/builds-v0/[TMP]/[BIN]/python

    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    // When layering the project on top (via `--with`), the overlay gui interpreter also should be used.
    #[cfg(windows)]
    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("--gui-script").arg("--with").arg(".").arg("main_gui"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/builds-v0/[TMP]/[BIN]/pythonw

    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn run_with_build_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.9");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.9"
        dependencies = ["anyio"]

        [tool.uv]
        build-constraint-dependencies = ["setuptools==1"]
        "#
    })?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import os
       "
    })?;

    // Installing requests with incompatible build constraints should fail.
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("requests==1.2").arg("main.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + anyio==4.3.0
     + exceptiongroup==1.2.0
     + idna==3.6
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    error: Failed to download and build `requests==1.2.0`
      cause: Failed to resolve requirements from `setup.py` build
      cause: No solution found when resolving: `setuptools>=40.8.0`
      cause: Because you require setuptools>=40.8.0 and setuptools==1, we can conclude that your requirements are unsatisfiable.
    ");

    // Change the build constraint to be compatible with `requests==1.2`.
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.9"
        dependencies = ["anyio"]

        [tool.uv]
        build-constraint-dependencies = ["setuptools>=42"]
        "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--with").arg("requests==1.2").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 5 packages in [TIME]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + requests==1.2.0
    ");

    Ok(())
}

/// Sync all members in a workspace.
#[test]
fn run_in_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio>3"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"

        [tool.uv.workspace]
        members = ["child1", "child2"]

        [tool.uv.sources]
        child1 = { workspace = true }
        child2 = { workspace = true }
        "#,
    )?;
    context
        .temp_dir
        .child("src")
        .child("project")
        .child("__init__.py")
        .touch()?;

    let child1 = context.temp_dir.child("child1");
    child1.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "child1"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig>1"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;
    child1
        .child("src")
        .child("child1")
        .child("__init__.py")
        .touch()?;

    let child2 = context.temp_dir.child("child2");
    child2.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "child2"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["typing-extensions>4"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;
    child2
        .child("src")
        .child("child2")
        .child("__init__.py")
        .touch()?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import anyio
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + project==0.1.0 (from file://[TEMP_DIR]/)
     + sniffio==1.3.1
    ");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import iniconfig
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Checked 4 packages in [TIME]
    Traceback (most recent call last):
      File "[TEMP_DIR]/main.py", line 1, in <module>
        import iniconfig
    ModuleNotFoundError: No module named 'iniconfig'
    "#);

    uv_snapshot!(context.filters(), context.run().arg("--package").arg("child1").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child1==0.1.0 (from file://[TEMP_DIR]/child1)
     + iniconfig==2.0.0
    ");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import typing_extensions
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Checked 4 packages in [TIME]
    Traceback (most recent call last):
      File "[TEMP_DIR]/main.py", line 1, in <module>
        import typing_extensions
    ModuleNotFoundError: No module named 'typing_extensions'
    "#);

    uv_snapshot!(context.filters(), context.run().arg("--all-packages").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child2==0.1.0 (from file://[TEMP_DIR]/child2)
     + typing-extensions==4.10.0
    ");

    Ok(())
}

#[test]
fn run_with_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let anyio_local = context.temp_dir.child("src").child("anyio_local");
    copy_dir_all(
        context.workspace_root.join("test/packages/anyio_local"),
        &anyio_local,
    )?;

    let black_editable = context.temp_dir.child("src").child("black_editable");
    copy_dir_all(
        context.workspace_root.join("test/packages/black_editable"),
        &black_editable,
    )?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio", "sniffio==1.3.1"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#
    })?;

    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import sniffio
       "
    })?;

    // Requesting an editable requirement should install it in a layer.
    uv_snapshot!(context.filters(), context.run().arg("--with-editable").arg("./src/black_editable").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + black==0.1.0 (from file://[TEMP_DIR]/src/black_editable)
    ");

    // Requesting an editable requirement should install it in a layer, even if it satisfied
    uv_snapshot!(context.filters(), context.run().arg("--with-editable").arg("./src/anyio_local").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.3.0+foo (from file://[TEMP_DIR]/src/anyio_local)
    ");

    // Requesting the project itself should use the base environment.
    uv_snapshot!(context.filters(), context.run().arg("--with-editable").arg(".").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    ");

    // Similarly, an already editable requirement does not require a layer
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio", "sniffio==1.3.1"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"

        [tool.uv.sources]
        anyio = { path = "./src/anyio_local", editable = true }
        "#
    })?;

    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 3 packages in [TIME]
    Installed 2 packages in [TIME]
     - anyio==4.3.0
     + anyio==4.3.0+foo (from file://[TEMP_DIR]/src/anyio_local)
     ~ foo==1.0.0 (from file://[TEMP_DIR]/)
     - idna==3.6
    ");

    uv_snapshot!(context.filters(), context.run().arg("--with-editable").arg("./src/anyio_local").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    // If invalid, we should reference `--with-editable`.
    uv_snapshot!(context.filters(), context.run().arg("--with-editable").arg("./foo").arg("main.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Checked 3 packages in [TIME]
    error: Failed to resolve `--with` requirement
      cause: Distribution not found at: file://[TEMP_DIR]/foo
    ");

    Ok(())
}

#[test]
fn run_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["typing-extensions"]

        [dependency-groups]
        foo = ["anyio"]
        bar = ["iniconfig"]
        dev = ["sniffio"]
        "#,
    )?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        try:
            import anyio
            print("imported `anyio`")
        except ImportError:
            print("failed to import `anyio`")

        try:
            import iniconfig
            print("imported `iniconfig`")
        except ImportError:
            print("failed to import `iniconfig`")

        try:
            import typing_extensions
            print("imported `typing_extensions`")
        except ImportError:
            print("failed to import `typing_extensions`")
       "#
    })?;

    context.lock().assert().success();

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    failed to import `anyio`
    failed to import `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.run().arg("--only-group").arg("bar").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    failed to import `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    uv_snapshot!(context.filters(), context.run().arg("--group").arg("foo").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    imported `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
    ");

    uv_snapshot!(context.filters(), context.run().arg("--group").arg("foo").arg("--group").arg("bar").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    imported `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 5 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.run().arg("--all-groups").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    imported `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 5 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.run().arg("--all-groups").arg("--no-group").arg("bar").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    imported `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.run().arg("--group").arg("foo").arg("--no-project").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    imported `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    warning: `--group foo` has no effect when used alongside `--no-project`
    ");

    uv_snapshot!(context.filters(), context.run().arg("--group").arg("foo").arg("--group").arg("bar").arg("--no-project").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    imported `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    warning: `--group` has no effect when used alongside `--no-project`
    ");

    uv_snapshot!(context.filters(), context.run().arg("--group").arg("dev").arg("--no-project").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    imported `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    warning: `--group dev` has no effect when used alongside `--no-project`
    ");

    uv_snapshot!(context.filters(), context.run().arg("--all-groups").arg("--no-project").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    imported `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    warning: `--all-groups` has no effect when used alongside `--no-project`
    ");

    uv_snapshot!(context.filters(), context.run().arg("--dev").arg("--no-project").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    imported `anyio`
    imported `iniconfig`
    imported `typing_extensions`

    ----- stderr -----
    warning: `--dev` has no effect when used alongside `--no-project`
    ");

    Ok(())
}

#[test]
fn run_dev_overrides_uv_no_dev() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = ["iniconfig"]
        "#,
    )?;

    context.lock().assert().success();

    uv_snapshot!(context.filters(), context
        .run()
        .arg("--dev")
        .arg("python")
        .arg("-c")
        .arg("import iniconfig; print(iniconfig.__name__)")
        .env(EnvVars::UV_NO_DEV, "1"), @"
    exit_code: 0 (success)
    ----- stdout -----
    iniconfig

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_locked() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#,
    )?;
    context
        .temp_dir
        .child("src")
        .child("project")
        .child("__init__.py")
        .touch()?;

    // Running with `--locked` should error, if no lockfile is present.
    uv_snapshot!(context.filters(), context.run().arg("--locked").arg("--").arg("python").arg("--version"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Unable to find lockfile at `uv.lock`, but `--locked` was provided. To create a lockfile, run `uv lock` or `uv sync` without the flag.
    ");

    // Lock the initial requirements.
    context.lock().assert().success();

    let existing = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            existing, @r#"
        version = 1
        revision = 5
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "3.7.0"
        source = { registry = "https://pypi.org/simple" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "https://files.pythonhosted.org/packages/c6/b3/fefbf7e78ab3b805dec67d698dc18dd505af7a18a8dd08868c9b4fa736b5/anyio-3.7.0.tar.gz", hash = "sha256:275d9973793619a5374e1c89a4f4ad3f4b0a5510a2b5b939444bee8f4c4d37ce", size = 142737, upload-time = "2023-05-27T11:12:46.688Z" }
        wheels = [
            { url = "https://files.pythonhosted.org/packages/68/fe/7ce1926952c8a403b35029e194555558514b365ad77d75125f521a2bec62/anyio-3.7.0-py3-none-any.whl", hash = "sha256:eddca883c4175f14df8aedce21054bfca3adb70ffe76a9f607aef9d7fa2ea7f0", size = 80873, upload-time = "2023-05-27T11:12:44.474Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "https://pypi.org/simple" }
        sdist = { url = "https://files.pythonhosted.org/packages/bf/3f/ea4b9117521a1e9c50344b909be7886dd00a519552724809bb1f486986c2/idna-3.6.tar.gz", hash = "sha256:9ecdbbd083b06798ae1e86adcbfe8ab1479cf864e4ee30fe4e46a003d12491ca", size = 175426, upload-time = "2023-11-25T15:40:54.902Z" }
        wheels = [
            { url = "https://files.pythonhosted.org/packages/c2/e7/a82b05cf63a603df6e68d59ae6a68bf5064484a0718ea5033660af4b54a9/idna-3.6-py3-none-any.whl", hash = "sha256:c05567e9c24a6b9faaa835c4821bad0590fbb9d5779e7caa6e1cc4978e7eb24f", size = 61567, upload-time = "2023-11-25T15:40:52.604Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { editable = "." }
        dependencies = [
            { name = "anyio" },
        ]

        [package.metadata]
        requires-dist = [{ name = "anyio", specifier = "==3.7.0" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "https://pypi.org/simple" }
        sdist = { url = "https://files.pythonhosted.org/packages/a2/87/a6771e1546d97e7e041b6ae58d80074f81b7d5121207425c964ddf5cfdbd/sniffio-1.3.1.tar.gz", hash = "sha256:f4324edc670a0f49750a81b895f35c3adb843cca46f0530f79fc1babb23789dc", size = 20372, upload-time = "2024-02-25T23:20:04.057Z" }
        wheels = [
            { url = "https://files.pythonhosted.org/packages/e9/44/75a9c9421471a6c4805dbf2356f7c181a29c1879239abab1ea2cc8f38b40/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2f6da418d1f1e0fddd844478f41680e794e6051915791a034ff65e5f100525a2", size = 10235, upload-time = "2024-02-25T23:20:01.196Z" },
        ]
        "#);
        }
    );

    // Update the requirements.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#,
    )?;

    // Running with `--locked` should error.
    uv_snapshot!(context.filters(), context.run().arg("--locked").arg("--").arg("python").arg("--version"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");

    let updated = context.read("uv.lock");

    // And the lockfile should be unchanged.
    assert_eq!(existing, updated);

    // Lock the updated requirements.
    uv_snapshot!(context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Removed anyio v3.7.0
    Removed idna v3.6
    Added iniconfig v2.0.0
    Removed sniffio v1.3.1
    ");

    // Lock the updated requirements.
    uv_snapshot!(context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    // Running with `--locked` should succeed.
    uv_snapshot!(context.filters(), context.run().arg("--locked").arg("--").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + iniconfig==2.0.0
     + project==0.1.0 (from file://[TEMP_DIR]/)
    ");

    Ok(())
}

#[test]
fn run_frozen() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#,
    )?;
    context
        .temp_dir
        .child("src")
        .child("project")
        .child("__init__.py")
        .touch()?;

    // Running with `--frozen` should error, if no lockfile is present.
    uv_snapshot!(context.filters(), context.run().arg("--frozen").arg("--").arg("python").arg("--version"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Unable to find lockfile at `uv.lock`, but `--frozen` was provided. To create a lockfile, run `uv lock` or `uv sync` without the flag.
    ");

    context.lock().assert().success();

    // Update the requirements.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#,
    )?;

    // Running with `--frozen` should install the stale lockfile.
    uv_snapshot!(context.filters(), context.run().arg("--frozen").arg("--").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + project==0.1.0 (from file://[TEMP_DIR]/)
     + sniffio==1.3.1
    ");

    Ok(())
}

#[test]
fn run_no_sync() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#,
    )?;
    context
        .temp_dir
        .child("src")
        .child("project")
        .child("__init__.py")
        .touch()?;

    // Running with `--no-sync` should succeed, even if the lockfile isn't present.
    uv_snapshot!(context.filters(), context.run().arg("--no-sync").arg("--").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    context.lock().assert().success();

    // Running with `--no-sync` should not install any requirements.
    uv_snapshot!(context.filters(), context.run().arg("--no-sync").arg("--").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    context.sync().assert().success();

    // But it should have access to the installed packages.
    uv_snapshot!(context.filters(), context.run().arg("--no-sync").arg("--").arg("python").arg("-c").arg("import anyio; print(anyio.__name__)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    anyio
    ");

    Ok(())
}

/// Test that `UV_NO_SYNC=1` environment variable works for `uv run`.
///
/// See: <https://github.com/astral-sh/uv/issues/17390>
#[test]
fn run_no_sync_env_var() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#,
    )?;
    context
        .temp_dir
        .child("src")
        .child("project")
        .child("__init__.py")
        .touch()?;

    // Running with `UV_NO_SYNC=1` should succeed, even if the lockfile isn't present.
    uv_snapshot!(context.filters(), context.run().env(EnvVars::UV_NO_SYNC, "1").arg("--").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    context.lock().assert().success();

    // Running with `UV_NO_SYNC=1` should not install any requirements.
    uv_snapshot!(context.filters(), context.run().env(EnvVars::UV_NO_SYNC, "1").arg("--").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    context.sync().assert().success();

    // But it should have access to the installed packages.
    uv_snapshot!(context.filters(), context.run().env(EnvVars::UV_NO_SYNC, "1").arg("--").arg("python").arg("-c").arg("import anyio; print(anyio.__name__)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    anyio
    ");

    Ok(())
}

#[test]
fn run_empty_requirements_txt() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio", "sniffio==1.3.1"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import sniffio
       "
    })?;

    let requirements_txt =
        ChildPath::new(context.temp_dir.canonicalize()?.join("requirements.txt"));
    requirements_txt.touch()?;

    // The project environment is synced on the first invocation.
    uv_snapshot!(context.filters(), context.run().arg("--with-requirements").arg(requirements_txt.as_os_str()).arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    warning: Requirements file `requirements.txt` does not contain any dependencies
    ");

    // Then reused in subsequent invocations
    uv_snapshot!(context.filters(), context.run().arg("--with-requirements").arg(requirements_txt.as_os_str()).arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    warning: Requirements file `requirements.txt` does not contain any dependencies
    ");

    Ok(())
}

#[test]
fn run_requirements_txt() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio", "sniffio==1.3.1"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import sniffio
       "
    })?;

    // Requesting an unsatisfied requirement should install it.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig")?;

    uv_snapshot!(context.filters(), context.run().arg("--with-requirements").arg(requirements_txt.as_os_str()).arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Requesting a satisfied requirement should use the base environment.
    requirements_txt.write_str("sniffio")?;

    uv_snapshot!(context.filters(), context.run().arg("--with-requirements").arg(requirements_txt.as_os_str()).arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    ");

    // Unless the user requests a different version.
    requirements_txt.write_str("sniffio<1.3.1")?;

    uv_snapshot!(context.filters(), context.run().arg("--with-requirements").arg(requirements_txt.as_os_str()).arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.0
    ");

    // Or includes an unsatisfied requirement via `--with`.
    requirements_txt.write_str("sniffio")?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--with-requirements")
        .arg(requirements_txt.as_os_str())
        .arg("--with")
        .arg("iniconfig")
        .arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    Resolved 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + iniconfig==2.0.0
     + sniffio==1.3.1
    ");

    // Allow `-` for stdin.
    uv_snapshot!(context.filters(), context.run()
        .arg("--with-requirements")
        .arg("-")
        .arg("--with")
        .arg("iniconfig")
        .arg("main.py")
        .stdin(std::fs::File::open(&requirements_txt)?), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 4 packages in [TIME]
    Resolved 2 packages in [TIME]
    ");

    // But not in combination with reading the script from stdin
    uv_snapshot!(context.filters(), context.run()
        .arg("--with-requirements")
        .arg("-")
        // The script to run
        .arg("-")
        .stdin(std::fs::File::open(&requirements_txt)?), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot read both requirements file and script from stdin
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--with-requirements")
        .arg("-")
        .arg("--script")
        .arg("-")
        .stdin(std::fs::File::open(&requirements_txt)?), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot read both requirements file and script from stdin
    ");

    Ok(())
}

/// Ignore and warn when (e.g.) the `--index-url` argument is a provided `requirements.txt`.
#[test]
fn run_requirements_txt_arguments() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["typing_extensions"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import typing_extensions
       "
    })?;

    // Requesting an unsatisfied requirement should install it.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! { r"
        --index-url https://test.pypi.org/simple
        idna
        "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--with-requirements").arg(requirements_txt.as_os_str()).arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + typing-extensions==4.10.0
    warning: Ignoring `--index-url` value `https://test.pypi.org/simple` from requirements file. Instead, use the `--index-url` command-line argument, or set `index-url` in a `uv.toml` or `pyproject.toml` file.
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + idna==3.6
    ");

    Ok(())
}

/// Ensure that we can import from the root project when layering `--with` requirements.
#[test]
fn run_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = []

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;

    let src = context.temp_dir.child("src").child("foo");
    src.create_dir_all()?;

    let init = src.child("__init__.py");
    init.touch()?;

    let main = context.temp_dir.child("main.py");
    main.write_str(indoc! { r"
        import foo
        print('Hello, world!')
       "
    })?;

    // We treat arguments before the command as uv arguments
    uv_snapshot!(context.filters(), context.run().arg("--with").arg("iniconfig").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    uv_snapshot!(context.filters(), context.run().arg("--no-editable-package").arg("foo").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ foo==1.0.0 (from file://[TEMP_DIR]/)
    ");

    Ok(())
}

#[test]
fn run_from_directory() -> Result<()> {
    // Default to 3.11 so that the `.python-version` is meaningful.
    let context = uv_test::test_context_with_versions!(&["3.10", "3.11", "3.12"])
        .with_filtered_missing_file_error();

    let project_dir = context.temp_dir.child("project");
    project_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.12")?;

    let pyproject_toml = project_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.10"
        dependencies = []

        [project.scripts]
        main = "main:main"

        [build-system]
        requires = ["setuptools>=42"]
        build-backend = "setuptools.build_meta"
        "#
    })?;

    let main_script = project_dir.child("main.py");
    main_script.write_str(indoc! { r"
        import platform

        def main():
            print(platform.python_version())
       "
    })?;

    let filters = TestContext::path_patterns(Path::new("project").join(".venv"))
        .into_iter()
        .map(|pattern| (pattern, "[PROJECT_VENV]/".to_string()))
        .collect::<Vec<_>>();
    let filters = context
        .filters()
        .into_iter()
        .chain(
            filters
                .iter()
                .map(|(pattern, replacement)| (pattern.as_str(), replacement.as_str())),
        )
        .collect::<Vec<_>>();

    // Use `--project`, which resolves configuration relative to the provided directory, but paths
    // relative to the current working directory.
    uv_snapshot!(filters.clone(), context.run().arg("--project").arg("project").arg("main"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12.[X]

    ----- stderr -----
    warning: `VIRTUAL_ENV=.venv` does not match the project environment path `[PROJECT_VENV]/` and will be ignored; use `--active` to target the active environment instead
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: [PROJECT_VENV]/
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/project)
    ");

    fs_err::remove_dir_all(context.temp_dir.join("project").join(".venv"))?;
    uv_snapshot!(filters.clone(), context.run().arg("--project").arg("project").arg("./project/main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `VIRTUAL_ENV=.venv` does not match the project environment path `[PROJECT_VENV]/` and will be ignored; use `--active` to target the active environment instead
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: [PROJECT_VENV]/
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/project)
    ");

    // Use `--directory`, which switches to the provided directory entirely.
    fs_err::remove_dir_all(context.temp_dir.join("project").join(".venv"))?;
    uv_snapshot!(filters.clone(), context.run().arg("--directory").arg("project").arg("main"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12.[X]

    ----- stderr -----
    warning: `VIRTUAL_ENV=[VENV]/` does not match the project environment path `.venv` and will be ignored; use `--active` to target the active environment instead
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/project)
    ");

    fs_err::remove_dir_all(context.temp_dir.join("project").join(".venv"))?;
    uv_snapshot!(filters.clone(), context.run().arg("--directory").arg("project").arg("./main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `VIRTUAL_ENV=[VENV]/` does not match the project environment path `.venv` and will be ignored; use `--active` to target the active environment instead
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/project)
    ");

    fs_err::remove_dir_all(context.temp_dir.join("project").join(".venv"))?;
    uv_snapshot!(filters.clone(), context.run().arg("--directory").arg("project").arg("./project/main.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: `VIRTUAL_ENV=[VENV]/` does not match the project environment path `.venv` and will be ignored; use `--active` to target the active environment instead
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/project)
    error: Failed to spawn: ./project/main.py
      cause: [OS ERROR 2]
    ");

    // Even if we write a `.python-version` file in the current directory, we should prefer the
    // one in the project directory in both cases.
    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.11")?;

    project_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.10")?;

    fs_err::remove_dir_all(context.temp_dir.join("project").join(".venv"))?;
    uv_snapshot!(filters.clone(), context.run().arg("--project").arg("project").arg("main"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.10.[X]

    ----- stderr -----
    warning: `VIRTUAL_ENV=.venv` does not match the project environment path `[PROJECT_VENV]/` and will be ignored; use `--active` to target the active environment instead
    Using CPython 3.10.[X] interpreter at: [PYTHON-3.10]
    Creating virtual environment at: [PROJECT_VENV]/
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/project)
    ");

    fs_err::remove_dir_all(context.temp_dir.join("project").join(".venv"))?;
    uv_snapshot!(filters.clone(), context.run().arg("--directory").arg("project").arg("main"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.10.[X]

    ----- stderr -----
    warning: `VIRTUAL_ENV=[VENV]/` does not match the project environment path `.venv` and will be ignored; use `--active` to target the active environment instead
    Using CPython 3.10.[X] interpreter at: [PYTHON-3.10]
    Creating virtual environment at: .venv
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/project)
    ");

    Ok(())
}

/// By default, omit resolver and installer output.
#[test]
fn run_without_output() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio", "sniffio==1.3.1"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import sniffio
       "
    })?;

    // On the first run, we only show the summary line for each environment.
    uv_snapshot!(context.filters(), context.run().env_remove(EnvVars::UV_SHOW_RESOLUTION).arg("--with").arg("iniconfig").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 4 packages in [TIME]
    Installed 1 package in [TIME]
    ");

    // Subsequent runs are quiet.
    uv_snapshot!(context.filters(), context.run().env_remove(EnvVars::UV_SHOW_RESOLUTION).arg("--with").arg("iniconfig").arg("main.py"), @"
    exit_code: 0 (success)
    ");

    Ok(())
}

/// Ensure that we can import from the root project when layering `--with` requirements.
#[test]
fn run_isolated_python_version() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.9", "3.12"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#
    })?;

    let src = context.temp_dir.child("src").child("foo");
    src.create_dir_all()?;

    let init = src.child("__init__.py");
    init.touch()?;

    let main = context.temp_dir.child("main.py");
    main.write_str(indoc! { r"
        import sys

        print((sys.version_info.major, sys.version_info.minor))
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 9)

    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    Creating virtual environment at: .venv
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + anyio==4.3.0
     + exceptiongroup==1.2.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.run().arg("--isolated").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 9)

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + anyio==4.3.0
     + exceptiongroup==1.2.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    // Set the `.python-version` to `3.12`.
    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.12")?;

    uv_snapshot!(context.filters(), context.run().arg("--isolated").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    (3, 12)

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    Ok(())
}

/// Ignore the existing project when executing with `--no-project`.
#[test]
fn run_no_project() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["anyio"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#
    })?;

    let src = context.temp_dir.child("src").child("foo");
    src.create_dir_all()?;

    let init = src.child("__init__.py");
    init.touch()?;

    // `run` should run in the context of the project.
    uv_snapshot!(context.filters(), context.run().arg("python").arg("-c").arg("import sys; print(sys.executable)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/[PYTHON]

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    // `run --no-project` should not (but it should still run in the same environment, as it would
    // if there were no project at all).
    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("python").arg("-c").arg("import sys; print(sys.executable)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/[PYTHON]
    ");

    // `run --no-project --isolated` should run in an entirely isolated environment.
    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("--isolated").arg("python").arg("-c").arg("import sys; print(sys.executable)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/builds-v0/[TMP]/[BIN]/[PYTHON]
    ");

    // `run --no-project` should not (but it should still run in the same environment, as it would
    // if there were no project at all).
    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("python").arg("-c").arg("import sys; print(sys.executable)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/[PYTHON]
    ");

    // `run --no-project --locked` should warn about `--locked`.
    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("--locked").arg("python").arg("-c").arg("import sys; print(sys.executable)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/[PYTHON]

    ----- stderr -----
    warning: `--locked` has no effect when used alongside `--no-project`
    ");

    Ok(())
}

#[test]
fn run_stdin() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        print("Hello, world!")
       "#
    })?;

    let mut command = context.run();
    let command_with_args = command.stdin(std::fs::File::open(test_script)?).arg("-");
    uv_snapshot!(context.filters(), command_with_args, @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!
    ");

    Ok(())
}

#[test]
fn run_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let main_script = context.temp_dir.child("__main__.py");
    main_script.write_str(indoc! { r#"
        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("."), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!
    ");

    Ok(())
}

#[test]
fn run_zipapp() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Create a zipapp.
    let child = context.temp_dir.child("app");
    child.create_dir_all()?;

    let main_script = child.child("__main__.py");
    main_script.write_str(indoc! { r#"
        print("Hello, world!")
       "#
    })?;

    let zipapp = context.temp_dir.child("app.pyz");
    let status = context
        .run()
        .arg("python")
        .arg("-m")
        .arg("zipapp")
        .arg(child.as_ref())
        .arg("--output")
        .arg(zipapp.as_ref())
        .status()?;
    assert!(status.success());

    // Run the zipapp.
    uv_snapshot!(context.filters(), context.run().arg(zipapp.as_ref()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!
    ");

    Ok(())
}

#[test]
fn run_stdin_args() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.run().arg("python").arg("-c").arg("import sys; print(sys.argv)").arg("foo").arg("bar"), @"
    exit_code: 0 (success)
    ----- stdout -----
    ['-c', 'foo', 'bar']
    ");
}

/// Run a module equivalent to `python -m foo`.
#[test]
fn run_module() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.run().arg("-m").arg("__hello__"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello world!
    ");

    uv_snapshot!(context.filters(), context.run().arg("-m").arg("http.server").arg("-h"), @"
    exit_code: 0 (success)
    ----- stdout -----
    usage: server.py [-h] [--cgi] [-b ADDRESS] [-d DIRECTORY] [-p VERSION] [port]

    positional arguments:
      port                  bind to this port (default: 8000)

    options:
      -h, --help            show this help message and exit
      --cgi                 run as CGI server
      -b ADDRESS, --bind ADDRESS
                            bind to this address (default: all interfaces)
      -d DIRECTORY, --directory DIRECTORY
                            serve this directory (default: current directory)
      -p VERSION, --protocol VERSION
                            conform to this HTTP version (default: HTTP/1.0)
    ");
}

#[test]
fn run_module_stdin() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.run().arg("-m").arg("-"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot run a Python module from stdin
    ");
}

/// Test for how run reacts to a pyproject.toml without a `[project]`
#[test]
fn virtual_empty() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    // Create an empty project
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.touch()?;

    let src = context.temp_dir.child("src").child("foo");
    src.create_dir_all()?;

    let init = src.child("__init__.py");
    init.touch()?;

    // `run` should work fine
    uv_snapshot!(context.filters(), context.run().arg("python").arg("-c").arg("import sys; print(sys.executable)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/[PYTHON]

    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved in [TIME]
    Checked in [TIME]
    ");

    // `run --no-project` should also work fine
    uv_snapshot!(context.filters(), context.run().arg("--no-project").arg("python").arg("-c").arg("import sys; print(sys.executable)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/[PYTHON]
    ");

    Ok(())
}

#[test]
fn run_isolated_incompatible_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.9", "3.11"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#
    })?;

    let python_version = context.temp_dir.child(PYTHON_VERSION_FILENAME);
    python_version.write_str("3.9")?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        import iniconfig

        x: str | int = "hello"
        print(x)
       "#
    })?;

    // We should reject Python 3.9...
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    error: The Python request from `.python-version` resolved to Python 3.9.[X], which is incompatible with the project's Python requirement: `>=3.12` (from `project.requires-python`)
    Use `uv python pin` to update the `.python-version` file to a compatible version
    ");

    // ...even if `--isolated` is provided.
    uv_snapshot!(context.filters(), context.run().arg("--isolated").arg("main.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The Python request from `.python-version` resolved to Python 3.9.[X], which is incompatible with the project's Python requirement: `>=3.12` (from `project.requires-python`)
    Use `uv python pin` to update the `.python-version` file to a compatible version
    ");

    Ok(())
}

#[test]
fn run_isolated_does_not_modify_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio>=3,<5",
        ]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        import importlib.metadata
        print(importlib.metadata.version("anyio"))
       "#
    })?;

    // Run with --isolated
    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    4.3.0

    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    // This should not create a lock file
    context
        .temp_dir
        .child("uv.lock")
        .assert(predicate::path::missing());

    // Create initial lock with default resolution
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    4.3.0

    ----- stderr -----
    Resolved 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    // Read the lock file content
    let pre_uv_lock = context.read("uv.lock");

    // Run with --isolated and --resolution lowest-direct to force different resolution
    // This should use anyio 3.x but not modify the lock file
    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--resolution")
        .arg("lowest-direct")
        .arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.0.0

    ----- stderr -----
    Ignoring existing lockfile due to change in resolution mode: `highest` vs. `lowest-direct`
    Resolved 4 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 4 packages in [TIME]
     + anyio==3.0.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    // Verify the lock file hasn't changed
    let post_uv_lock = context.read("uv.lock");
    assert_eq!(
        pre_uv_lock, post_uv_lock,
        "Lock file should not be modified with --isolated"
    );

    Ok(())
}

#[test]
fn run_isolated_with_frozen() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio>=3,<5",
        ]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;
    context
        .temp_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        import importlib.metadata
        print(importlib.metadata.version("anyio"))
       "#
    })?;

    // Create an initial lockfile with lowest-direct resolution
    uv_snapshot!(context.filters(), context.run()
        .arg("--resolution")
        .arg("lowest-direct")
        .arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.0.0

    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==3.0.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    // Run with `--isolated` and `--frozen` to use the existing lock
    // We should not re-resolve to the highest version here
    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--frozen")
        .arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.0.0

    ----- stderr -----
    Installed 4 packages in [TIME]
     + anyio==3.0.0
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + idna==3.6
     + sniffio==1.3.1
    ");

    Ok(())
}

#[test]
fn run_compiled_python_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Write a non-PEP 723 script.
    let test_non_script = context.temp_dir.child("main.py");
    test_non_script.write_str(indoc! { r#"
        print("Hello, world!")
       "#
    })?;

    // Run a non-PEP 723 script.
    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!
    ");

    let compile_output = context
        .run()
        .arg("python")
        .arg("-m")
        .arg("compileall")
        .arg(test_non_script.path())
        .output()?;

    assert!(
        compile_output.status.success(),
        "Failed to compile the python script"
    );

    // Run the compiled non-PEP 723 script.
    let compiled_non_script = context.temp_dir.child("__pycache__/main.cpython-312.pyc");
    uv_snapshot!(context.filters(), context.run().arg(compiled_non_script.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!
    ");

    // If the script contains a PEP 723 tag, we should install its requirements.
    let test_script = context.temp_dir.child("script.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///
        import iniconfig
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Compile the PEP 723 script.
    let compile_output = context
        .run()
        .arg("python")
        .arg("-m")
        .arg("compileall")
        .arg(test_script.path())
        .output()?;

    assert!(
        compile_output.status.success(),
        "Failed to compile the python script"
    );

    // Run the compiled PEP 723 script. This fails, since we can't read the script tag.
    let compiled_script = context.temp_dir.child("__pycache__/script.cpython-312.pyc");
    uv_snapshot!(context.filters(), context.run().arg(compiled_script.path()), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Traceback (most recent call last):
      File "[TEMP_DIR]/script.py", line 7, in <module>
        import iniconfig
    ModuleNotFoundError: No module named 'iniconfig'
    "#);

    Ok(())
}

#[test]
fn run_exit_code() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("script.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # ///

        exit(42)
       "#
    })?;

    context.run().arg("script.py").assert().code(42);

    Ok(())
}

#[test]
fn run_invalid_project_table() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project.urls]
        repository = 'https://github.com/octocat/octocat-python'

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
    })?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: pyproject.toml
      cause: TOML parse error at line 1, column 2
               |
             1 | [project.urls]
               |  ^^^^^^^
             `pyproject.toml` is using the `[project]` table, but the required `project.name` field is not set
    ");

    Ok(())
}

#[test]
#[cfg(target_family = "unix")]
fn run_script_without_build_system() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.scripts]
        entry = "foo:custom_entry"
        "#
    })?;

    let test_script = context.temp_dir.child("src/__init__.py");
    test_script.write_str(indoc! { r#"
        def custom_entry():
            print!("Hello")
       "#
    })?;

    // TODO(lucab): this should match `entry` and warn
    // <https://github.com/astral-sh/uv/issues/7428>
    uv_snapshot!(context.filters(), context.run().arg("entry"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    error: Failed to spawn: entry
      cause: No such file or directory (os error 2)
    ");

    Ok(())
}

#[test]
fn run_script_module_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.scripts]
        foo = "foo:app"

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#
    })?;

    let init = context.temp_dir.child("src/foo/__init__.py");
    init.write_str(indoc! { r#"
        def app():
            print("Hello from `__init__`")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from `__init__`

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/)
    ");

    // Creating `__main__` should not change the behavior, the entrypoint should take precedence
    let main = context.temp_dir.child("src/foo/__main__.py");
    main.write_str(indoc! { r#"
        print("Hello from `__main__`")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from `__init__`

    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    // Even if the working directory is `src`
    uv_snapshot!(context.filters(), context.run().arg("--directory").arg("src").arg("foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from `__init__`

    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    // Unless the user opts-in to module running with `-m`
    uv_snapshot!(context.filters(), context.run().arg("-m").arg("foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from `__main__`

    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    Ok(())
}

#[test]
fn run_script_explicit() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("script");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///
        import iniconfig
        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--script").arg("script"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_script_explicit_stdin() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("script");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///
        import iniconfig
        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--script").arg("-").stdin(std::fs::File::open(test_script)?), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_script_explicit_no_file() {
    let context = uv_test::test_context!("3.12");
    context
        .run()
        .arg("--script")
        .arg("script")
        .assert()
        .stderr(contains("can't open file"))
        .stderr(contains("[Errno 2] No such file or directory"));
}

#[cfg(target_family = "unix")]
#[test]
fn run_script_explicit_directory() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    fs_err::create_dir(context.temp_dir.child("script"))?;

    uv_snapshot!(context.filters(), context.run().arg("--script").arg("script"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: failed to read from file `script`: Is a directory (os error 21)
    ");

    Ok(())
}

#[test]
#[cfg(windows)]
fn run_gui_script_explicit_windows() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("script");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = []
        # ///
        import sys
        import os

        executable = os.path.basename(sys.executable).lower()
        if not executable.startswith("pythonw"):
            print(f"Error: Expected pythonw.exe but got: {executable}", file=sys.stderr)
            sys.exit(1)

        print(f"Using executable: {executable}", file=sys.stderr)
    "#})?;

    uv_snapshot!(context.filters(), context.run().arg("--gui-script").arg("script"), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    Using executable: pythonw.exe
    "###);

    Ok(())
}

#[test]
#[cfg(windows)]
fn run_gui_script_explicit_stdin_windows() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("script");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///
        import iniconfig
        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--gui-script").arg("-").stdin(std::fs::File::open(test_script)?), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    "###);

    Ok(())
}

#[test]
#[cfg(not(windows))]
fn run_gui_script_explicit_unix() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let test_script = context.temp_dir.child("script");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = []
        # ///
        import sys
        import os

        executable = os.path.basename(sys.executable).lower()
        print(f"Using executable: {executable}", file=sys.stderr)
    "#})?;

    uv_snapshot!(context.filters(), context.run().arg("--gui-script").arg("script"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using executable: python
    ");

    Ok(())
}

#[test]
#[cfg(unix)]
fn run_linked_environment_path() -> Result<()> {
    use anyhow::Ok;

    let context = uv_test::test_context!("3.12")
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["black"]
        "#,
    )?;

    // Create a link from `target` -> virtual environment
    fs_err::os::unix::fs::symlink(&context.venv, context.temp_dir.child("target"))?;

    // Running `uv sync` should use the environment at `target``
    uv_snapshot!(context.filters(), context.sync()
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "target"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    ");

    // `sys.prefix` and `sys.executable` should be from the `target` directory
    uv_snapshot!(context.filters(), context.run()
        .env_remove(EnvVars::VIRTUAL_ENV)  // Ignore the test context's active virtual environment
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "target")
        .arg("python").arg("-c").arg("import sys; print(sys.prefix); print(sys.executable)"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/target
    [TEMP_DIR]/target/[BIN]/[PYTHON]

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Checked 6 packages in [TIME]
    ");

    // And, similarly, the entrypoint should use `target`
    let black_entrypoint = context.read("target/bin/black");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            black_entrypoint, @r#"
        #![TEMP_DIR]/target/[BIN]/[PYTHON]
        # -*- coding: utf-8 -*-
        import sys
        from black import patched_main
        if __name__ == "__main__":
            if sys.argv[0].endswith("-script.pyw"):
                sys.argv[0] = sys.argv[0][:-11]
            elif sys.argv[0].endswith(".exe"):
                sys.argv[0] = sys.argv[0][:-4]
            sys.exit(patched_main())
        "#
        );
    });

    Ok(())
}

#[test]
fn run_active_project_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names();

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

    // Running `uv run` with `VIRTUAL_ENV` should warn
    uv_snapshot!(context.filters(), context.run()
        .arg("python").arg("--version")
        .env(EnvVars::VIRTUAL_ENV, "foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    warning: `VIRTUAL_ENV=foo` does not match the project environment path `.venv` and will be ignored; use `--active` to target the active environment instead
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Using `--no-active` should silence the warning
    uv_snapshot!(context.filters(), context.run()
        .arg("--no-active")
        .arg("python").arg("--version")
        .env(EnvVars::VIRTUAL_ENV, "foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 1 package in [TIME]
    ");

    context
        .temp_dir
        .child(".venv")
        .assert(predicate::path::is_dir());

    context
        .temp_dir
        .child("foo")
        .assert(predicate::path::missing());

    // Using `--active` should create the environment
    uv_snapshot!(context.filters(), context.run()
        .arg("--active")
        .arg("python").arg("--version")
        .env(EnvVars::VIRTUAL_ENV, "foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: foo
    Resolved 2 packages in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    context
        .temp_dir
        .child("foo")
        .assert(predicate::path::is_dir());

    // Requesting a different Python version should invalidate the environment
    uv_snapshot!(context.filters(), context.run()
        .arg("--active")
        .arg("-p").arg("3.12")
        .arg("python").arg("--version")
        .env(EnvVars::VIRTUAL_ENV, "foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Removed virtual environment at: foo
    Creating virtual environment at: foo
    Resolved 2 packages in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_active_script_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names();

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///

        import iniconfig

        print("Hello, world!")
       "#
    })?;

    // Running `uv run --script` with `VIRTUAL_ENV` should _not_ warn.
    uv_snapshot!(context.filters(), context.run()
        .arg("--script")
        .arg("main.py")
        .env(EnvVars::VIRTUAL_ENV, "foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Using `--no-active` should also _not_ warn.
    uv_snapshot!(context.filters(), context.run()
        .arg("--no-active")
        .arg("--script")
        .arg("main.py")
        .env(EnvVars::VIRTUAL_ENV, "foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!
    ");

    context
        .temp_dir
        .child("foo")
        .assert(predicate::path::missing());

    // Using `--active` should create the environment
    uv_snapshot!(context.filters(), context.run()
        .arg("--active")
        .arg("--script")
        .arg("main.py")
        .env(EnvVars::VIRTUAL_ENV, "foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    context
        .temp_dir
        .child("foo")
        .assert(predicate::path::is_dir());

    // Requesting a different Python version should invalidate the environment
    uv_snapshot!(context.filters(), context.run()
        .arg("--active")
        .arg("-p").arg("3.12")
        .arg("--script")
        .arg("main.py")
        .env(EnvVars::VIRTUAL_ENV, "foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

/// Regression test for <https://github.com/astral-sh/uv/issues/21364>.
#[test]
fn run_active_script_environment_non_virtualenv() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # ///

        print("Hello, world!")
       "#
    })?;

    let active_environment = context.temp_dir.child("foo");
    active_environment.create_dir_all()?;
    active_environment
        .child("important.txt")
        .write_str("important data")?;

    context
        .run()
        .arg("--active")
        .arg("--script")
        .arg("main.py")
        .env(EnvVars::VIRTUAL_ENV, "foo")
        .assert()
        .success();

    active_environment.assert(predicate::path::is_dir());
    // Silently deleting user data outside a virtual environment is undesirable.
    active_environment
        .child("important.txt")
        .assert(predicate::path::missing());

    Ok(())
}

#[test]
#[cfg(not(windows))]
fn run_gui_script_explicit_stdin_unix() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("script");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///
        import iniconfig
        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--gui-script").arg("-").stdin(std::fs::File::open(test_script)?), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_remote_pep723_script() {
    let context = uv_test::test_context!("3.12").with_filtered_python_names();
    uv_snapshot!(context.filters(), context.run().arg("https://raw.githubusercontent.com/astral-sh/uv/df45b9ac2584824309ff29a6a09421055ad730f6/scripts/uv-run-remote-script-test.py").arg(EnvVars::CI), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello CI, from uv!

    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + markdown-it-py==3.0.0
     + mdurl==0.1.2
     + pygments==2.17.2
     + rich==13.7.1
    ");
}

#[test]
fn run_remote_pep723_script_with_nonexistent_ssl_cert_file() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.run()
        .arg("https://raw.githubusercontent.com/astral-sh/uv/df45b9ac2584824309ff29a6a09421055ad730f6/scripts/uv-run-remote-script-test.py")
        .arg(EnvVars::CI)
        .env(EnvVars::SSL_CERT_FILE, context.temp_dir.join("missing.pem"))
        .env(EnvVars::UV_HTTP_RETRIES, "0")
        .env_remove(EnvVars::SSL_CERT_DIR)
        .env_remove(EnvVars::UV_NATIVE_TLS)
        .env_remove(EnvVars::UV_SYSTEM_CERTS), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Invalid `SSL_CERT_FILE`. Path does not exist: [TEMP_DIR]/missing.pem. No default certificates will be trusted.
    error: error sending request for url (https://raw.githubusercontent.com/astral-sh/uv/df45b9ac2584824309ff29a6a09421055ad730f6/scripts/uv-run-remote-script-test.py)
      cause: client error (Connect)
      cause: invalid peer certificate: UnknownIssuer
    ");
}

#[test]
fn run_remote_requirements_offline_redacts_credentials() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("main.py");
    script.write_str("print('hello')")?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--offline")
        .arg("--with-requirements")
        .arg("http://username:password@example.com/requirements.txt")
        .arg(script.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Network connectivity is disabled, but a remote requirements file was requested: http://username:****@example.com/requirements.txt
    ");

    Ok(())
}

#[test]
fn run_remote_pep723_requirements_fetch_error_does_not_leak_credentials() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter((
        r"(?m)^  cause: .*(Connection refused|No connection could be made).*$",
        "  cause: [CONNECTION_REFUSED]",
    ));

    let script = context.temp_dir.child("main.py");
    script.write_str("print('hello')")?;

    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let url = format!("http://username:password@127.0.0.1:{port}/requirements.py");

    uv_snapshot!(context.filters(), context.run()
        .arg("--with-requirements")
        .arg(url)
        .arg(script.as_os_str())
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Request failed after 3 retries
      cause: error sending request for url (http://[LOCALHOST]/requirements.py)
      cause: client error (Connect)
      cause: tcp connect error
      cause: [CONNECTION_REFUSED]
    ");

    Ok(())
}

#[cfg(unix)] // A URL could be a valid filepath on Unix but not on Windows
#[test]
fn run_url_like_with_local_file_priority() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let url = "https://example.com/path/to/main.py";
    let local_path: std::path::PathBuf = ["https:", "", "example.com", "path", "to", "main.py"]
        .iter()
        .collect();

    // replace with URL-like filepath
    let test_script = context.temp_dir.child(local_path);
    test_script.write_str(indoc! { r#"
        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg(url), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!
    ");

    Ok(())
}

#[test]
fn run_stdin_with_pep723() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig",
        # ]
        # ///
        import iniconfig
        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().stdin(std::fs::File::open(test_script)?).arg("-"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_with_env() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("test.py").write_str(indoc! { "
        import os
        print(os.environ.get('THE_EMPIRE_VARIABLE'))
        print(os.environ.get('REBEL_1'))
        print(os.environ.get('REBEL_2'))
        print(os.environ.get('REBEL_3'))
       "
    })?;

    context.temp_dir.child(".env").write_str(indoc! { "
        THE_EMPIRE_VARIABLE=palpatine
        REBEL_1=leia_organa
        REBEL_2=obi_wan_kenobi
        REBEL_3=C3PO
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("test.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    None
    None
    None
    None
    ");

    uv_snapshot!(context.filters(), context.run().arg("--env-file").arg(".env").arg("test.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    palpatine
    leia_organa
    obi_wan_kenobi
    C3PO
    ");

    Ok(())
}

#[test]
fn run_with_env_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("test.py").write_str(indoc! { "
        import os
        print(os.environ.get('THE_EMPIRE_VARIABLE'))
        print(os.environ.get('REBEL_1'))
        print(os.environ.get('REBEL_2'))
        print(os.environ.get('REBEL_3'))
       "
    })?;

    context.temp_dir.child(".file").write_str(indoc! { "
        THE_EMPIRE_VARIABLE=palpatine
        REBEL_1=leia_organa
        REBEL_2=obi_wan_kenobi
        REBEL_3=C3PO
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--env-file").arg(".file").arg("test.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    palpatine
    leia_organa
    obi_wan_kenobi
    C3PO
    ");

    context.temp_dir.child(".file").write_str(indoc! { "
        UV_PYTHON_SEARCH_PATH=.no-python
        THE_EMPIRE_VARIABLE=palpatine
        REBEL_1=leia_organa
        REBEL_2=obi_wan_kenobi
        REBEL_3=C3PO
       "
    })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--no-project")
        .arg("--no-managed-python")
        .arg("--python").arg("3.12")
        .arg("--env-file").arg(".file")
        .arg("test.py")
        .env_remove(EnvVars::VIRTUAL_ENV)
        .env_remove(EnvVars::UV_PYTHON_SEARCH_PATH)
        .env(EnvVars::PATH, context.python_path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    palpatine
    leia_organa
    obi_wan_kenobi
    C3PO
    ");

    Ok(())
}

#[test]
fn run_with_multiple_env_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("test.py").write_str(indoc! { "
        import os
        print(os.environ.get('THE_EMPIRE_VARIABLE'))
        print(os.environ.get('REBEL_1'))
        print(os.environ.get('REBEL_2'))
       "
    })?;

    context.temp_dir.child(".env1").write_str(indoc! { "
        THE_EMPIRE_VARIABLE=palpatine
        REBEL_1=leia_organa
       "
    })?;

    context.temp_dir.child(".env2").write_str(indoc! { "
        THE_EMPIRE_VARIABLE=palpatine
        REBEL_1=obi_wan_kenobi
        REBEL_2=C3PO
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--env-file").arg(".env1").arg("--env-file").arg(".env2").arg("test.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    palpatine
    obi_wan_kenobi
    C3PO
    ");

    uv_snapshot!(context.filters(), context.run().arg("test.py").env(EnvVars::UV_ENV_FILE, ".env1 .env2"), @"
    exit_code: 0 (success)
    ----- stdout -----
    palpatine
    obi_wan_kenobi
    C3PO
    ");

    Ok(())
}

#[test]
fn run_with_env_omitted() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("test.py").write_str(indoc! { "
        import os
        print(os.environ.get('THE_EMPIRE_VARIABLE'))
       "
    })?;

    context.temp_dir.child(".env").write_str(indoc! { "
        THE_EMPIRE_VARIABLE=palpatine
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--env-file").arg(".env").arg("--no-env-file").arg("test.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    None
    ");

    Ok(())
}

#[test]
fn run_with_malformed_env() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("test.py").write_str(indoc! { "
        import os
        print(os.environ.get('THE_EMPIRE_VARIABLE'))
       "
    })?;

    context.temp_dir.child(".env").write_str(indoc! { "
        THE_^EMPIRE_VARIABLE=darth_vader
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--env-file").arg(".env").arg("test.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    None

    ----- stderr -----
    warning: Failed to parse environment file `.env` at position 4: THE_^EMPIRE_VARIABLE=darth_vader
    ");

    Ok(())
}

#[test]
fn run_with_not_existing_env_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("test.py").write_str(indoc! { "
        import os
        print(os.environ.get('THE_EMPIRE_VARIABLE'))
       "
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--env-file").arg(".env.development").arg("test.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No environment file found at: .env.development
    ");

    uv_snapshot!(context.filters(), context.run().arg("--env-file").arg(".env.development").arg("--quiet").arg("test.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No environment file found at: .env.development
    ");

    uv_snapshot!(context.filters(), context.run().arg("--env-file").arg(".env.development").arg("--quiet").arg("--quiet").arg("test.py"), @"
    exit_code: 2 (failure)
    ");

    Ok(())
}

#[test]
fn run_with_extra_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12.0"
        dependencies = []

        [project.optional-dependencies]
        foo = ["iniconfig==2.0.0"]
        bar = ["iniconfig==1.1.1"]

        [tool.uv]
        conflicts = [
          [
            { extra = "foo" },
            { extra = "bar" },
          ],
        ]
        "#
    })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--extra")
        .arg("foo")
        .arg("python")
        .arg("-c")
        .arg("import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_with_group_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12.0"
        dependencies = []

        [dependency-groups]
        foo = ["iniconfig==2.0.0"]
        bar = ["iniconfig==1.1.1"]

        [tool.uv]
        conflicts = [
          [
            { group = "foo" },
            { group = "bar" },
          ],
        ]
        "#
    })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--group")
        .arg("foo")
        .arg("python")
        .arg("-c")
        .arg("import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn run_default_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["typing-extensions"]

        [dependency-groups]
        foo = ["anyio"]
        bar = ["iniconfig"]
        dev = ["sniffio"]
        "#,
    )?;

    context.lock().assert().success();

    // Only the main dependencies and `dev` group should be installed.
    uv_snapshot!(context.filters(), context.run().arg("python").arg("-c").arg("import typing_extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    // If we set a different default group, it should be synced instead.
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["typing-extensions"]

        [dependency-groups]
        foo = ["anyio"]
        bar = ["iniconfig"]
        dev = ["sniffio"]

        [tool.uv]
        default-groups = ["foo"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--exact")
        .arg("python")
        .arg("-c")
        .arg("import typing_extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
    ");

    // `--no-group` should remove from the defaults.
    uv_snapshot!(context.filters(), context.run()
        .arg("--exact")
        .arg("--no-group")
        .arg("foo")
        .arg("python")
        .arg("-c")
        .arg("import typing_extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Uninstalled 3 packages in [TIME]
     - anyio==4.3.0
     - idna==3.6
     - sniffio==1.3.1
    ");

    // Using `--group` should include the defaults
    uv_snapshot!(context.filters(), context.run()
        .arg("--exact")
        .arg("--group")
        .arg("bar")
        .arg("python")
        .arg("-c")
        .arg("import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + iniconfig==2.0.0
     + sniffio==1.3.1
    ");

    // Using `--all-groups` should include the defaults
    uv_snapshot!(context.filters(), context.run()
        .arg("--exact")
        .arg("--all-groups")
        .arg("python")
        .arg("-c")
        .arg("import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 5 packages in [TIME]
    ");

    // Using `--only-group` should exclude the defaults
    uv_snapshot!(context.filters(), context.run()
        .arg("--exact")
        .arg("--only-group")
        .arg("bar")
        .arg("python")
        .arg("-c")
        .arg("import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Uninstalled 4 packages in [TIME]
     - anyio==4.3.0
     - idna==3.6
     - sniffio==1.3.1
     - typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--exact")
        .arg("--all-groups")
        .arg("python")
        .arg("-c")
        .arg("import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    // Using `--no-default-groups` should exclude all groups.
    uv_snapshot!(context.filters(), context.run()
        .arg("--exact")
        .arg("--no-default-groups")
        .arg("python")
        .arg("-c")
        .arg("import typing_extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Uninstalled 4 packages in [TIME]
     - anyio==4.3.0
     - idna==3.6
     - iniconfig==2.0.0
     - sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--all-groups")
        .arg("python")
        .arg("-c")
        .arg("import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + iniconfig==2.0.0
     + sniffio==1.3.1
    ");

    // Using `--no-default-groups` with `--group foo` and `--group bar` should include those
    // groups.
    uv_snapshot!(context.filters(), context.run()
        .arg("--exact")
        .arg("--no-default-groups")
        .arg("--group")
        .arg("foo")
        .arg("--group")
        .arg("bar")
        .arg("python")
        .arg("-c")
        .arg("import typing_extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 5 packages in [TIME]
    ");

    Ok(())
}

/// Ensures default dependency groups participate in automatic Python selection.
#[test]
fn run_groups_requires_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        foo = []
        dev = []

        [tool.uv.dependency-groups]
        foo = { requires-python = ">=3.100" }
        dev = { requires-python = ">=3.13" }
        "#,
    )?;

    // With --no-default-groups only the main requires-python should be consulted
    uv_snapshot!(context.filters(), context.run()
        .arg("--no-default-groups")
        .arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    // The main requires-python and the default group's requires-python should be consulted
    // (This should trigger a version bump)
    uv_snapshot!(context.filters(), context.run()
        .arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.13.[X]

    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    Ok(())
}

/// Ensures relaxing group requirements reuses a compatible environment, while an explicit Python
/// request can downgrade it.
#[test]
fn run_groups_requires_python_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        foo = []
        dev = []

        [tool.uv.dependency-groups]
        foo = { requires-python = ">=3.100" }
        dev = { requires-python = ">=3.13" }
        "#,
    )?;

    // Start with an environment that includes "dev" and requires Python 3.13.
    context
        .run()
        .arg("python")
        .arg("--version")
        .assert()
        .success();

    // TMP: Attempt to catch this flake with verbose output
    // See https://github.com/astral-sh/uv/issues/14160
    let output = context
        .run()
        .arg("-vv")
        .arg("--no-default-groups")
        .arg("python")
        .arg("--version")
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Removed virtual environment"),
        "{}",
        stderr
    );

    // Disabling the default group shouldn't churn a compatible environment.
    uv_snapshot!(context.filters(), context.run()
        .arg("--no-default-groups")
        .arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.13.[X]

    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    // Explicitly requesting an in-range python can downgrade
    uv_snapshot!(context.filters(), context.run()
        .arg("--no-default-groups")
        .arg("-p").arg("3.12")
        .arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    Ok(())
}

/// Ensures `uv run` distinguishes an incompatible explicit Python request from a group requirement
/// that no available interpreter satisfies.
#[test]
fn run_groups_requires_python_errors() -> Result<()> {
    let context =
        uv_test::test_context_with_versions!(&["3.12", "3.13"]).with_filtered_python_sources();
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        foo = []
        dev = []

        [tool.uv.dependency-groups]
        foo = { requires-python = ">=3.100" }
        dev = { requires-python = ">=3.13" }
        "#,
    )?;

    // Explicitly requesting an out-of-range python fails
    uv_snapshot!(context.filters(), context.run()
        .arg("-p").arg("3.12")
        .arg("python").arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    error: The requested interpreter resolved to Python 3.12.[X], which is incompatible with the project's Python requirement: `>=3.13` (from `tool.uv.dependency-groups.dev.requires-python`).
    ");

    // An isolated environment must satisfy the selected groups too.
    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("-p").arg("3.12")
        .arg("python").arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The requested interpreter resolved to Python 3.12.[X], which is incompatible with the project's Python requirement: `>=3.13` (from `tool.uv.dependency-groups.dev.requires-python`).
    ");

    // Enabling foo we can't find an interpreter
    uv_snapshot!(context.filters(), context.run()
        .arg("--group").arg("foo")
        .arg("python").arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python >=3.100 in [PYTHON SOURCES]
    ");

    Ok(())
}

#[test]
fn run_groups_include_requires_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12", "3.13"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = ["typing-extensions"]

        [dependency-groups]
        foo = ["anyio"]
        bar = ["iniconfig"]
        baz = ["iniconfig"]
        dev = ["sniffio", {include-group = "foo"}, {include-group = "baz"}]

        [tool.uv.dependency-groups]
        foo = {requires-python="<3.13"}
        bar = {requires-python=">=3.13"}
        baz = {requires-python=">=3.12"}
        "#,
    )?;

    context.lock().assert().success();

    // With --no-default-groups only the main requires-python should be consulted
    uv_snapshot!(context.filters(), context.run()
        .arg("--no-default-groups")
        .arg("python").arg("-c").arg("import typing_extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    // The main requires-python and the default group's requires-python should be consulted
    // (This should trigger a version bump)
    uv_snapshot!(context.filters(), context.run()
        .arg("python").arg("-c").arg("import typing_extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Removed virtual environment at: .venv
    Creating virtual environment at: .venv
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 5 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + iniconfig==2.0.0
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    // The main requires-python and "dev" and "bar" requires-python should be consulted
    // (This should trigger a conflict)
    uv_snapshot!(context.filters(), context.run()
        .arg("--group").arg("bar")
        .arg("python").arg("-c").arg("import typing_extensions"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Found conflicting Python requirements:
    - project: >=3.11
    - project:bar: >=3.13
    - project:dev: >=3.12, <3.13
    ");

    // Explicitly requesting an out-of-range python fails
    uv_snapshot!(context.filters(), context.run()
        .arg("-p").arg("3.13")
        .arg("python").arg("-c").arg("import typing_extensions"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    error: The requested interpreter resolved to Python 3.13.[X], which is incompatible with the project's Python requirement: `==3.12.*` (from `tool.uv.dependency-groups.dev.requires-python`).
    ");
    Ok(())
}

/// Test that a signal n makes the process exit with code 128+n.
#[cfg(unix)]
#[test]
fn exit_status_signal() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("segfault.py");
    script.write_str(indoc! {r"
        import os
        os.kill(os.getpid(), 11)
    "})?;
    let status = context.run().arg(script.path()).status()?;
    assert_eq!(status.code().expect("a status code"), 139);
    Ok(())
}

#[test]
fn run_repeated() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.13", "3.12"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = ["iniconfig"]
        "#
    })?;

    // Import `iniconfig` in the context of the project.
    uv_snapshot!(
        context.filters(),
        context.run().arg("--with").arg("typing-extensions").arg("python").arg("-c").arg("import typing_extensions; import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    // Re-running shouldn't require reinstalling `typing-extensions`, since the environment is cached.
    uv_snapshot!(
        context.filters(),
        context.run().arg("--with").arg("typing-extensions").arg("python").arg("-c").arg("import typing_extensions; import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 1 package in [TIME]
    Resolved 1 package in [TIME]
    ");

    // Import `iniconfig` in the context of a `tool run` command, which should fail.
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("--with").arg("typing-extensions").arg("python").arg("-c").arg("import typing_extensions; import iniconfig"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Traceback (most recent call last):
      File "<string>", line 1, in <module>
        import typing_extensions; import iniconfig
                                  ^^^^^^^^^^^^^^^^
    ModuleNotFoundError: No module named 'iniconfig'
    "#);

    Ok(())
}

/// See: <https://github.com/astral-sh/uv/issues/11117>
#[test]
fn run_without_overlay() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.13"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = ["iniconfig"]
        "#
    })?;

    // Import `iniconfig` in the context of the project.
    uv_snapshot!(
        context.filters(),
        context.run().arg("--with").arg("typing-extensions").arg("python").arg("-c").arg("import typing_extensions; import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    // Import `iniconfig` in the context of a `tool run` command, which should fail.
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("--with").arg("typing-extensions").arg("python").arg("-c").arg("import typing_extensions; import iniconfig"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Traceback (most recent call last):
      File "<string>", line 1, in <module>
        import typing_extensions; import iniconfig
                                  ^^^^^^^^^^^^^^^^
    ModuleNotFoundError: No module named 'iniconfig'
    "#);

    // Re-running in the context of the project should reset the overlay.
    uv_snapshot!(
        context.filters(),
        context.run().arg("--with").arg("typing-extensions").arg("python").arg("-c").arg("import typing_extensions; import iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 1 package in [TIME]
    Resolved 1 package in [TIME]
    ");

    Ok(())
}

/// See: <https://github.com/astral-sh/uv/issues/11220>
#[cfg(unix)]
#[test]
fn detect_infinite_recursion() -> Result<()> {
    use indoc::formatdoc;
    use std::os::unix::fs::PermissionsExt;
    use uv_test::get_bin;

    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main");
    test_script.write_str(&formatdoc! { r#"
        #!{uv} run

        print("Hello, world!")
    "#, uv = get_bin!().display() })?;

    fs_err::set_permissions(test_script.path(), PermissionsExt::from_mode(0o0744))?;

    let mut command = context.external_command(&test_script);

    // Set the max recursion depth to a lower amount to speed up testing.
    command.env(EnvVars::UV_RUN_MAX_RECURSION_DEPTH, "5");

    uv_snapshot!(context.filters(), command, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `uv run` was recursively invoked 6 times which exceeds the limit of 5

    hint: If you are running a script with `uv run` in the shebang, you may need to include the `--script` flag
    ");

    Ok(())
}

#[test]
fn run_uv_variable() {
    let context = uv_test::test_context!("3.12");

    // Display the `UV` variable
    uv_snapshot!(
        context.filters(),
        context.run().arg("python").arg("-c").arg("import os; print(os.environ['UV'])"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [UV]
    ");
}

/// Test legacy scripts <https://packaging.python.org/en/latest/guides/distributing-packages-using-setuptools/#scripts>.
///
/// This tests for execution and detection of legacy windows scripts with .bat, .cmd, and .ps1 extensions.
#[cfg(windows)]
#[test]
fn run_windows_legacy_scripts() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");

    // Use `script-files` which enables legacy scripts packaging.
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = []

        [tool.setuptools]
        packages = []
        script-files = [
            "misc/custom_pydoc.bat",
            "misc/custom_pydoc.cmd",
            "misc/custom_pydoc.ps1"
        ]

        [build-system]
        requires = ["setuptools>=42"]
        build-backend = "setuptools.build_meta"
        "#
    })?;

    let custom_pydoc_bat = context.temp_dir.child("misc").child("custom_pydoc.bat");
    let custom_pydoc_cmd = context.temp_dir.child("misc").child("custom_pydoc.cmd");
    let custom_pydoc_ps1 = context.temp_dir.child("misc").child("custom_pydoc.ps1");

    custom_pydoc_bat.write_str("python.exe -m pydoc %*")?;
    custom_pydoc_cmd.write_str("python.exe -m pydoc %*")?;
    custom_pydoc_ps1.write_str("python.exe -m pydoc $args")?;

    uv_snapshot!(context.filters(), context.run(), @r###"
    exit_code: 2 (failure)
    ----- stdout -----
    Provide a command or script to invoke with `uv run <command>` or `uv run <script>.py`.

    The following commands are available in the environment:

    - custom_pydoc.bat
    - custom_pydoc.cmd
    - custom_pydoc.ps1
    - pydoc.bat
    - python
    - pythonw

    See `uv run --help` for more information.

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
    "###);

    // Test with explicit .bat extension
    uv_snapshot!(context.filters(), context.run().arg("custom_pydoc.bat"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    pydoc - the Python documentation tool

    pydoc <name> ...
        Show text documentation on something.  <name> may be the name of a
        Python keyword, topic, function, module, or package, or a dotted
        reference to a class or function within a module or module in a
        package.  If <name> contains a '\', it is used as the path to a
        Python source file to document. If name is 'keywords', 'topics',
        or 'modules', a listing of these things is displayed.

    pydoc -k <keyword>
        Search for a keyword in the synopsis lines of all available modules.

    pydoc -n <hostname>
        Start an HTTP server with the given hostname (default: localhost).

    pydoc -p <port>
        Start an HTTP server on the given port on the local machine.  Port
        number 0 can be used to get an arbitrary unused port.

    pydoc -b
        Start an HTTP server on an arbitrary unused port and open a web browser
        to interactively browse documentation.  This option can be used in
        combination with -n and/or -p.

    pydoc -w <name> ...
        Write out the HTML documentation for a module to a file in the current
        directory.  If <name> contains a '\', it is treated as a filename; if
        it names a directory, documentation is written for all the contents.


    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "###);

    // Test with explicit .cmd extension
    uv_snapshot!(context.filters(), context.run().arg("custom_pydoc.cmd"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    pydoc - the Python documentation tool

    pydoc <name> ...
        Show text documentation on something.  <name> may be the name of a
        Python keyword, topic, function, module, or package, or a dotted
        reference to a class or function within a module or module in a
        package.  If <name> contains a '\', it is used as the path to a
        Python source file to document. If name is 'keywords', 'topics',
        or 'modules', a listing of these things is displayed.

    pydoc -k <keyword>
        Search for a keyword in the synopsis lines of all available modules.

    pydoc -n <hostname>
        Start an HTTP server with the given hostname (default: localhost).

    pydoc -p <port>
        Start an HTTP server on the given port on the local machine.  Port
        number 0 can be used to get an arbitrary unused port.

    pydoc -b
        Start an HTTP server on an arbitrary unused port and open a web browser
        to interactively browse documentation.  This option can be used in
        combination with -n and/or -p.

    pydoc -w <name> ...
        Write out the HTML documentation for a module to a file in the current
        directory.  If <name> contains a '\', it is treated as a filename; if
        it names a directory, documentation is written for all the contents.


    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "###);

    // Test with explicit .ps1 extension
    uv_snapshot!(context.filters(), context.run().arg("custom_pydoc.ps1"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    pydoc - the Python documentation tool

    pydoc <name> ...
        Show text documentation on something.  <name> may be the name of a
        Python keyword, topic, function, module, or package, or a dotted
        reference to a class or function within a module or module in a
        package.  If <name> contains a '\', it is used as the path to a
        Python source file to document. If name is 'keywords', 'topics',
        or 'modules', a listing of these things is displayed.

    pydoc -k <keyword>
        Search for a keyword in the synopsis lines of all available modules.

    pydoc -n <hostname>
        Start an HTTP server with the given hostname (default: localhost).

    pydoc -p <port>
        Start an HTTP server on the given port on the local machine.  Port
        number 0 can be used to get an arbitrary unused port.

    pydoc -b
        Start an HTTP server on an arbitrary unused port and open a web browser
        to interactively browse documentation.  This option can be used in
        combination with -n and/or -p.

    pydoc -w <name> ...
        Write out the HTML documentation for a module to a file in the current
        directory.  If <name> contains a '\', it is treated as a filename; if
        it names a directory, documentation is written for all the contents.


    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "###);

    // Test without explicit extension (.ps1 should be used) as there's no .exe available.
    uv_snapshot!(context.filters(), context.run().arg("custom_pydoc"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    pydoc - the Python documentation tool

    pydoc <name> ...
        Show text documentation on something.  <name> may be the name of a
        Python keyword, topic, function, module, or package, or a dotted
        reference to a class or function within a module or module in a
        package.  If <name> contains a '\', it is used as the path to a
        Python source file to document. If name is 'keywords', 'topics',
        or 'modules', a listing of these things is displayed.

    pydoc -k <keyword>
        Search for a keyword in the synopsis lines of all available modules.

    pydoc -n <hostname>
        Start an HTTP server with the given hostname (default: localhost).

    pydoc -p <port>
        Start an HTTP server on the given port on the local machine.  Port
        number 0 can be used to get an arbitrary unused port.

    pydoc -b
        Start an HTTP server on an arbitrary unused port and open a web browser
        to interactively browse documentation.  This option can be used in
        combination with -n and/or -p.

    pydoc -w <name> ...
        Write out the HTML documentation for a module to a file in the current
        directory.  If <name> contains a '\', it is treated as a filename; if
        it names a directory, documentation is written for all the contents.


    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "###);

    Ok(())
}

/// If a `--with` requirement overlaps with a locked script requirement, respect the lockfile as a
/// preference.
///
/// See: <https://github.com/astral-sh/uv/issues/13173>
#[test]
fn run_pep723_script_with_constraints_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig<2",
        # ]
        # ///

        import iniconfig

        print("Hello, world!")
       "#
    })?;

    // Explicitly lock the script.
    uv_snapshot!(context.filters(), context.lock().arg("--script").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    let lock = context.read("main.py.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 5
        requires-python = ">=3.11"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [{ name = "iniconfig", specifier = "<2" }]

        [[package]]
        name = "iniconfig"
        version = "1.1.1"
        source = { registry = "https://pypi.org/simple" }
        sdist = { url = "https://files.pythonhosted.org/packages/23/a2/97899f6bd0e873fed3a7e67ae8d3a08b21799430fb4da15cfedf10d6e2c2/iniconfig-1.1.1.tar.gz", hash = "sha256:bc3af051d7d14b2ee5ef9969666def0cd1a000e121eaea580d4a313df4b37f32", size = 8104, upload-time = "2020-10-14T10:20:18.572Z" }
        wheels = [
            { url = "https://files.pythonhosted.org/packages/9b/dd/b3c12c6d707058fa947864b67f0c4e0c39ef8610988d7baea9578f3c48f3/iniconfig-1.1.1-py2.py3-none-any.whl", hash = "sha256:011e24c64b7f47f6ebd835bb12a743f2fbe9a26d4cecaa7f53bc4f35ee9da8b3", size = 4990, upload-time = "2020-10-16T17:37:23.05Z" },
        ]
        "#
        );
    });

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.10"
        dependencies = [
          "iniconfig",
        ]
        "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--with").arg(".").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==1.1.1
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + iniconfig==1.1.1
    ");

    Ok(())
}

/// If a `--with` requirement overlaps with a non-locked script requirement, respect the environment
/// site-packages as preferences.
///
/// See: <https://github.com/astral-sh/uv/issues/13173>
#[test]
fn run_pep723_script_with_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "iniconfig<2",
        # ]
        # ///

        import iniconfig

        print("Hello, world!")
       "#
    })?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.10"
        dependencies = [
          "iniconfig",
        ]
        "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("--with").arg(".").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==1.1.1
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + iniconfig==1.1.1
    ");

    Ok(())
}

#[test]
fn run_no_sync_incompatible_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11", "3.9"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = [
          "iniconfig"
        ]
        "#
    })?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r#"
        import iniconfig
        print("Hello, world!")
       "#
    })?;

    uv_snapshot!(context.filters(), context.run().arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    uv_snapshot!(context.filters(), context.run().arg("--no-sync").arg("--python").arg("3.9").arg("main.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    warning: Using incompatible environment (`.venv`) due to `--no-sync` (The project environment's Python version does not satisfy the request: `Python 3.9`)
    ");

    Ok(())
}

#[test]
fn run_python_preference_no_project() {
    let context =
        uv_test::test_context_with_versions!(&["3.12", "3.11"]).with_versions_as_managed(&["3.12"]);

    context.venv().assert().success();

    uv_snapshot!(context.filters(), context.run().arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    uv_snapshot!(context.filters(), context.run().arg("--managed-python").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    // `VIRTUAL_ENV` is set here, so we'll ignore the flag
    uv_snapshot!(context.filters(), context.run().arg("--no-managed-python").arg("python").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]
    ");

    // If we remove the `VIRTUAL_ENV` variable, we should get the unmanaged Python
    uv_snapshot!(context.filters(), context.run().arg("--no-managed-python").arg("python").arg("--version").env_remove(EnvVars::VIRTUAL_ENV), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]
    ");
}

/// Regression test for: <https://github.com/astral-sh/uv/issues/15518>
#[test]
fn isolate_child_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = [
          "iniconfig"
        ]

        [tool.uv.workspace]
        members = ["child"]
        "#
    })?;

    context
        .temp_dir
        .child("child")
        .child("pyproject.toml")
        .write_str(indoc! { r#"
        [project]
        name = "child"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
        })?;

    // Sync the parent package.
    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Ensure that the isolated environment can't access `iniconfig` (from the parent package).
    uv_snapshot!(context.filters(), context.run().arg("--package").arg("child").arg("--isolated").arg("python").arg("-c").arg("import iniconfig"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Checked in [TIME]
    Traceback (most recent call last):
      File "<string>", line 1, in <module>
    ModuleNotFoundError: No module named 'iniconfig'
    "#);

    // Ensure that the isolated environment can't access `iniconfig` (from the parent package).
    uv_snapshot!(context.filters(), context.run().arg("--package").arg("child").arg("--isolated").arg("--with").arg("typing-extensions").arg("python").arg("-c").arg("import iniconfig"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Checked in [TIME]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    Traceback (most recent call last):
      File "<string>", line 1, in <module>
    ModuleNotFoundError: No module named 'iniconfig'
    "#);

    Ok(())
}

#[test]
fn run_only_group_and_extra_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        test = ["pytest"]

        [dependency-groups]
        dev = ["ruff"]
        "#,
    )?;

    // Using --only-group and --extra together should error.
    uv_snapshot!(context.filters(), context.run().arg("--only-group").arg("dev").arg("--extra").arg("test").arg("python").arg("-c").arg("print('hello')"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--only-group <ONLY_GROUP>' cannot be used with '--extra <EXTRA>'

    Usage: uv run --cache-dir [CACHE_DIR] --only-group <ONLY_GROUP> --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");

    // Using --only-group and --all-extras together should also error.
    uv_snapshot!(context.filters(), context.run().arg("--only-group").arg("dev").arg("--all-extras").arg("python").arg("-c").arg("print('hello')"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--only-group <ONLY_GROUP>' cannot be used with '--all-extras'

    Usage: uv run --cache-dir [CACHE_DIR] --only-group <ONLY_GROUP> --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");

    Ok(())
}

fn setup_target_workspace_discovery_context() -> Result<TestContext> {
    let context = uv_test::test_context!("3.12");

    // Create a workspace in a subdirectory.
    let workspace = context.temp_dir.child("project");
    workspace.create_dir_all()?;

    workspace.child("pyproject.toml").write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.workspace]
        "#
    })?;
    workspace
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    // Create a script in the workspace that imports from the project.
    workspace.child("script.py").write_str(indoc! { r"
        import iniconfig
        print('success')
        "
    })?;

    Ok(context)
}

/// Test that `uv run` discovers the workspace from the target's directory rather than the current
/// working directory.
#[test]
fn run_target_workspace_discovery() -> Result<()> {
    let context = setup_target_workspace_discovery_context()?;

    // Write invalid configuration files to the cwd to verify that the
    // target workspace discovery skips parsing them.
    context.temp_dir.child("uv.toml").write_str("bad")?;
    context.temp_dir.child("pyproject.toml").write_str("bad")?;

    uv_snapshot!(context.filters(), context.run().arg("project/script.py").env_remove(EnvVars::VIRTUAL_ENV), @"
    exit_code: 0 (success)
    ----- stdout -----
    success

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: project/.venv
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/project)
     + iniconfig==2.0.0
    ");

    Ok(())
}

/// Regression test for <https://github.com/astral-sh/uv/issues/8851#issuecomment-5123317996>.
#[test]
fn run_target_workspace_discovery_workspace_root_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! { r#"
        [project]
        name = "myproj"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv.workspace]
        members = ["subproj-a", "subproj-b"]

        [dependency-groups]
        test = ["iniconfig"]
        "#
        })?;

    let subproject_a = context.temp_dir.child("subproj-a");
    subproject_a.child("pyproject.toml").write_str(indoc! { r#"
        [project]
        name = "subproj-a"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        integration = ["typing-extensions"]
        "#
    })?;

    context
        .temp_dir
        .child("subproj-b")
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [project]
            name = "subproj-b"
            version = "0.1.0"
            requires-python = ">=3.12"
            "#
        })?;

    subproject_a
        .child("scripts")
        .child("thing.py")
        .write_str(indoc! { r"
            import iniconfig

            print('success')
            "
        })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--only-group")
        .arg("test")
        .arg("subproj-a/scripts/thing.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    success

    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

/// Excluded inherited groups must still be recognized during group validation.
#[test]
fn run_target_workspace_discovery_excluded_workspace_root_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [project]
            name = "root"
            version = "0.1.0"
            requires-python = ">=3.12"

            [dependency-groups]
            root-only = []

            [tool.uv.workspace]
            members = ["child"]
            "#
        })?;

    context
        .temp_dir
        .child("child")
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [project]
            name = "child"
            version = "0.1.0"
            requires-python = ">=3.12"
            "#
        })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--offline")
        .arg("--project")
        .arg("child")
        .arg("--no-group")
        .arg("root-only")
        .arg("python")
        .arg("-c")
        .arg("print('success')"), @"
    exit_code: 0 (success)
    ----- stdout -----
    success

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked in [TIME]
    ");

    Ok(())
}

/// Workspace defaults and member-defined groups remain distinct when a member is selected.
#[test]
fn run_target_workspace_discovery_workspace_group_defaults() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [project]
            name = "root"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["iniconfig"]

            [dependency-groups]
            dev = ["sniffio"]
            root-only = ["idna"]

            [tool.uv]
            default-groups = ["root-only"]

            [tool.uv.workspace]
            members = ["child"]
            "#
        })?;

    let child = context.temp_dir.child("child");
    child.child("pyproject.toml").write_str(indoc! { r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["typing-extensions"]
        "#
    })?;

    child
        .child("scripts")
        .child("groups.py")
        .write_str(indoc! { r#"
            import importlib.util

            installed = [
                package
                for package in (
                    "iniconfig",
                    "typing_extensions",
                    "sniffio",
                    "packaging",
                    "idna",
                    "six",
                )
                if importlib.util.find_spec(package) is not None
            ]
            print(f"installed: {', '.join(installed)}")
            "#
        })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: typing_extensions

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--no-default-groups")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: typing_extensions

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    child.child("pyproject.toml").write_str(indoc! { r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["typing-extensions"]

        [dependency-groups]
        dev = ["packaging"]
        "#
    })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: typing_extensions, packaging

    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + packaging==24.0
     + typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("dev")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: packaging

    ----- stderr -----
    Resolved 7 packages in [TIME]
    Installed 1 package in [TIME]
     + packaging==24.0
    ");

    child.child("pyproject.toml").write_str(indoc! { r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["typing-extensions"]

        [dependency-groups]
        dev = []
        "#
    })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("dev")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed:

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked in [TIME]
    ");

    Ok(())
}

/// Member-defined groups override inherited groups from a project-backed workspace root.
#[test]
fn run_target_workspace_discovery_workspace_project_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [project]
            name = "root"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["iniconfig"]

            [dependency-groups]
            root-only = ["sniffio"]
            shared = ["idna"]

            [tool.uv]
            default-groups = ["root-only"]

            [tool.uv.workspace]
            members = ["child"]
            "#
        })?;

    let child = context.temp_dir.child("child");
    child.child("pyproject.toml").write_str(indoc! { r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["typing-extensions"]

        [dependency-groups]
        member-only = ["packaging"]
        shared = ["six"]

        [tool.uv]
        default-groups = ["member-only"]
        "#
    })?;

    child
        .child("scripts")
        .child("groups.py")
        .write_str(indoc! { r#"
            import importlib.util

            installed = [
                package
                for package in (
                    "iniconfig",
                    "typing_extensions",
                    "sniffio",
                    "packaging",
                    "idna",
                    "six",
                )
                if importlib.util.find_spec(package) is not None
            ]
            print(f"installed: {', '.join(installed)}")
            "#
        })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("root-only")
        .arg("python")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: sniffio

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("member-only")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: packaging

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + packaging==24.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--group")
        .arg("root-only")
        .arg("--group")
        .arg("member-only")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: typing_extensions, sniffio, packaging

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 3 packages in [TIME]
     + packaging==24.0
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("shared")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: six

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + six==1.16.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--all-packages")
        .arg("--only-group")
        .arg("shared")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: idna, six

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + idna==3.6
     + six==1.16.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--all-groups")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: typing_extensions, sniffio, packaging, six

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Installed 4 packages in [TIME]
     + packaging==24.0
     + six==1.16.0
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--project")
        .arg(".")
        .arg("--only-group")
        .arg("root-only")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: sniffio

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--project")
        .arg("child")
        .arg("--only-group")
        .arg("root-only")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: sniffio

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("root-only")
        .arg("python")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: sniffio

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("member-only")
        .arg("python")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: packaging

    ----- stderr -----
    Resolved 8 packages in [TIME]
    Installed 1 package in [TIME]
     + packaging==24.0
    ");

    Ok(())
}

/// Non-project workspace roots retain manifest-level groups even for selected members.
#[test]
fn run_target_workspace_discovery_virtual_workspace_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [dependency-groups]
            root-only = ["sniffio"]
            shared = ["idna"]

            [tool.uv]
            default-groups = ["root-only"]

            [tool.uv.workspace]
            members = ["child"]
            "#
        })?;

    let child = context.temp_dir.child("child");
    child.child("pyproject.toml").write_str(indoc! { r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["typing-extensions"]

        [dependency-groups]
        member-only = ["packaging"]
        shared = ["six"]

        [tool.uv]
        default-groups = ["member-only"]
        "#
    })?;

    child
        .child("scripts")
        .child("groups.py")
        .write_str(indoc! { r#"
            import importlib.util

            installed = [
                package
                for package in (
                    "iniconfig",
                    "typing_extensions",
                    "sniffio",
                    "packaging",
                    "idna",
                    "six",
                )
                if importlib.util.find_spec(package) is not None
            ]
            print(f"installed: {', '.join(installed)}")
            "#
        })?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("root-only")
        .arg("python")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: sniffio

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("member-only")
        .arg("python")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: packaging

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + packaging==24.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("root-only")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: sniffio

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("member-only")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: packaging

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 1 package in [TIME]
     + packaging==24.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--only-group")
        .arg("shared")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: six

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + six==1.16.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--all-groups")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: typing_extensions, sniffio, packaging, six

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 4 packages in [TIME]
     + packaging==24.0
     + six==1.16.0
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--project")
        .arg(".")
        .arg("--only-group")
        .arg("root-only")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: sniffio

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("root-only")
        .arg("python")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: sniffio

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("shared")
        .arg("python")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: six

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 1 package in [TIME]
     + six==1.16.0
    ");

    uv_snapshot!(context.filters(), context.run()
        .arg("--isolated")
        .arg("--all-packages")
        .arg("--only-group")
        .arg("shared")
        .arg("child/scripts/groups.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed: idna, six

    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + idna==3.6
     + six==1.16.0
    ");

    Ok(())
}

/// Workspace group selection should be consistent across run, sync, and export.
#[test]
fn run_target_workspace_discovery_workspace_project_group_commands() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [project]
            name = "root"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["iniconfig"]

            [dependency-groups]
            root-only = ["sniffio"]
            shared = ["idna"]

            [tool.uv]
            default-groups = ["root-only"]

            [tool.uv.workspace]
            members = ["child"]
            "#
        })?;

    context
        .temp_dir
        .child("child")
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [project]
            name = "child"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["typing-extensions"]

            [dependency-groups]
            member-only = ["packaging"]
            shared = ["six"]

            [tool.uv]
            default-groups = ["member-only"]
            "#
        })?;

    uv_snapshot!(context.filters(), context.sync()
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("root-only"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.sync()
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("shared"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     + six==1.16.0
     - sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.sync()
        .arg("--package")
        .arg("child")
        .arg("--all-groups"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 3 packages in [TIME]
     + packaging==24.0
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.export()
        .args(["--no-header", "--no-hashes", "--no-annotate"])
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("root-only"), @"
    exit_code: 0 (success)
    ----- stdout -----
    sniffio==1.3.1

    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.export()
        .args(["--no-header", "--no-hashes", "--no-annotate"])
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("shared"), @"
    exit_code: 0 (success)
    ----- stdout -----
    six==1.16.0

    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.export()
        .args(["--no-header", "--no-hashes", "--no-annotate"])
        .arg("--package")
        .arg("child")
        .arg("--all-groups"), @"
    exit_code: 0 (success)
    ----- stdout -----
    packaging==24.0
    six==1.16.0
    sniffio==1.3.1
    typing-extensions==4.10.0

    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");

    Ok(())
}

/// Projectless root groups should also behave consistently across commands.
#[test]
fn run_target_workspace_discovery_virtual_workspace_group_commands() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [dependency-groups]
            root-only = ["sniffio"]
            shared = ["idna"]

            [tool.uv]
            default-groups = ["root-only"]

            [tool.uv.workspace]
            members = ["child"]
            "#
        })?;

    context
        .temp_dir
        .child("child")
        .child("pyproject.toml")
        .write_str(indoc! { r#"
            [project]
            name = "child"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["typing-extensions"]

            [dependency-groups]
            member-only = ["packaging"]
            shared = ["six"]

            [tool.uv]
            default-groups = ["member-only"]
            "#
        })?;

    uv_snapshot!(context.filters(), context.sync()
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("root-only"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.sync()
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("shared"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     + six==1.16.0
     - sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.export()
        .args(["--no-header", "--no-hashes", "--no-annotate"])
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("root-only"), @"
    exit_code: 0 (success)
    ----- stdout -----
    sniffio==1.3.1

    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.export()
        .args(["--no-header", "--no-hashes", "--no-annotate"])
        .arg("--package")
        .arg("child")
        .arg("--only-group")
        .arg("shared"), @"
    exit_code: 0 (success)
    ----- stdout -----
    six==1.16.0

    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    Ok(())
}

/// Test target workspace discovery with a bare script filename (no directory component), which
/// would otherwise cause `Path::parent()` to return an empty path.
#[test]
fn run_target_workspace_discovery_bare_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("script.py")
        .write_str(r"print('success')")?;

    uv_snapshot!(context.filters(), context.run()
        .arg("script.py"), @"
    exit_code: 0 (success)
    ----- stdout -----
    success
    ");

    Ok(())
}

/// `--project` should still take precedence over target workspace discovery.
#[test]
fn run_project_precedes_target_workspace_discovery() -> Result<()> {
    let context = setup_target_workspace_discovery_context()?;
    let missing_project = context.temp_dir.child("missing-project");

    uv_snapshot!(context.filters(), context.run()
        .arg("--project")
        .arg(missing_project.path())
        .arg("project/script.py")
        .env_remove(EnvVars::VIRTUAL_ENV), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project directory `missing-project` does not exist
    ");

    Ok(())
}

/// Using `--project` with a non-existent directory should error.
#[test]
fn run_project_not_found() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.run().arg("--project").arg("/tmp/does-not-exist-uv-test").arg("python").arg("-c").arg("print('hello')"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project directory `/tmp/does-not-exist-uv-test` does not exist
    ");
}

/// Using `--project` with a non-existent directory should error with `UV_PREVIEW=1`.
#[test]
fn run_project_not_found_uv_preview_env() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.run().env("UV_PREVIEW", "1").arg("--project").arg("/tmp/does-not-exist-uv-test").arg("python").arg("-c").arg("print('hello')"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project directory `/tmp/does-not-exist-uv-test` does not exist
    ");
}

/// When `--project` points to a `pyproject.toml` file, resolve to its parent directory without
/// warning.
///
/// See: <https://github.com/astral-sh/uv/issues/18508>
#[test]
fn run_project_pyproject_toml_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project_dir = context.temp_dir.child("project");
    project_dir.create_dir_all()?;

    let pyproject_toml = project_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
    })?;

    // Passing `--project project/pyproject.toml` should resolve to the parent directory without warning.
    uv_snapshot!(context.filters(), context.run()
        .arg("--project")
        .arg(project_dir.join("pyproject.toml"))
        .env_remove(EnvVars::VIRTUAL_ENV)
        .arg("--")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: project/.venv
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    Ok(())
}

/// Using `--project` with a non-`pyproject.toml` file should error.
#[test]
fn run_project_non_pyproject_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project_dir = context.temp_dir.child("project");
    project_dir.create_dir_all()?;

    let pyproject_toml = project_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
    })?;

    let other_file = project_dir.child("README.md");
    other_file.write_str("")?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--project")
        .arg(other_file.path())
        .env_remove(EnvVars::VIRTUAL_ENV)
        .arg("--")
        .arg("python")
        .arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project path `project/README.md` is not a directory
    ");

    Ok(())
}

/// Using `--project` with a nested non-`pyproject.toml` file should error.
#[test]
fn run_project_nested_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project_dir = context.temp_dir.child("project");
    project_dir.create_dir_all()?;

    let pyproject_toml = project_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
    })?;

    let subdir = project_dir.child("subdir");
    subdir.create_dir_all()?;
    let nested_file = subdir.child("somefile");
    nested_file.write_str("")?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--project")
        .arg(nested_file.path())
        .env_remove(EnvVars::VIRTUAL_ENV)
        .arg("--")
        .arg("python")
        .arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project path `project/subdir/somefile` is not a directory
    ");

    Ok(())
}

/// Using `--project` with a file that has no ancestor project should error.
#[test]
#[cfg(unix)]
fn run_project_file_no_ancestor_project() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let isolated_dir = context.temp_dir.child("isolated");
    isolated_dir.create_dir_all()?;
    let file_path = isolated_dir.child("somefile");
    file_path.write_str("")?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--project")
        .arg(file_path.path())
        .arg("--")
        .arg("python")
        .arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project path `isolated/somefile` is not a directory
    ");

    Ok(())
}

/// Ensure that `uv run` aborts when malware is detected in a dependency.
#[tokio::test]
async fn run_malware_detected() {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig==2.0.0"]
    "#})
        .unwrap();

    context.lock().assert().success();

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"vulns": [{"id": "MAL-2026-1234"}]}]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/vulns/MAL-2026-1234"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "MAL-2026-1234",
            "modified": "2026-01-01T00:00:00Z",
        })))
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context
        .run()
        .arg("--preview-features").arg("malware-check")
        .arg("python")
        .arg("--version")
        .env(EnvVars::UV_MALWARE_CHECK, "1")
        .env(EnvVars::UV_MALWARE_CHECK_URL, server.uri()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    warning: Malware detected in locked dependencies:
      - `iniconfig==2.0.0`: MAL-2026-1234 (https://osv.dev/vulnerability/MAL-2026-1234)
    error: Malware detected in one or more dependencies that would be installed; aborting sync. Set `UV_MALWARE_CHECK=0` to bypass this check.
    ");
}

#[test]
fn run_centralized_environment_no_sync_uses_incompatible_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_centralized_environment_hashes();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []
    "#})?;
    context
        .sync()
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();

    // `--no-sync` reuses the existing environment despite the Python request.
    uv_snapshot!(context.filters(), context.run()
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .arg("--no-sync")
        .arg("--python")
        .arg("3.11")
        .arg("python")
        .arg("-c")
        .arg("import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    3.12

    ----- stderr -----
    warning: Using incompatible environment (`project-cp3.12.[X]-[HASH]`) due to `--no-sync` (The project environment's Python version does not satisfy the request: `Python 3.11`)
    "#);

    // Without the project link, discovery must reuse the cached environment before
    // rejecting the selected interpreter against the updated requirement.
    uv_fs::remove_virtualenv(&context.temp_dir.join(".venv"))?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&context.read("pyproject.toml").replace(">=3.11", ">=3.13"))?;

    uv_snapshot!(context.filters(), context.run()
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .arg("--no-sync")
        .arg("--python")
        .arg("3.12")
        .arg("python")
        .arg("-c")
        .arg("import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    3.12

    ----- stderr -----
    warning: Using incompatible environment (`project-cp3.12.[X]-[HASH]`) due to `--no-sync` (The project environment's Python version does not meet the Python requirement: `>=3.13`)
    "#);

    Ok(())
}

#[test]
fn run_centralized_environment_path_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_centralized_environment_hashes();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []
    "#})?;
    context
        .sync()
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();

    // Point the path file at an environment outside the centralized store.
    let environment = context.temp_dir.child(".venv");
    uv_fs::remove_virtualenv(environment.path())?;
    let external = context.temp_dir.child("external");
    context
        .venv()
        .arg(external.path())
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();
    // Resolve a relative path file target from `.venv`'s parent.
    environment.write_str("external")?;

    // Like a directory link, use the path file's interpreter to select the cached environment.
    uv_snapshot!(context.filters(), context.run()
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .arg("--no-sync")
        .arg("--python")
        .arg("3.11")
        .arg("python")
        .arg("-c")
        .arg("import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    3.12

    ----- stderr -----
    warning: Using incompatible environment (`project-cp3.12.[X]-[HASH]`) due to `--no-sync` (The project environment's Python version does not satisfy the request: `Python 3.11`)
    "#);
    Ok(())
}

/// Enabling shared script environments cannot reuse packages installed in an ordinary script venv.
#[test]
fn run_pep723_shared_mode_replaces_normal_installations() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str(indoc! {r#"
        name = "shared-script-transition"
        [root]
        requires = ["example"]
        [expected]
        satisfiable = true
        [packages.example.versions."1.0.0"]
        sdist = false
        [packages.example.versions."2.0.0"]
        sdist = false
    "#})?;
    let server = PackseServer::from_scenario(&scenario);
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["example==1.0.0"]
        # ///
        from importlib.metadata import version
        import sys
        print(version("example"))
        print(sys.prefix)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .arg("--index-url").arg(server.index_url())
        .env_remove(EnvVars::UV_EXCLUDE_NEWER).arg("script.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0
    [CACHE_DIR]/environments-v2/script-[HASH]

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0
    ");
    script.write_str(
        &fs_err::read_to_string(script.path())?.replace("example==1.0.0", "example==2.0.0"),
    )?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments"])
        .arg("--index-url").arg(server.index_url())
        .env_remove(EnvVars::UV_EXCLUDE_NEWER).arg("script.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    2.0.0
    [CACHE_DIR]/environments-v2/shared-script-[HASH]

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==2.0.0
    ");
    Ok(())
}

/// Upgrade bounds constrain shared resolution while equivalent resolutions reuse the same base.
#[test]
fn run_pep723_shared_upgrade_package_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (first_name, first) = generate_wheel_with_files(
        &"shared-upgrade".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels.child(first_name).write_binary(&first)?;
    let (second_name, second) = generate_wheel_with_files(
        &"shared-upgrade".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels.child(second_name).write_binary(&second)?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-upgrade"]
        # ///
        from importlib.metadata import distribution
        from pathlib import Path
        dist = distribution("shared-upgrade")
        base = next(parent for parent in Path(dist.locate_file("")).parents if parent.joinpath("pyvenv.cfg").is_file())
        Path("shared-base").write_text(base.as_posix())
        print(dist.version)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    2.0.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-upgrade==2.0.0
    ");
    let latest_base = context.read("shared-base");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "--upgrade-package", "shared-upgrade<2", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-upgrade==1.0.0
    ");
    let constrained_base = context.read("shared-base");
    assert_ne!(constrained_base, latest_base);
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "--upgrade-package", "shared-upgrade<=1", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert_eq!(context.read("shared-base"), constrained_base);
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "--upgrade-package", "unrelated-package<2", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    2.0.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert_eq!(context.read("shared-base"), latest_base);
    Ok(())
}

/// A conflicting upgrade bound must fail before an unlocked shared script executes.
#[test]
fn run_pep723_shared_upgrade_package_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-upgrade".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels.child(filename).write_binary(&wheel)?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-upgrade>=2"]
        # ///
        from pathlib import Path
        Path("ran").touch()
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "--upgrade-package", "shared-upgrade<2", "script.py"]), @r"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving script dependencies
      cause: Because you require shared-upgrade>=2 and shared-upgrade<2, we can conclude that your requirements are unsatisfiable.
    ");
    context
        .temp_dir
        .child("ran")
        .assert(predicate::path::missing());
    Ok(())
}

/// Editable overlay sources and startup customizations precede shared dependencies in all run layers.
#[test]
fn run_pep723_shared_editable_overlay_precedence() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-editable".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("shared_editable/value.py", "VALUE = 'shared'\n")],
    );
    wheels.child(filename).write_binary(&wheel)?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-extra".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels.child(filename).write_binary(&wheel)?;
    context
        .temp_dir
        .child("editable/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared-editable"
        version = "2.0.0"
        requires-python = ">=3.12"
        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("editable/src/shared_editable/__init__.py")
        .touch()?;
    context
        .temp_dir
        .child("editable/src/shared_editable/value.py")
        .write_str("VALUE = 'editable'\n")?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-editable==1.0.0"]
        # ///
        import builtins
        from importlib.metadata import version
        from pathlib import Path
        from shared_editable.value import VALUE
        import sys
        import sysconfig
        Path("overlay-python").write_text(sys.executable)
        Path("overlay-site-packages").write_text(sysconfig.get_path("purelib"))
        print(VALUE)
        print(version("shared-editable"))
        print(getattr(builtins, "_uv_sitecustomize", "absent"))
        if "--extra" in sys.argv:
            import shared_extra
            print(shared_extra.__version__)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    shared
    1.0.0
    absent

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-editable==1.0.0
    ");
    let overlay_python = context.read("overlay-python");
    let overlay_site_packages = context.read("overlay-site-packages");
    context
        .pip_install()
        .args([
            "--offline",
            "--no-index",
            "--editable",
            "editable",
            "--python",
        ])
        .arg(&overlay_python)
        .assert()
        .success();
    fs_err::write(
        Path::new(&overlay_site_packages).join("sitecustomize.py"),
        "import builtins\nbuiltins._uv_sitecustomize = 'customized'\n",
    )?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    editable
    2.0.0
    customized

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "--with", "shared-editable==2.0.0", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    editable
    2.0.0
    customized

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "--with", "shared-extra==1.0.0", "script.py", "--extra"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    editable
    2.0.0
    customized
    1.0.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-extra==1.0.0
    ");
    Ok(())
}

/// Startup hooks can import shared dependencies after an editable overlay path becomes available.
#[test]
fn run_pep723_shared_startup_hooks() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-hook-target".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("shared_hook_target/value.py", "VALUE = 'shared'\n")],
    );
    wheels.child(filename).write_binary(&wheel)?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-startup".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[(
            "shared_startup/hook.py",
            indoc! {r"
            def initialize():
                import builtins
                from shared_hook_target.value import VALUE
                builtins._uv_startup_value = VALUE
        "},
        )],
    );
    wheels.child(filename).write_binary(&wheel)?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-extra".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels.child(filename).write_binary(&wheel)?;
    context
        .temp_dir
        .child("editable/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "shared-hook-target"
        version = "2.0.0"
        requires-python = ">=3.12"
        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("editable/src/shared_hook_target/__init__.py")
        .touch()?;
    context
        .temp_dir
        .child("editable/src/shared_hook_target/value.py")
        .write_str("VALUE = 'editable'\n")?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-hook-target==1.0.0", "shared-startup==1.0.0"]
        # ///
        import builtins
        from pathlib import Path
        import sys
        import sysconfig
        Path("overlay-python").write_text(sys.executable)
        Path("overlay-site-packages").write_text(sysconfig.get_path("purelib"))
        print(getattr(builtins, "_uv_startup_value", "missing"))
        print(getattr(builtins, "_uv_startup_hooks_restored", False))
        if "--extra" in sys.argv:
            import shared_extra
            print(shared_extra.__version__)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    missing
    False

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + shared-hook-target==1.0.0
     + shared-startup==1.0.0
    ");
    let overlay_python = context.read("overlay-python");
    let overlay_site_packages = context.read("overlay-site-packages");
    context
        .pip_install()
        .args([
            "--offline",
            "--no-index",
            "--editable",
            "editable",
            "--python",
        ])
        .arg(&overlay_python)
        .assert()
        .success();
    fs_err::write(
        Path::new(&overlay_site_packages).join("zz_shared_startup.pth"),
        "import shared_startup.hook; shared_startup.hook.initialize()\n",
    )?;
    fs_err::write(
        Path::new(&overlay_site_packages).join("sitecustomize.py"),
        indoc! {r#"
            import builtins
            from pathlib import Path
            import site
            site.addsitedir(str(Path(__file__).parent))
            builtins._uv_startup_hooks_restored = (
                site.addpackage.__module__ == "site"
                and site.execsitecustomize.__module__ == "site"
            )
        "#},
    )?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    editable
    True

    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "--with", "shared-extra==1.0.0", "script.py", "--extra"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    editable
    True
    1.0.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-extra==1.0.0
    ");
    Ok(())
}

/// Reinstallation replaces a shared base without mutating another script's environment.
#[test]
fn run_pep723_shared_reinstall_replaces_base() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-repair".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("shared_repair/payload.txt", "pristine\n")],
    );
    wheels.child(filename).write_binary(&wheel)?;
    let script = indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-repair==1.0.0"]
        # ///
        import json
        from pathlib import Path
        import shared_repair
        import sys
        module = Path(shared_repair.__file__)
        base = next(parent for parent in module.parents if parent.joinpath("pyvenv.cfg").is_file())
        payload = module.with_name("payload.txt")
        print(json.dumps({"overlay": Path(sys.prefix).as_posix(), "base": base.as_posix(), "payload": payload.as_posix(), "value": payload.read_text().strip() if payload.is_file() else "missing"}))
    "#};
    context.temp_dir.child("first.py").write_str(script)?;
    context.temp_dir.child("second.py").write_str(script)?;
    let first = context
        .run()
        .args([
            "--preview-features",
            "shared-script-environments",
            "--no-index",
            "--find-links",
            "wheels",
            "first.py",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let first: serde_json::Value = serde_json::from_slice(&first.stdout)?;
    assert_eq!(first["value"], "pristine");
    let second = context
        .run()
        .args([
            "--preview-features",
            "shared-script-environments",
            "--no-index",
            "--find-links",
            "wheels",
            "second.py",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let second: serde_json::Value = serde_json::from_slice(&second.stdout)?;
    assert_eq!(first["base"], second["base"]);
    assert_ne!(first["overlay"], second["overlay"]);
    let old_base = Path::new(first["base"].as_str().context("missing shared base")?);
    let old_payload = Path::new(first["payload"].as_str().context("missing payload path")?);
    let second_config = Path::new(
        second["overlay"]
            .as_str()
            .context("missing second overlay")?,
    )
    .join("pyvenv.cfg");
    let second_config_before = context.read(&second_config);
    fs_err::remove_file(old_payload)?;

    // A targeted request for an unrelated package does not replace this dependency environment.
    let unrelated = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "--reinstall-package", "unrelated-package", "first.py"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {"overlay": "[CACHE_DIR]/environments-v2/shared-first-[HASH]", "base": "[CACHE_DIR]/archive-v0/[HASH]", "payload": "[CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/shared_repair/payload.txt", "value": "missing"}

    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);
    let unrelated: serde_json::Value = serde_json::from_slice(&unrelated.stdout)?;
    assert_eq!(unrelated["base"], first["base"]);
    assert_eq!(unrelated["value"], "missing");

    let reinstalled = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "--reinstall", "first.py"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {"overlay": "[CACHE_DIR]/environments-v2/shared-first-[HASH]", "base": "[CACHE_DIR]/archive-v0/[HASH]", "payload": "[CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/shared_repair/payload.txt", "value": "pristine"}

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-repair==1.0.0
    "#);
    let reinstalled: serde_json::Value = serde_json::from_slice(&reinstalled.stdout)?;
    assert_eq!(reinstalled["value"], "pristine");
    assert_eq!(reinstalled["overlay"], first["overlay"]);
    assert_ne!(reinstalled["base"], first["base"]);
    assert_eq!(context.read(&second_config), second_config_before);
    context
        .temp_dir
        .child(old_base)
        .assert(predicate::path::is_dir());
    context
        .temp_dir
        .child(old_payload)
        .assert(predicate::path::missing());
    Ok(())
}

/// A targeted reinstall repairs declared dependencies hidden by an overlay installation.
#[test]
fn run_pep723_shared_reinstall_package_removes_overlay_collision() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (shared_name, shared) = generate_wheel_with_files(
        &"shared-repair".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("shared_repair/payload.txt", "pristine\n")],
    );
    wheels.child(shared_name).write_binary(&shared)?;
    let (overlay_name, overlay) = generate_wheel_with_files(
        &"overlay-only".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels.child(overlay_name).write_binary(&overlay)?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-repair==1.0.0"]
        # ///
        import json
        from importlib.util import find_spec
        from pathlib import Path
        import shared_repair
        import sys
        module = Path(shared_repair.__file__)
        base = next(parent for parent in module.parents if parent.joinpath("pyvenv.cfg").is_file())
        payload = module.with_name("payload.txt")
        print(json.dumps({"overlay": Path(sys.prefix).as_posix(), "base": base.as_posix(), "payload": payload.as_posix(), "value": payload.read_text().strip() if payload.is_file() else "missing", "overlay_only": find_spec("overlay_only") is not None}))
    "#})?;
    let first = context
        .run()
        .args([
            "--preview-features",
            "shared-script-environments",
            "--no-index",
            "--find-links",
            "wheels",
            "script.py",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let first: serde_json::Value = serde_json::from_slice(&first.stdout)?;
    assert_eq!(first["value"], "pristine");
    let overlay_root = first["overlay"].as_str().context("missing overlay")?;
    let python =
        venv_bin_path(overlay_root).join(format!("python{}", std::env::consts::EXE_SUFFIX));
    context
        .pip_install()
        .arg("--python")
        .arg(&python)
        .args([
            "--reinstall",
            "--link-mode",
            "copy",
            "--no-index",
            "--find-links",
            "wheels",
            "shared-repair==1.0.0",
            "overlay-only==1.0.0",
        ])
        .assert()
        .success();
    let installed = context
        .run()
        .args([
            "--preview-features",
            "shared-script-environments",
            "--no-index",
            "--find-links",
            "wheels",
            "script.py",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let installed: serde_json::Value = serde_json::from_slice(&installed.stdout)?;
    assert_eq!(installed["base"], first["overlay"]);
    let overlay_payload = Path::new(
        installed["payload"]
            .as_str()
            .context("missing overlay payload")?,
    );
    fs_err::write(overlay_payload, "overlay\n")?;
    let old_payload = Path::new(
        first["payload"]
            .as_str()
            .context("missing shared payload")?,
    );
    fs_err::remove_file(old_payload)?;

    let reinstalled = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "--reinstall-package", "shared-repair", "script.py"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {"overlay": "[CACHE_DIR]/environments-v2/shared-script-[HASH]", "base": "[CACHE_DIR]/archive-v0/[HASH]", "payload": "[CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/shared_repair/payload.txt", "value": "pristine", "overlay_only": true}

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-repair==1.0.0
    Uninstalled 1 package in [TIME]
     - shared-repair==1.0.0
    "#);
    let reinstalled: serde_json::Value = serde_json::from_slice(&reinstalled.stdout)?;
    assert_eq!(reinstalled["value"], "pristine");
    assert_eq!(reinstalled["overlay"], first["overlay"]);
    assert_ne!(reinstalled["base"], first["base"]);
    assert_ne!(reinstalled["base"], first["overlay"]);
    assert_eq!(reinstalled["overlay_only"], true);
    context
        .temp_dir
        .child(overlay_payload)
        .assert(predicate::path::missing());
    context
        .temp_dir
        .child(old_payload)
        .assert(predicate::path::missing());
    Ok(())
}

/// Exact synchronization removes undeclared packages from a shared script's writable overlay.
#[test]
fn run_pep723_shared_exact_removes_overlay_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (filename, wheel) = generate_wheel_with_files(
        &"overlay-only".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels.child(filename).write_binary(&wheel)?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # ///
        from importlib.util import find_spec
        import sys
        print(sys.prefix)
        print(find_spec("overlay_only") is not None)
    "#})?;
    let first_run = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    False

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");
    let first_stdout = std::str::from_utf8(&first_run.stdout)?;
    let overlay_root = first_stdout
        .lines()
        .next()
        .context("script did not report its overlay environment")?;
    context
        .pip_install()
        .arg("--python")
        .arg(venv_bin_path(overlay_root).join(format!("python{}", std::env::consts::EXE_SUFFIX)))
        .args([
            "--no-index",
            "--find-links",
            "wheels",
            "overlay-only==1.0.0",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    True

    ----- stderr -----
    Resolved in [TIME]
    ");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--exact", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    False

    ----- stderr -----
    Resolved in [TIME]
    Uninstalled 1 package in [TIME]
     - overlay-only==1.0.0
    ");
    Ok(())
}

/// Exact overlay synchronization leaves declared packages available from the immutable shared base.
#[test]
fn run_pep723_shared_exact_retains_declared_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (shared_name, shared) = generate_wheel_with_files(
        &"shared-dep".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("shared_dep/value.py", "VALUE = 'shared'\n")],
    );
    wheels.child(shared_name).write_binary(&shared)?;
    let (overlay_name, overlay) = generate_wheel_with_files(
        &"overlay-only".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels.child(overlay_name).write_binary(&overlay)?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-dep==1.0.0"]
        # ///
        from importlib.util import find_spec
        from shared_dep.value import VALUE
        import sys
        print(sys.prefix)
        print(VALUE)
        print(find_spec("overlay_only") is not None)
    "#})?;
    let first_run = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    shared
    False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-dep==1.0.0
    ");
    let first_stdout = std::str::from_utf8(&first_run.stdout)?;
    let overlay_root = first_stdout
        .lines()
        .next()
        .context("script did not report its overlay environment")?;
    context
        .pip_install()
        .arg("--python")
        .arg(venv_bin_path(overlay_root).join(format!("python{}", std::env::consts::EXE_SUFFIX)))
        .args([
            "--no-index",
            "--find-links",
            "wheels",
            "overlay-only==1.0.0",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--exact", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    shared
    False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Uninstalled 1 package in [TIME]
     - overlay-only==1.0.0
    ");
    Ok(())
}

/// Shared dependency entrypoints run in the writable overlay and track changed dependencies.
#[test]
fn run_pep723_shared_entrypoints_follow_dependency_changes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let cli = indoc! {r#"
        def main():
            from importlib.metadata import version
            import overlay_value
            import sys
            print(version("shared-cli"), overlay_value.VALUE)
            print(sys.prefix)
    "#};
    let (first_name, first) = generate_wheel_with_files(
        &"shared-cli".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_cli/cli.py", cli),
            (
                "shared_cli-1.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nuv-shared-command = shared_cli.cli:main\nuv-shared-old = shared_cli.cli:main\n",
            ),
        ],
    );
    let (second_name, second) = generate_wheel_with_files(
        &"shared-cli".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_cli/cli.py", cli),
            (
                "shared_cli-2.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nuv-shared-command = shared_cli.cli:main\n",
            ),
        ],
    );
    wheels.child(first_name).write_binary(&first)?;
    wheels.child(second_name).write_binary(&second)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-cli==1.0.0"]
        # ///
        from pathlib import Path
        import shutil
        import subprocess
        import sysconfig
        Path(sysconfig.get_path("purelib"), "overlay_value.py").write_text("VALUE = 'overlay'\n")
        subprocess.run(["uv-shared-command"], check=True)
        print(shutil.which("uv-shared-old") is None)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels"]).arg("script.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0 overlay
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-cli==1.0.0
    ");
    script.write_str(
        &fs_err::read_to_string(script.path())?.replace("shared-cli==1.0.0", "shared-cli==2.0.0"),
    )?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels"]).arg("script.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    2.0.0 overlay
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    True

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-cli==2.0.0
    ");
    Ok(())
}

/// Non-Python commands remain available behind commands installed in the writable overlay.
#[test]
#[cfg(unix)]
fn run_pep723_shared_native_entrypoints() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-native".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[(
            "shared_native-1.0.0.data/scripts/uv-shared-native",
            "#!/bin/sh\nprintf 'shared\\n'\n",
        )],
    );
    wheels.child(filename).write_binary(&wheel)?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-native==1.0.0"]
        # ///
        import subprocess
        import sys
        subprocess.run(["uv-shared-native"], check=True)
        print(sys.prefix)
    "#})?;
    let first_run = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels"])
        .arg("script.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    shared
    [CACHE_DIR]/environments-v2/shared-script-[HASH]

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-native==1.0.0
    ");
    let first_stdout = std::str::from_utf8(&first_run.stdout)?;
    let overlay_root = first_stdout
        .lines()
        .nth(1)
        .context("script did not report its overlay environment")?;
    let overlay_command = venv_bin_path(overlay_root).join("uv-shared-native");
    fs_err::write(&overlay_command, "#!/bin/sh\nprintf 'overlay\\n'\n")?;
    fs_err::set_permissions(&overlay_command, std::fs::Permissions::from_mode(0o755))?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels"])
        .arg("script.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    overlay
    [CACHE_DIR]/environments-v2/shared-script-[HASH]

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    Ok(())
}

/// Immutable cache hits leave their configuration contents and modification time untouched.
#[test]
fn run_cached_environment_configuration_is_stable() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_pyvenv_cfg_filters();
    let scenario = toml::from_str(indoc! {r#"
        name = "immutable-configuration"
        [root]
        requires = ["example"]
        [expected]
        satisfiable = true
        [packages.example.versions."1.0.0"]
        sdist = false
    "#})?;
    let server = PackseServer::from_scenario(&scenario);
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        from pathlib import Path
        import example
        for parent in Path(example.__file__).parents:
            configuration = parent / "pyvenv.cfg"
            if configuration.is_file():
                print(configuration)
                break
    "#})?;
    let output = context
        .run()
        .args(["--with", "example==1.0.0"])
        .arg("--index-url")
        .arg(server.index_url())
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("script.py")
        .output()?
        .assert()
        .success();
    let path = String::from_utf8(output.get_output().stdout.clone())?;
    let path = Path::new(path.trim());
    let contents = fs_err::read_to_string(path)?;
    let original_time = filetime::FileTime::from_unix_time(1_700_000_000, 0);
    filetime::set_file_mtime(path, original_time)?;
    context
        .run()
        .args(["--with", "example==1.0.0", "--offline"])
        .arg("--index-url")
        .arg(server.index_url())
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("script.py")
        .assert()
        .success();
    assert_eq!(
        filetime::FileTime::from_last_modification_time(&fs_err::metadata(path)?),
        original_time
    );
    assert_eq!(fs_err::read_to_string(path)?, contents);

    Ok(())
}

/// Shared script dependencies satisfy `--with` and remain preferences when extra packages are needed.
#[test]
fn run_pep723_shared_script_with_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["iniconfig<2"]
        # ///
        from importlib.metadata import version
        print(version("iniconfig"))
    "#})?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "example"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--with", "iniconfig", "script.py"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    1.1.1

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==1.1.1
    "#);
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--with", ".", "script.py"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    1.1.1

    ----- stderr -----
    Resolved 1 package in [TIME]
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + example==1.0.0 (from file://[TEMP_DIR]/)
     + iniconfig==1.1.1
    "#);
    Ok(())
}

/// Overlay-installed commands retain precedence when the shared dependency environment changes.
#[test]
fn run_pep723_shared_entrypoints_preserve_overlay_installations() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let cli = indoc! {r#"
        def main():
            from importlib.metadata import version
            import overlay_value
            import sys
            print(version("shared-cli"), overlay_value.VALUE)
            print(sys.prefix)
    "#};
    let (first_name, first) = generate_wheel_with_files(
        &"shared-cli".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_cli/cli.py", cli),
            (
                "shared_cli-1.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nuv-shared-command = shared_cli.cli:main\nuv-shared-old = shared_cli.cli:main\n",
            ),
        ],
    );
    let (second_name, second) = generate_wheel_with_files(
        &"shared-cli".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            (
                "shared_cli/cli.py",
                "def replacement():\n    print('shared replacement')\n",
            ),
            (
                "shared_cli-2.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nuv-shared-command = shared_cli.cli:replacement\n",
            ),
        ],
    );
    wheels.child(first_name).write_binary(&first)?;
    wheels.child(second_name).write_binary(&second)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-cli==1.0.0"]
        # ///
        from pathlib import Path
        import shutil
        import subprocess
        import sysconfig
        Path(sysconfig.get_path("purelib"), "overlay_value.py").write_text("VALUE = 'overlay'\n")
        subprocess.run(["uv-shared-command"], check=True)
        print(shutil.which("uv-shared-old") is None)
    "#})?;
    let first_run = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels"]).arg("script.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0 overlay
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-cli==1.0.0
    ");
    let first_stdout = std::str::from_utf8(&first_run.stdout)?;
    let overlay_root = first_stdout
        .lines()
        .nth(1)
        .context("shared command did not report its overlay environment")?;
    context
        .pip_install()
        .arg("--python")
        .arg(venv_bin_path(overlay_root).join(format!("python{}", std::env::consts::EXE_SUFFIX)))
        .args(["--no-index", "--find-links", "wheels", "shared-cli==1.0.0"])
        .assert()
        .success();
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-cli==1.0.0", "shared-cli==2.0.0"),
    )?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels"]).arg("script.py"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    1.0.0 overlay
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-cli==2.0.0
    ");
    Ok(())
}

/// Shared data files are refreshed and removed when the dependency environment changes.
#[test]
fn run_pep723_shared_data_follows_dependency_changes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (first_name, first) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-1.0.0.data/data/etc/jupyter/config.txt",
                "one\n",
            ),
            (
                "shared_data-1.0.0.data/data/share/jupyter/value.txt",
                "one\n",
            ),
        ],
    );
    wheels.child(first_name).write_binary(&first)?;
    let (second_name, second) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-2.0.0.data/data/etc/jupyter/config.txt",
                "two\n",
            ),
            (
                "shared_data-2.0.0.data/data/share/jupyter/value.txt",
                "two\n",
            ),
        ],
    );
    wheels.child(second_name).write_binary(&second)?;
    let (third_name, third) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"3.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-3.0.0.data/data/etc/jupyter/config.txt",
                "three\n",
            ),
        ],
    );
    wheels.child(third_name).write_binary(&third)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-data==1.0.0"]
        # ///
        from pathlib import Path
        import sys
        configuration = Path(sys.prefix, "etc/jupyter/config.txt")
        data = Path(sys.prefix, "share/jupyter/value.txt")
        print(configuration.read_text().strip())
        print(data.read_text().strip() if data.exists() else "missing")
        print(configuration.parent.is_symlink(), data.parent.is_symlink())
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    one
    one
    False False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==1.0.0
    ");
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-data==1.0.0", "shared-data==2.0.0"),
    )?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    two
    two
    False False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==2.0.0
    ");
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-data==2.0.0", "shared-data==3.0.0"),
    )?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    three
    missing
    False False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==3.0.0
    ");
    Ok(())
}

/// Recover command and data ownership when a later shared data copy fails.
#[test]
#[cfg(unix)]
fn run_pep723_shared_data_recovers_after_copy_failure() -> Result<()> {
    use uv_test::ReadOnlyDirectoryGuard;

    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (first_name, first) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-1.0.0.data/data/share/jupyter/value.txt",
                "one\n",
            ),
            (
                "shared_data-1.0.0.data/data/share/jupyter/edited.txt",
                "original one\n",
            ),
        ],
    );
    wheels.child(first_name).write_binary(&first)?;
    let (second_name, second) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            (
                "shared_data/__init__.py",
                "def main():\n    print('shared command')\n",
            ),
            (
                "shared_data-2.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nuv-shared-transient = shared_data:main\n",
            ),
            (
                "shared_data-2.0.0.data/data/share/jupyter/value.txt",
                "two\n",
            ),
            (
                "shared_data-2.0.0.data/data/share/jupyter/edited.txt",
                "original two\n",
            ),
            (
                "shared_data-2.0.0.data/data/share/jupyter/obsolete.txt",
                "version two only\n",
            ),
            (
                "shared_data-2.0.0.data/data/etc/jupyter/later.txt",
                "later two\n",
            ),
        ],
    );
    wheels.child(second_name).write_binary(&second)?;
    let (third_name, third) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"3.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-3.0.0.data/data/share/jupyter/value.txt",
                "three\n",
            ),
            (
                "shared_data-3.0.0.data/data/share/jupyter/edited.txt",
                "original three\n",
            ),
            (
                "shared_data-3.0.0.data/data/etc/jupyter/later.txt",
                "later three\n",
            ),
        ],
    );
    wheels.child(third_name).write_binary(&third)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-data==1.0.0"]
        # ///
        from pathlib import Path
        import sys
        Path("overlay-root").write_text(sys.prefix)
        root = Path(sys.prefix)
        print(root.joinpath("share/jupyter/value.txt").read_text().strip())
        print(root.joinpath("share/jupyter/edited.txt").read_text().strip())
        unrelated = root.joinpath("share/jupyter/unrelated.txt")
        obsolete = root.joinpath("share/jupyter/obsolete.txt")
        later = root.joinpath("etc/jupyter/later.txt")
        print(unrelated.read_text().strip() if unrelated.exists() else "missing")
        print(obsolete.read_text().strip() if obsolete.exists() else "missing")
        print(later.read_text().strip() if later.exists() else "missing")
    "#})?;
    uv_snapshot!(context.filters(), context.run().args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    one
    original one
    missing
    missing
    missing

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==1.0.0
    ");
    let overlay_root = context.read("overlay-root");
    let overlay = Path::new(&overlay_root);
    fs_err::write(overlay.join("share/jupyter/edited.txt"), "local edit\n")?;
    fs_err::write(
        overlay.join("share/jupyter/unrelated.txt"),
        "unrelated file\n",
    )?;
    fs_err::create_dir_all(overlay.join("etc/jupyter"))?;
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-data==1.0.0", "shared-data==2.0.0"),
    )?;
    let failed = {
        let _readonly = ReadOnlyDirectoryGuard::new(overlay.join("etc/jupyter"))?;
        uv_snapshot!(context.filters(), context.run().args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r#"
        exit_code: 2 (failure)
        ----- stderr -----
        Resolved 1 package in [TIME]
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + shared-data==2.0.0
        error: Permission denied (os error 13) at path "[CACHE_DIR]/environments-v2/shared-script-[HASH]/etc/jupyter/[TMP]"
        "#)
    };
    let stderr = std::str::from_utf8(&failed.stderr)?;
    assert!(stderr.contains(&format!("{}/.tmp", overlay.join("etc/jupyter").display())));
    assert_eq!(
        context.read(overlay.join("share/jupyter/value.txt")),
        "two\n"
    );
    assert_eq!(
        context.read(overlay.join("share/jupyter/obsolete.txt")),
        "version two only\n"
    );
    ChildPath::new(overlay.join("bin/uv-shared-transient")).assert(predicate::path::is_file());
    uv_snapshot!(context.filters(), context.run().args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    two
    local edit
    unrelated file
    version two only
    later two

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-data==2.0.0", "shared-data==3.0.0"),
    )?;
    uv_snapshot!(context.filters(), context.run().args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    three
    local edit
    unrelated file
    missing
    later three

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==3.0.0
    ");
    assert_eq!(
        context.read(overlay.join("share/jupyter/value.txt")),
        "three\n"
    );
    assert_eq!(
        context.read(overlay.join("share/jupyter/edited.txt")),
        "local edit\n"
    );
    assert_eq!(
        context.read(overlay.join("share/jupyter/unrelated.txt")),
        "unrelated file\n"
    );
    ChildPath::new(overlay.join("share/jupyter/obsolete.txt")).assert(predicate::path::missing());
    ChildPath::new(overlay.join("bin/uv-shared-transient")).assert(predicate::path::missing());
    Ok(())
}

/// Recover ownership when the environment manifest cannot be persisted.
#[test]
#[cfg(unix)]
fn run_pep723_shared_data_recovers_after_manifest_failure() -> Result<()> {
    use uv_test::ReadOnlyDirectoryGuard;

    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (first_name, first) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-1.0.0.data/data/share/jupyter/value.txt",
                "one\n",
            ),
            (
                "shared_data-1.0.0.data/data/share/jupyter/edited.txt",
                "original one\n",
            ),
        ],
    );
    wheels.child(first_name).write_binary(&first)?;
    let (second_name, second) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-2.0.0.data/data/share/jupyter/value.txt",
                "two\n",
            ),
            (
                "shared_data-2.0.0.data/data/share/jupyter/edited.txt",
                "original two\n",
            ),
            (
                "shared_data-2.0.0.data/data/share/jupyter/obsolete.txt",
                "version two only\n",
            ),
            (
                "shared_data-2.0.0.data/data/etc/jupyter/later.txt",
                "later two\n",
            ),
        ],
    );
    wheels.child(second_name).write_binary(&second)?;
    let (third_name, third) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"3.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-3.0.0.data/data/share/jupyter/value.txt",
                "three\n",
            ),
            (
                "shared_data-3.0.0.data/data/share/jupyter/edited.txt",
                "original three\n",
            ),
            (
                "shared_data-3.0.0.data/data/etc/jupyter/later.txt",
                "later three\n",
            ),
        ],
    );
    wheels.child(third_name).write_binary(&third)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-data==1.0.0"]
        # ///
        from pathlib import Path
        import sys
        Path("overlay-root").write_text(sys.prefix)
        root = Path(sys.prefix)
        print(root.joinpath("share/jupyter/value.txt").read_text().strip())
        print(root.joinpath("share/jupyter/edited.txt").read_text().strip())
        unrelated = root.joinpath("share/jupyter/unrelated.txt")
        obsolete = root.joinpath("share/jupyter/obsolete.txt")
        later = root.joinpath("etc/jupyter/later.txt")
        print(unrelated.read_text().strip() if unrelated.exists() else "missing")
        print(obsolete.read_text().strip() if obsolete.exists() else "missing")
        print(later.read_text().strip() if later.exists() else "missing")
    "#})?;
    uv_snapshot!(context.filters(), context.run().args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    one
    original one
    missing
    missing
    missing

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==1.0.0
    ");
    let overlay_root = context.read("overlay-root");
    let overlay = Path::new(&overlay_root);
    fs_err::write(overlay.join("share/jupyter/edited.txt"), "local edit\n")?;
    fs_err::write(
        overlay.join("share/jupyter/unrelated.txt"),
        "unrelated file\n",
    )?;
    fs_err::create_dir_all(overlay.join("etc/jupyter"))?;
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-data==1.0.0", "shared-data==2.0.0"),
    )?;
    let failed = {
        let _readonly = ReadOnlyDirectoryGuard::new(overlay.to_path_buf())?;
        uv_snapshot!(context.filters(), context.run().args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r#"
        exit_code: 2 (failure)
        ----- stderr -----
        Resolved 1 package in [TIME]
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + shared-data==2.0.0
        error: Permission denied (os error 13) at path "[CACHE_DIR]/environments-v2/shared-script-[HASH]/[TMP]"
        "#)
    };
    let stderr = std::str::from_utf8(&failed.stderr)?;
    assert!(stderr.contains(&format!("{}/.tmp", overlay.to_path_buf().display())));
    uv_snapshot!(context.filters(), context.run().args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    two
    local edit
    unrelated file
    version two only
    later two

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-data==2.0.0", "shared-data==3.0.0"),
    )?;
    uv_snapshot!(context.filters(), context.run().args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    three
    local edit
    unrelated file
    missing
    later three

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==3.0.0
    ");
    assert_eq!(
        context.read(overlay.join("share/jupyter/value.txt")),
        "three\n"
    );
    assert_eq!(
        context.read(overlay.join("share/jupyter/edited.txt")),
        "local edit\n"
    );
    assert_eq!(
        context.read(overlay.join("share/jupyter/unrelated.txt")),
        "unrelated file\n"
    );
    ChildPath::new(overlay.join("share/jupyter/obsolete.txt")).assert(predicate::path::missing());
    Ok(())
}

/// Overlay data installations retain precedence without modifying the immutable shared data.
#[test]
fn run_pep723_shared_data_preserves_overlay_installations() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (first_name, first) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-1.0.0.data/data/share/jupyter/value.txt",
                "one\n",
            ),
        ],
    );
    wheels.child(first_name).write_binary(&first)?;
    let (second_name, second) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-2.0.0.data/data/share/jupyter/value.txt",
                "two\n",
            ),
        ],
    );
    wheels.child(second_name).write_binary(&second)?;
    let (overlay_name, overlay) = generate_wheel_with_files(
        &"overlay-data".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("overlay_data/__init__.py", ""),
            (
                "overlay_data-1.0.0.data/data/share/jupyter/value.txt",
                "overlay\n",
            ),
        ],
    );
    wheels.child(overlay_name).write_binary(&overlay)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-data==1.0.0"]
        # ///
        from pathlib import Path
        import shared_data
        import sys
        print(sys.prefix)
        print(next(parent for parent in Path(shared_data.__file__).parents if parent.joinpath("pyvenv.cfg").is_file()))
        print(Path(sys.prefix, "share/jupyter/value.txt").read_text().strip())
    "#})?;
    let first_run = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]
    one

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==1.0.0
    ");
    let first_stdout = std::str::from_utf8(&first_run.stdout)?;
    let mut first_lines = first_stdout.lines();
    let overlay_root = first_lines
        .next()
        .context("script did not report its overlay")?;
    let first_base = first_lines
        .next()
        .context("script did not report its shared base")?;
    let python =
        venv_bin_path(overlay_root).join(format!("python{}", std::env::consts::EXE_SUFFIX));
    context
        .pip_install()
        .arg("--python")
        .arg(&python)
        .args([
            "--no-index",
            "--find-links",
            "wheels",
            "overlay-data==1.0.0",
        ])
        .assert()
        .success();
    assert_eq!(
        context.read(Path::new(first_base).join("share/jupyter/value.txt")),
        "one\n"
    );
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-data==1.0.0", "shared-data==2.0.0"),
    )?;
    let second_run = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]
    overlay

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==2.0.0
    ");
    let second_stdout = std::str::from_utf8(&second_run.stdout)?;
    let second_base = second_stdout
        .lines()
        .nth(1)
        .context("script did not report its shared base")?;
    assert_eq!(
        context.read(Path::new(second_base).join("share/jupyter/value.txt")),
        "two\n"
    );
    context
        .pip_uninstall()
        .arg("--python")
        .arg(&python)
        .arg("overlay-data")
        .assert()
        .success();
    assert_eq!(
        context.read(Path::new(second_base).join("share/jupyter/value.txt")),
        "two\n"
    );
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]
    two

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    Ok(())
}

/// Installing and uninstalling between runs must not leave managed shared data missing.
#[test]
fn run_pep723_shared_data_restores_removed_overlay_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (shared_name, shared) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-1.0.0.data/data/etc/jupyter/config.txt",
                "shared configuration\n",
            ),
            (
                "shared_data-1.0.0.data/data/share/jupyter/value.txt",
                "shared data\n",
            ),
        ],
    );
    wheels.child(shared_name).write_binary(&shared)?;
    let (overlay_name, overlay) = generate_wheel_with_files(
        &"overlay-data".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("overlay_data/__init__.py", ""),
            (
                "overlay_data-1.0.0.data/data/share/jupyter/value.txt",
                "overlay data\n",
            ),
        ],
    );
    wheels.child(overlay_name).write_binary(&overlay)?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-data==1.0.0"]
        # ///
        from pathlib import Path
        import shared_data
        import sys
        print(sys.prefix)
        print(next(parent for parent in Path(shared_data.__file__).parents if parent.joinpath("pyvenv.cfg").is_file()))
        print(Path(sys.prefix, "share/jupyter/value.txt").read_text().strip())
        print(Path(sys.prefix, "etc/jupyter/config.txt").read_text().strip())
    "#})?;
    let first_run = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]
    shared data
    shared configuration

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==1.0.0
    ");
    let first_stdout = std::str::from_utf8(&first_run.stdout)?;
    let mut lines = first_stdout.lines();
    let overlay_root = lines.next().context("script did not report its overlay")?;
    let shared_root = lines
        .next()
        .context("script did not report its shared base")?;
    let python =
        venv_bin_path(overlay_root).join(format!("python{}", std::env::consts::EXE_SUFFIX));
    fs_err::write(
        Path::new(overlay_root).join("etc/jupyter/config.txt"),
        "local configuration\n",
    )?;
    context
        .pip_install()
        .arg("--python")
        .arg(&python)
        .args([
            "--no-index",
            "--find-links",
            "wheels",
            "overlay-data==1.0.0",
        ])
        .assert()
        .success();
    context
        .pip_uninstall()
        .arg("--python")
        .arg(&python)
        .arg("overlay-data")
        .assert()
        .success();
    context
        .temp_dir
        .child(overlay_root)
        .child("share/jupyter/value.txt")
        .assert(predicate::path::missing());
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    [CACHE_DIR]/archive-v0/[HASH]
    shared data
    local configuration

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert_eq!(
        context.read(Path::new(shared_root).join("share/jupyter/value.txt")),
        "shared data\n",
    );
    assert_eq!(
        context.read(Path::new(shared_root).join("etc/jupyter/config.txt")),
        "shared configuration\n",
    );
    Ok(())
}

/// Local edits to copied shared data survive a dependency environment change.
#[test]
fn run_pep723_shared_data_preserves_local_edits() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (first_name, first) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-1.0.0.data/data/etc/jupyter/config.txt",
                "one\n",
            ),
            (
                "shared_data-1.0.0.data/data/share/jupyter/value.txt",
                "one\n",
            ),
        ],
    );
    wheels.child(first_name).write_binary(&first)?;
    let (second_name, second) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("shared_data/__init__.py", ""),
            (
                "shared_data-2.0.0.data/data/etc/jupyter/config.txt",
                "two\n",
            ),
            (
                "shared_data-2.0.0.data/data/share/jupyter/value.txt",
                "two\n",
            ),
        ],
    );
    wheels.child(second_name).write_binary(&second)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-data==1.0.0"]
        # ///
        from pathlib import Path
        import sys
        configuration = Path(sys.prefix, "etc/jupyter/config.txt")
        data = Path(sys.prefix, "share/jupyter/value.txt")
        print(sys.prefix)
        print(configuration.read_text().strip())
        print(data.read_text().strip() if data.exists() else "missing")
        print(configuration.parent.is_symlink(), data.parent.is_symlink())
    "#})?;
    let first_run = uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    one
    one
    False False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==1.0.0
    ");
    let first_stdout = std::str::from_utf8(&first_run.stdout)?;
    let overlay_root = first_stdout
        .lines()
        .next()
        .context("script did not report its overlay")?;
    fs_err::write(
        Path::new(overlay_root).join("etc/jupyter/config.txt"),
        "local\n",
    )?;
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-data==1.0.0", "shared-data==2.0.0"),
    )?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/environments-v2/shared-script-[HASH]
    local
    two
    False False

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==2.0.0
    ");
    Ok(())
}

/// Copied shared launchers must execute the overlay interpreter when its path contains spaces.
#[test]
#[cfg(unix)]
fn run_pep723_shared_entrypoints_with_spaces() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let cache = context.temp_dir.child("cache with spaces");
    let context = context.with_cache_dir(cache.path());
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-cli".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            (
                "shared_cli/cli.py",
                indoc! {r"
                def main():
                    import overlay_value
                    print(overlay_value.VALUE)
            "},
            ),
            (
                "shared_cli-1.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nuv-shared-command = shared_cli.cli:main\n",
            ),
        ],
    );
    wheels.child(filename).write_binary(&wheel)?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-cli==1.0.0"]
        # ///
        from pathlib import Path
        import subprocess
        import sysconfig
        Path(sysconfig.get_path("purelib"), "overlay_value.py").write_text("VALUE = 'overlay'\n")
        subprocess.run([str(Path(sysconfig.get_path("scripts")) / "uv-shared-command")], check=True)
        subprocess.run(["uv-shared-command"], check=True)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    overlay
    overlay

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-cli==1.0.0
    ");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    overlay
    overlay

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    Ok(())
}

/// A long overlay path needs a shell wrapper even when it contains no spaces.
#[test]
#[cfg(unix)]
fn run_pep723_shared_entrypoints_with_long_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let cache = context
        .temp_dir
        .child("a".repeat(160))
        .child("b".repeat(160))
        .child("c".repeat(160));
    let context = context.with_cache_dir(cache.path());
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (filename, wheel) = generate_wheel_with_files(
        &"shared-cli".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            (
                "shared_cli/cli.py",
                indoc! {r"
                def main():
                    import overlay_value
                    print(overlay_value.VALUE)
            "},
            ),
            (
                "shared_cli-1.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nuv-shared-command = shared_cli.cli:main\n",
            ),
        ],
    );
    wheels.child(filename).write_binary(&wheel)?;
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-cli==1.0.0"]
        # ///
        from pathlib import Path
        import subprocess
        import sysconfig
        Path(sysconfig.get_path("purelib"), "overlay_value.py").write_text("VALUE = 'overlay'\n")
        subprocess.run([str(Path(sysconfig.get_path("scripts")) / "uv-shared-command")], check=True)
        subprocess.run(["uv-shared-command"], check=True)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    overlay
    overlay

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-cli==1.0.0
    ");
    Ok(())
}

/// Non-isolated script builds use dependencies installed in the script environment.
#[test]
fn run_pep723_shared_no_build_isolation() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (backend_name, backend) = generate_wheel_with_files(
        &"fixture-backend".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[(
            "fixture_backend/backend.py",
            indoc! {r#"
            from pathlib import Path
            import shutil
            def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
                name = "example-1.0.0-py3-none-any.whl"
                shutil.copyfile(name, Path(wheel_directory) / name)
                return name
        "#},
        )],
    );
    wheels.child(backend_name).write_binary(&backend)?;
    let source = context.temp_dir.child("example");
    source.create_dir_all()?;
    source.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "example"
        version = "1.0.0"
        [build-system]
        requires = []
        build-backend = "fixture_backend.backend"
    "#})?;
    let (filename, wheel) = generate_wheel_with_files(
        &"example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("example/value.py", "VALUE = 'built'\n")],
    );
    source.child(filename).write_binary(&wheel)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # [tool.uv.sources]
        # example = { path = "example" }
        # ///
        from pathlib import Path
        import sys
        Path("script-python").write_text(sys.executable)
    "#})?;
    context
        .run()
        .args([
            "--preview-features",
            "shared-script-environments",
            "--offline",
            "--no-index",
            "--no-build-isolation",
            "script.py",
        ])
        .assert()
        .success();
    context
        .pip_install()
        .args([
            "--no-index",
            "--offline",
            "--find-links",
            "wheels",
            "fixture-backend",
        ])
        .arg("--python")
        .arg(context.read("script-python"))
        .assert()
        .success();
    script.write_str(&format!(
        "{}\nfrom example.value import VALUE\nprint(VALUE)\n",
        context
            .read("script.py")
            .replace("dependencies = []", "dependencies = [\"example\"]")
    ))?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--no-build-isolation", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    built

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0 (from file://[TEMP_DIR]/example)
    ");
    Ok(())
}

/// A per-package script build policy retains the environment containing its build backend.
#[test]
fn run_pep723_shared_no_build_isolation_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (backend_name, backend) = generate_wheel_with_files(
        &"fixture-backend".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[(
            "fixture_backend/backend.py",
            indoc! {r#"
            from pathlib import Path
            import shutil
            def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
                name = "example-1.0.0-py3-none-any.whl"
                shutil.copyfile(name, Path(wheel_directory) / name)
                return name
        "#},
        )],
    );
    wheels.child(backend_name).write_binary(&backend)?;
    let source = context.temp_dir.child("example");
    source.create_dir_all()?;
    source.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "example"
        version = "1.0.0"
        [build-system]
        requires = []
        build-backend = "fixture_backend.backend"
    "#})?;
    let (filename, wheel) = generate_wheel_with_files(
        &"example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("example/value.py", "VALUE = 'built'\n")],
    );
    source.child(filename).write_binary(&wheel)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # [tool.uv]
        # no-build-isolation-package = ["example"]
        # [tool.uv.sources]
        # example = { path = "example" }
        # ///
        from pathlib import Path
        import sys
        Path("script-python").write_text(sys.executable)
    "#})?;
    context
        .run()
        .args([
            "--preview-features",
            "shared-script-environments",
            "--offline",
            "--no-index",
            "script.py",
        ])
        .assert()
        .success();
    context
        .pip_install()
        .args([
            "--no-index",
            "--offline",
            "--find-links",
            "wheels",
            "fixture-backend",
        ])
        .arg("--python")
        .arg(context.read("script-python"))
        .assert()
        .success();
    script.write_str(&format!(
        "{}\nfrom example.value import VALUE\nprint(VALUE)\n",
        context
            .read("script.py")
            .replace("dependencies = []", "dependencies = [\"example\"]")
    ))?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    built

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0 (from file://[TEMP_DIR]/example)
    ");
    Ok(())
}

/// An explicit build-isolation override can enable sharing despite the script's default policy.
#[test]
fn run_pep723_shared_build_isolation_override() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names()
        .with_filtered_exe_suffix();
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # [tool.uv]
        # no-build-isolation = true
        # ///
        from pathlib import Path
        import sys
        print(Path(sys.prefix, ".uv-shared-entrypoints.json").is_file())
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    False
    ");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--build-isolation", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    True

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");
    Ok(())
}

/// Invalid manifest paths must fail before earlier managed files are removed.
#[test]
fn run_pep723_shared_manifest_validates_before_cleanup() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let (first_name, first) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("shared_data-1.0.0.data/data/etc/jupyter/value.txt", "one\n")],
    );
    wheels.child(first_name).write_binary(&first)?;
    let (second_name, second) = generate_wheel_with_files(
        &"shared-data".parse()?,
        &"2.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    wheels.child(second_name).write_binary(&second)?;
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["shared-data==1.0.0"]
        # ///
        from pathlib import Path
        import sys
        Path("overlay-root").write_text(sys.prefix)
        print(Path(sys.prefix, "etc/jupyter/value.txt").read_text().strip())
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    one

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==1.0.0
    ");
    let overlay = context.read("overlay-root");
    let manifest_path = Path::new(&overlay).join(".uv-shared-entrypoints.json");
    let mut manifest: serde_json::Value = serde_json::from_str(&context.read(&manifest_path))?;
    manifest["data"]["share/jupyter/../invalid.txt"] = json!("invalid");
    fs_err::write(manifest_path, serde_json::to_vec(&manifest)?)?;
    script.write_str(
        &context
            .read("script.py")
            .replace("shared-data==1.0.0", "shared-data==2.0.0"),
    )?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "--find-links", "wheels", "script.py"]), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + shared-data==2.0.0
    error: Invalid shared data path: share/jupyter/../invalid.txt
    ");
    assert_eq!(
        context.read(Path::new(&overlay).join("etc/jupyter/value.txt")),
        "one\n"
    );
    Ok(())
}

/// Shared source installations distinguish global build configuration.
#[test]
fn run_pep723_shared_build_config_settings() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = context.temp_dir.child("example");
    source.create_dir_all()?;
    source.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "example"
        version = "1.0.0"
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    source.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path
        import os
        import shutil

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            flavor = (config_settings or {}).get("flavor", os.environ.get("BUILD_FLAVOR", "first"))
            if isinstance(flavor, list):
                flavor = flavor[0]
            name = "example-1.0.0-py3-none-any.whl"
            shutil.copyfile(Path(flavor) / name, Path(wheel_directory) / name)
            return name
    "#})?;
    let first = source.child("first");
    first.create_dir_all()?;
    let (first_name, first_wheel) = generate_wheel_with_files(
        &"example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("example/value.py", "VALUE = 'first'\n")],
    );
    first.child(first_name).write_binary(&first_wheel)?;
    let second = source.child("second");
    second.create_dir_all()?;
    let (second_name, second_wheel) = generate_wheel_with_files(
        &"example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("example/value.py", "VALUE = 'second'\n")],
    );
    second.child(second_name).write_binary(&second_wheel)?;
    context.temp_dir.child("first.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["example"]
        # [tool.uv.sources]
        # example = { path = "example" }
        # [tool.uv.config-settings]
        # flavor = "first"
        # ///
        from example.value import VALUE
        print(VALUE)
    "#})?;
    context.temp_dir.child("second.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["example"]
        # [tool.uv.sources]
        # example = { path = "example" }
        # [tool.uv.config-settings]
        # flavor = "second"
        # ///
        from example.value import VALUE
        print(VALUE)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "first.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    first

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0 (from file://[TEMP_DIR]/example)
    ");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "second.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    second

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0 (from file://[TEMP_DIR]/example)
    ");
    Ok(())
}

/// Package build configuration takes precedence over global values in shared installations.
#[test]
fn run_pep723_shared_build_package_config_settings() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = context.temp_dir.child("example");
    source.create_dir_all()?;
    source.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "example"
        version = "1.0.0"
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    source.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path
        import os
        import shutil

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            flavor = (config_settings or {}).get("flavor", os.environ.get("BUILD_FLAVOR", "first"))
            if isinstance(flavor, list):
                flavor = flavor[0]
            name = "example-1.0.0-py3-none-any.whl"
            shutil.copyfile(Path(flavor) / name, Path(wheel_directory) / name)
            return name
    "#})?;
    let first = source.child("first");
    first.create_dir_all()?;
    let (first_name, first_wheel) = generate_wheel_with_files(
        &"example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("example/value.py", "VALUE = 'first'\n")],
    );
    first.child(first_name).write_binary(&first_wheel)?;
    let second = source.child("second");
    second.create_dir_all()?;
    let (second_name, second_wheel) = generate_wheel_with_files(
        &"example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("example/value.py", "VALUE = 'second'\n")],
    );
    second.child(second_name).write_binary(&second_wheel)?;
    context.temp_dir.child("first.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["example"]
        # [tool.uv.sources]
        # example = { path = "example" }
        # [tool.uv.config-settings]
        # flavor = "global"
        # [tool.uv.config-settings-package]
        # example = { flavor = "first" }
        # ///
        from example.value import VALUE
        print(VALUE)
    "#})?;
    context.temp_dir.child("second.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["example"]
        # [tool.uv.sources]
        # example = { path = "example" }
        # [tool.uv.config-settings]
        # flavor = "global"
        # [tool.uv.config-settings-package]
        # example = { flavor = "second" }
        # ///
        from example.value import VALUE
        print(VALUE)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "first.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    first

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0 (from file://[TEMP_DIR]/example)
    ");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "second.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    second

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0 (from file://[TEMP_DIR]/example)
    ");
    Ok(())
}

/// Shared source installations distinguish package-specific build environment variables.
#[test]
fn run_pep723_shared_build_variables() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = context.temp_dir.child("example");
    source.create_dir_all()?;
    source.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "example"
        version = "1.0.0"
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    source.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path
        import os
        import shutil

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            flavor = (config_settings or {}).get("flavor", os.environ.get("BUILD_FLAVOR", "first"))
            if isinstance(flavor, list):
                flavor = flavor[0]
            name = "example-1.0.0-py3-none-any.whl"
            shutil.copyfile(Path(flavor) / name, Path(wheel_directory) / name)
            return name
    "#})?;
    let first = source.child("first");
    first.create_dir_all()?;
    let (first_name, first_wheel) = generate_wheel_with_files(
        &"example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("example/value.py", "VALUE = 'first'\n")],
    );
    first.child(first_name).write_binary(&first_wheel)?;
    let second = source.child("second");
    second.create_dir_all()?;
    let (second_name, second_wheel) = generate_wheel_with_files(
        &"example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("example/value.py", "VALUE = 'second'\n")],
    );
    second.child(second_name).write_binary(&second_wheel)?;
    context.temp_dir.child("first.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["example"]
        # [tool.uv.sources]
        # example = { path = "example" }
        # [tool.uv.extra-build-variables]
        # example = { BUILD_FLAVOR = "first" }
        # ///
        from example.value import VALUE
        print(VALUE)
    "#})?;
    context.temp_dir.child("second.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["example"]
        # [tool.uv.sources]
        # example = { path = "example" }
        # [tool.uv.extra-build-variables]
        # example = { BUILD_FLAVOR = "second" }
        # ///
        from example.value import VALUE
        print(VALUE)
    "#})?;
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "first.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    first

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0 (from file://[TEMP_DIR]/example)
    ");
    uv_snapshot!(context.filters(), context.run()
        .args(["--preview-features", "shared-script-environments", "--offline", "--no-index", "second.py"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    second

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0 (from file://[TEMP_DIR]/example)
    ");
    Ok(())
}
