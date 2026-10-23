#![cfg(feature = "test-universal")]

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;
use serde_json::json;
use sha2::{Digest, Sha256};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use uv_test::archive::write_tar_gz;
use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

/// A source-only dependency introduced by an optional parent downgrade must not start a build.
/// Its dynamic metadata remains available to an ordinary resolution that actually needs it.
#[tokio::test]
async fn lock_fewest_coordinated_backtracking_source_build() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated-backtracking.toml");
    let source_server = MockServer::start().await;
    let source_index = format!("{}/simple/", source_server.uri());
    let source_path = "/files/unavailable-1.0.0.tar.gz";
    let sentinel = context.temp_dir.child("backend-executed");

    // Neither `PKG-INFO` nor static project metadata is present, so reading the dependency metadata
    // requires importing the in-tree backend.
    let mut source = Vec::new();
    write_tar_gz(
        &mut source,
        &[
            (
                "unavailable-1.0.0/pyproject.toml",
                indoc! {r#"
                    [build-system]
                    requires = []
                    build-backend = "backend"
                    backend-path = ["."]
                "#},
            ),
            (
                "unavailable-1.0.0/backend.py",
                indoc! {r#"
                    import os
                    from pathlib import Path

                    Path(os.environ["UV_COORDINATED_SOURCE_SENTINEL"]).write_text("executed\n")

                    def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
                        dist_info = Path(metadata_directory) / "unavailable-1.0.0.dist-info"
                        dist_info.mkdir()
                        (dist_info / "METADATA").write_text(
                            "Metadata-Version: 2.2\n"
                            "Name: unavailable\n"
                            "Version: 1.0.0\n"
                            "Requires-Python: >=3.11\n"
                        )
                        return dist_info.name
                "#},
            ),
        ],
    )?;
    let source_digest = hex::encode(Sha256::digest(&source));
    let simple_index = json!({
        "meta": { "api-version": "1.0" },
        "name": "unavailable",
        "files": [{
            "filename": "unavailable-1.0.0.tar.gz",
            "url": format!("{}{source_path}", source_server.uri()),
            "hashes": { "sha256": source_digest },
            "requires-python": ">=3.11",
            "upload-time": "2024-01-01T00:00:00Z",
        }],
    });
    Mock::given(method("GET"))
        .and(path("/simple/unavailable/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            simple_index.to_string(),
            "application/vnd.pypi.simple.v1+json",
        ))
        .expect(1..)
        .mount(&source_server)
        .await;
    let speculative_source = Mock::given(method("GET"))
        .and(path(source_path))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(source.clone()))
        .expect(0)
        .mount_as_scoped(&source_server)
        .await;

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.11,<3.13"
            dependencies = [
                "fallible-switchable ; python_version < '3.12'",
                "delayed ; python_version >= '3.12'",
            ]

            [tool.uv]
            fork-strategy = "fewest"
            environments = [
                "python_version == '3.11'",
                "python_version == '3.12'",
            ]
        "#})?;

    uv_snapshot!(context.filters(), context.lock()
        .arg("--index-url").arg(server.index_url())
        .arg("--extra-index-url").arg(&source_index)
        .env("UV_COORDINATED_SOURCE_SENTINEL", sentinel.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    source_server.verify().await;
    assert!(
        !sentinel.exists(),
        "optional resolution imported the backend"
    );

    uv_snapshot!(context.filters(), context.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    constrained==1.0.0 ; python_full_version >= '3.12'
    delay-one==1.0.0 ; python_full_version >= '3.12'
    delay-two==1.0.0 ; python_full_version >= '3.12'
    delayed==1.0.0 ; python_full_version >= '3.12'
    fallible-switchable==2.0.0 ; python_full_version < '3.12'
    shared==1.0.0 ; python_full_version >= '3.12'
    shared==2.0.0 ; python_full_version < '3.12'
    ");

    let locked = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock()
        .arg("--locked")
        .arg("--offline")
        .arg("--index-url").arg(server.index_url())
        .arg("--extra-index-url").arg(&source_index)
        .env("UV_COORDINATED_SOURCE_SENTINEL", sentinel.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");
    assert_eq!(locked, context.read("uv.lock"));
    assert!(!sentinel.exists(), "locked resolution imported the backend");

    drop(speculative_source);
    Mock::given(method("GET"))
        .and(path(source_path))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(source))
        .expect(1..)
        .mount(&source_server)
        .await;

    let control = uv_test::test_context!("3.12");
    control
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["unavailable==1.0.0"]
        "#})?;

    uv_snapshot!(control.filters(), control.lock()
        .arg("--index-url").arg(&source_index)
        .env("UV_COORDINATED_SOURCE_SENTINEL", sentinel.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    source_server.verify().await;
    sentinel.assert("executed\n");

    uv_snapshot!(control.filters(), control.export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    unavailable==1.0.0
    ");

    Ok(())
}
