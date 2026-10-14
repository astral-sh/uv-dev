//! Cache reuse and eviction for distribution sources.

use std::path::Path;

use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use sha2::{Digest, Sha256};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

use uv_redacted::SafeUrl;
use uv_static::EnvVars;
use uv_test::archive::generate_source_archive;
use uv_test::package_server::PackageServer;
use uv_test::packse::{generate_wheel, generate_wheel_with_files};
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

/// Enabling hash verification must not trust a wheel cached without verifying its digest.
#[tokio::test]
async fn require_hashes_re_download() -> Result<()> {
    let name = "anyio".parse()?;
    let (filename, wheel) = generate_wheel(
        &name,
        &"4.0.0".parse()?,
        &[],
        &Default::default(),
        None,
        "py3-none-any",
        &[],
    );
    let actual_hash = hex::encode(Sha256::digest(&wheel));
    let wrong_hash = "0".repeat(64);
    assert_ne!(actual_hash, wrong_hash);
    let server = PackageServer::new(&name).await;
    server.serve(&filename, &wheel, None).await;
    let context = uv_test::test_context!("3.12")
        .with_filtered_http_retries()
        .with_env(EnvVars::UV_DEFAULT_INDEX, server.index_url())
        .with_filter((actual_hash.clone(), "[WHEEL_SHA256]"))
        .with_filter((wrong_hash.clone(), "[WRONG_SHA256]"));
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("anyio==4.0.0")?;

    uv_snapshot!(context.pip_sync().arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0
    ");
    let original_downloads = artifact_downloads(server.mock_server(), &filename).await;
    assert!(
        original_downloads > 0,
        "initial install did not fetch the wheel"
    );

    requirements.write_str(&format!("anyio==4.0.0 --hash=sha256:{wrong_hash}"))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--reinstall", "--require-hashes"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio==4.0.0`
      cause: Hash mismatch for `anyio==4.0.0`

             Expected:
               sha256:[WRONG_SHA256]

             Computed:
               sha256:[WHEEL_SHA256]
    ");
    assert!(
        artifact_downloads(server.mock_server(), &filename).await > original_downloads,
        "enabling hash verification did not re-download the wheel"
    );
    context.assert_installed("anyio", "4.0.0");

    requirements.write_str(&format!("anyio==4.0.0 --hash=sha256:{actual_hash}"))?;
    uv_snapshot!(context.pip_sync()
        .args(["requirements.txt", "--reinstall", "--require-hashes"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0
    ");
    Ok(())
}

async fn serve_index_wheel(server: &MockServer, package: &str, version: &str) -> Result<String> {
    let (filename, wheel) = generate_wheel(
        &package.parse()?,
        &version.parse()?,
        &[],
        &Default::default(),
        None,
        "py3-none-any",
        &[],
    );
    Mock::given(path(format!("/simple/{package}/")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "max-age=3600")
                .set_body_raw(
                    format!(r#"<a href="/{filename}" data-upload-time="2024-01-01T00:00:00Z">{filename}</a>"#),
                    "text/html",
                ),
        )
        .mount(server)
        .await;
    Mock::given(path(format!("/{filename}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "max-age=3600")
                .set_body_bytes(wheel),
        )
        .mount(server)
        .await;
    Ok(filename)
}

fn write_tomli_wheel(archive: &Path, payload: &str, mtime: i64) -> Result<()> {
    let (_, wheel) = generate_wheel_with_files(
        &"tomli".parse()?,
        &"2.0.1".parse()?,
        &[],
        &Default::default(),
        None,
        "py3-none-any",
        &[("tomli/payload.txt", payload)],
    );
    fs_err::write(archive, wheel)?;
    filetime::set_file_mtime(archive, filetime::FileTime::from_unix_time(mtime, 0))?;
    Ok(())
}

/// Refresh every package even while its HTTP response is fresh.
#[tokio::test]
async fn refresh_all_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let markupsafe = serve_index_wheel(&server, "markupsafe", "2.1.3").await?;
    let tomli = serve_index_wheel(&server, "tomli", "2.0.1").await?;
    let index_url = format!("{}/simple/", server.uri());
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("MarkupSafe==2.1.3\ntomli==2.0.1")?;

    uv_snapshot!(context.pip_sync().args(["requirements.txt", "--strict", "--index-url"]).arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + markupsafe==2.1.3
     + tomli==2.0.1
    ");
    context.assert_installed("markupsafe", "2.1.3");
    context.assert_installed("tomli", "2.0.1");
    let markupsafe_downloads = artifact_downloads(&server, &markupsafe).await;
    let tomli_downloads = artifact_downloads(&server, &tomli).await;

    context.reset_venv();
    uv_snapshot!(context.pip_sync().args(["requirements.txt", "--refresh", "--strict", "--index-url"]).arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + markupsafe==2.1.3
     + tomli==2.0.1
    ");
    assert!(
        artifact_downloads(&server, &markupsafe).await > markupsafe_downloads,
        "refresh must fetch markupsafe again"
    );
    assert!(
        artifact_downloads(&server, &tomli).await > tomli_downloads,
        "refresh must fetch tomli again"
    );
    context.assert_installed("markupsafe", "2.1.3");
    context.assert_installed("tomli", "2.0.1");
    Ok(())
}

/// Refresh only the selected package and reuse the other package's cached wheel.
#[tokio::test]
async fn refresh_selected_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let markupsafe = serve_index_wheel(&server, "markupsafe", "2.1.3").await?;
    let tomli = serve_index_wheel(&server, "tomli", "2.0.1").await?;
    let index_url = format!("{}/simple/", server.uri());
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("MarkupSafe==2.1.3\ntomli==2.0.1")?;

    uv_snapshot!(context.pip_sync().args(["requirements.txt", "--strict", "--index-url"]).arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + markupsafe==2.1.3
     + tomli==2.0.1
    ");
    context.assert_installed("markupsafe", "2.1.3");
    context.assert_installed("tomli", "2.0.1");
    let markupsafe_downloads = artifact_downloads(&server, &markupsafe).await;
    let tomli_downloads = artifact_downloads(&server, &tomli).await;

    context.reset_venv();
    uv_snapshot!(context.pip_sync().args(["requirements.txt", "--refresh-package", "tomli", "--strict", "--index-url"]).arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + markupsafe==2.1.3
     + tomli==2.0.1
    ");
    assert!(
        artifact_downloads(&server, &tomli).await > tomli_downloads,
        "refresh must fetch the selected tomli wheel again"
    );
    assert_eq!(
        artifact_downloads(&server, &markupsafe).await,
        markupsafe_downloads,
        "refresh must reuse the unselected markupsafe wheel"
    );
    context.assert_installed("markupsafe", "2.1.3");
    context.assert_installed("tomli", "2.0.1");
    Ok(())
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

/// Refresh a local wheel when its timestamp changes, including in an existing environment.
#[test]
fn install_local_wheel() -> Result<()> {
    // Use distinct timestamps even when the test runs faster than the filesystem clock resolution.
    const INITIAL_MTIME: i64 = 1_700_000_000;

    let context = uv_test::test_context!("3.12");
    let archive = context.temp_dir.child("tomli-2.0.1-py3-none-any.whl");
    write_tomli_wheel(archive.path(), "initial", INITIAL_MTIME)?;
    let initial_size = fs_err::metadata(archive.path())?.len();
    let url = SafeUrl::from_file_path(archive.path()).expect("absolute archive path");
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str(&format!("tomli @ {url}"))?;
    let installed_payload = ChildPath::new(context.site_packages().join("tomli/payload.txt"));

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    ");
    installed_payload.assert("initial");

    context.reset_venv();
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    ");
    installed_payload.assert("initial");

    context.reset_venv();
    write_tomli_wheel(archive.path(), "changed", INITIAL_MTIME + 2)?;
    assert_eq!(fs_err::metadata(archive.path())?.len(), initial_size);
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    ");
    installed_payload.assert("changed");

    write_tomli_wheel(archive.path(), "updated", INITIAL_MTIME + 4)?;
    assert_eq!(fs_err::metadata(archive.path())?.len(), initial_size);
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    ");
    installed_payload.assert("updated");

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    requirements.write_str(url.as_str())?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");
    installed_payload.assert("updated");
    context.assert_installed("tomli", "2.0.1");
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
