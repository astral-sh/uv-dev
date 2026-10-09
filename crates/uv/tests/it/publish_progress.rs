//! Publication status reporting with local wheels.

use std::collections::BTreeMap;

use anyhow::Result;
use assert_fs::prelude::*;
use sha2::{Digest, Sha256};
use wiremock::matchers::{basic_auth, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_static::EnvVars;
use uv_test::packse::generate_wheel_with_files;
use uv_test::uv_snapshot;

const WHEEL_FILENAME: &str = "publish_progress-1.0.0-py3-none-any.whl";
/// Generate a local wheel with optional padding above the progress-reporting threshold.
fn wheel(large: bool) -> Result<Vec<u8>> {
    let padding = "x".repeat(if large { 1024 * 1024 } else { 0 });
    let files = if large {
        vec![("padding.txt", padding.as_str())]
    } else {
        Vec::new()
    };
    let (filename, wheel) = generate_wheel_with_files(
        &"publish-progress".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &files,
    );
    assert_eq!(filename, WHEEL_FILENAME);
    Ok(wheel)
}

fn index_response(wheel: &[u8]) -> ResponseTemplate {
    let hash = hex::encode(Sha256::digest(wheel));
    ResponseTemplate::new(200).set_body_raw(
        format!("<a href=\"/files/{WHEEL_FILENAME}#sha256={hash}\">{WHEEL_FILENAME}</a>"),
        "text/html",
    )
}

#[tokio::test]
async fn large_publish_progress_success() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_sizes();
    let wheel = wheel(true)?;
    assert!(wheel.len() > 1024 * 1024);
    context
        .temp_dir
        .child(WHEEL_FILENAME)
        .write_binary(&wheel)?;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .and(basic_auth("dummy", "dummy"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.publish()
        .args(["-u", "dummy", "-p", "dummy", "--trusted-publishing", "never"])
        .arg("--publish-url")
        .arg(format!("{}/upload", server.uri()))
        .arg(context.temp_dir.child(WHEEL_FILENAME).path())
        .env_remove(EnvVars::UV_INTERNAL__TEST_NO_CLI_PROGRESS), @"
    exit_code: 0 (success)
    ----- stderr -----
    Publishing 1 file to http://[LOCALHOST]/upload
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
     Hashed publish_progress-1.0.0-py3-none-any.whl
    Uploading publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    Uploaded publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    ");
    Ok(())
}

#[tokio::test]
async fn large_publish_progress_rejected() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_sizes();
    let wheel = wheel(true)?;
    assert!(wheel.len() > 1024 * 1024);
    context
        .temp_dir
        .child(WHEEL_FILENAME)
        .write_binary(&wheel)?;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .and(basic_auth("dummy", "dummy"))
        .respond_with(ResponseTemplate::new(400).set_body_string("rejected"))
        .expect(1)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.publish()
        .args(["-u", "dummy", "-p", "dummy", "--trusted-publishing", "never"])
        .arg("--publish-url")
        .arg(format!("{}/upload", server.uri()))
        .arg(context.temp_dir.child(WHEEL_FILENAME).path())
        .env_remove(EnvVars::UV_INTERNAL__TEST_NO_CLI_PROGRESS).arg("--no-progress"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Publishing 1 file to http://[LOCALHOST]/upload
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
     Hashed publish_progress-1.0.0-py3-none-any.whl
    Uploading publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    error: Failed to publish `publish_progress-1.0.0-py3-none-any.whl` to `http://[LOCALHOST]/upload`
      cause: Server returned status code 400 Bad Request. Server says: rejected
    ");
    Ok(())
}

#[tokio::test]
async fn publish_progress_already_exists() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_sizes();
    let wheel = wheel(false)?;
    context
        .temp_dir
        .child(WHEEL_FILENAME)
        .write_binary(&wheel)?;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/publish-progress/"))
        .respond_with(ResponseTemplate::new(404))
        .with_priority(1)
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/simple/publish-progress/"))
        .respond_with(index_response(&wheel))
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .and(basic_auth("dummy", "dummy"))
        .respond_with(ResponseTemplate::new(400).set_body_string("already exists"))
        .expect(1)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.publish()
        .args(["-u", "dummy", "-p", "dummy", "--trusted-publishing", "never"])
        .arg("--publish-url")
        .arg(format!("{}/upload", server.uri()))
        .arg(context.temp_dir.child(WHEEL_FILENAME).path())
        .env_remove(EnvVars::UV_INTERNAL__TEST_NO_CLI_PROGRESS)
        .arg("--check-url")
        .arg(format!("{}/simple/", server.uri())), @"
    exit_code: 0 (success)
    ----- stderr -----
    Publishing 1 file to http://[LOCALHOST]/upload
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]KiB)
    Uploading publish_progress-1.0.0-py3-none-any.whl ([SIZE]KiB)
    File already exists, skipping
    ");
    Ok(())
}

#[tokio::test]
async fn publish_progress_skipped() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_sizes();
    let wheel = wheel(false)?;
    context
        .temp_dir
        .child(WHEEL_FILENAME)
        .write_binary(&wheel)?;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/publish-progress/"))
        .respond_with(index_response(&wheel))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.publish()
        .args(["-u", "dummy", "-p", "dummy", "--trusted-publishing", "never"])
        .arg("--publish-url")
        .arg(format!("{}/upload", server.uri()))
        .arg(context.temp_dir.child(WHEEL_FILENAME).path())
        .env_remove(EnvVars::UV_INTERNAL__TEST_NO_CLI_PROGRESS)
        .arg("--check-url")
        .arg(format!("{}/simple/", server.uri())), @"
    exit_code: 0 (success)
    ----- stderr -----
    Publishing 1 file to http://[LOCALHOST]/upload
    File publish_progress-1.0.0-py3-none-any.whl already exists, skipping
    ");
    Ok(())
}

#[tokio::test]
async fn publish_progress_dry_run() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_sizes();
    let wheel = wheel(false)?;
    context
        .temp_dir
        .child(WHEEL_FILENAME)
        .write_binary(&wheel)?;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.publish()
        .args(["-u", "dummy", "-p", "dummy", "--trusted-publishing", "never"])
        .arg("--publish-url")
        .arg(format!("{}/upload", server.uri()))
        .arg(context.temp_dir.child(WHEEL_FILENAME).path())
        .env_remove(EnvVars::UV_INTERNAL__TEST_NO_CLI_PROGRESS).arg("--dry-run"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checking 1 file against http://[LOCALHOST]/upload
    Checking publish_progress-1.0.0-py3-none-any.whl ([SIZE]KiB)
    ");
    Ok(())
}
