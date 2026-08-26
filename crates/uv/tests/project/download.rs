use std::fmt::Write;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use indoc::{formatdoc, indoc};
use insta::allow_duplicates;
use serde_json::json;
use sha2::{Digest, Sha256, Sha512};
use wiremock::matchers::{basic_auth, method, path};
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

#[tokio::test]
async fn download_credentials_dependency() -> Result<()> {
    download_credentials(
        indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["basic-package @ {url}"]
        "#},
        None,
    )
    .await
}

#[tokio::test]
async fn download_credentials_source() -> Result<()> {
    download_credentials(
        indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["basic-package"]

        [tool.uv.sources]
        basic-package = { url = "{url}" }
        "#},
        None,
    )
    .await
}

#[tokio::test]
async fn download_credentials_workspace() -> Result<()> {
    download_credentials(
        indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["basic-package"]

        [tool.uv.workspace]
        members = ["member", "missing-member"]
        "#},
        Some(indoc! {r#"
        [project]
        name = "member"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["basic-package @ {url}"]
        "#}),
    )
    .await
}

/// Credentials are read from project files, since lockfile URLs do not contain credentials.
async fn download_credentials(pyproject: &str, member: Option<&str>) -> Result<()> {
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
    let authenticated_url = url.replace("http://", "http://username:password@");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&pyproject.replace("{url}", &authenticated_url))?;
    if let Some(member) = member {
        context.temp_dir.child("member").create_dir_all()?;
        context
            .temp_dir
            .child("member/pyproject.toml")
            .write_str(&member.replace("{url}", &authenticated_url))?;
    }
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(
            ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Basic realm=\"test\""),
        )
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .and(basic_auth("username", "password"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    allow_duplicates! {
        uv_snapshot!(context.filters(), download(&context), @"
        exit_code: 0 (success)
        ----- stderr -----
        Downloaded 1 distributions (1 total)
        ");
    }
    server.verify().await;
    Ok(())
}

/// An output directory preserves distribution filenames without populating the packed cache.
#[tokio::test]
async fn download_to_flat_directory() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let filename = "basic_package-0.1.0-py3-none-any.whl";
    let wheel = fs_err::read(context.workspace_root.join("test/links").join(filename))?;
    let sdist_filename = "basic_package-0.1.0.tar.gz";
    let sdist = fs_err::read(
        context
            .workspace_root
            .join("test/links")
            .join(sdist_filename),
    )?;
    Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel.clone()))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/files/{sdist_filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(sdist.clone()))
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
        source = {{ registry = "{url}/simple" }}
        sdist = {{ url = "{url}/files/{sdist_filename}", hash = "sha256:{sdist_hash}", size = {sdist_size} }}
        wheels = [{{ url = "{url}/files/{filename}", hash = "sha256:{hash}", size = {size} }}]
    "#, size=wheel.len(), sdist_hash=digest(&sdist), sdist_size=sdist.len()},
    )?;
    let output = context.temp_dir.child("downloads");
    uv_snapshot!(context.filters(), download(&context)
        .arg("--output-dir")
        .arg(output.path()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 2 distributions to [TEMP_DIR]/downloads (2 total)
    ");
    assert_eq!(fs_err::read(output.join(filename))?, wheel);
    assert_eq!(fs_err::read(output.join(sdist_filename))?, sdist);
    assert!(!context.cache_dir.join("packed-v0").exists());

    uv_snapshot!(context.filters(), download(&context)
        .arg("--output-dir")
        .arg(output.path())
        .arg("--offline"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions to [TEMP_DIR]/downloads (2 total)
    ");

    output.child(filename).write_binary(b"corrupt")?;
    uv_snapshot!(context.filters(), download(&context)
        .arg("--output-dir")
        .arg(output.path())
        .arg("--offline"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/files/basic_package-0.1.0-py3-none-any.whl
      Caused by: Hash or size mismatch for existing archive `[TEMP_DIR]/downloads/basic_package-0.1.0-py3-none-any.whl` from http://[LOCALHOST]/files/basic_package-0.1.0-py3-none-any.whl; use `--refresh` to replace it
    ");
    uv_snapshot!(context.filters(), download(&context)
        .arg("--output-dir")
        .arg(output.path())
        .arg("--refresh"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 2 distributions to [TEMP_DIR]/downloads (2 total)
    ");
    assert_eq!(fs_err::read(output.join(filename))?, wheel);
    assert_eq!(fs_err::read(output.join(sdist_filename))?, sdist);
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

/// Requirements files act as universal manifests, independent of project discovery.
#[tokio::test]
async fn download_requirements_universal() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let url = server.uri();
    let wheel = fs_err::read(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0-py3-none-any.whl"),
    )?;
    let sdist = fs_err::read(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0.tar.gz"),
    )?;
    let wheel_hash = digest(&wheel);
    let sdist_hash = digest(&sdist);
    let excluded = b"archive not authorized by the manifest";
    let files = [
        ("basic_package-0.1.0-py3-none-any.whl", wheel.as_slice()),
        (
            "basic_package-0.1.0-cp313-cp313-win_amd64.whl",
            wheel.as_slice(),
        ),
        ("basic_package-0.1.0.tar.gz", sdist.as_slice()),
        (
            "basic_package-0.1.0-cp313-cp313-manylinux_2_17_x86_64.whl",
            excluded.as_slice(),
        ),
    ];
    let index = json!({
        "meta": {"api-version": "1.1"},
        "name": "basic-package",
        "files": files.iter().map(|(filename, bytes)| json!({
            "filename": filename,
            "url": format!("{url}/files/{filename}"),
            "hashes": {"sha256": digest(bytes)},
            "size": bytes.len(),
            "upload-time": "2024-01-01T00:00:00Z",
            "core-metadata": true
        })).collect::<Vec<_>>()
    });
    Mock::given(method("GET"))
        .and(path("/simple/basic-package/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(index.to_string(), "application/vnd.pypi.simple.v1+json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    for (filename, bytes) in files {
        Mock::given(method("GET"))
            .and(path(format!("/files/{filename}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.to_vec()))
            .expect(u64::from(bytes != excluded))
            .mount(&server)
            .await;
    }
    context.temp_dir.child("nested").create_dir_all()?;
    context
        .temp_dir
        .child("nested/pins.txt")
        .write_str(&formatdoc! {r#"
        basic-package==0.1.0 ; sys_platform == "not-a-platform" --hash=sha256:{wheel_hash} --hash=sha256:{sdist_hash}
    "#})?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&formatdoc! {"
        --index-url {url}/simple
        --require-hashes
        -r nested/pins.txt
    "})?;
    context.temp_dir.child("direct.txt").write_str(&format!(
        "basic-package @ {url}/files/basic_package-0.1.0-py3-none-any.whl --hash=sha256:{wheel_hash}\n"
    ))?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "-r", "direct.txt"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 3 distributions (4 total)
    ");
    assert!(!context.temp_dir.join("uv.lock").exists());
    assert!(!context.cache_dir.join("archive-v0").exists());
    assert!(!context.cache_dir.join("wheels-v6").exists());
    assert_eq!(
        fs_err::read_dir(context.cache_dir.join("packed-v0"))?.count(),
        3
    );
    server.verify().await;
    drop(server);
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "-r", "direct.txt", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions (4 total)
    ");
    // Preserve the download's hash policy during installation. Otherwise, Linux would prefer
    // the deliberately excluded platform wheel over the cached universal wheel. Explicitly
    // target Linux so the same selection is exercised on every host.
    context.temp_dir.child("install.txt").write_str(&format!(
        "basic-package==0.1.0 --hash=sha256:{wheel_hash} --hash=sha256:{sdist_hash}\n"
    ))?;
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-r").arg("install.txt").arg("--require-hashes")
        .arg("--index-url").arg(format!("{url}/simple"))
        .arg("--python-platform").arg("x86_64-unknown-linux-gnu").arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0
    ");
    Ok(())
}

#[test]
fn download_requirements_find_links() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0-py3-none-any.whl"),
        links.join("basic_package-0.1.0-py3-none-any.whl"),
    )?;
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0.tar.gz"),
        links.join("basic_package-0.1.0.tar.gz"),
    )?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(indoc! {"
        --no-index
        --find-links links
        --only-binary :all:
        basic-package==0.1.0
    "})?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert!(!context.venv.exists());
    assert!(!context.temp_dir.join("uv.lock").exists());
    assert!(!context.cache_dir.join("archive-v0").exists());
    assert_eq!(
        fs_err::read_dir(context.cache_dir.join("packed-v0"))?.count(),
        1
    );
    Ok(())
}

#[test]
fn download_requirements_to_flat_directory() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let filename = "basic_package-0.1.0-py3-none-any.whl";
    let wheel = fs_err::read(context.workspace_root.join("test/links").join(filename))?;
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    links.child(filename).write_binary(&wheel)?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&formatdoc! {"
        --no-index
        --find-links links
        --require-hashes
        basic-package==0.1.0 --hash=sha256:{hash}
    ", hash=digest(&wheel)})?;
    let output = context.temp_dir.child("downloads");
    uv_snapshot!(context.filters(), download(&context)
        .args(["-r", "requirements.txt", "--output-dir"])
        .arg(output.path())
        .arg("--offline"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions to [TEMP_DIR]/downloads (1 total)
    ");
    assert_eq!(fs_err::read(output.join(filename))?, wheel);
    assert!(!context.cache_dir.join("packed-v0").exists());

    uv_snapshot!(context.filters(), download(&context)
        .args(["-r", "requirements.txt", "--output-dir"])
        .arg(output.path())
        .arg("--offline"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions to [TEMP_DIR]/downloads (1 total)
    ");
    Ok(())
}

/// Even `unsafe-best-match` takes a pinned version from only its first index.
#[tokio::test]
async fn download_requirements_index_priority() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let server = MockServer::start().await;
    let url = server.uri();
    for index in ["first", "second"] {
        let filename = "basic_package-0.1.0-py3-none-any.whl";
        let bytes = index.as_bytes();
        let metadata = json!({
            "meta": {"api-version": "1.1"},
            "name": "basic-package",
            "files": [{
                "filename": filename,
                "url": format!("{url}/{index}/{filename}"),
                "hashes": {"sha256": digest(bytes)},
                "size": bytes.len(),
                "upload-time": "2024-01-01T00:00:00Z"
            }]
        });
        Mock::given(method("GET"))
            .and(path(format!("/{index}/basic-package/")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(metadata.to_string(), "application/vnd.pypi.simple.v1+json"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{index}/{filename}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.to_vec()))
            .expect(u64::from(index == "first"))
            .mount(&server)
            .await;
    }
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&formatdoc! {"
        --extra-index-url {url}/first
        --index-url {url}/second
        basic-package==0.1.0
    "})?;
    uv_snapshot!(context.filters(), download(&context).args([
        "-r", "requirements.txt", "--index-strategy", "unsafe-best-match"
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert!(!context.venv.exists());
    assert!(!context.cache_dir.join("archive-v0").exists());
    Ok(())
}

/// A concrete archive must satisfy every digest, including its URL-fragment digest.
#[tokio::test]
async fn download_requirements_stdin_hashes() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let server = MockServer::start().await;
    let wheel = fs_err::read(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0-py3-none-any.whl"),
    )?;
    let sha256 = digest(&wheel);
    let sha512 = hex::encode(Sha512::digest(&wheel));
    let wheel_url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let input = context.temp_dir.child("input.txt");
    input.write_str(&formatdoc! {"
        --require-hashes
        {wheel_url}#sha256={sha256} --hash=sha512:{sha512}
    "})?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "-"])
        .stdin(fs_err::File::open(input.path())?.into_file()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    server.verify().await;
    drop(server);
    // A matching SHA-256 must not excuse a mismatched SHA-512 on the same archive.
    let bad_hash = "0".repeat(128);
    input.write_str(&formatdoc! {"
        {wheel_url}#sha256={sha256} --hash=sha512:{bad_hash}
    "})?;
    let mut filters = context.filters();
    filters.push((&sha256, "[SHA256]"));
    filters.push((&bad_hash, "[BAD_HASH]"));
    uv_snapshot!(filters, download(&context).args(["-r", "-", "--offline"])
        .stdin(fs_err::File::open(input.path())?.into_file()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl#sha256=[SHA256]
      Caused by: Hash mismatch for http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl#sha256=[SHA256]: expected sha256:[SHA256], sha512:[BAD_HASH]
    ");
    // Local paths can supply their digests with `--hash`, without any network access.
    context
        .temp_dir
        .child("basic_package-0.1.0-py3-none-any.whl")
        .write_binary(&wheel)?;
    input.write_str(&formatdoc! {"
        --require-hashes
        ./basic_package-0.1.0-py3-none-any.whl --hash=sha256:{sha256} --hash=sha512:{sha512}
    "})?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "-", "--offline"])
        .stdin(fs_err::File::open(input.path())?.into_file()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert!(!context.cache_dir.join("archive-v0").exists());
    Ok(())
}

/// When an index has no hashes, authorize the actual archive bytes before caching them.
#[test]
fn download_requirements_hash_selection() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let wheel = fs_err::read(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0-py3-none-any.whl"),
    )?;
    links
        .child("basic_package-0.1.0-py3-none-any.whl")
        .write_binary(&wheel)?;
    let sdist = fs_err::read(
        context
            .workspace_root
            .join("test/links/basic_package-0.1.0.tar.gz"),
    )?;
    links
        .child("basic_package-0.1.0.tar.gz")
        .write_binary(&sdist)?;
    let wheel_hash = hex::encode(Sha512::digest(&wheel));
    let sdist_hash = hex::encode(Sha512::digest(&sdist));
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str(&formatdoc! {r#"
        --no-index
        --find-links links
        basic-package==0.1.0 ; sys_platform == "win32" --hash=sha512:{wheel_hash}
    "#})?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert_eq!(
        fs_err::read_dir(context.cache_dir.join("packed-v0"))?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .filter(|entry| entry.path().join("metadata.msgpack").exists())
            .count(),
        1
    );
    // Hashes on mutually exclusive marker alternatives must not be intersected.
    requirements.write_str(&formatdoc! {r#"
        --no-index
        --find-links links
        basic-package==0.1.0 ; sys_platform == "win32" --hash=sha512:{wheel_hash}
        basic-package==0.1.0 ; sys_platform != "win32" --hash=sha512:{sdist_hash}
    "#})?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (2 total)
    ");
    requirements.write_str(&formatdoc! {"
        --no-index
        --find-links links
        basic-package==0.1.0 --hash=sha512:{}
    ", "0".repeat(128)})?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No distributions found for `basic-package==0.1.0` matching the requested hashes and archive types
    ");
    assert!(!context.venv.exists());
    assert!(!context.cache_dir.join("archive-v0").exists());
    Ok(())
}

#[test]
fn download_requirements_rejects_unpinned() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("basic-package>=0.1.0\n")?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `uv download -r` requires exact `==` pins or archive URLs; found `basic-package>=0.1.0`
    ");
    requirements.write_str("basic-package==0.1.*\n")?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `uv download -r` requires exact `==` pins or archive URLs; found `basic-package==0.1.*`
    ");
    assert!(!context.cache_dir.join("packed-v0").exists());
    Ok(())
}

#[test]
fn download_requirements_rejects_constraints() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("constraints.txt")
        .write_str("basic-package==0.1.0\n")?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("-c constraints.txt\nbasic-package==0.1.0\n")?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Constraints are not supported by `uv download -r`; compile the requirements first
    ");
    assert!(!context.cache_dir.join("packed-v0").exists());
    Ok(())
}

#[test]
fn download_requirements_requires_hashes() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("--require-hashes\nbasic-package==0.1.0\n")?;
    uv_snapshot!(context.filters(), download(&context).args(["-r", "requirements.txt", "--offline"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: basic-package==0.1.0
    ");
    assert!(!context.cache_dir.join("packed-v0").exists());
    Ok(())
}
