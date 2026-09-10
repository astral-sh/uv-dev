use std::path::Path;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;
use predicates::prelude::*;
use serde_json::{Value, json};

use uv_cache::{Cache, CacheBucket};
use uv_fs::Simplified;
use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

fn write_script(context: &TestContext) -> Result<()> {
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # ///
    "#})?;
    Ok(())
}

/// An isolated script does not need to warn about an unrelated active environment.
#[test]
fn add_script_ignores_active_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_script(&context)?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("# No dependencies are needed for interpreter discovery.\n")?;

    uv_snapshot!(context.filters(), context.add()
        .args(["--script", "script.py", "-r", "requirements.txt"])
        .args(["--offline", "--no-index", "--no-python-downloads"])
        .env(EnvVars::VIRTUAL_ENV, context.venv.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    Resolved in [TIME]
    ");

    Ok(())
}

/// Silence the default warning without changing the script's environment selection.
#[test]
fn sync_script_active_environment_selection() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_script(&context)?;

    let sync = |active: Option<&str>| -> Result<Value> {
        let mut command = context.sync();
        command
            .args([
                "--script",
                "script.py",
                "--dry-run",
                "--output-format",
                "json",
            ])
            .args(["--offline", "--no-index", "--no-python-downloads"])
            .env(EnvVars::VIRTUAL_ENV, context.venv.path());
        if let Some(active) = active {
            command.arg(active);
        }
        let assert = command
            .assert()
            .success()
            .stderr(predicate::str::contains("does not match the script environment path").not());
        Ok(serde_json::from_slice(&assert.get_output().stdout)?)
    };

    let default = sync(None)?;
    let ignored = sync(Some("--no-active"))?;
    assert_eq!(default, ignored);
    assert_eq!(default["sync"]["action"], "create");
    assert_eq!(default["sync"]["changes"], json!([]));
    assert_eq!(default["dry_run"], true);
    let cache_env = Path::new(default["sync"]["environment"]["path"].as_str().unwrap());
    assert_eq!(
        cache_env.parent().unwrap().simplified(),
        Cache::from_path(context.cache_dir.path())
            .bucket(CacheBucket::Environments)
            .simplified()
    );
    assert!(!cache_env.exists());

    // The active environment is a real, task-owned virtual environment.
    let active = sync(Some("--active"))?;
    assert_eq!(
        Path::new(active["sync"]["environment"]["path"].as_str().unwrap()).simplified(),
        context.venv.path().simplified()
    );
    assert_eq!(active["sync"]["action"], "check");
    assert_eq!(active["sync"]["changes"], json!([]));
    assert_eq!(active["dry_run"], true);

    Ok(())
}

/// Project commands still warn when an active environment differs from the project environment.
#[test]
fn sync_project_still_warns_about_active_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    context
        .sync()
        .args([
            "--dry-run",
            "--offline",
            "--no-index",
            "--no-python-downloads",
        ])
        .env(EnvVars::VIRTUAL_ENV, context.venv.path())
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "project-env")
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "does not match the project environment path `project-env` and will be ignored",
        ));
    context
        .temp_dir
        .child("project-env")
        .assert(predicate::path::missing());

    Ok(())
}
