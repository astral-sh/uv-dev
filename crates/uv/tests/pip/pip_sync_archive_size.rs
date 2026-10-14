//! Archive sizes in pylock installations.

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::formatdoc;
use sha2::{Digest, Sha256};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

use uv_test::archive::generate_source_archive;
use uv_test::packse::PackseServer;
use uv_test::{TestContext, uv_snapshot};

const WHEEL: &str = "a-1.0.0-py3-none-any.whl";
const SDIST: &str = "a-1.0.0.tar.gz";

struct GeneratedArtifact {
    filename: &'static str,
    url: String,
    size: usize,
    sha256: String,
}

impl GeneratedArtifact {
    fn new(server: &PackseServer, filename: &'static str) -> Self {
        let size = server
            .generated_file_size(filename)
            .unwrap_or_else(|| panic!("missing generated artifact: {filename}"));
        let sha256 = server
            .files()
            .find_map(|(name, hash)| (name == filename).then(|| hash.to_owned()))
            .unwrap_or_else(|| panic!("missing generated artifact hash: {filename}"));
        Self {
            filename,
            url: server.file_url(filename),
            size,
            sha256,
        }
    }

    fn size_filter(&self) -> (String, String) {
        (
            format!("downloaded {} bytes", self.size),
            format!("downloaded [SIZE:{}] bytes", self.filename),
        )
    }
}

fn setup(filename: &'static str) -> (TestContext, PackseServer, GeneratedArtifact) {
    let server = PackseServer::new("simple/single-package.toml");
    let artifact = GeneratedArtifact::new(&server, filename);
    let context = uv_test::test_context!("3.12")
        .with_filtered_http_retries()
        .with_filter(artifact.size_filter());
    (context, server, artifact)
}

/// Build without fetching backend dependencies from the test server's vendor cache.
async fn self_contained_sdist() -> Result<(TestContext, MockServer, GeneratedArtifact)> {
    let archive = generate_source_archive(&"a".parse()?, &"1.0.0".parse()?, "", None)?;
    let server = MockServer::start().await;
    let artifact = GeneratedArtifact {
        filename: SDIST,
        url: format!("{}/files/{SDIST}", server.uri()),
        size: archive.len(),
        sha256: hex::encode(Sha256::digest(&archive)),
    };
    Mock::given(path(format!("/files/{SDIST}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "max-age=3600")
                .set_body_bytes(archive),
        )
        .mount(&server)
        .await;
    let context = uv_test::test_context!("3.12")
        .with_filtered_http_retries()
        .with_filter(artifact.size_filter());
    Ok((context, server, artifact))
}

#[test]
fn pep_751_validates_remote_archive_size() -> Result<()> {
    let (context, _server, wheel) = setup(WHEEL);
    context
        .temp_dir
        .child("pylock.toml")
        .write_str(&formatdoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "a"
        version = "1.0.0"
        archive = {{ url = "{url}", size = 1, hashes = {{ sha256 = "{sha256}" }} }}
        "#,
            url = wheel.url,
            sha256 = wheel.sha256,
        })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Size mismatch for `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`: expected 1 bytes, but downloaded [SIZE:a-1.0.0-py3-none-any.whl] bytes
    ");
    context.assert_not_installed("a");
    Ok(())
}

#[test]
fn pep_751_validates_remote_registry_wheel_size() -> Result<()> {
    let (context, _server, wheel) = setup(WHEEL);
    context
        .temp_dir
        .child("pylock.toml")
        .write_str(&formatdoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "a"
        version = "1.0.0"
        wheels = [{{ url = "{url}", size = 1, hashes = {{ sha256 = "{sha256}" }} }}]
        "#,
            url = wheel.url,
            sha256 = wheel.sha256,
        })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `a==1.0.0`
      cause: Size mismatch for `a==1.0.0`: expected 1 bytes, but downloaded [SIZE:a-1.0.0-py3-none-any.whl] bytes
    ");
    context.assert_not_installed("a");
    Ok(())
}

#[test]
fn pep_751_validates_remote_registry_sdist_size() -> Result<()> {
    let (context, _server, sdist) = setup(SDIST);
    context
        .temp_dir
        .child("pylock.toml")
        .write_str(&formatdoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "a"
        version = "1.0.0"
        sdist = {{ url = "{url}", size = 1, hashes = {{ sha256 = "{sha256}" }} }}
        "#,
            url = sdist.url,
            sha256 = sdist.sha256,
        })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `a==1.0.0`
      cause: Size mismatch for `a==1.0.0`: expected 1 bytes, but downloaded [SIZE:a-1.0.0.tar.gz] bytes
    ");
    context.assert_not_installed("a");
    Ok(())
}

#[test]
fn pep_751_validates_cached_remote_archive_size() -> Result<()> {
    let (context, _server, wheel) = setup(WHEEL);

    context
        .pip_install()
        .arg(format!("a @ {}", wheel.url))
        .arg("--no-index")
        .assert()
        .success();

    context.reset_venv();

    let pylock = |size| {
        formatdoc! {
            r#"
            lock-version = "1.0"
            created-by = "uv"

            [[packages]]
            name = "a"
            version = "1.0.0"
            archive = {{ url = "{url}", size = {size}, hashes = {{ sha256 = "{sha256}" }} }}
            "#,
            url = wheel.url,
            sha256 = wheel.sha256,
        }
    };
    let lockfile = context.temp_dir.child("pylock.toml");
    lockfile.write_str(&pylock(wheel.size))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("pylock.toml"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 1 package in [TIME]
     + a==1.0.0 (from http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl)
    "#);
    context.assert_installed("a", "1.0.0");

    context.reset_venv();
    lockfile.write_str(&pylock(1))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Size mismatch for `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`: expected 1 bytes, but downloaded [SIZE:a-1.0.0-py3-none-any.whl] bytes
    ");
    context.assert_not_installed("a");

    Ok(())
}

#[test]
fn pep_751_validates_cached_registry_wheel_size() -> Result<()> {
    let (context, server, wheel) = setup(WHEEL);
    let pylock = |size| {
        formatdoc! {
            r#"
            lock-version = "1.0"
            created-by = "uv"

            [[packages]]
            name = "a"
            version = "1.0.0"
            index = "{index}"
            wheels = [{{ url = "{url}", size = {size}, hashes = {{ sha256 = "{sha256}" }} }}]
            "#,
            index = server.index_url(),
            url = wheel.url,
            sha256 = wheel.sha256,
        }
    };
    let lockfile = context.temp_dir.child("pylock.toml");
    lockfile.write_str(&pylock(wheel.size))?;
    context
        .pip_sync()
        .arg("--preview")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("pylock.toml")
        .assert()
        .success();

    context.reset_venv();
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--offline")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 1 package in [TIME]
     + a==1.0.0
    ");
    context.assert_installed("a", "1.0.0");

    context.reset_venv();
    lockfile.write_str(&pylock(1))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--offline")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `a==1.0.0`
      cause: Size mismatch for `a==1.0.0`: expected 1 bytes, but downloaded [SIZE:a-1.0.0-py3-none-any.whl] bytes
    ");
    context.assert_not_installed("a");

    Ok(())
}

#[tokio::test]
async fn pep_751_validates_cached_registry_sdist_size() -> Result<()> {
    let (context, server, sdist) = self_contained_sdist().await?;
    let index_url = format!("{}/simple/", server.uri());
    let pylock = |size| {
        formatdoc! {
            r#"
            lock-version = "1.0"
            created-by = "uv"

            [[packages]]
            name = "a"
            version = "1.0.0"
            index = "{index}"
            sdist = {{ url = "{url}", size = {size}, hashes = {{ sha256 = "{sha256}" }} }}
            "#,
            index = index_url,
            url = sdist.url,
            sha256 = sdist.sha256,
        }
    };
    let lockfile = context.temp_dir.child("pylock.toml");
    lockfile.write_str(&pylock(sdist.size))?;
    context
        .pip_sync()
        .arg("--preview")
        .arg("--index-url")
        .arg(&index_url)
        .arg("pylock.toml")
        .assert()
        .success();

    context.reset_venv();
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--index-url")
        .arg(&index_url)
        .arg("--offline")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 1 package in [TIME]
     + a==1.0.0
    ");
    context.assert_installed("a", "1.0.0");

    context.reset_venv();
    lockfile.write_str(&pylock(1))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--index-url")
        .arg(&index_url)
        .arg("--offline")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `a==1.0.0`
      cause: Size mismatch for `a==1.0.0`: expected 1 bytes, but downloaded [SIZE:a-1.0.0.tar.gz] bytes
    ");
    context.assert_not_installed("a");

    Ok(())
}

#[tokio::test]
async fn pep_751_validates_cached_url_sdist_size() -> Result<()> {
    let (context, _server, sdist) = self_contained_sdist().await?;
    let pylock = |size| {
        formatdoc! {
            r#"
            lock-version = "1.0"
            created-by = "uv"

            [[packages]]
            name = "a"
            version = "1.0.0"
            archive = {{ url = "{url}", size = {size}, hashes = {{ sha256 = "{sha256}" }} }}
            "#,
            url = sdist.url,
            sha256 = sdist.sha256,
        }
    };
    let lockfile = context.temp_dir.child("pylock.toml");
    lockfile.write_str(&pylock(sdist.size))?;
    context
        .pip_sync()
        .arg("--preview")
        .arg("--no-index")
        .arg("pylock.toml")
        .assert()
        .success();

    context.reset_venv();
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("pylock.toml"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 1 package in [TIME]
     + a==1.0.0 (from http://[LOCALHOST]/files/a-1.0.0.tar.gz)
    "#);
    context.assert_installed("a", "1.0.0");

    context.reset_venv();
    lockfile.write_str(&pylock(1))?;
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("pylock.toml"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `a @ http://[LOCALHOST]/files/a-1.0.0.tar.gz`
      cause: Size mismatch for `a @ http://[LOCALHOST]/files/a-1.0.0.tar.gz`: expected 1 bytes, but downloaded [SIZE:a-1.0.0.tar.gz] bytes
    "#);
    context.assert_not_installed("a");

    Ok(())
}
