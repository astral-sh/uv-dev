//! Synchronization with direct source URLs.

use anyhow::{Context, Result};
use assert_fs::prelude::*;
use indoc::formatdoc;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_test::archive::generate_source_archive;
use uv_test::uv_snapshot;

/// A source URL's query parameters must reach the archive server and remain in the lock.
#[tokio::test]
async fn sync_url_with_query_parameters() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let filename = "source_distribution-0.0.3.tar.gz";
    let archive =
        generate_source_archive(&"source-distribution".parse()?, &"0.0.3".parse()?, "", None)?;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}")))
        .and(query_param("foo", "bar"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive))
        .mount(&server)
        .await;
    let url = format!("{}/{filename}?foo=bar", server.uri());
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "example"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["source-distribution @ {url}"]
    "#})?;

    uv_snapshot!(context.filters(), context.sync().arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.3 (from http://[LOCALHOST]/source_distribution-0.0.3.tar.gz?foo=bar)
    ");
    context.assert_installed("source_distribution", "0.0.3");

    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    let package = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .and_then(|packages| {
            packages.iter().find(|package| {
                package.get("name").and_then(toml::Value::as_str) == Some("source-distribution")
            })
        })
        .context("missing source-distribution package in lockfile")?;
    assert_eq!(
        package
            .get("source")
            .and_then(|source| source.get("url"))
            .and_then(toml::Value::as_str),
        Some(url.as_str())
    );
    Ok(())
}
