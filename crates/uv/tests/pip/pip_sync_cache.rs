//! Cache reuse and eviction for distribution sources.

use std::path::Path;

use anyhow::Result;
use assert_fs::prelude::*;
use uv_redacted::SafeUrl;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

use uv_test::archive::generate_source_archive;
use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

const SOURCE_ARCHIVE: &str = "source_distribution-0.0.1.tar.gz";

fn source_distribution_archive(marker: &Path) -> Result<Vec<u8>> {
    generate_source_archive(
        &"source-distribution".parse()?,
        &"0.0.1".parse()?,
        "",
        Some(marker),
    )
}

async fn artifact_downloads(server: &MockServer, filename: &str) -> usize {
    let artifact_path = format!("/{filename}");
    server
        .received_requests()
        .await
        .expect("request recording is enabled")
        .iter()
        .filter(|request| request.method.as_str() == "GET" && request.url.path() == artifact_path)
        .count()
}

/// Reuse a direct URL source build until its package cache entry is removed.
#[tokio::test]
async fn install_url_source_dist_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();
    let marker = context.temp_dir.child("backend-ran");
    let archive = source_distribution_archive(marker.path())?;
    let server = MockServer::start().await;
    // A fresh response allows the direct URL to reuse its cached source revision.
    Mock::given(path(format!("/{SOURCE_ARCHIVE}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "max-age=3600")
                .set_body_bytes(archive),
        )
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!(
            "source_distribution @ {}/{SOURCE_ARCHIVE}",
            server.uri()
        ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from http://[LOCALHOST]/source_distribution-0.0.1.tar.gz)
    ");
    context.assert_installed("source_distribution", "0.0.1");
    marker.assert(predicates::path::is_file());
    fs_err::remove_file(&marker)?;

    context.reset_venv();
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from http://[LOCALHOST]/source_distribution-0.0.1.tar.gz)
    ");
    context.assert_installed("source_distribution", "0.0.1");
    marker.assert(predicates::path::missing());

    context.reset_venv();
    uv_snapshot!(context.filters(), context.clean().arg("source_distribution"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    ");
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from http://[LOCALHOST]/source_distribution-0.0.1.tar.gz)
    ");
    context.assert_installed("source_distribution", "0.0.1");
    marker.assert(predicates::path::is_file());
    Ok(())
}

/// Reuse a registry source build until its package cache entry is removed.
#[tokio::test]
async fn install_registry_source_dist_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();
    let marker = context.temp_dir.child("backend-ran");
    let archive = source_distribution_archive(marker.path())?;
    let server = MockServer::start().await;
    Mock::given(path("/simple/source-distribution/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                r#"<a href="/{SOURCE_ARCHIVE}" data-upload-time="2024-01-01T00:00:00Z">{SOURCE_ARCHIVE}</a>"#
            ),
            "text/html",
        ))
        .mount(&server)
        .await;
    Mock::given(path(format!("/{SOURCE_ARCHIVE}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive))
        .mount(&server)
        .await;
    let index_url = format!("{}/simple/", server.uri());
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("source_distribution==0.0.1")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--index-url"])
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1
    ");
    context.assert_installed("source_distribution", "0.0.1");
    marker.assert(predicates::path::is_file());
    fs_err::remove_file(&marker)?;

    context.reset_venv();
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--index-url"])
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1
    ");
    context.assert_installed("source_distribution", "0.0.1");
    marker.assert(predicates::path::missing());

    context.reset_venv();
    uv_snapshot!(context.filters(), context.clean().arg("source_distribution"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    ");
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--index-url"])
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1
    ");
    context.assert_installed("source_distribution", "0.0.1");
    marker.assert(predicates::path::is_file());
    Ok(())
}

/// A cached direct URL wheel is not downloaded again until its package cache entry is removed.
#[tokio::test]
async fn install_url_built_dist_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();
    let (filename, wheel) = generate_wheel(
        &"tqdm".parse()?,
        &"4.66.1".parse()?,
        &[],
        &Default::default(),
        None,
        "py3-none-any",
        &[],
    );
    let server = MockServer::start().await;
    Mock::given(path(format!("/{filename}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "max-age=3600")
                .set_body_bytes(wheel),
        )
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("tqdm @ {}/{filename}", server.uri()))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==4.66.1 (from http://[LOCALHOST]/tqdm-4.66.1-py3-none-any.whl)
    ");
    context.assert_installed("tqdm", "4.66.1");
    let initial_downloads = artifact_downloads(&server, &filename).await;
    assert!(
        initial_downloads > 0,
        "the initial install must fetch the wheel"
    );

    context.reset_venv();
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==4.66.1 (from http://[LOCALHOST]/tqdm-4.66.1-py3-none-any.whl)
    ");
    context.assert_installed("tqdm", "4.66.1");
    assert_eq!(
        artifact_downloads(&server, &filename).await,
        initial_downloads
    );

    context.reset_venv();
    uv_snapshot!(context.filters(), context.clean().arg("tqdm"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    ");
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==4.66.1 (from http://[LOCALHOST]/tqdm-4.66.1-py3-none-any.whl)
    ");
    context.assert_installed("tqdm", "4.66.1");
    assert!(
        artifact_downloads(&server, &filename).await > initial_downloads,
        "clearing the package cache must fetch the wheel again"
    );
    Ok(())
}

#[test]
fn install_path_source_dist_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();
    let archive = context.temp_dir.child("tqdm-1000.0.0.tar.gz");
    let marker = context.temp_dir.child("backend-ran");
    archive.write_binary(&generate_source_archive(
        &"tqdm".parse()?,
        &"1000.0.0".parse()?,
        "",
        Some(marker.path()),
    )?)?;
    let url = SafeUrl::from_file_path(archive.path()).expect("absolute archive path");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("tqdm @ {url}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0 (from file://[TEMP_DIR]/tqdm-1000.0.0.tar.gz)
    ");
    context.assert_command("import tqdm").success();
    marker.assert(predicates::path::is_file());
    fs_err::remove_file(&marker)?;

    context.reset_venv();
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0 (from file://[TEMP_DIR]/tqdm-1000.0.0.tar.gz)
    ");
    context.assert_command("import tqdm").success();
    marker.assert(predicates::path::missing());

    context.reset_venv();
    uv_snapshot!(context.filters(), context.clean().arg("tqdm"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    ");
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0 (from file://[TEMP_DIR]/tqdm-1000.0.0.tar.gz)
    ");
    context.assert_command("import tqdm").success();
    marker.assert(predicates::path::is_file());
    Ok(())
}

#[test]
fn install_path_built_dist_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();
    let filename = "tqdm-1000.0.0-py3-none-any.whl";
    let archive = context.temp_dir.child(filename);
    fs_err::copy(
        context.workspace_root.join("test/links").join(filename),
        &archive,
    )?;
    let url = SafeUrl::from_file_path(archive.path()).expect("absolute archive path");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("tqdm @ {url}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0 (from file://[TEMP_DIR]/tqdm-1000.0.0-py3-none-any.whl)
    ");
    context.assert_command("import tqdm").success();

    context.reset_venv();
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0 (from file://[TEMP_DIR]/tqdm-1000.0.0-py3-none-any.whl)
    ");
    context.assert_command("import tqdm").success();

    context.reset_venv();
    uv_snapshot!(context.filters(), context.clean().arg("tqdm"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    ");
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0 (from file://[TEMP_DIR]/tqdm-1000.0.0-py3-none-any.whl)
    ");
    context.assert_command("import tqdm").success();
    Ok(())
}
