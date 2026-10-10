use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use sha2::{Digest, Sha256};
use wiremock::matchers::{basic_auth, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_cache::{Cache, CacheBucket, WheelCache};
use uv_distribution_types::IndexUrl;
use uv_redacted::DisplaySafeUrl;
use uv_static::EnvVars;
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

fn wheel(revision: &str) -> Result<Vec<u8>> {
    let (_, wheel) = uv_test::packse::generate_wheel_with_files(
        &"basic-package".parse()?,
        &"0.1.0".parse()?,
        &[],
        &std::collections::BTreeMap::new(),
        None,
        "py3-none-any",
        &[(
            "basic_package/revision.py",
            &format!("REVISION = {revision:?}\n"),
        )],
    );
    Ok(wheel)
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
    let sdist = source_archive(&wheel)?;
    let files = [
        ("basic_package-0.1.0-py3-none-any.whl", wheel.clone()),
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
            {{ url = "{url}/files/basic_package-0.1.0-py3-none-any.whl", hash = "sha256:{wheel_hash}", size = {wheel_size} }},
            {{ url = "{url}/files/basic_package-0.1.0-cp313-cp313-win_amd64.whl", hash = "sha256:{wheel_hash}", size = {wheel_size} }},
        ]
    "#, sdist_size=1, wheel_size=2},
    )?;
    let original_lock = fs_err::read(context.temp_dir.join("uv.lock"))?;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 3 distributions (3 total)
    ");
    assert_eq!(
        fs_err::read(context.temp_dir.join("uv.lock"))?,
        original_lock
    );
    let cache = Cache::from_path(context.cache_dir.path());
    context
        .cache_dir
        .child(cache.bucket(CacheBucket::Wheels))
        .assert(predicates::path::missing());
    context
        .cache_dir
        .child(cache.bucket(CacheBucket::Archive))
        .assert(predicates::path::missing());
    let index = IndexUrl::from(uv_pep508::VerbatimUrl::parse_url(format!("{url}/simple"))?);
    let packed = cache
        .bucket(CacheBucket::Packed)
        .join(WheelCache::Index(&index).wheel_dir("basic-package"));
    for (hash, bytes) in [(&wheel_hash, &wheel), (&sdist_hash, &sdist)] {
        assert_eq!(fs_err::read(packed.join(hash))?, *bytes);
    }
    for key in [
        "0.1.0-py3-none-any.whl",
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
    Downloaded 0 distributions (3 total)
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

/// A refreshed wheel replaces an already prepared revision during offline installation.
#[tokio::test]
async fn download_replaces_prepared_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let url = server.uri();
    let filename = "basic_package-0.1.0-py3-none-any.whl";
    let bytes = wheel("old")?;
    let hash = digest(&bytes);
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ registry = "{url}/simple" }}
        wheels = [{{ url = "{url}/{filename}", hash = "sha256:{hash}" }}]
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
    uv_snapshot!(context.filters(), download(&context).arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    server.verify().await;
    server.reset().await;
    uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0
    ");
    let bytes = wheel("replacement")?;
    let hash = digest(&bytes);
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ registry = "{url}/simple" }}
        wheels = [{{ url = "{url}/{filename}", hash = "sha256:{hash}" }}]
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
    uv_snapshot!(context.filters(), download(&context).arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    server.verify().await;
    server.reset().await;
    drop(server);
    uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--offline", "--reinstall"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ basic-package==0.1.0
    ");
    uv_snapshot!(context.filters(), context.python_command().args(["-c", "from basic_package.revision import REVISION; print(REVISION)"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    replacement
    ");
    Ok(())
}

/// A refreshed sdist replaces an already prepared revision during offline installation.
#[tokio::test]
async fn download_replaces_prepared_sdist() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let url = server.uri();
    let filename = "basic_package-0.1.0.tar.gz";
    let bytes = source_archive(&wheel("old")?)?;
    let hash = digest(&bytes);
    let size = bytes.len();
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ registry = "{url}/simple" }}
        sdist = {{ url = "{url}/{filename}", hash = "sha256:{hash}", size = {size} }}
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
    uv_snapshot!(context.filters(), download(&context).arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    server.verify().await;
    server.reset().await;
    uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0
    ");
    let bytes = source_archive(&wheel("replacement")?)?;
    let hash = digest(&bytes);
    let size = bytes.len();
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ registry = "{url}/simple" }}
        sdist = {{ url = "{url}/{filename}", hash = "sha256:{hash}", size = {size} }}
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
    uv_snapshot!(context.filters(), download(&context).arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    server.verify().await;
    server.reset().await;
    drop(server);
    uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--offline", "--reinstall"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ basic-package==0.1.0
    ");
    uv_snapshot!(context.filters(), context.python_command().args(["-c", "from basic_package.revision import REVISION; print(REVISION)"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    replacement
    ");
    Ok(())
}

#[tokio::test]
async fn download_refresh() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = wheel("original")?;
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
        .expect(2)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").args(["--refresh"]), @"
        exit_code: 0 (success)
        ----- stderr -----
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + basic-package==0.1.0
        ");
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn download_refresh_package() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = wheel("original")?;
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
        .expect(2)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").args(["--refresh-package", "basic-package"]), @"
        exit_code: 0 (success)
        ----- stderr -----
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + basic-package==0.1.0
        ");
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn download_refresh_other_package() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = wheel("original")?;
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
        .expect(1)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").args(["--refresh-package", "other-package"]), @"
        exit_code: 0 (success)
        ----- stderr -----
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + basic-package==0.1.0
        ");
    server.verify().await;
    Ok(())
}

/// Metadata refresh must not use a stale packed response.
#[tokio::test]
async fn download_refresh_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let original = wheel("original")?;
    let hash = digest(&original);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &hash)?;
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(original),
        )
        .expect(1)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    server.verify().await;
    server.reset().await;

    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let (filename, dependency) = uv_test::packse::generate_wheel(
        &"dependency".parse()?,
        &"1.0.0".parse()?,
        &[],
        &std::collections::BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    links.child(filename).write_binary(&dependency)?;
    let (_, replacement) = uv_test::packse::generate_wheel(
        &"basic-package".parse()?,
        &"0.1.0".parse()?,
        &["dependency==1".parse()?],
        &std::collections::BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    Mock::given(method("HEAD"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Length", replacement.len().to_string()),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/basic_package-0.1.0-py3-none-any.whl"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(replacement),
        )
        .expect(1)
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!("basic-package @ {url}"))?;
    // The fixture server falls back from range requests to a full metadata read.
    let mut filters = context.filters();
    filters.push((r"(?m)^WARN Range requests not supported[^\n]*\n", ""));
    uv_snapshot!(filters, context.pip_compile().args([
        "requirements.in", "--refresh", "--no-header", "--no-index", "--find-links", "links",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    basic-package @ http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
        # via -r requirements.in
    dependency==1.0.0
        # via basic-package

    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let shard = packed_url_shard(&context, &url)?;
    context
        .cache_dir
        .child(shard.join("0.1.0-py3-none-any.whl.http"))
        .assert(predicates::path::missing());
    uv_snapshot!(context.filters(), context.pip_compile().args([
        "requirements.in", "--no-header", "--no-index", "--find-links", "links",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    basic-package @ http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
        # via -r requirements.in
    dependency==1.0.0
        # via basic-package

    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.pip_compile().args([
        "requirements.in", "--no-header", "--no-index", "--find-links", "links", "--offline",
    ]), @"
    exit_code: 0 (success)
    ----- stdout -----
    basic-package @ http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
        # via -r requirements.in
    dependency==1.0.0
        # via basic-package

    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn download_credentials_dependency() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = wheel("original")?;
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
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["basic-package @ {authenticated_url}"]
    "#})?;
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
    uv_snapshot!(context.filters(), download(&context), @"
        exit_code: 0 (success)
        ----- stderr -----
        Downloaded 1 distributions (1 total)
        ");
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn download_credentials_source() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = wheel("original")?;
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
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["basic-package"]
        [tool.uv.sources]
        basic-package = {{ url = "{authenticated_url}" }}
    "#})?;
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
    uv_snapshot!(context.filters(), download(&context), @"
        exit_code: 0 (success)
        ----- stderr -----
        Downloaded 1 distributions (1 total)
        ");
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn download_credentials_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = wheel("original")?;
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
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["basic-package"]
        [tool.uv.workspace]
        members = ["member", "missing-member"]
    "#})?;
    context
        .temp_dir
        .child("member/pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "member"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = ["basic-package @ {authenticated_url}"]
    "#})?;
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
    uv_snapshot!(context.filters(), download(&context), @"
        exit_code: 0 (success)
        ----- stderr -----
        Downloaded 1 distributions (1 total)
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
    uv_snapshot!(context.filters(), download(&context), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      cause: Hash mismatch for packed archive at `http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl`

             Expected:
               sha256:0000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:e7dc6be13be0055dd03d1fe10caf78db0daa6327eb1f5b08c3e2dc8c06431cd2
    "#);
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
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      cause: Hash mismatch for packed archive at `http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl`

             Expected:
               sha256:7b6229db79b5800e4e98a351b5628c1c8a944533a2d428aeeaa7275a30d4ea82

             Computed:
               sha256:11d510e067d2cdcd7559bd86d27a2f4c20babd43670346b97af99b522c1f0075
    "#);
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
    let bytes = wheel("original")?;
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
    let pypi_shard = cache.bucket(CacheBucket::Packed).join("pypi/basic-package");
    write_locked_wheel(
        &context,
        "registry = \"https://pypi.org/simple\"",
        &url,
        &hash,
    )?;
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
    ");
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert_eq!(fs_err::read(pypi_shard.join(&hash))?, bytes);
    assert!(pypi_shard.join("0.1.0-py3-none-any.whl.http").is_file());
    assert!(!pypi_shard.join("package").exists());
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions (1 total)
    ");

    let custom_shard = cache
        .bucket(CacheBucket::Packed)
        .join(WheelCache::Index(&index).wheel_dir("basic-package"));
    write_locked_wheel(
        &context,
        &format!("registry = \"{}/simple\"", server.uri()),
        &url,
        &hash,
    )?;
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
    ");
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert_eq!(fs_err::read(custom_shard.join(&hash))?, bytes);
    assert!(custom_shard.join("0.1.0-py3-none-any.whl.http").is_file());
    assert!(!custom_shard.join("package").exists());
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions (1 total)
    ");

    let direct_shard = packed_url_shard(&context, &url)?;
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &hash)?;
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
    ");
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert_eq!(fs_err::read(direct_shard.join(&hash))?, bytes);
    assert!(direct_shard.join("0.1.0-py3-none-any.whl.http").is_file());
    assert!(!direct_shard.join("package").exists());
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions (1 total)
    ");

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
    context.prune().assert().success();
    assert!(!context.cache_dir.join("packed-v0").exists());
    download(&context).arg("--offline").assert().success();
    context.clean().arg("basic-package").assert().success();
    assert!(!pypi_shard.exists());
    assert!(!custom_shard.exists());
    assert!(!direct_shard.exists());
    Ok(())
}

/// Revalidation uses the saved `ETag`, including after packed bytes produce a prepared wheel.
#[tokio::test]
async fn download_preserves_http_policy() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original")?;
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
    let bytes = wheel("original")?;
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
    let bytes = wheel("original")?;
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
      cause: Response for http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl does not permit caching
    ");
    let shard = packed_url_shard(&context, &url)?;
    assert!(!shard.join("0.1.0-py3-none-any.whl.http").exists());
    assert!(!shard.join(hash).exists());
    download(&context).arg("--offline").assert().failure();
    server.verify().await;
    Ok(())
}

/// Revalidation cannot promise offline reuse after the server withdraws cacheability.
#[tokio::test]
async fn download_revalidation_no_store() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original")?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &hash)?;
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
                .insert_header("cache-control", "no-store")
                .insert_header("etag", "\"original\""),
        )
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      cause: Response for http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl does not permit caching
    ");
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `basic-package` from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl
    ");
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
    let bytes = wheel("original")?;
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
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert_eq!(fs_err::read(shard.join(digest(&bytes)))?, bytes);
    assert!(shard.join("0.1.0-py3-none-any.whl.rev").is_file());
    assert!(!shard.join("0.1.0-py3-none-any.whl.http").exists());
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions (1 total)
    ");

    let bytes = wheel("replacement")?;
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
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    assert_eq!(fs_err::read(shard.join(digest(&bytes)))?, bytes);
    assert!(shard.join("0.1.0-py3-none-any.whl.rev").is_file());
    assert!(!shard.join("0.1.0-py3-none-any.whl.http").exists());
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions (1 total)
    ");
    context
        .command()
        .args(["cache", "clean", "basic-package"])
        .assert()
        .success();
    assert!(!shard.exists());
    Ok(())
}

/// Local wheel metadata and installation can use a prefetched archive after its source is removed.
#[tokio::test]
async fn download_removed_local_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let filename = "basic_package-0.1.0-py3-none-any.whl";
    let bytes = wheel("prefetched")?;
    let hash = digest(&bytes);
    let path = context.temp_dir.child(filename);
    path.write_binary(&bytes)?;
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
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            "{}\n[tool.uv.sources]\nbasic-package = {{ path = \"{filename}\" }}\n",
            context.read("pyproject.toml"),
        ))?;
    download(&context).arg("--offline").assert().success();
    fs_err::remove_file(path.path())?;
    download(&context).arg("--offline").assert().success();

    uv_snapshot!(context.filters(), context.sync().arg("--offline"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0 (from file://[TEMP_DIR]/basic_package-0.1.0-py3-none-any.whl)
    ");
    uv_snapshot!(context.filters(), context.run().args(["--offline", "python", "-c", "from basic_package.revision import REVISION; print(REVISION)"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    prefetched

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 1 package in [TIME]
    ");
    Ok(())
}

/// A local source archive can be built from its prefetched bytes after its source is removed.
#[tokio::test]
async fn download_removed_local_sdist() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let filename = "basic_package-0.1.0.tar.gz";
    let bytes = source_archive(&wheel("prefetched")?)?;
    let hash = digest(&bytes);
    let path = context.temp_dir.child(filename);
    path.write_binary(&bytes)?;
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ path = "{filename}" }}
        sdist = {{ hash = "sha256:{hash}" }}
    "#},
    )?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            "{}\n[tool.uv.sources]\nbasic-package = {{ path = \"{filename}\" }}\n",
            context.read("pyproject.toml"),
        ))?;
    download(&context).arg("--offline").assert().success();
    fs_err::remove_file(path.path())?;
    download(&context).arg("--offline").assert().success();

    uv_snapshot!(context.filters(), context.sync().arg("--offline"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0 (from file://[TEMP_DIR]/basic_package-0.1.0.tar.gz)
    ");
    uv_snapshot!(context.filters(), context.run().args(["--offline", "python", "-c", "from basic_package.revision import REVISION; print(REVISION)"]), @r"
    exit_code: 0 (success)
    ----- stdout -----
    prefetched

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 1 package in [TIME]
    ");
    Ok(())
}

/// A prepared pointer may survive removal of its extracted payload.
#[tokio::test]
async fn download_repairs_missing_prepared_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original")?;
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
    let bytes = source_archive(&wheel("original")?)?;
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
    let cache = Cache::from_path(context.cache_dir.path());
    let shard = cache
        .bucket(CacheBucket::SourceDistributions)
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
    let bytes = wheel("original")?;
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

/// Corrupt optional packed metadata falls back to the online archive.
#[tokio::test]
async fn download_corrupt_http_metadata_falls_back_online() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("online")?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &hash)?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(bytes),
        )
        .expect(2)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    fs_err::write(
        packed_url_shard(&context, &url)?.join("0.1.0-py3-none-any.whl.http"),
        b"invalid HTTP cache metadata",
    )?;
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0 (from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl)
    ");
    server.verify().await;
    Ok(())
}

/// Fragment-bearing source archives share their packed identity with offline preparation.
#[tokio::test]
async fn download_source_subdirectory_offline() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let wheel = wheel("subdirectory")?;
    let mut bytes = Vec::new();
    write_tar_gz(
        &mut bytes,
        &[
            (
                "archive/package/pyproject.toml",
                indoc! {r#"
            [build-system]
            requires = []
            build-backend = "backend"
            backend-path = ["."]
        "#}
                .as_bytes(),
            ),
            (
                "archive/package/backend.py",
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
            ("archive/package/prebuilt.whl", &wheel),
            (
                "archive/package/PKG-INFO",
                b"Metadata-Version: 2.2\nName: basic-package\nVersion: 0.1.0\n",
            ),
        ],
    )?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0.tar.gz", server.uri());
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ url = "{url}", subdirectory = "package" }}
        sdist = {{ hash = "sha256:{hash}" }}
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
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0 (from http://[LOCALHOST]/basic_package-0.1.0.tar.gz#subdirectory=package)
    ");
    server.verify().await;
    Ok(())
}

/// Pruning retains payloads until every HTTP pointer to them has been removed.
#[tokio::test]
async fn download_prunes_unreferenced_http_payloads() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    let original = wheel("original")?;
    let old_hash = digest(&original);
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &old_hash)?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(original),
        )
        .expect(1)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    server.verify().await;
    server.reset().await;
    let shard = packed_url_shard(&context, &url)?;
    let retained = shard.join("retained.http");
    fs_err::copy(shard.join("0.1.0-py3-none-any.whl.http"), &retained)?;
    let replacement = wheel("replacement")?;
    let new_hash = digest(&replacement);
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &new_hash)?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(replacement),
        )
        .expect(1)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context).arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    context
        .command()
        .args(["cache", "prune"])
        .assert()
        .success();
    assert!(shard.join(&old_hash).is_file());
    assert!(shard.join(&new_hash).is_file());
    fs_err::remove_file(retained)?;
    context
        .command()
        .args(["cache", "prune"])
        .assert()
        .success();
    assert!(!shard.join(old_hash).exists());
    assert!(shard.join(new_hash).is_file());
    server.verify().await;
    Ok(())
}

/// Pruning also follows every retained local revision pointer in a shard.
#[test]
fn download_prunes_unreferenced_local_payloads() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let filename = "basic_package-0.1.0-py3-none-any.whl";
    let path = context.temp_dir.child(filename);
    let url = DisplaySafeUrl::from_file_path(path.path()).expect("absolute file path");
    let cache = Cache::from_path(context.cache_dir.path());
    let shard = cache
        .bucket(CacheBucket::Packed)
        .join(WheelCache::Path(&url).wheel_dir("basic-package"));
    let original = wheel("original")?;
    let old_hash = digest(&original);
    path.write_binary(&original)?;
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ path = "{filename}" }}
        wheels = [{{ filename = "{filename}", hash = "sha256:{old_hash}" }}]
    "#},
    )?;
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    let retained = shard.join("retained.rev");
    fs_err::copy(shard.join("0.1.0-py3-none-any.whl.rev"), &retained)?;
    let replacement = wheel("replacement")?;
    let new_hash = digest(&replacement);
    path.write_binary(&replacement)?;
    write_project(
        &context,
        &formatdoc! {r#"
        [[package]]
        name = "basic-package"
        version = "0.1.0"
        source = {{ path = "{filename}" }}
        wheels = [{{ filename = "{filename}", hash = "sha256:{new_hash}" }}]
    "#},
    )?;
    uv_snapshot!(context.filters(), download(&context).arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    context
        .command()
        .args(["cache", "prune"])
        .assert()
        .success();
    assert!(shard.join(&old_hash).is_file());
    assert!(shard.join(&new_hash).is_file());
    fs_err::remove_file(retained)?;
    context
        .command()
        .args(["cache", "prune"])
        .assert()
        .success();
    assert!(!shard.join(old_hash).exists());
    assert!(shard.join(new_hash).is_file());
    Ok(())
}

/// Refresh retains an index response policy override while forcing a new request.
#[tokio::test]
async fn download_refresh_retains_file_cache_override() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original")?;
    let hash = digest(&bytes);
    let index = format!("{}/simple", server.uri());
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(&context, &format!("registry = \"{index}\""), &url, &hash)?;
    context
        .temp_dir
        .child("uv.toml")
        .write_str(&formatdoc! {r#"
        [[index]]
        url = "{index}"
        cache-control = {{ files = "max-age=3600" }}
    "#})?;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "no-store")
                .set_body_bytes(bytes),
        )
        .expect(2)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    uv_snapshot!(context.filters(), download(&context).arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 0 distributions (1 total)
    ");
    server.verify().await;
    Ok(())
}

/// Prefetching a lockfile does not normalize or warn about resolver-only prerelease settings.
#[test]
fn download_ignores_resolver_only_settings() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let filename = "basic_package-0.1.0-py3-none-any.whl";
    let bytes = wheel("original")?;
    let hash = digest(&bytes);
    context.temp_dir.child(filename).write_binary(&bytes)?;
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
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            "{}\n[tool.uv]\nprerelease = \"if-necessary-or-explicit\"\n",
            context.read("pyproject.toml"),
        ))?;
    uv_snapshot!(context.filters(), download(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    Ok(())
}

/// Fresh PEP 658 metadata does not supersede an expired archive retained for offline preparation.
#[tokio::test]
async fn download_retains_archive_on_cached_pep658_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original")?;
    let hash = digest(&bytes);
    let filename = "basic_package-0.1.0-py3-none-any.whl";
    let url = format!("{}/{filename}", server.uri());
    let index = format!("{}/simple/", server.uri());
    let metadata = "Metadata-Version: 2.3\nName: basic-package\nVersion: 0.1.0\n";
    write_locked_wheel(&context, &format!("registry = \"{index}\""), &url, &hash)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("basic-package==0.1.0\n")?;
    Mock::given(method("GET"))
        .and(path("/simple/basic-package/"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("cache-control", "public, max-age=3600")
            .set_body_raw(format!(r#"<a href="{url}#sha256={hash}" data-core-metadata="sha256={}">{filename}</a>"#, digest(metadata.as_bytes())).into_bytes(), "text/html"))
        .expect(1).mount(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}.metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_string(metadata),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=0")
                .set_body_bytes(bytes),
        )
        .expect(1)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), context.pip_compile().args(["requirements.in", "--no-header", "--no-annotate"])
        .arg("--index-url").arg(&index).env_remove(EnvVars::UV_EXCLUDE_NEWER), @"
    exit_code: 0 (success)
    ----- stdout -----
    basic-package==0.1.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    uv_snapshot!(context.filters(), context.pip_compile().args(["requirements.in", "--no-header", "--no-annotate"])
        .arg("--index-url").arg(&index).env_remove(EnvVars::UV_EXCLUDE_NEWER), @"
    exit_code: 0 (success)
    ----- stdout -----
    basic-package==0.1.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0
    ");
    server.verify().await;
    Ok(())
}

/// Cached metadata obtained from a wheel does not discard a later prefetched archive.
#[tokio::test]
async fn download_retains_archive_on_cached_wheel_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original")?;
    let hash = digest(&bytes);
    let filename = "basic_package-0.1.0-py3-none-any.whl";
    let url = format!("{}/{filename}", server.uri());
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &hash)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!("basic-package @ {url}\n"))?;
    Mock::given(method("HEAD"))
        .and(path(format!("/{filename}")))
        .respond_with(
            ResponseTemplate::new(200).insert_header("content-length", bytes.len().to_string()),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=3600")
                .set_body_bytes(bytes.clone()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut filters = context.filters();
    filters.push((r"(?m)^WARN Range requests not supported[^\n]*\n", ""));
    uv_snapshot!(filters, context.pip_compile().args(["requirements.in", "--no-index", "--no-header", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    basic-package @ http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    server.verify().await;
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "public, max-age=0")
                .set_body_bytes(bytes),
        )
        .expect(1)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Downloaded 1 distributions (1 total)
    ");
    uv_snapshot!(context.filters(), context.pip_compile().args(["requirements.in", "--no-index", "--no-header", "--no-annotate"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    basic-package @ http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    uv_snapshot!(context.filters(), context.sync().args(["--frozen", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0 (from http://[LOCALHOST]/basic_package-0.1.0-py3-none-any.whl)
    ");
    server.verify().await;
    Ok(())
}

/// Prefetch needs persistent storage and rejects a temporary cache before requesting artifacts.
#[tokio::test]
async fn download_requires_persistent_cache() -> Result<()> {
    let context = uv_test::test_context!("3.13");
    let server = MockServer::start().await;
    let bytes = wheel("original")?;
    let hash = digest(&bytes);
    let url = format!("{}/basic_package-0.1.0-py3-none-any.whl", server.uri());
    write_locked_wheel(&context, &format!("url = \"{url}\""), &url, &hash)?;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .expect(0)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), download(&context).arg("--no-cache"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `uv download` requires caching to be enabled
    ");
    server.verify().await;
    Ok(())
}
