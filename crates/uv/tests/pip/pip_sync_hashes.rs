use anyhow::Result;
use assert_fs::prelude::*;
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_cache::{Cache, CacheBucket};
use uv_static::EnvVars;
use uv_test::archive::generate_source_archive;
use uv_test::{TestContext, uv_snapshot};

/// A wheel and source archive whose advertised hashes differ from their contents.
struct MisleadingIndex {
    context: TestContext,
    server: MockServer,
    wheel_hash: String,
    unrelated_hash: String,
    advertised_wheel_hash: String,
    advertised_source_hash: String,
}

impl MisleadingIndex {
    async fn new() -> Result<Self> {
        let context = uv_test::test_context!("3.12").with_filtered_http_retries();
        let wheel = "basic_package-0.1.0-py3-none-any.whl";
        let source = "basic_package-0.1.0.tar.gz";
        let wheel_bytes = context.read_bytes(context.workspace_root.join("test/links").join(wheel));
        let source_bytes =
            generate_source_archive(&"basic-package".parse()?, &"0.1.0".parse()?, "", None)?;
        let wheel_hash = hex::encode(Sha256::digest(&wheel_bytes));
        let source_hash = hex::encode(Sha256::digest(&source_bytes));
        let unrelated_hash = "0".repeat(64);
        let advertised_wheel_hash = "1".repeat(64);
        let advertised_source_hash = "f".repeat(64);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/basic-package/"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"<a href="/{wheel}#sha256={advertised_wheel_hash}">{wheel}</a><a href="/{source}#sha256={advertised_source_hash}">{source}</a>"#
                ),
                "text/html",
            ))
            .mount(&server)
            .await;
        for (filename, bytes) in [(wheel, wheel_bytes), (source, source_bytes)] {
            Mock::given(method("GET"))
                .and(path(format!("/{filename}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
                .mount(&server)
                .await;
        }

        let context = context
            .with_filter((wheel_hash.clone(), "[WHEEL_SHA256]"))
            .with_filter((source_hash, "[SOURCE_SHA256]"))
            .with_filter((unrelated_hash.clone(), "[UNRELATED_HASH]"))
            .with_filter((advertised_wheel_hash.clone(), "[ADVERTISED_WHEEL_HASH]"))
            .with_filter((advertised_source_hash.clone(), "[ADVERTISED_SOURCE_HASH]"));
        Ok(Self {
            context,
            server,
            wheel_hash,
            unrelated_hash,
            advertised_wheel_hash,
            advertised_source_hash,
        })
    }
}

#[tokio::test]
async fn require_hashes_find_links_invalid_hash() -> Result<()> {
    let fixture = MisleadingIndex::new().await?;
    let context = &fixture.context;
    let requirements = context.temp_dir.child("requirements.txt");
    let index_url = format!("{}/simple/basic-package/", fixture.server.uri());

    requirements.write_str(&format!(
        "basic-package==0.1.0 --hash=sha256:{}",
        fixture.unrelated_hash
    ))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--no-index", "--find-links"])
        .arg(&index_url), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:[UNRELATED_HASH]

             Computed:
               sha256:[WHEEL_SHA256]
    ");

    requirements.write_str(&format!(
        "basic-package==0.1.0 --hash=sha256:{}",
        fixture.advertised_wheel_hash
    ))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--no-index", "--find-links"])
        .arg(&index_url), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:[ADVERTISED_WHEEL_HASH]

             Computed:
               sha256:[WHEEL_SHA256]
    ");
    uv_test::assert_path_missing(
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::Archive),
    );

    // An explicit correct hash overrides the misleading index hash, including on refresh.
    requirements.write_str(&format!(
        "basic-package==0.1.0 --hash=sha256:{}",
        fixture.wheel_hash
    ))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--no-index", "--find-links"])
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0
    ");
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--refresh", "--no-index", "--find-links"])
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ basic-package==0.1.0
    ");

    // The matching advertised source hash selects the source even though the wheel is valid.
    requirements.write_str(&format!(
        "basic-package==0.1.0 --hash=sha256:{} --hash=sha256:{}",
        fixture.wheel_hash, fixture.advertised_source_hash
    ))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--refresh", "--no-index", "--find-links"])
        .arg(&index_url), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download and build `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:[WHEEL_SHA256]
               sha256:[ADVERTISED_SOURCE_HASH]

             Computed:
               sha256:[SOURCE_SHA256]
    ");
    context
        .assert_command("from importlib.metadata import version; import basic_package; print(version('basic-package'), basic_package.hello())")
        .success()
        .stdout("0.1.0 Hello from basic-package!\n");
    Ok(())
}

#[tokio::test]
async fn require_hashes_registry_invalid_hash() -> Result<()> {
    let fixture = MisleadingIndex::new().await?;
    let context = &fixture.context;
    let requirements = context.temp_dir.child("requirements.txt");
    let index_url = format!("{}/simple/", fixture.server.uri());

    requirements.write_str(&format!(
        "basic-package==0.1.0 --hash=sha256:{}",
        fixture.unrelated_hash
    ))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--index-url"])
        .arg(&index_url), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:[UNRELATED_HASH]

             Computed:
               sha256:[WHEEL_SHA256]
    ");

    requirements.write_str(&format!(
        "basic-package==0.1.0 --hash=sha256:{}",
        fixture.advertised_wheel_hash
    ))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--index-url"])
        .arg(&index_url), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:[ADVERTISED_WHEEL_HASH]

             Computed:
               sha256:[WHEEL_SHA256]
    ");
    uv_test::assert_path_missing(
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::Archive),
    );

    // An explicit correct hash overrides the misleading index hash, including on refresh.
    requirements.write_str(&format!(
        "basic-package==0.1.0 --hash=sha256:{}",
        fixture.wheel_hash
    ))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--index-url"])
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0
    ");
    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--refresh", "--index-url"])
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ basic-package==0.1.0
    ");

    // The matching advertised source hash selects the source even though the wheel is valid.
    requirements.write_str(&format!(
        "basic-package==0.1.0 --hash=sha256:{} --hash=sha256:{}",
        fixture.wheel_hash, fixture.advertised_source_hash
    ))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .args(["requirements.txt", "--reinstall", "--require-hashes", "--refresh", "--index-url"])
        .arg(&index_url), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download and build `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:[WHEEL_SHA256]
               sha256:[ADVERTISED_SOURCE_HASH]

             Computed:
               sha256:[SOURCE_SHA256]
    ");
    context
        .assert_command("from importlib.metadata import version; import basic_package; print(version('basic-package'), basic_package.hello())")
        .success()
        .stdout("0.1.0 Hello from basic-package!\n");
    Ok(())
}
