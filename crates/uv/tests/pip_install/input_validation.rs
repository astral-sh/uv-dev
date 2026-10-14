//! Input errors reported before interpreter discovery.

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::uv_snapshot;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn invalid_editable_no_url() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("-e black==0.1.0")?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unsupported editable requirement in `requirements.txt` at line 1: `black==0.1.0`
      cause: Registry requirements cannot be editable

    hint: Editable requirements must refer to a local directory
    ");

    Ok(())
}

#[test]
fn invalid_editable_unnamed_remote_url_requirements_txt() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("-e http://user:password@example.com/black-1.0.0-py3-none-any.whl")?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unsupported editable requirement in `requirements.txt` at line 1: `http://user:****@example.com/black-1.0.0-py3-none-any.whl`
      cause: Remote archives cannot be editable

    hint: Editable requirements must refer to a local directory
    ");

    Ok(())
}

#[test]
fn invalid_editable_unnamed_remote_url_cli() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e")
        .arg("http://user:password@example.com/black-1.0.0-py3-none-any.whl"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unsupported editable requirement: `http://user:****@example.com/black-1.0.0-py3-none-any.whl`
      cause: Remote archives cannot be editable

    hint: Editable requirements must refer to a local directory
    ");
}

#[test]
fn invalid_editable_named_https_url() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        # This requirement is not editable.
        anyio==3.7.0
        -e black @ https://example.invalid/black-24.4.2-py3-none-any.whl
    "})?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unsupported editable requirement in `requirements.txt` at line 3: `black @ https://example.invalid/black-24.4.2-py3-none-any.whl`
      cause: Remote archives cannot be editable

    hint: Editable requirements must refer to a local directory
    ");

    Ok(())
}

async fn remote_file() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("ok==1.0.0\n"))
        .expect(0)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn requirements_file_respects_offline_mode() {
    let server = remote_file().await;
    let context =
        uv_test::test_context_with_versions!(&[]).with_filter((server.uri(), "http://[LOCALHOST]"));
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--offline", "-r"])
        .arg(format!("{}/requirements.txt", server.uri())), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Network connectivity is disabled, but a remote requirements file was requested: http://[LOCALHOST]/requirements.txt
    ");
}

#[tokio::test]
async fn constraints_file_respects_offline_mode() {
    let server = remote_file().await;
    let context =
        uv_test::test_context_with_versions!(&[]).with_filter((server.uri(), "http://[LOCALHOST]"));
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--offline", "-c"])
        .arg(format!("{}/constraints.txt", server.uri()))
        .arg("ok==1.0.0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Network connectivity is disabled, but a remote requirements file was requested: http://[LOCALHOST]/constraints.txt
    ");
}
