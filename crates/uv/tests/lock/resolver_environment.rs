use std::process::Command;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::formatdoc;

use uv_static::EnvVars;
use uv_test::TestContext;

fn write_project(context: &TestContext, missing_dependency: bool) -> Result<()> {
    let dependency = if missing_dependency {
        "\"uv-missing-resolver-environment-test\""
    } else {
        ""
    };
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "resolver-environment-test"
        version = "0.1.0"
        requires-python = ">=3.12,<3.13"
        dependencies = [{dependency}]

        [tool.uv]
        environments = ["sys_platform == 'win32'", "sys_platform != 'win32'"]
    "#})?;
    Ok(())
}

fn lock(context: &TestContext) -> Command {
    let mut command = context.lock();
    command
        .args([
            "--offline",
            "--no-index",
            "--no-python-downloads",
            "--color",
            "never",
        ])
        .env_remove(EnvVars::RUST_LOG);
    command
}

fn trace_environments<'a>(stderr: &'a str, prefix: &str) -> Vec<&'a str> {
    stderr
        .lines()
        .filter_map(|line| line.strip_prefix(prefix))
        .collect()
}

#[test]
fn traces_each_resolver_environment() -> Result<()> {
    let control = uv_test::test_context!("3.12");
    write_project(&control, false)?;
    let ordinary = lock(&control).assert().success();
    assert!(!String::from_utf8_lossy(&ordinary.get_output().stderr).contains("TRACE "));

    let context = uv_test::test_context!("3.12");
    write_project(&context, false)?;
    let traced = lock(&context)
        .env(EnvVars::RUST_LOG, "uv_resolver=trace")
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&traced.get_output().stderr);
    let started = trace_environments(&stderr, "TRACE Solving with resolver environment: ");
    let solved = trace_environments(&stderr, "TRACE Resolution: ");

    assert_eq!(started.len(), 2);
    assert_eq!(started, solved);
    for marker in [
        "markers: sys_platform == 'win32'",
        "markers: sys_platform != 'win32'",
    ] {
        assert!(
            started.iter().any(|env| env.contains(marker)),
            "{started:?}"
        );
    }
    assert_eq!(context.read("uv.lock"), control.read("uv.lock"));

    Ok(())
}

#[test]
fn traces_the_failing_resolver_environment() -> Result<()> {
    let control = uv_test::test_context!("3.12");
    write_project(&control, true)?;
    let ordinary = lock(&control).assert().code(1);
    let ordinary_stderr = String::from_utf8_lossy(&ordinary.get_output().stderr);
    assert!(!ordinary_stderr.contains("TRACE "));

    let context = uv_test::test_context!("3.12");
    write_project(&context, true)?;
    let traced = lock(&context)
        .env(EnvVars::RUST_LOG, "uv_resolver=trace")
        .assert()
        .code(1);
    let stderr = String::from_utf8_lossy(&traced.get_output().stderr);
    let started = trace_environments(&stderr, "TRACE Solving with resolver environment: ");
    let failed = trace_environments(
        &stderr,
        "TRACE No solution found for resolver environment: ",
    );

    assert_eq!(started.len(), 1);
    assert_eq!(started, failed);
    assert!(trace_environments(&stderr, "TRACE Resolution: ").is_empty());

    // Trace logging must not change the error or its hints.
    let error = "  × No solution found";
    assert_eq!(
        stderr.split_once(error).expect("resolution error").1,
        ordinary_stderr
            .split_once(error)
            .expect("resolution error")
            .1,
    );

    Ok(())
}
