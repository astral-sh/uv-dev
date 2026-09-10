//! Publication status reporting with local, metadata-only wheels.

use std::process::Command;

use anyhow::Result;
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use indoc::indoc;
use sha2::{Digest, Sha256};
use wiremock::matchers::{basic_auth, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

const WHEEL_FILENAME: &str = "publish_progress-1.0.0-py3-none-any.whl";
const DIST_INFO: &str = "publish_progress-1.0.0.dist-info";

/// Generate a wheel containing only distribution metadata, without importable package code.
async fn wheel(large: bool) -> Result<Vec<u8>> {
    let mut metadata =
        "Metadata-Version: 2.3\nName: publish-progress\nVersion: 1.0.0\n".to_string();
    if large {
        metadata.push('\n');
        metadata.push_str(&"x".repeat(1024 * 1024));
    }
    let wheel = indoc! {"
        Wheel-Version: 1.0
        Generator: uv-test
        Root-Is-Purelib: true
        Tag: py3-none-any
    "};
    let record = format!("{DIST_INFO}/METADATA,,\n{DIST_INFO}/WHEEL,,\n{DIST_INFO}/RECORD,,\n");
    let mut writer = ZipFileWriter::new(Vec::new());
    for (filename, contents) in [
        ("METADATA", metadata.as_str()),
        ("WHEEL", wheel),
        ("RECORD", record.as_str()),
    ] {
        let entry = ZipEntryBuilder::new(
            format!("{DIST_INFO}/{filename}").into(),
            Compression::Stored,
        );
        writer.write_entry_whole(entry, contents.as_bytes()).await?;
    }
    Ok(writer.close().await?)
}

fn publish(context: &TestContext, server: &MockServer) -> Command {
    let mut command = context.publish();
    command
        .args([
            "-u",
            "dummy",
            "-p",
            "dummy",
            "--trusted-publishing",
            "never",
        ])
        .arg("--publish-url")
        .arg(format!("{}/upload", server.uri()))
        .arg(context.temp_dir.child(WHEEL_FILENAME).path())
        .env_remove(EnvVars::UV_TEST_NO_CLI_PROGRESS);
    command
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
    let wheel = wheel(true).await?;
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
        .expect(2)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), publish(&context, &server), @"
    exit_code: 0 (success)
    ----- stderr -----
    Publishing 1 file to http://[LOCALHOST]/upload
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
     Hashed publish_progress-1.0.0-py3-none-any.whl
    Uploading publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    Uploaded publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    ");
    uv_snapshot!(context.filters(), publish(&context, &server).arg("--no-progress"), @"
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
    let wheel = wheel(true).await?;
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

    uv_snapshot!(context.filters(), publish(&context, &server).arg("--no-progress"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Publishing 1 file to http://[LOCALHOST]/upload
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
     Hashed publish_progress-1.0.0-py3-none-any.whl
    Uploading publish_progress-1.0.0-py3-none-any.whl ([SIZE]MiB)
    error: Failed to publish `publish_progress-1.0.0-py3-none-any.whl` to http://[LOCALHOST]/upload
      Caused by: Server returned status code 400 Bad Request. Server says: rejected
    ");
    Ok(())
}

#[tokio::test]
async fn publish_progress_already_exists() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_sizes();
    let wheel = wheel(false).await?;
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

    uv_snapshot!(context.filters(), publish(&context, &server)
        .arg("--check-url")
        .arg(format!("{}/simple/", server.uri())), @"
    exit_code: 0 (success)
    ----- stderr -----
    Publishing 1 file to http://[LOCALHOST]/upload
    Hashing publish_progress-1.0.0-py3-none-any.whl ([SIZE]B)
    Uploading publish_progress-1.0.0-py3-none-any.whl ([SIZE]B)
    File already exists, skipping
    ");
    Ok(())
}

#[tokio::test]
async fn publish_progress_skipped() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_sizes();
    let wheel = wheel(false).await?;
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

    uv_snapshot!(context.filters(), publish(&context, &server)
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
    let wheel = wheel(false).await?;
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

    uv_snapshot!(context.filters(), publish(&context, &server).arg("--dry-run"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checking 1 file against http://[LOCALHOST]/upload
    Checking publish_progress-1.0.0-py3-none-any.whl ([SIZE]B)
    ");
    Ok(())
}
