use std::fmt::Write;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use indoc::{formatdoc, indoc};
use insta::allow_duplicates;
use sha2::{Digest, Sha256};
use wiremock::matchers::{basic_auth, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_cache::{Cache, CacheBucket, WheelCache};
use uv_distribution_types::IndexUrl;
use uv_redacted::DisplaySafeUrl;
use uv_test::archive::write_tar_gz;
use uv_test::{TestContext, uv_snapshot};

fn packed_url_shard(context: &TestContext, url: &str) -> Result<std::path::PathBuf> {
    let url = DisplaySafeUrl::parse(url)?;
    Ok(context
        .cache_dir
        .join("packed-v1")
        .join(WheelCache::Url(&url).wheel_dir("basic-package")))
}

fn write_locked_wheel(context: &TestContext, source: &str, url: &str, hash: &str) -> Result<()> {
    write_project(
        context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ {source} }}
        wheels = [{{ url = "{url}", hash = "sha256:{hash}" }}]
    "#},
    )
}

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
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("cache-control", "public, max-age=3600")
                    .set_body_bytes(bytes.clone()),
            )
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
    let cache = Cache::from_path(context.cache_dir.path());
    let index = IndexUrl::from(uv_pep508::VerbatimUrl::parse_url(format!("{url}/simple"))?);
    let packed = cache
        .bucket(CacheBucket::Packed)
        .join(WheelCache::Index(&index).wheel_dir("basic-package"));
    for (hash, bytes) in [
        (&wheel_hash, &wheel),
        (&zstd_hash, &zstd),
        (&sdist_hash, &sdist),
    ] {
        assert_eq!(fs_err::read(packed.join(hash))?, *bytes);
    }
    for key in [
        "0.1.0-py3-none-any.whl",
        "0.1.0-py3-none-any.whl.tar.zst",
        "0.1.0-cp313-cp313-win_amd64.whl",
        "0.1.0.tar.gz",
    ] {
        assert!(packed.join(format!("{key}.http")).is_file());
    }
    assert!(!packed.join("package").exists());
    assert!(!packed.join("metadata.msgpack").exists());
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
    // A registry cache entry must not silently become a direct-URL dependency.
    context
        .pip_install()
        .arg("--offline")
        .arg("--reinstall")
        .arg(format!("{url}/files/basic_package-0.1.0-py3-none-any.whl"))
        .assert()
        .failure();
    context
        .command()
        .args(["cache", "clean", "basic-package"])
        .assert()
        .success();
    assert!(!packed.exists());
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
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("cache-control", "public, max-age=3600")
                    .set_body_bytes(bytes),
            )
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
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(wheel),
        )
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

/// Metadata refresh must not use a stale packed response.
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
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(wheel),
        )
        .expect(2)
        .mount(&server)
        .await;
    download(&context).assert().success();
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!("basic-package @ {url}"))?;
    // Metadata resolution races with the refreshed archive download. If the archive arrives first,
    // the range-request fallback is unnecessary and its warning is omitted.
    let mut filters = context.filters();
    filters.push((r"(?m)^WARN Range requests not supported[^\n]*\n", ""));
    uv_snapshot!(filters, context.pip_compile().arg("requirements.in").args(["--refresh", "--no-header"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    basic-package @ http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
        # via -r requirements.in

    ----- stderr -----
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
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(wheel),
        )
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

/// A mismatched digest must never become a reusable packed archive.
#[tokio::test]
async fn download_rejects_hash_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(b"not a wheel"),
        )
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
    assert!(
        !packed_url_shard(
            &context,
            &format!("{url}/basic_package-0.1.0-py3-none-any.whl")
        )?
        .join("0.1.0-py3-none-any.whl.http")
        .exists()
    );
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
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(wheel.clone()),
        )
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
    fs_err::write(
        packed_url_shard(
            &context,
            &format!("{url}/basic_package-0.1.0-py3-none-any.whl"),
        )?
        .join(&hash),
        b"corrupt",
    )?;
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

/// Index identity is part of the lookup, even when two indexes advertise the same artifact URL.
#[tokio::test]
async fn download_source_shards() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original").await?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(bytes.clone()),
        )
        .expect(3)
        .mount(&server)
        .await;

    let cache = Cache::from_path(context.cache_dir.path());
    let index = IndexUrl::from(uv_pep508::VerbatimUrl::parse_url(format!(
        "{}/simple",
        server.uri()
    ))?);
    let shards = [
        (
            "registry = \"https://pypi.org/simple\"".to_string(),
            cache.bucket(CacheBucket::Packed).join("pypi/basic-package"),
        ),
        (
            format!("registry = \"{}/simple\"", server.uri()),
            cache
                .bucket(CacheBucket::Packed)
                .join(WheelCache::Index(&index).wheel_dir("basic-package")),
        ),
        (
            format!("url = \"{url}\""),
            packed_url_shard(&context, &url)?,
        ),
    ];
    for (source, shard) in &shards {
        write_locked_wheel(&context, source, &url, &hash)?;
        download(&context).arg("--offline").assert().failure();
        download(&context).assert().success();
        assert_eq!(fs_err::read(shard.join(&hash))?, bytes);
        assert!(shard.join("0.1.0-py3-none-any.whl.http").is_file());
        assert!(!shard.join("package").exists());
        download(&context).arg("--offline").assert().success();
    }
    server.verify().await;
    drop(server);
    // The explicitly prefetched direct URL supplies wheel metadata and installation bytes.
    context
        .pip_install()
        .arg("--offline")
        .arg(&url)
        .assert()
        .success();
    // Pruning drops the incompatible prototype bucket without removing packed-v1 artifacts.
    context.cache_dir.child("packed-v0/old").create_dir_all()?;
    context
        .command()
        .args(["cache", "prune"])
        .assert()
        .success();
    assert!(!context.cache_dir.join("packed-v0").exists());
    download(&context).arg("--offline").assert().success();
    context
        .command()
        .args(["cache", "clean", "basic-package"])
        .assert()
        .success();
    for (_, shard) in shards {
        assert!(!shard.exists());
    }
    Ok(())
}

/// Revalidation uses the saved `ETag`, including after packed bytes produce a prepared wheel.
#[tokio::test]
async fn download_preserves_http_policy() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original").await?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(
        &context,
        &format!("registry = \"{}/simple\"", server.uri()),
        &url,
        &hash,
    )?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=0")
                .insert_header("etag", "\"original\"")
                .set_body_bytes(bytes),
        )
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(header("if-none-match", "\"original\""))
        .respond_with(
            ResponseTemplate::new(304)
                .insert_header("cache-control", "public, max-age=0")
                .insert_header("etag", "\"original\""),
        )
        .with_priority(1)
        .expect(2)
        .mount(&server)
        .await;
    download(&context).assert().success();
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions (1 total)
    ");
    context
        .sync()
        .args(["--frozen", "--offline"])
        .assert()
        .success();
    context
        .sync()
        .args(["--frozen", "--reinstall"])
        .assert()
        .success();
    server.verify().await;
    Ok(())
}

/// A stale packed archive is not made fresh just because no prepared HTTP pointer exists yet.
#[tokio::test]
async fn download_expired_packed_archive() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original").await?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(
        &context,
        &format!("registry = \"{}/simple\"", server.uri()),
        &url,
        &hash,
    )?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=0")
                .set_body_bytes(bytes),
        )
        .expect(2)
        .mount(&server)
        .await;
    download(&context).assert().success();
    context.sync().arg("--frozen").assert().success();
    server.verify().await;
    Ok(())
}

/// Prefetch cannot promise offline reuse when the server prohibits caching.
#[tokio::test]
async fn download_no_store() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original").await?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &hash)?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "no-store")
                .set_body_bytes(bytes),
        )
        .expect(1)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      Caused by: Response for http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl does not permit caching
    ");
    let shard = packed_url_shard(&context, &url)?;
    assert!(!shard.join("0.1.0-py3-none-any.whl.http").exists());
    assert!(!shard.join(hash).exists());
    download(&context).arg("--offline").assert().failure();
    server.verify().await;
    Ok(())
}

/// Local archives use timestamped revision pointers, not HTTP policies.
#[tokio::test]
async fn download_local_revision() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let filename = "basic_package-0.1.0-py3-none-any.whl";
    let path = context.temp_dir.join(filename);
    let url = DisplaySafeUrl::from_file_path(&path).expect("absolute file path");
    let shard = context
        .cache_dir
        .join("packed-v1")
        .join(WheelCache::Path(&url).wheel_dir("basic-package"));
    for revision in ["original", "replacement"] {
        let bytes = wheel(revision).await?;
        let hash = digest(&bytes);
        write_project(
            &context,
            &formatdoc! {r#"
            [[package]]
            name = "basic-package"
            version = "0.1.0"
            source = {{ path = "{filename}" }}
            wheels = [{{ filename = "{filename}", hash = "sha256:{hash}" }}]
        "#},
        )?;
        fs_err::write(&path, &bytes)?;
        allow_duplicates! {
            uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Downloaded 1 distributions (1 total)
            ");
        }
        assert_eq!(fs_err::read(shard.join(digest(&bytes)))?, bytes);
        assert!(shard.join("0.1.0-py3-none-any.whl.rev").is_file());
        assert!(!shard.join("0.1.0-py3-none-any.whl.http").exists());
        allow_duplicates! {
            uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Downloaded 0 distributions (1 total)
            ");
        }
    }
    context
        .command()
        .args(["cache", "clean", "basic-package"])
        .assert()
        .success();
    assert!(!shard.exists());
    Ok(())
}

/// A prepared pointer may survive removal of its extracted payload.
#[tokio::test]
async fn download_repairs_missing_prepared_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original").await?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(
        &context,
        &format!("registry = \"{}/simple\"", server.uri()),
        &url,
        &hash,
    )?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(bytes),
        )
        .expect(1)
        .mount(&server)
        .await;
    download(&context).assert().success();
    context
        .sync()
        .args(["--frozen", "--offline"])
        .assert()
        .success();
    fs_err::remove_dir_all(context.cache_dir.join("archive-v0"))?;
    context
        .sync()
        .args(["--frozen", "--offline", "--reinstall"])
        .assert()
        .success();
    server.verify().await;
    Ok(())
}

/// A cached source revision can recover its extracted source and built wheel from packed bytes.
#[tokio::test]
async fn download_repairs_missing_prepared_sdist() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = source_archive(&wheel("original").await?)?;
    let hash = digest(&bytes);
    let url = server.uri();
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ registry = "{url}/simple" }}
        sdist = {{ url = "{url}/basic_package-0.1.0.tar.gz", hash = "sha256:{hash}" }}
    "#},
    )?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(bytes),
        )
        .expect(1)
        .mount(&server)
        .await;
    download(&context).assert().success();
    context
        .sync()
        .args(["--frozen", "--offline"])
        .assert()
        .success();
    let index = IndexUrl::from(uv_pep508::VerbatimUrl::parse_url(format!("{url}/simple"))?);
    let shard = context
        .cache_dir
        .join("sdists-v9")
        .join(WheelCache::Index(&index).wheel_dir("basic-package"))
        .join("0.1.0");
    // Keep the revision HTTP pointer, but remove the extracted revision and its built wheels.
    for entry in fs_err::read_dir(shard)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            fs_err::remove_dir_all(entry.path())?;
        }
    }
    context
        .sync()
        .args(["--frozen", "--offline", "--reinstall"])
        .assert()
        .success();
    server.verify().await;
    Ok(())
}

/// HTTP metadata alone is not a cache hit if its packed payload has been removed.
#[tokio::test]
async fn download_repairs_missing_packed_archive() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original").await?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &hash)?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(bytes.clone()),
        )
        .expect(2)
        .mount(&server)
        .await;
    download(&context).assert().success();
    let archive = packed_url_shard(&context, &url)?.join(hash);
    fs_err::remove_file(&archive)?;
    download(&context).arg("--offline").assert().failure();
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert_eq!(fs_err::read(archive)?, bytes);
    server.verify().await;
    Ok(())
}
