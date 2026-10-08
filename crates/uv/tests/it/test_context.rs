use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{env, fs::Permissions, os::unix::fs::PermissionsExt};

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
#[cfg(unix)]
use indoc::indoc;

use uv_fs::is_same_file_allow_missing;
#[cfg(unix)]
use uv_static::EnvVars;
use uv_test::TestContext;

fn assert_base_interpreter(context: &TestContext, expected: &Path) -> Result<()> {
    let assert = context
        .python_command()
        .args([
            "-c",
            "import json, sys; print(json.dumps(sys._base_executable))",
        ])
        .assert()
        .success();
    let executable: PathBuf = serde_json::from_slice(&assert.get_output().stdout)?;
    let executable = fs_err::canonicalize(executable)?;
    let expected = fs_err::canonicalize(expected)?;
    assert_eq!(
        is_same_file_allow_missing(&executable, &expected),
        Some(true)
    );
    assert!(context.site_packages().is_dir());
    Ok(())
}

#[test]
fn venv_uses_retained_default_interpreter() -> Result<()> {
    let mut context = uv_test::test_context_with_versions!(&["3.12", "3.11"]);
    let selected = context.python_versions[0].1.clone();

    context.reset_venv();
    assert_base_interpreter(&context, &selected)?;

    let sentinel = context.venv.child("sentinel.txt");
    sentinel.write_str("removed when resetting the environment")?;

    // Reordering the discovery path does not select a different default for the environment.
    context.python_versions.reverse();
    context.reset_venv();
    assert!(!sentinel.exists());
    assert_base_interpreter(&context, &selected)?;
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
