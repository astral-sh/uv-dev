use anyhow::Result;
use assert_fs::prelude::*;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
fn audit_offline_flag() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.audit().arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Auditing requires network access and cannot be performed in offline mode
    ");
}

#[test]
fn audit_offline_environment() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.filters(), context.audit().env(EnvVars::UV_OFFLINE, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Auditing requires network access and cannot be performed in offline mode
    ");
}

#[test]
fn audit_offline_configuration() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("uv.toml")
        .write_str("offline = true\n")?;

    uv_snapshot!(context.filters(), context.audit(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Auditing requires network access and cannot be performed in offline mode
    ");

    Ok(())
}

#[test]
fn audit_invalid_service_url() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.audit()
        .arg("--preview-features")
        .arg("audit")
        .arg("--service-url")
        .arg("not-a-url"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value 'not-a-url' for '--service-url <SERVICE_URL>': relative URL without a base

    For more information, try '--help'.
    ");
}
