//! Failures while updating an installed tool from a local wheel or index.

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::assert::PathAssert;
use assert_fs::fixture::{FileWriteStr, PathChild};
use indoc::indoc;
use predicates::prelude::predicate;
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

use uv_static::EnvVars;
use uv_test::package_server::PackageServer;
use uv_test::{assert_path_missing, uv_snapshot};

const SIMPLE_LAUNCHER_WHEEL: &str = "simple_launcher-0.1.0-py3-none-any.whl";
const SIMPLE_LAUNCHER_SHA256: &str =
    "5327e0bb67cdb46800999de6dcf034bf0a5335702883494af0d8b7f6ca48cee4";
const MISMATCHED_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn tool_install_lock_verifies_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let wheel = context
        .workspace_root
        .join("test/links")
        .join(SIMPLE_LAUNCHER_WHEEL);

    context
        .tool_install()
        .arg(&wheel)
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();
    let launcher = bin_dir.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    launcher.assert(predicate::path::is_file().follow_links(true));

    let lock_path = tool_dir.child("simple-launcher").child("uv.lock");
    let lock = context.read(&lock_path);
    lock_path.write_str(&lock.replace(
        &format!("sha256:{SIMPLE_LAUNCHER_SHA256}"),
        &format!("sha256:{MISMATCHED_SHA256}"),
    ))?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg(&wheel)
        .arg("--force")
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to read `simple-launcher @ file://[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl`
      cause: Hash mismatch for `simple-launcher @ file://[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl`

             Expected:
               sha256:0000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:5327e0bb67cdb46800999de6dcf034bf0a5335702883494af0d8b7f6ca48cee4
    "#);
    assert_path_missing(tool_dir.child("simple-launcher"));
    assert_path_missing(launcher);

    Ok(())
}

/// Mount a minimal package index for `simple-launcher` with a caller-provided wheel hash.
///
/// Serving the metadata separately lets the resolver record the advertised hash without
/// downloading the wheel that the installer will later verify.
async fn mount_simple_launcher_index(server: &PackageServer, hash: &str, wheel: &[u8]) {
    server
        .serve_with(
            SIMPLE_LAUNCHER_WHEEL,
            wheel,
            Some(hash),
            json!({ "core-metadata": true }),
        )
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{SIMPLE_LAUNCHER_WHEEL}.metadata")))
        .respond_with(ResponseTemplate::new(200).set_body_string(indoc! {"
            Metadata-Version: 2.1
            Name: simple-launcher
            Version: 0.1.0
            Requires-Python: >=3.8
        "}))
        .mount(server.mock_server())
        .await;
}

#[tokio::test]
async fn tool_upgrade_resolution_hints() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let wheel = context.read_bytes(
        context
            .workspace_root
            .join("test/links")
            .join(SIMPLE_LAUNCHER_WHEEL),
    );
    let server = PackageServer::new(&"simple-launcher".parse()?).await;
    mount_simple_launcher_index(&server, SIMPLE_LAUNCHER_SHA256, &wheel).await;
    let index_url = server.index_url();
    context
        .tool_install()
        .arg("simple-launcher")
        .arg("--index-url")
        .arg(&index_url)
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    server.mock_server().reset().await;
    Mock::given(method("GET"))
        .and(path("/simple/simple-launcher/"))
        .respond_with(ResponseTemplate::new(401))
        .mount(server.mock_server())
        .await;

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("simple-launcher>0.1.0")
        .arg("--index-url")
        .arg(&index_url)
        .arg("--no-cache")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to upgrade simple-launcher
      cause: Because simple-launcher was not found in the package registry and you require simple-launcher>0.1.0, we can conclude that your requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)
    ");

    Ok(())
}

/// Ensure that `tool upgrade` verifies distributions against its newly generated tool lock.
///
/// The initial install and upgrade use the same index URL so that the installed distribution's
/// source preference remains valid, while the index changes the advertised hash before upgrade.
#[tokio::test]
async fn tool_upgrade_lock_verifies_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_tool_dirs()
        .with_filtered_http_retries();
    let bin_dir = context.temp_dir.child("bin");
    let wheel = context.read_bytes(
        context
            .workspace_root
            .join("test/links")
            .join(SIMPLE_LAUNCHER_WHEEL),
    );
    let server = PackageServer::new(&"simple-launcher".parse()?).await;
    mount_simple_launcher_index(&server, SIMPLE_LAUNCHER_SHA256, &wheel).await;
    let index_url = server.index_url();

    context
        .tool_install()
        .arg("simple-launcher")
        .arg("--index-url")
        .arg(&index_url)
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();
    let receipt = context
        .temp_dir
        .child("tools/simple-launcher/uv-receipt.toml");
    let original_receipt = context.read(&receipt);

    mount_simple_launcher_index(&server, MISMATCHED_SHA256, &wheel).await;

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("simple-launcher")
        .arg("--index-url")
        .arg(&index_url)
        .arg("--reinstall")
        .arg("--no-cache")
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to upgrade simple-launcher
      cause: Failed to download `simple-launcher==0.1.0`
      cause: Hash mismatch for `simple-launcher==0.1.0`

             Expected:
               sha256:0000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:5327e0bb67cdb46800999de6dcf034bf0a5335702883494af0d8b7f6ca48cee4
    ");
    assert_eq!(context.read(receipt), original_receipt);
    context
        .external_command(bin_dir.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX)))
        .assert()
        .success()
        .stdout("Hi from the simple launcher!\n");

    Ok(())
}
