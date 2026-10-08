#[cfg(unix)]
use std::{env, fs::Permissions, os::unix::fs::PermissionsExt};

use anyhow::Result;
#[cfg(unix)]
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
#[cfg(unix)]
use indoc::indoc;
#[cfg(unix)]
use regex::escape;

#[cfg(unix)]
use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[cfg(unix)]
const SELECTED_PYTHON: &str = "UV_TEST_RETAINED_SELECTED_PYTHON";

#[test]
fn venv_uses_retained_default_interpreter() -> Result<()> {
    let mut context = uv_test::test_context_with_versions!(&["3.12", "3.11"]);
    // The child discovers through the parent's normalized symlink; Python can report its target.
    #[cfg(unix)]
    if let Some(executable) = env::var_os(SELECTED_PYTHON) {
        context = context.with_filter((escape(&executable.to_string_lossy()), "[PYTHON-3.12]"));
    }

    context.reset_venv();
    uv_snapshot!(context.filters(), context.python_command()
        .args(["-c", "import sys; print(sys._base_executable)"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");
    assert!(context.site_packages().is_dir());

    let sentinel = context.venv.child("sentinel.txt");
    sentinel.write_str("removed when resetting the environment")?;

    // Reordering the discovery path does not select a different default for the environment.
    context.python_versions.reverse();
    context.reset_venv();
    assert!(!sentinel.exists());
    uv_snapshot!(context.filters(), context.python_command()
        .args(["-c", "import sys; print(sys._base_executable)"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PYTHON-3.12]
    ");
    assert!(context.site_packages().is_dir());
    Ok(())
}

/// Interpreter discovery is complete before the venv subprocess starts.
#[test]
#[cfg(unix)]
fn venv_does_not_rediscover_selected_interpreter() -> Result<()> {
    const CHILD: &str = "UV_TEST_RETAINED_INTERPRETER_CHILD";
    if env::var_os(CHILD).is_some() {
        return venv_uses_retained_default_interpreter();
    }

    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]);
    let empty = context.temp_dir.child("empty");
    empty.create_dir_all()?;
    let wrapper = context.temp_dir.child("uv-without-discovery");
    wrapper.write_str(indoc! {r#"
        #!/bin/sh
        export UV_PYTHON_INSTALL_DIR="$UV_TEST_EMPTY_PYTHON_DIR"
        export UV_PYTHON_SEARCH_PATH="$UV_TEST_EMPTY_PYTHON_DIR"
        export UV_PYTHON_DOWNLOADS=never
        exec "$UV_TEST_REAL_UV" --no-config "$@"
    "#})?;
    fs_err::set_permissions(wrapper.path(), Permissions::from_mode(0o755))?;

    let assert = context
        .external_command(env::current_exe()?)
        .args([
            "--exact",
            "test_context::venv_does_not_rediscover_selected_interpreter",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(SELECTED_PYTHON, &context.python_versions[0].1)
        .env("UV_TEST_REAL_UV", uv_test::get_bin!())
        .env("UV_TEST_EMPTY_PYTHON_DIR", empty.path())
        .env("CARGO_BIN_EXE_uv", wrapper.path())
        .env_remove("NEXTEST_BIN_EXE_uv")
        .env(EnvVars::CARGO_MANIFEST_DIR, env!("CARGO_MANIFEST_DIR"))
        .env(EnvVars::UV_PYTHON_INSTALL_DIR, empty.path())
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, context.python_path())
        .env(EnvVars::PATH, empty.path())
        .assert()
        .success();
    assert!(String::from_utf8_lossy(&assert.get_output().stdout).contains("1 passed"));
    Ok(())
}
