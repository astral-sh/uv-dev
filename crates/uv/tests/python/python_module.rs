use std::path::{Path, PathBuf};

use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::{FileWriteStr, PathChild};
use fs_err as fs;
use indoc::{formatdoc, indoc};

use uv_fs::Simplified;
use uv_static::EnvVars;

use uv_test::{TestContext, copy_dir_ignore, site_packages_path, uv_snapshot, venv_bin_path};

/// Isolate the user installation and ask the fixture interpreter for its scripts directory.
fn with_user_scheme(context: TestContext) -> anyhow::Result<(TestContext, PathBuf)> {
    let user_base = context.home_dir.join("user-base");
    let context = context.with_env("PYTHONUSERBASE", user_base.as_os_str());
    let output = context
        .python_command()
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_path('scripts', scheme=sysconfig.get_preferred_scheme('user')))",
        ])
        .assert()
        .success()
        .stderr("");
    let scripts = PathBuf::from(
        std::str::from_utf8(&output.get_output().stdout)?.trim_end_matches(['\r', '\n']),
    );
    anyhow::ensure!(
        scripts.starts_with(&user_base),
        "user scripts directory {} is outside {}",
        scripts.display(),
        user_base.display()
    );
    let context = context.with_filtered_path(&scripts, "USER_SCRIPTS");
    Ok((context, scripts))
}

// Keep base-prefix discovery within the test environment instead of picking up a system uv.
// See #15368.
const TEST_SCRIPT: &str = indoc! {r"
    import os
    import sys
    import uv

    sys.base_prefix = os.path.join(sys.prefix, 'missing-base-prefix')
    if os.path.exists(sys.base_prefix):
        raise RuntimeError(f'base prefix unexpectedly exists: {sys.base_prefix}')
    print(uv.find_uv_bin())
"};

/// Copy the current Python sources into a fixture independent of Git's symlink support.
fn fake_uv(context: &TestContext) -> anyhow::Result<PathBuf> {
    let package = context.workspace_root.join("test/packages/fake-uv");
    let destination = context.temp_dir.join("fake-uv");
    fs::create_dir(&destination)?;
    fs::copy(
        package.join("pyproject.toml"),
        destination.join("pyproject.toml"),
    )?;
    copy_dir_ignore(package.join("scripts"), destination.join("scripts"))?;
    copy_dir_ignore(
        context.workspace_root.join("python"),
        destination.join("src"),
    )?;
    Ok(destination)
}

#[test]
fn find_uv_bin_target() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix()
        // Target installs always use "bin" on all platforms. On Windows,
        // `with_filtered_virtualenv_bin` only filters "Scripts", not "bin"
        .with_filter((r"[\\/]bin".to_string(), "/[BIN]".to_string()));

    // Install in a target directory
    uv_snapshot!(context.filters(), context.pip_install()
        .arg(fake_uv(&context)?)
        .arg("--target")
        .arg("target"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv==0.1.0 (from file://[TEMP_DIR]/fake-uv)
    "
    );

    // We should find the binary in the target directory
    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg(TEST_SCRIPT)
        .env(EnvVars::PYTHONPATH, context.temp_dir.child("target").path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/target/[BIN]/uv
    "
    );

    Ok(())
}

#[test]
fn find_uv_bin_prefix() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix()
        // Target installs always use "bin" on all platforms. On Windows,
        // `with_filtered_virtualenv_bin` only filters "Scripts", not "bin"
        .with_filter((r"[\\/]bin".to_string(), "/[BIN]".to_string()));

    // Install in a prefix directory
    let prefix = context.temp_dir.child("prefix");

    uv_snapshot!(context.filters(), context.pip_install()
        .arg(fake_uv(&context)?)
        .arg("--prefix")
        .arg(prefix.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv==0.1.0 (from file://[TEMP_DIR]/fake-uv)
    "
    );

    // We should find the binary in the prefix directory
    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg(TEST_SCRIPT)
        .env(
            EnvVars::PYTHONPATH,
            site_packages_path(&context.temp_dir.join("prefix"), "python3.12"),
        ), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/prefix/[BIN]/uv
    "
    );

    Ok(())
}

#[test]
fn find_uv_bin_base_prefix() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    // Keep the module outside the base prefix so module-relative lookup cannot find this binary.
    let base_prefix = context.temp_dir.join("base-prefix");
    let scripts = venv_bin_path(&base_prefix);
    fs::create_dir_all(&scripts)?;
    let executable = scripts.join(format!("uv{}", std::env::consts::EXE_SUFFIX));
    fs::write(executable, "")?;

    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg("import sys, uv; sys.base_prefix = sys.argv[1]; print(uv.find_uv_bin())")
        .arg(&base_prefix)
        .env(EnvVars::PYTHONPATH, context.workspace_root.join("python")), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/base-prefix/[BIN]/uv
    "
    );

    Ok(())
}

#[test]
fn find_uv_bin_in_ephemeral_environment() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix()
        // Target installs always use "bin" on all platforms. On Windows,
        // `with_filtered_virtualenv_bin` only filters "Scripts", not "bin"
        .with_filter((r"[\\/]bin".to_string(), "/[BIN]".to_string()));

    // Create a minimal pyproject.toml
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "test-project"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = []
        "#
    })?;

    // We should find the binary in an ephemeral `--with` environment
    uv_snapshot!(context.filters(), context.run()
        .arg("--with")
        .arg(fake_uv(&context)?)
        .arg("python")
        .arg("-c")
        .arg(TEST_SCRIPT), @"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/archive-v0/[HASH]/[BIN]/uv

    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv==0.1.0 (from file://[TEMP_DIR]/fake-uv)
    "
    );

    Ok(())
}

#[test]
fn find_uv_bin_in_parent_of_ephemeral_environment() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix()
        // Target installs always use "bin" on all platforms. On Windows,
        // `with_filtered_virtualenv_bin` only filters "Scripts", not "bin"
        .with_filter((r"[\\/]bin".to_string(), "/[BIN]".to_string()));

    // Add the fake-uv package as a dependency
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! { r#"
        [project]
        name = "test-project"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["uv"]

        [tool.uv.sources]
        uv = {{ path = "{}" }}
        "#,
        fake_uv(&context)?.portable_display()
    })?;

    // Import the package from the ephemeral environment to verify it is active while finding
    // the binary in the project environment.
    uv_snapshot!(context.filters(), context.run()
        .arg("--with")
        .arg(context.workspace_root.join("test/links/ok-1.0.0-py3-none-any.whl"))
        .arg("python")
        .arg("-c")
        .arg(format!("import ok\n{TEST_SCRIPT}")),
     @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/uv

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv==0.1.0 (from file://[TEMP_DIR]/fake-uv)
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0 (from file://[WORKSPACE]/test/links/ok-1.0.0-py3-none-any.whl)
    "
    );

    Ok(())
}

#[test]
fn find_uv_bin_user_bin() -> anyhow::Result<()> {
    let (context, scripts) = with_user_scheme(
        uv_test::test_context!("3.12")
            .with_filtered_python_names()
            .with_filtered_virtualenv_bin()
            .with_filtered_exe_suffix(),
    )?;

    let executable = format!("uv{}", std::env::consts::EXE_SUFFIX);
    fs::create_dir_all(&scripts)?;
    fs::write(scripts.join(&executable), "")?;

    // Install in a virtual environment
    uv_snapshot!(context.filters(), context.pip_install()
        .arg(fake_uv(&context)?), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv==0.1.0 (from file://[TEMP_DIR]/fake-uv)
    "
    );

    // We should find the binary in the virtual environment first
    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg(TEST_SCRIPT), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/[BIN]/uv
    "
    );

    // Remove the virtual environment executable to exercise the user-scheme fallback.
    fs::remove_file(venv_bin_path(&context.venv).join(&executable))?;

    // We should find the binary in the user scripts directory now.
    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg(TEST_SCRIPT), @"
    exit_code: 0 (success)
    ----- stdout -----
    [USER_SCRIPTS]/uv
    "
    );

    Ok(())
}

#[test]
fn find_uv_bin_error_message() -> anyhow::Result<()> {
    let (context, _) = with_user_scheme(
        uv_test::test_context!("3.12")
            .with_filtered_python_names()
            .with_filtered_virtualenv_bin()
            .with_filtered_exe_suffix()
            // Target installs always use "bin" on all platforms. On Windows,
            // `with_filtered_virtualenv_bin` only filters "Scripts", not "bin".
            .with_filter((r"[\\/]bin".to_string(), "/[BIN]".to_string())),
    )?;

    // Install in a virtual environment
    uv_snapshot!(context.filters(), context.pip_install()
        .arg(fake_uv(&context)?), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv==0.1.0 (from file://[TEMP_DIR]/fake-uv)
    "
    );

    // Remove the fixture's executable to inspect the attempted search paths.
    fs::remove_file(
        venv_bin_path(&context.venv).join(format!("uv{}", std::env::consts::EXE_SUFFIX)),
    )?;

    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg(TEST_SCRIPT), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Traceback (most recent call last):
      File "<string>", line 8, in <module>
      File "[SITE_PACKAGES]/uv/_find_uv.py", line 50, in find_uv_bin
        raise UvNotFound(
    uv._find_uv.UvNotFound: Could not find the uv binary in any of the following locations:
     - [VENV]/[BIN]
     - [VENV]/missing-base-prefix/[BIN]
     - [SITE_PACKAGES]/[BIN]
     - [USER_SCRIPTS]/
    "#
    );

    Ok(())
}

/// Exercise venv-installed binary discovery with the requested Python version.
fn find_uv_bin_in_venv(python_version: &str) -> anyhow::Result<()> {
    let context = uv_test::test_context!(python_version);
    context
        .pip_install()
        .arg(fake_uv(&context)?)
        .assert()
        .success();

    let output = context
        .python_command()
        .args(["-c", TEST_SCRIPT])
        .assert()
        .success()
        .stderr("");
    let stdout = std::str::from_utf8(&output.get_output().stdout)?;
    let actual = Path::new(
        stdout
            .strip_suffix("\r\n")
            .or_else(|| stdout.strip_suffix('\n'))
            .expect("the discovered executable path should end with a newline"),
    );
    assert!(
        actual.is_absolute(),
        "expected an absolute path: {actual:?}"
    );
    let expected = venv_bin_path(&context.venv).join(format!("uv{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(fs::canonicalize(actual)?, fs::canonicalize(expected)?);

    Ok(())
}

#[cfg(feature = "test-python-eol")]
#[test]
fn find_uv_bin_py38() -> anyhow::Result<()> {
    find_uv_bin_in_venv("3.8")
}

#[test]
fn find_uv_bin_py39() -> anyhow::Result<()> {
    find_uv_bin_in_venv("3.9")
}

#[test]
fn find_uv_bin_py310() -> anyhow::Result<()> {
    find_uv_bin_in_venv("3.10")
}

#[test]
fn find_uv_bin_py311() -> anyhow::Result<()> {
    find_uv_bin_in_venv("3.11")
}

#[test]
fn find_uv_bin_py312() -> anyhow::Result<()> {
    find_uv_bin_in_venv("3.12")
}

#[test]
fn find_uv_bin_py313() -> anyhow::Result<()> {
    find_uv_bin_in_venv("3.13")
}

#[test]
fn find_uv_bin_py314() -> anyhow::Result<()> {
    find_uv_bin_in_venv("3.14")
}
