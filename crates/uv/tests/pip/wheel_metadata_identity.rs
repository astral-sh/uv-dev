use std::collections::BTreeMap;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use serde_json::json;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_static::EnvVars;
use uv_test::archive::generate_source_archive;
use uv_test::package_server::PackageServer;
use uv_test::packse::{generate_wheel, mount_mismatched_distribution};
use uv_test::uv_snapshot;

#[test]
fn local_wheel_version_mismatch_precedes_dependency_build() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let marker = context.temp_dir.child("backend-ran");
    let dependency = context.temp_dir.child("child-1.0.tar.gz");
    dependency.write_binary(&generate_source_archive(
        &"child".parse()?,
        &"1.0".parse()?,
        "",
        Some(marker.path()),
    )?)?;
    let dependency_url = Url::from_file_path(dependency.path()).expect("absolute fixture path");
    let (_, bytes) = generate_wheel(
        &"demo".parse()?,
        &"2.0".parse()?,
        &[format!("child @ {dependency_url}").parse()?],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child("demo-1.0-py3-none-any.whl");
    wheel.write_binary(&bytes)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!(
            "demo @ {}",
            Url::from_file_path(wheel.path()).expect("absolute fixture path")
        ))?;

    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--no-index"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to read `demo @ file://[TEMP_DIR]/demo-1.0-py3-none-any.whl`
      cause: Wheel metadata version `2.0` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    ");
    assert_eq!(output.status.code(), Some(1));
    assert!(!marker.exists());

    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--no-index").arg("--generate-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to read `demo @ file://[TEMP_DIR]/demo-1.0-py3-none-any.whl`
      cause: Package metadata version `2.0` does not match `1.0` from the wheel filename
    ");
    assert_eq!(output.status.code(), Some(1));
    assert!(!marker.exists());
    Ok(())
}

#[tokio::test]
async fn remote_wheel_version_mismatch() -> Result<()> {
    for range_requests in [false, true] {
        let context = uv_test::test_context!("3.12");
        let server = MockServer::start().await;
        let filename = "demo-1.0-py3-none-any.whl";
        let (_, bytes) = generate_wheel(
            &"demo".parse()?,
            &"2.0".parse()?,
            &[],
            &BTreeMap::new(),
            None,
            "py3-none-any",
            &[],
        );
        if range_requests {
            mount_mismatched_distribution(
                &server,
                &format!("/{filename}"),
                filename,
                bytes.clone(),
                bytes,
            )
            .await;
        } else {
            Mock::given(method("HEAD"))
                .respond_with(ResponseTemplate::new(405))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/{filename}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
                .expect(1)
                .mount(&server)
                .await;
        }
        context
            .temp_dir
            .child("requirements.in")
            .write_str(&format!("demo @ {}/{filename}", server.uri()))?;
        insta::allow_duplicates! {
            let output = uv_snapshot!(context.filters(), context.pip_compile()
                .arg("requirements.in").arg("--no-index"), @"
            exit_code: 1 (failure)
            ----- stderr -----
            error: Failed to download `demo @ http://[LOCALHOST]/demo-1.0-py3-none-any.whl`
              cause: Wheel metadata version `2.0` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
            ");
            assert_eq!(output.status.code(), Some(1));
        }
    }
    Ok(())
}

#[tokio::test]
async fn mismatched_sidecar_is_not_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let name = "demo".parse()?;
    let server = PackageServer::new(&name).await;
    let (filename, bytes) = generate_wheel(
        &name,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    server
        .serve_with(&filename, &bytes, None, json!({ "core-metadata": true }))
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}.metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=86400")
                .set_body_string("Metadata-Version: 2.3\nName: demo\nVersion: 2.0\n"),
        )
        .expect(2)
        .mount(server.mock_server())
        .await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("demo==1.0")?;
    insta::allow_duplicates! {
        for _ in 0..2 {
            let output = uv_snapshot!(context.filters(), context.pip_compile()
                .arg("requirements.in").arg("--index-url").arg(server.index_url()), @"
            exit_code: 1 (failure)
            ----- stderr -----
            error: Wheel metadata version `2.0` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
            ");
            assert_eq!(output.status.code(), Some(1));
        }
    }
    Ok(())
}

#[tokio::test]
async fn cached_sidecar_version_is_rechecked() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let name = "demo".parse()?;
    let server = PackageServer::new(&name).await;
    let (filename, bytes) = generate_wheel(
        &name,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    server
        .serve_with(&filename, &bytes, None, json!({ "core-metadata": true }))
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}.metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=86400")
                .set_body_string("Metadata-Version: 2.3\nName: demo\nVersion: 2.0\n"),
        )
        .expect(1)
        .mount(server.mock_server())
        .await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("demo==1.0")?;
    context
        .pip_compile()
        .arg("requirements.in")
        .arg("--index-url")
        .arg(server.index_url())
        .env(EnvVars::UV_SKIP_WHEEL_FILENAME_CHECK, "1")
        .assert()
        .success();

    let output = uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--index-url").arg(server.index_url())
        .arg("--offline"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Wheel metadata version `2.0` does not match `1.0` from the wheel filename. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    ");
    assert_eq!(output.status.code(), Some(1));

    context
        .pip_compile()
        .arg("requirements.in")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--offline")
        .env(EnvVars::UV_SKIP_WHEEL_FILENAME_CHECK, "1")
        .assert()
        .success();
    Ok(())
}

#[test]
fn wheel_metadata_can_omit_local_version() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (_, bytes) = generate_wheel(
        &"demo".parse()?,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child("demo-1.0+local-py3-none-any.whl");
    wheel.write_binary(&bytes)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!(
            "demo @ {}",
            Url::from_file_path(wheel.path()).expect("absolute fixture path")
        ))?;
    for generate_hashes in [false, true] {
        let mut command = context.pip_compile();
        command.arg("requirements.in").arg("--no-index");
        if generate_hashes {
            command.arg("--generate-hashes");
        }
        command.assert().success();
    }
    context
        .pip_install()
        .arg(wheel.path())
        .arg("--no-index")
        .assert()
        .success();
    Ok(())
}
