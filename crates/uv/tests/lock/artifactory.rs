use anyhow::{Context, Result};
use assert_fs::prelude::*;
use indoc::formatdoc;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_static::EnvVars;
use uv_test::uv_snapshot;

const WHEEL: &str = "ok-1.0.0-py3-none-any.whl";
const HASH: &str = "79f0b33e6ce1e09eaa1784c8eee275dfe84d215d9c65c652f07c18e85fdaac5f";
const WRONG_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const METADATA: &str =
    "Metadata-Version: 2.3\nName: ok\nVersion: 1.0.0\nRequires-Python: >=3.8\n\n";

async fn mount_index(server: &MockServer, advertised: bool, hash: &str) {
    let mut file = json!({
        "filename": WHEEL,
        "url": format!("{}/{WHEEL}", server.uri()),
        "hashes": {"sha256": hash},
        "size": 875,
    });
    if advertised {
        file["core-metadata"] = true.into();
    }
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-JFrog-Version", "Artifactory/7.0.0")
                .insert_header("Cache-Control", "max-age=3600")
                .set_body_raw(
                    json!({"files": [file]}).to_string(),
                    "application/vnd.pypi.simple.v1+json",
                ),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{WHEEL}.metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "max-age=3600")
                .set_body_string(METADATA),
        )
        .mount(server)
        .await;
}

async fn mount_wheel(server: &MockServer) -> Result<()> {
    let wheel = fs_err::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test/links/ok-1.0.0-py3-none-any.whl"
    ))?;
    Mock::given(method("GET"))
        .and(path(format!("/{WHEEL}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "max-age=3600")
                .set_body_raw(wheel, "application/octet-stream"),
        )
        .mount(server)
        .await;
    Ok(())
}

#[tokio::test]
async fn artifactory_registry_lock_uses_sidecar() -> Result<()> {
    for advertised in [false, true] {
        let context = uv_test::test_context!("3.13");
        let server = MockServer::start().await;
        mount_index(&server, advertised, HASH).await;
        context
            .temp_dir
            .child("pyproject.toml")
            .write_str(&formatdoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.13"
            dependencies = ["ok==1.0.0"]

            [[tool.uv.index]]
            url = "{}/simple"
            default = true
            "#, server.uri()})?;

        insta::allow_duplicates!(
            uv_snapshot!(context.filters(), context.lock().env_remove(EnvVars::UV_EXCLUDE_NEWER), @"
        exit_code: 0 (success)
        ----- stderr -----
        Resolved 2 packages in [TIME]
        ")
        );
        insta::allow_duplicates!(
            uv_snapshot!(context.filters(), context.lock().arg("--check").env_remove(EnvVars::UV_EXCLUDE_NEWER), @"
        exit_code: 0 (success)
        ----- stderr -----
        Resolved 2 packages in [TIME]
        ")
        );

        let requests = server.received_requests().await.context("requests")?;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url.path(), "/simple/ok/");
        assert_eq!(requests[1].url.path(), format!("/{WHEEL}.metadata"));

        mount_wheel(&server).await?;
        insta::allow_duplicates!(
            uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
        exit_code: 0 (success)
        ----- stderr -----
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + ok==1.0.0
        ")
        );
        assert_eq!(
            server.received_requests().await.context("requests")?.len(),
            3
        );
    }
    Ok(())
}

#[tokio::test]
async fn artifactory_sidecar_does_not_replace_wheel_hash_check() -> Result<()> {
    for advertised in [false, true] {
        let context = uv_test::test_context!("3.13");
        let server = MockServer::start().await;
        mount_index(&server, advertised, WRONG_HASH).await;
        mount_wheel(&server).await?;
        context
            .temp_dir
            .child("pyproject.toml")
            .write_str(&formatdoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.13"
            dependencies = ["ok==1.0.0"]

            [[tool.uv.index]]
            url = "{}/simple"
            default = true
            "#, server.uri()})?;

        // Seed both the registry and metadata caches without downloading the wheel.
        insta::allow_duplicates!(
            uv_snapshot!(context.filters(), context.lock().env_remove(EnvVars::UV_EXCLUDE_NEWER), @"
        exit_code: 0 (success)
        ----- stderr -----
        Resolved 2 packages in [TIME]
        ")
        );
        insta::allow_duplicates!(
            uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
        exit_code: 1 (failure)
        ----- stderr -----
          × Failed to download `ok==1.0.0`
          ╰─▶ Hash mismatch for `ok==1.0.0`

              Expected:
                sha256:0000000000000000000000000000000000000000000000000000000000000000

              Computed:
                sha256:79f0b33e6ce1e09eaa1784c8eee275dfe84d215d9c65c652f07c18e85fdaac5f

        hint: `ok` (v1.0.0) was included because `project` (v0.1.0) depends on `ok`
        ")
        );
    }
    Ok(())
}

#[tokio::test]
async fn artifactory_direct_lock_still_hashes_the_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    mount_index(&server, false, HASH).await;
    mount_wheel(&server).await?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["ok"]

        [tool.uv.sources]
        ok = {{ url = "{}/{WHEEL}" }}
        "#, server.uri()})?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock().arg("--check"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert_eq!(
        lock["package"][0]["wheels"][0]["hash"].as_str(),
        Some(format!("sha256:{HASH}").as_str())
    );
    let requests = server.received_requests().await.context("requests")?;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method.as_str(), "GET");
    assert_eq!(requests[0].url.path(), format!("/{WHEEL}"));
    Ok(())
}
