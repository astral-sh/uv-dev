//! Cache reuse and eviction for distribution sources.

use anyhow::Result;
use assert_fs::prelude::*;
use uv_redacted::SafeUrl;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

use uv_test::archive::generate_source_archive;
use uv_test::uv_snapshot;

/// Reuse a direct URL source build until its package cache entry is removed.
#[tokio::test]
async fn install_url_source_dist_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();
    let filename = "source_distribution-0.0.1.tar.gz";
    let marker = context.temp_dir.child("backend-ran");
    let archive = generate_source_archive(
        &"source-distribution".parse()?,
        &"0.0.1".parse()?,
        "",
        Some(marker.path()),
    )?;
    let server = MockServer::start().await;
    Mock::given(path(format!("/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive))
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!(
            "source_distribution @ {}/{filename}",
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
