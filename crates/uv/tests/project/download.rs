use std::fmt::Write;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use indoc::{formatdoc, indoc};
use insta::allow_duplicates;
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_test::archive::write_tar_gz;
use uv_test::{TestContext, uv_snapshot};

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

async fn wheel(revision: &str) -> Result<Vec<u8>> {
    let mut writer = ZipFileWriter::new(Vec::new());
    let mut record = String::new();
    for (name, contents) in [
        (
            "basic_package/__init__.py",
            format!("REVISION = {revision:?}\n"),
        ),
        (
            "basic_package-0.1.0.dist-info/METADATA",
            "Metadata-Version: 2.2\nName: basic-package\nVersion: 0.1.0\n".to_string(),
        ),
        (
            "basic_package-0.1.0.dist-info/WHEEL",
            "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n".to_string(),
        ),
    ] {
        writer
            .write_entry_whole(
                ZipEntryBuilder::new(name.into(), Compression::Stored),
                contents.as_bytes(),
            )
            .await?;
        writeln!(record, "{name},,")?;
    }
    record.push_str("basic_package-0.1.0.dist-info/RECORD,,\n");
    writer
        .write_entry_whole(
            ZipEntryBuilder::new(
                "basic_package-0.1.0.dist-info/RECORD".into(),
                Compression::Stored,
            ),
            record.as_bytes(),
        )
        .await?;
    Ok(writer.close().await?)
}

fn source_archive(wheel: &[u8]) -> Result<Vec<u8>> {
    let mut sdist = Vec::new();
    write_tar_gz(
        &mut sdist,
        &[
            (
                "basic_package-0.1.0/pyproject.toml",
                indoc! {r#"
                [build-system]
                requires = []
                build-backend = "backend"
                backend-path = ["."]
                "#}
                .as_bytes(),
            ),
            (
                "basic_package-0.1.0/backend.py",
                indoc! {r#"
                import os
                import shutil
                def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
                    name = "basic_package-0.1.0-py3-none-any.whl"
                    shutil.copyfile(os.path.join(os.path.dirname(__file__), "prebuilt.whl"),
                                    os.path.join(wheel_directory, name))
                    return name
                "#}
                .as_bytes(),
            ),
            ("basic_package-0.1.0/prebuilt.whl", wheel),
            (
                "basic_package-0.1.0/PKG-INFO",
                b"Metadata-Version: 2.2\nName: basic-package\nVersion: 0.1.0\n",
            ),
        ],
    )?;
    Ok(sdist)
}

fn download(context: &TestContext) -> std::process::Command {
    let mut command = context.command();
    command.args(["download", "--preview-features", "download-command"]);
    command
}

fn write_project(context: &TestContext, packages: &str) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["basic-package"]
    "#})?;
    context
        .temp_dir
        .child("uv.lock")
        .write_str(&formatdoc! {r#"
        version = 1
        revision = 3
        requires-python = ">=3.13"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        {packages}

        [[package]]
        name = "project"
        version = "0.1.0"
        source = {{ virtual = "." }}
        dependencies = [{{ name = "basic-package" }}]

        [package.metadata]
        requires-dist = [{{ name = "basic-package" }}]
    "#})?;
    Ok(())
}

#[test]
fn download_preview() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str("[project]\nname = \"project\"\nversion = \"0.1.0\"\n")?;

    uv_snapshot!(context.filters(), context.command().args(["download", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: `uv download` is experimental and may change without warning. Pass `--preview-features download-command` to disable this warning.
    error: No uv.lock found; run `uv lock` first
    ");

    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No uv.lock found; run `uv lock` first
    ");

    Ok(())
}

/// Every locked archive is retained, including incompatible wheels and the sdist.
/// Both wheel installation and source building work with no index or HTTP cache.
#[tokio::test]
async fn download_packed_offline() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = fs_err::read(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0-py3-none-any.whl"),
    )?;
    let zstd = fs_err::read(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0-py3-none-any.whl.tar.zst"),
    )?;
    let sdist = source_archive(&wheel)?;
    let files = [
        ("basic_package-0.1.0-py3-none-any.whl", wheel.clone()),
        ("basic_package-0.1.0-py3-none-any.whl.tar.zst", zstd.clone()),
        (
            "basic_package-0.1.0-cp313-cp313-win_amd64.whl",
            wheel.clone(),
        ),
        ("basic_package-0.1.0.tar.gz", sdist.clone()),
    ];
    for (filename, bytes) in &files {
        Mock::given(method("GET"))
            .and(path(format!("/files/{filename}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
            .expect(1)
            .mount(&server)
            .await;
    }
    let url = server.uri();
    let wheel_hash = digest(&wheel);
    let zstd_hash = digest(&zstd);
    let sdist_hash = digest(&sdist);
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ registry = "{url}/simple" }}
        sdist = {{ url = "{url}/files/basic_package-0.1.0.tar.gz", hash = "sha256:{sdist_hash}", size = {sdist_size} }}
        wheels = [
            {{ url = "{url}/files/basic_package-0.1.0-py3-none-any.whl", hash = "sha256:{wheel_hash}", size = {wheel_size}, zstd = {{ hash = "sha256:{zstd_hash}", size = {zstd_size} }} }},
            {{ url = "{url}/files/basic_package-0.1.0-cp313-cp313-win_amd64.whl", hash = "sha256:{wheel_hash}", size = {wheel_size} }},
        ]
    "#, sdist_size=sdist.len(), wheel_size=wheel.len(), zstd_size=zstd.len()},
    )?;
    let original_lock = fs_err::read(context.temp_dir.join("uv.lock"))?;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 4 distributions (4 total)
    ");
    assert_eq!(
        fs_err::read(context.temp_dir.join("uv.lock"))?,
        original_lock
    );
    assert!(!context.cache_dir.join("wheels-v6").exists());
    assert!(!context.cache_dir.join("archive-v0").exists());
    let packed = context.cache_dir.join("packed-v0");
    assert_eq!(fs_err::read_dir(&packed)?.count(), 4);
    for directory in fs_err::read_dir(&packed)? {
        let directory = directory?.path();
        let bytes = if directory.join(&wheel_hash).exists() {
            fs_err::read(directory.join(&wheel_hash))?
        } else if directory.join(&zstd_hash).exists() {
            fs_err::read(directory.join(&zstd_hash))?
        } else {
            fs_err::read(directory.join(&sdist_hash))?
        };
        assert!(bytes == wheel || bytes == sdist || bytes == zstd);
    }
    server.verify().await;
    drop(server);
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions (4 total)
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--offline")
        .arg("--reinstall").arg("--no-binary-package").arg("basic-package"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ basic-package==0.1.0
    ");
    // Direct-URL resolution must also read METADATA from the packed wheel.
    context
        .pip_install()
        .arg("--offline")
        .arg("--reinstall")
        .arg(format!("{url}/files/basic_package-0.1.0-py3-none-any.whl"))
        .assert()
        .success();
    context
        .command()
        .args(["cache", "clean", "basic-package"])
        .assert()
        .success();
    assert_eq!(fs_err::read_dir(&packed)?.count(), 0);
    Ok(())
}

#[tokio::test]
async fn download_replaces_prepared_wheel() -> Result<()> {
    replaces_prepared_archive(false).await
}

#[tokio::test]
async fn download_replaces_prepared_sdist() -> Result<()> {
    replaces_prepared_archive(true).await
}

/// Refreshing a packed archive must make it usable even if an older revision was prepared.
async fn replaces_prepared_archive(source: bool) -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let url = server.uri();
    let filename = if source {
        "basic_package-0.1.0.tar.gz"
    } else {
        "basic_package-0.1.0-py3-none-any.whl"
    };
    for revision in ["old", "replacement"] {
        let wheel = wheel(revision).await?;
        let bytes = if source {
            source_archive(&wheel)?
        } else {
            wheel
        };
        let hash = digest(&bytes);
        let size = bytes.len();
        let artifact = if source {
            format!(
                r#"sdist = {{ url = "{url}/{filename}", hash = "sha256:{hash}", size = {size} }}"#
            )
        } else {
            // Exercise the hash-only fallback when the lockfile omits archive sizes.
            format!(r#"wheels = [{{ url = "{url}/{filename}", hash = "sha256:{hash}" }}]"#)
        };
        write_project(
            &context,
            &formatdoc! {r#"
            [[package]]
            name = "basic-package"
            version = "0.1.0"
            source = {{ registry = "{url}/simple" }}
            {artifact}
        "#},
        )?;
        Mock::given(method("GET"))
            .and(path(format!("/{filename}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
            .expect(1)
            .mount(&server)
            .await;
        download(&context).arg("--refresh").assert().success();
        server.verify().await;
        server.reset().await;
        if revision == "old" {
            context
                .sync()
                .args(["--frozen", "--offline"])
                .assert()
                .success();
        }
    }
    drop(server);
    allow_duplicates! {
        uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--offline", "--reinstall"]), @"
        exit_code: 0 (success)
        ----- stderr -----
        Prepared 1 package in [TIME]
        Uninstalled 1 package in [TIME]
        Installed 1 package in [TIME]
         ~ basic-package==0.1.0
        ");
    }
    context
        .python_command()
        .arg("-c")
        .arg("from basic_package import REVISION; assert REVISION == 'replacement'")
        .assert()
        .success();
    Ok(())
}

#[tokio::test]
async fn download_refresh() -> Result<()> {
    refresh_packed_archive(&["--refresh"], 2).await
}

#[tokio::test]
async fn download_refresh_package() -> Result<()> {
    refresh_packed_archive(&["--refresh-package", "basic-package"], 2).await
}

#[tokio::test]
async fn download_refresh_other_package() -> Result<()> {
    refresh_packed_archive(&["--refresh-package", "other-package"], 1).await
}

/// Refresh applies to the packed entry even before a prepared HTTP entry exists.
async fn refresh_packed_archive(args: &[&str], requests: u64) -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = wheel("original").await?;
    let hash = digest(&wheel);
    let url = server.uri();
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ registry = "{url}/simple" }}
        wheels = [{{ url = "{url}/basic_package-0.1.0-py3-none-any.whl", hash = "sha256:{hash}" }}]
    "#},
    )?;
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .expect(requests)
        .mount(&server)
        .await;
    download(&context).assert().success();
    allow_duplicates! {
        uv_snapshot!(context.filters(), context.sync().arg("--frozen").args(args), @"
        exit_code: 0 (success)
        ----- stderr -----
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + basic-package==0.1.0
        ");
    }
    server.verify().await;
    Ok(())
}

/// Metadata fallback must not use a packed response when refresh was explicitly requested.
#[tokio::test]
async fn download_refresh_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = wheel("original").await?;
    let hash = digest(&wheel);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ url = "{url}" }}
        wheels = [{{ url = "{url}", hash = "sha256:{hash}" }}]
    "#},
    )?;
    Mock::given(method("HEAD"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(
            ResponseTemplate::new(200).insert_header("Content-Length", wheel.len().to_string()),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .expect(2)
        .mount(&server)
        .await;
    download(&context).assert().success();
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!("basic-package @ {url}"))?;
    uv_snapshot!(context.filters(), context.pip_compile().arg("requirements.in").args(["--refresh", "--no-header"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    basic-package @ http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
        # via -r requirements.in

    ----- stderr -----
    WARN Range requests not supported for basic_package-0.1.0-py3-none-any.whl; streaming wheel
    Resolved 1 package in [TIME]
    ");
    server.verify().await;
    Ok(())
}

/// A mismatched digest must never become a reusable packed archive.
#[tokio::test]
async fn download_rejects_hash_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"not a wheel"))
        .mount(&server)
        .await;
    let url = server.uri();
    let hash = "0".repeat(64);
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ url = "{url}/basic_package-0.1.0-py3-none-any.whl" }}
        wheels = [{{ url = "{url}/basic_package-0.1.0-py3-none-any.whl", hash = "sha256:{hash}" }}]
    "#},
    )?;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      Caused by: Hash mismatch for http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl: expected sha256:0000000000000000000000000000000000000000000000000000000000000000
    ");
    for directory in fs_err::read_dir(context.cache_dir.join("packed-v0"))? {
        assert!(!directory?.path().join("metadata.msgpack").exists());
    }
    assert!(!context.cache_dir.join("archive-v0").exists());
    Ok(())
}

/// Corrupt packed bytes are rejected before extraction and can be refreshed.
#[tokio::test]
async fn download_repairs_corrupt_archive() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = fs_err::read(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0-py3-none-any.whl"),
    )?;
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel.clone()))
        .expect(2)
        .mount(&server)
        .await;
    let url = server.uri();
    let hash = digest(&wheel);
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ url = "{url}/basic_package-0.1.0-py3-none-any.whl" }}
        wheels = [{{ url = "{url}/basic_package-0.1.0-py3-none-any.whl", hash = "sha256:{hash}" }}]
    "#},
    )?;
    download(&context).assert().success();
    for directory in fs_err::read_dir(context.cache_dir.join("packed-v0"))? {
        fs_err::write(directory?.path().join(&hash), b"corrupt")?;
    }
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      Caused by: Hash or size mismatch for packed archive http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
    ");
    context
        .sync()
        .args(["--frozen", "--offline"])
        .assert()
        .failure();
    assert!(!context.cache_dir.join("archive-v0").exists());
    uv_snapshot!(context.filters(), download(&context).arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    server.verify().await;
    drop(server);
    context
        .sync()
        .args(["--frozen", "--offline"])
        .assert()
        .success();
    Ok(())
}
