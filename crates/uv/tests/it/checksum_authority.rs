use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use anyhow::{Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use ring::signature::Ed25519KeyPair;
use serde_json::json;
use sha2::{Digest, Sha256, Sha512};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;
use walkdir::WalkDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_cache::{Cache, CacheBucket, WheelCache};
use uv_checksum_authority::{ArtifactId, ChecksumRecord, Sha256Digest};
use uv_checksum_authority_service::{AuthorityService, Catalog};
use uv_distribution::HttpArchivePointer;
use uv_distribution_filename::WheelFilename;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pypi_types::HashDigests;
use uv_redacted::DisplaySafeUrl;
use uv_static::EnvVars;
use uv_test::archive::write_tar_gz;
use uv_test::packse::{generate_wheel, generate_wheel_with_files};
use uv_test::uv_snapshot;

const WHEEL: &str = "checksum_example-1.0.0-py3-none-any.whl";

struct Authority {
    url: String,
    public_key: String,
    task: JoinHandle<Result<()>>,
}

impl Authority {
    async fn start(records: Vec<ChecksumRecord>) -> Result<Self> {
        let key = Ed25519KeyPair::from_seed_unchecked(&[17; 32])
            .map_err(|_| anyhow!("invalid test key"))?;
        let service = AuthorityService::new(Catalog::from_records(records)?, &key)?;
        let public_key = service.public_key().to_string();
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(service.serve(listener, std::future::pending()));
        Ok(Self {
            url,
            public_key,
            task,
        })
    }

    fn configure<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        command
            .env(EnvVars::UV_CHECKSUM_AUTHORITY, &self.url)
            .env(EnvVars::UV_CHECKSUM_AUTHORITY_KEY, &self.public_key)
    }
}

impl Drop for Authority {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn record(source: &str, filename: &str, bytes: &[u8]) -> Result<ChecksumRecord> {
    Ok(ChecksumRecord::new(
        ArtifactId::new(&Url::parse(source)?, filename)?,
        Sha256Digest::from_bytes(Sha256::digest(bytes).into()),
        bytes.len() as u64,
    ))
}

fn wheel() -> Result<Vec<u8>> {
    let (filename, bytes) = generate_wheel(
        &PackageName::from_str("checksum-example")?,
        &Version::from_str("1.0.0")?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    assert_eq!(filename, WHEEL);
    Ok(bytes)
}

async fn index(server: &MockServer, filename: &str, bytes: &[u8], metadata: bool) {
    let name = filename
        .split('-')
        .next()
        .unwrap_or_default()
        .replace('_', "-")
        .to_ascii_lowercase();
    let body = json!({
        "name": name,
        "files": [{
            "filename": filename,
            "url": format!("/files/{filename}"),
            "hashes": {"sha256": hex::encode(Sha256::digest(bytes))},
            "core-metadata": metadata,
            "upload-time": "2024-01-01T00:00:00Z"
        }]
    });
    Mock::given(method("GET"))
        .and(path(format!("/simple/{name}/")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=31536000")
                .set_body_raw(body.to_string(), "application/vnd.pypi.simple.v1+json"),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=31536000")
                .set_body_bytes(bytes.to_vec()),
        )
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_install_and_authenticated_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, true).await;
    // The registry's metadata sidecar lies. Authority mode must read the verified wheel instead.
    Mock::given(method("GET"))
        .and(path(format!("/files/{WHEEL}.metadata")))
        .respond_with(ResponseTemplate::new(200).set_body_string("Metadata-Version: 2.1\nName: checksum-example\nVersion: 1.0.0\nRequires-Dist: nonexistent-malicious-dependency\n"))
        .expect(0)
        .mount(&server)
        .await;
    let index_url = format!("{}/simple", server.uri());
    let authority = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    context
        .assert_command("from checksum_example import __version__; assert __version__ == '1.0.0'")
        .success();
    Ok(())
}

/// Admission authenticates bytes but cannot make a foreign-platform wheel installable.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_rejects_incompatible_direct_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let (filename, bytes) = generate_wheel(
        &"checksum-example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "cp312-cp312-win32",
        &[],
    );
    let url = format!("{}/files/{filename}", server.uri());
    let authority = Authority::start(vec![record(&url, &filename, &bytes)?]).await?;
    Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .expect(1)
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("checksum-example @ {url}"))?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_sync()
        .arg("requirements.txt").args(["--python-platform", "linux"])), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to determine installation plan
      cause: A URL (http://[LOCALHOST]/files/checksum_example-1.0.0-cp312-cp312-win32.whl) dependency is incompatible with the current platform

    hint: The wheel is compatible with Windows (`win32`), but you're on Linux (`manylinux_2_28_x86_64`)
    ");
    context.assert_command("import checksum_example").failure();
    Ok(())
}

/// Authority requests honor proxy settings from uv.toml while archive requests can bypass it.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_uses_configured_proxy() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri().replace("127.0.0.1", "localhost"));
    let proxy = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    context
        .temp_dir
        .child("uv.toml")
        .write_str(&formatdoc! {r#"
        http-proxy = "{}"
        no-proxy = ["localhost"]
    "#, proxy.url})?;
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--index-url").arg(&index_url)
        .arg("checksum-example")
        .env(EnvVars::UV_CHECKSUM_AUTHORITY, "http://127.0.0.1:9")
        .env(EnvVars::UV_CHECKSUM_AUTHORITY_KEY, &proxy.public_key)
        .env_remove("HTTP_PROXY").env_remove("http_proxy")
        .env_remove("HTTPS_PROXY").env_remove("https_proxy")
        .env_remove("ALL_PROXY").env_remove("all_proxy")
        .env_remove("NO_PROXY").env_remove("no_proxy"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    Ok(())
}

/// An authority admission names the original filename, not its normalized wheel spelling.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_preserves_wheel_filename() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "Checksum_Example-1.0.0-py3-none-any.whl";
    let bytes = wheel()?;
    index(&server, filename, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    let normalized = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    uv_snapshot!(context.filters(), normalized.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority has no trusted record for `Checksum_Example-1.0.0-py3-none-any.whl` from `http://[LOCALHOST]/simple`
    ");
    context.assert_command("import checksum_example").failure();

    let authority = Authority::start(vec![record(&index_url, filename, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    context.assert_command("import checksum_example").success();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_decodes_direct_url_filename() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "Checksum_Example-1.0.0.tar.gz";
    let encoded_filename = "Checksum_Example%2D1.0.0.tar.gz";
    let backend = formatdoc! {r"
        from pathlib import Path

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            Path(wheel_directory, {WHEEL:?}).write_bytes(bytes.fromhex({wheel:?}))
            return {WHEEL:?}
        ", wheel = hex::encode(wheel()?),
    };
    let mut bytes = Vec::new();
    write_tar_gz(
        &mut bytes,
        &[
            (
                "checksum_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n[project]\nname = 'checksum-example'\nversion = '1.0.0'\n",
            ),
            ("checksum_example-1.0.0/backend.py", backend.as_str()),
        ],
    )?;
    Mock::given(method("GET"))
        .and(path(format!("/files/{encoded_filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
        .mount(&server)
        .await;
    let url = format!("{}/files/{encoded_filename}", server.uri());
    let authority = Authority::start(vec![record(&url, filename, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--no-index")
        .arg(&url)), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0 (from http://[LOCALHOST]/files/Checksum_Example%2D1.0.0.tar.gz)
    ");
    context.assert_command("import checksum_example").success();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_rejects_replacement_and_old_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    // Populate the ordinary cache without the authority, then remove the installation.
    context
        .pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")
        .assert()
        .success();
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();
    let authority =
        Authority::start(vec![record(&index_url, WHEEL, &vec![0; bytes.len()])?]).await?;
    let filters = context
        .filters()
        .into_iter()
        .chain([(r"sha256:[a-f0-9]{64}", "sha256:[HASH]")])
        .collect::<Vec<_>>();
    uv_snapshot!(filters, authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority mismatch for `checksum_example-1.0.0-py3-none-any.whl` from `http://[LOCALHOST]/simple`: expected sha256:[HASH], received sha256:[HASH]
    ");
    context.assert_command("import checksum_example").failure();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_unknown_and_wrong_key() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    let unknown = Authority::start(vec![]).await?;
    uv_snapshot!(context.filters(), unknown.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority has no trusted record for `checksum_example-1.0.0-py3-none-any.whl` from `http://[LOCALHOST]/simple`
    ");
    let authority = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example"))
        .env(EnvVars::UV_CHECKSUM_AUTHORITY_KEY, "00".repeat(32)), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority signature verification failed
    ");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_rejects_sdist_before_backend() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "checksum_example-1.0.0.tar.gz";
    let marker = context.temp_dir.child("backend-ran");
    let backend = format!(
        "from pathlib import Path\nPath({:?}).write_text('ran')\nraise RuntimeError('backend executed')\n",
        marker.path().to_string_lossy()
    );
    let mut bytes = Vec::new();
    write_tar_gz(
        &mut bytes,
        &[
            (
                "checksum_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n",
            ),
            ("checksum_example-1.0.0/backend.py", backend.as_str()),
        ],
    )?;
    index(&server, filename, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    let authority =
        Authority::start(vec![record(&index_url, filename, &vec![0; bytes.len()])?]).await?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("checksum-example\n")?;
    let filters = context
        .filters()
        .into_iter()
        .chain([(r"sha256:[a-f0-9]{64}", "sha256:[HASH]")])
        .collect::<Vec<_>>();
    uv_snapshot!(filters, authority.configure(context.pip_compile()
        .arg("--index-url")
        .arg(&index_url)
        .arg("requirements.in")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `checksum-example==1.0.0`
      cause: Checksum authority mismatch for `checksum_example-1.0.0.tar.gz` from `http://[LOCALHOST]/simple`: expected sha256:[HASH], received sha256:[HASH]
    ");
    assert!(!marker.path().exists());

    // Independent authority approval does not replace the requirements file's hash policy.
    let url = format!("{}/files/{filename}", server.uri());
    let authority = Authority::start(vec![record(&url, filename, &bytes)?]).await?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!(
            "checksum-example @ {url} --hash=sha256:{}\n",
            "0".repeat(64),
        ))?;
    let filters = context
        .filters()
        .into_iter()
        .chain([(r"sha256:[a-f0-9]{64}", "sha256:[HASH]")])
        .collect::<Vec<_>>();
    uv_snapshot!(filters, authority.configure(context.pip_install()
        .arg("--no-index")
        .arg("--require-hashes")
        .arg("-r")
        .arg("requirements.txt")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0.tar.gz`
      cause: Hash mismatch for `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0.tar.gz`

             Expected:
               sha256:[HASH]

             Computed:
               sha256:[HASH]
    ");
    assert!(!marker.path().exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_project_lock_and_sync() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, true).await;
    let index_url = format!("{}/simple", server.uri());
    let authority = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    context.temp_dir.child("pyproject.toml").write_str(
        "[project]\nname = 'checksum-project'\nversion = '0.1.0'\nrequires-python = '>=3.12'\ndependencies = ['checksum-example']\n",
    )?;
    uv_snapshot!(context.filters(), authority.configure(context.lock()
        .arg("--index-url")
        .arg(&index_url)), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    Mock::given(method("GET"))
        .and(path(format!("/files/{WHEEL}")))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .expect(0)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), authority.configure(context.sync()
        .arg("--frozen")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    context
        .assert_command("from checksum_example import __version__; assert __version__ == '1.0.0'")
        .success();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_direct_url_keeps_required_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, false).await;
    let url = format!("{}/files/{WHEEL}", server.uri());
    let authority = Authority::start(vec![record(&url, WHEEL, &bytes)?]).await?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!(
            "checksum-example @ {url} --hash=sha256:{}\n",
            "0".repeat(64),
        ))?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--no-index")
        .arg("--require-hashes")
        .arg("-r")
        .arg("requirements.txt")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl`
      cause: Hash mismatch for `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl`

             Expected:
               sha256:0000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:de957d73d37350560035ae6ac5ff08831f3b910331970a929255dc2f85162a93
    ");
    context.assert_command("import checksum_example").failure();
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!(
            "checksum-example @ {url} --hash=sha256:{}\n",
            hex::encode(Sha256::digest(&bytes)),
        ))?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--no-index")
        .arg("--require-hashes")
        .arg("-r")
        .arg("requirements.txt")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0 (from http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl)
    ");
    Ok(())
}

/// Authority verification does not bypass policies using a different hash algorithm.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_direct_url_keeps_additional_hash_algorithms() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, false).await;
    let url = format!("{}/files/{WHEEL}", server.uri());
    let authority = Authority::start(vec![record(&url, WHEEL, &bytes)?]).await?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!(
            "checksum-example @ {url} --hash=sha512:{}\n",
            "0".repeat(128),
        ))?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .args(["--no-index", "--require-hashes", "-r", "requirements.txt"])), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl`
      cause: Hash mismatch for `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl`

             Expected:
               sha512:00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:de957d73d37350560035ae6ac5ff08831f3b910331970a929255dc2f85162a93
               sha512:bc122ff3a6af4a678a4fee6fdd74bbd82fea06a46ba50c2bd0449b8b5d1310d5aa5ad1b370169a755d06e19edc679516bdd804c3acd6da8d16cf9631b09a7a30
    "#);
    context.assert_command("import checksum_example").failure();

    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!(
            "checksum-example @ {url} --hash=sha512:{}\n",
            hex::encode(Sha512::digest(&bytes)),
        ))?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .args(["--no-index", "--require-hashes", "-r", "requirements.txt"])), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0 (from http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl)
    "#);
    context.assert_command("import checksum_example").success();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_build_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let wheel_bytes = wheel()?;
    index(&server, WHEEL, &wheel_bytes, true).await;
    let filename = "checksum_source-1.0.0.tar.gz";
    let backend = r"from pathlib import Path
from zipfile import ZipFile

DIST_INFO = 'checksum_source-1.0.0.dist-info'
METADATA = 'Metadata-Version: 2.1\nName: checksum-source\nVersion: 1.0.0\n'

def get_requires_for_build_wheel(config_settings=None):
    return []

def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    import checksum_example
    directory = Path(metadata_directory) / DIST_INFO
    directory.mkdir()
    (directory / 'METADATA').write_text(METADATA)
    return DIST_INFO

def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    import checksum_example
    filename = 'checksum_source-1.0.0-py3-none-any.whl'
    files = {
        'checksum_source/__init__.py': 'VALUE = 42\n',
        DIST_INFO + '/METADATA': METADATA,
        DIST_INFO + '/WHEEL': 'Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n',
    }
    record = ''.join(name + ',,\n' for name in files) + DIST_INFO + '/RECORD,,\n'
    with ZipFile(Path(wheel_directory) / filename, 'w') as archive:
        for name, contents in files.items():
            archive.writestr(name, contents)
        archive.writestr(DIST_INFO + '/RECORD', record)
    return filename
";
    let mut source_bytes = Vec::new();
    write_tar_gz(
        &mut source_bytes,
        &[
            (
                "checksum_source-1.0.0/pyproject.toml",
                "[build-system]\nrequires = ['checksum-example==1.0.0']\nbuild-backend = 'backend'\nbackend-path = ['.']\n",
            ),
            ("checksum_source-1.0.0/backend.py", backend),
        ],
    )?;
    index(&server, filename, &source_bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    let source_record = record(&index_url, filename, &source_bytes)?;
    let incomplete = Authority::start(vec![source_record.clone()]).await?;
    uv_snapshot!(context.filters(), incomplete.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-source")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `checksum-source==1.0.0`
      cause: Failed to resolve requirements from `build-system.requires`
      cause: No solution found when resolving: `checksum-example==1.0.0`
      cause: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority has no trusted record for `checksum_example-1.0.0-py3-none-any.whl` from `http://[LOCALHOST]/simple`
    ");
    let authority = Authority::start(vec![
        source_record,
        record(&index_url, WHEEL, &wheel_bytes)?,
    ])
    .await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-source")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-source==1.0.0
    ");
    context
        .assert_command("from checksum_source import VALUE; assert VALUE == 42")
        .success();
    context
        .pip_uninstall()
        .arg("checksum-source")
        .assert()
        .success();
    uv_snapshot!(context.filters(), incomplete.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-source")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `checksum-source==1.0.0`
      cause: Checksum authority has no trusted record for `checksum_example-1.0.0-py3-none-any.whl` from `http://[LOCALHOST]/simple`
    ");
    context.assert_command("import checksum_source").failure();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_unavailable_fails_closed() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, false).await;
    let authority = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/checksum"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&authority)
        .await;
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--index-url")
        .arg(format!("{}/simple", server.uri()))
        .arg("checksum-example")
        .env(EnvVars::UV_CHECKSUM_AUTHORITY, authority.uri())
        .env(EnvVars::UV_CHECKSUM_AUTHORITY_KEY, "00".repeat(32)), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority returned HTTP 503 Service Unavailable
    ");
    context.assert_command("import checksum_example").failure();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_remote_index_cannot_use_local_archive() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    let local = context.temp_dir.child(WHEEL);
    fs_err::write(&local, &bytes)?;
    let url =
        Url::from_file_path(local.path()).map_err(|()| anyhow!("invalid local wheel path"))?;
    let body = json!({"name": "checksum-example", "files": [{
        "filename": WHEEL, "url": url, "hashes": {}, "upload-time": "2024-01-01T00:00:00Z"
    }]});
    Mock::given(method("GET"))
        .and(path("/simple/checksum-example/"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=31536000")
                .set_body_raw(body.to_string(), "application/vnd.pypi.simple.v1+json"),
        )
        .mount(&server)
        .await;
    let authority = Authority::start(vec![]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(format!("{}/simple", server.uri()))
        .arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority does not support a local archive supplied by a remote index: file://[TEMP_DIR]/checksum_example-1.0.0-py3-none-any.whl
    ");
    context.assert_command("import checksum_example").failure();
    Ok(())
}

/// Ordinary cache entries can be reused once their computed hashes match the authority.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_reuses_existing_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    context
        .pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")
        .assert()
        .success();
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();

    Mock::given(method("GET"))
        .and(path(format!("/files/{WHEEL}")))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .expect(0)
        .mount(&server)
        .await;
    let authority = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();

    let offline_authority = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&offline_authority)
        .await;
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--offline")
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")
        .env(EnvVars::UV_CHECKSUM_AUTHORITY, offline_authority.uri())
        .env(EnvVars::UV_CHECKSUM_AUTHORITY_KEY, &authority.public_key), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority verification is unavailable in offline mode
    ");

    // An earlier approval must not authorize a later invocation with a different catalog or key.
    let unknown = Authority::start(vec![]).await?;
    uv_snapshot!(context.filters(), unknown.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority has no trusted record for `checksum_example-1.0.0-py3-none-any.whl` from `http://[LOCALHOST]/simple`
    ");
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example"))
        .env(EnvVars::UV_CHECKSUM_AUTHORITY_KEY, "00".repeat(32)), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `checksum-example==1.0.0`
      cause: Checksum authority signature verification failed
    ");
    context.assert_command("import checksum_example").failure();
    Ok(())
}

/// Cache entries without a computed digest or archive size must be downloaded again.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_repairs_legacy_wheel_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel()?;
    index(&server, WHEEL, &bytes, false).await;
    let url = format!("{}/files/{WHEEL}", server.uri());
    context
        .pip_install()
        .arg("--no-index")
        .arg(&url)
        .assert()
        .success();
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();

    let cache = Cache::from_path(context.cache_dir.path());
    let filename = WheelFilename::from_str(WHEEL)?;
    let entry = cache.entry(
        CacheBucket::Wheels,
        WheelCache::Url(&DisplaySafeUrl::parse(&url)?).wheel_dir("checksum-example"),
        format!("{}.http", filename.cache_key()),
    );
    let original = fs_err::read(entry.path())?;
    let mut reader = Cursor::new(&original);
    let _: serde::de::IgnoredAny = rmp_serde::from_read(&mut reader)?;
    let archive_length = usize::try_from(reader.position())?;
    let mut archive = HttpArchivePointer::read_from(entry.path())?
        .expect("cached wheel pointer")
        .into_archive();
    archive.hashes = HashDigests::empty();
    archive.size = None;
    let mut legacy = rmp_serde::to_vec(&archive)?;
    legacy.extend_from_slice(&original[archive_length..]);
    fs_err::write(entry.path(), legacy)?;

    Mock::given(method("GET"))
        .and(path(format!("/files/{WHEEL}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=31536000")
                .set_body_bytes(bytes.clone()),
        )
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    let authority = Authority::start(vec![record(&url, WHEEL, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--no-index")
        .arg(&url)), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0 (from http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl)
    ");
    let repaired = HttpArchivePointer::read_from(entry.path())?
        .expect("repaired wheel pointer")
        .into_archive();
    assert_eq!(repaired.size, Some(bytes.len() as u64));
    assert!(!repaired.hashes.is_empty());
    Ok(())
}

/// Package names with an authority prefix do not turn an index into a build revision.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_prefix_package_survives_ci_prune() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let (authority_wheel_name, authority_wheel) = generate_wheel(
        &"authority-example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let authority_backend = formatdoc! {r"
        from pathlib import Path
        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            Path(wheel_directory, {authority_wheel_name:?}).write_bytes(bytes.fromhex({wheel:?}))
            return {authority_wheel_name:?}
    ", wheel = hex::encode(authority_wheel)};
    let mut authority_bytes = Vec::new();
    write_tar_gz(
        &mut authority_bytes,
        &[
            (
                "authority_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n[project]\nname = 'authority-example'\nversion = '1.0.0'\n",
            ),
            ("authority_example-1.0.0/backend.py", &authority_backend),
        ],
    )?;
    index(
        &server,
        "authority_example-1.0.0.tar.gz",
        &authority_bytes,
        false,
    )
    .await;

    let other_backend = formatdoc! {r"
        from pathlib import Path
        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            Path(wheel_directory, {WHEEL:?}).write_bytes(bytes.fromhex({wheel:?}))
            return {WHEEL:?}
    ", wheel = hex::encode(wheel()?)};
    let mut other_bytes = Vec::new();
    write_tar_gz(
        &mut other_bytes,
        &[
            (
                "checksum_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n[project]\nname = 'checksum-example'\nversion = '1.0.0'\n",
            ),
            ("checksum_example-1.0.0/backend.py", &other_backend),
        ],
    )?;
    index(
        &server,
        "checksum_example-1.0.0.tar.gz",
        &other_bytes,
        false,
    )
    .await;
    let index_url = format!("{}/simple", server.uri());
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--index-url").arg(&index_url)
        .args(["authority-example", "checksum-example"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + authority-example==1.0.0
     + checksum-example==1.0.0
    "#);
    context
        .pip_uninstall()
        .args(["authority-example", "checksum-example"])
        .assert()
        .success();
    context.prune().arg("--ci").assert().success();
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--index-url").arg(&index_url)
        .args(["--offline", "authority-example", "checksum-example"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + authority-example==1.0.0
     + checksum-example==1.0.0
    "#);
    Ok(())
}

/// Verified builds cannot consume source files or metadata generated by an ordinary build.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_isolates_mutated_source_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "checksum_example-1.0.0.tar.gz";
    let metadata = "Metadata-Version: 2.3\nName: checksum-example\nVersion: 1.0.0\n";
    let backend = formatdoc! {r#"
        from pathlib import Path

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            directory = Path(metadata_directory, 'checksum_example-1.0.0.dist-info')
            directory.mkdir()
            (directory / 'METADATA').write_text({metadata:?})
            return directory.name

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            Path(wheel_directory, {WHEEL:?}).write_bytes(bytes.fromhex({wheel:?}))
            Path('PKG-INFO').write_text({metadata:?} + 'Requires-Dist: unauthenticated-dependency\n')
            Path('backend.py').write_text("raise RuntimeError('ordinary build modified this source tree')\n")
            return {WHEEL:?}
    "#, wheel = hex::encode(wheel()?)};
    let mut bytes = Vec::new();
    write_tar_gz(
        &mut bytes,
        &[
            (
                "checksum_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n[project]\nname = 'checksum-example'\nversion = '1.0.0'\ndynamic = ['dependencies']\n",
            ),
            ("checksum_example-1.0.0/PKG-INFO", metadata),
            ("checksum_example-1.0.0/backend.py", &backend),
        ],
    )?;
    index(&server, filename, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    context
        .pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")
        .assert()
        .success();
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();
    let generated_metadata = WalkDir::new(context.cache_dir.path())
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|entry| entry.file_name() == "PKG-INFO")
        .ok_or_else(|| anyhow!("expected backend-generated source metadata"))?;
    insta::assert_snapshot!(context.read(generated_metadata.path()), @"
    Metadata-Version: 2.3
    Name: checksum-example
    Version: 1.0.0
    Requires-Dist: unauthenticated-dependency
    ");
    let authority = Authority::start(vec![record(&index_url, filename, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url").arg(&index_url).arg("checksum-example")), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    "#);
    context.assert_command("import checksum_example").success();
    Ok(())
}

/// Entering authority mode can heal static metadata without running a build backend.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_healed_static_metadata_without_building() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "checksum_example-1.0.0.tar.gz";
    let marker = context.temp_dir.child("backend-executed");
    let backend = formatdoc! {r"
        from pathlib import Path
        Path({marker:?}).write_text('executed')
        raise RuntimeError('static metadata must not execute a backend')
    ", marker = marker.path().to_string_lossy()};
    let mut bytes = Vec::new();
    write_tar_gz(
        &mut bytes,
        &[
            (
                "checksum_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n[project]\nname = 'checksum-example'\nversion = '1.0.0'\n",
            ),
            ("checksum_example-1.0.0/backend.py", &backend),
        ],
    )?;
    Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=31536000")
                .set_body_bytes(bytes.clone()),
        )
        .expect(2)
        .mount(&server)
        .await;
    let url = format!("{}/files/{filename}", server.uri());
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!("checksum-example @ {url}\n"))?;
    context
        .pip_compile()
        .arg("requirements.in")
        .arg("--no-index")
        .assert()
        .success();
    let authority = Authority::start(vec![record(&url, filename, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_compile()
        .arg("requirements.in").args(["--no-index", "--no-build"])), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    # This file was autogenerated by uv via the following command:
    #    uv pip compile --cache-dir [CACHE_DIR] requirements.in --no-index --no-build
    checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0.tar.gz
        # via -r requirements.in

    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);
    marker.assert(predicates::path::missing());
    Ok(())
}

/// A cached build is reusable only after its original source archive is approved.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_reuses_source_revision() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "checksum_example-1.0.0.tar.gz";
    let marker = context.temp_dir.child("builds");
    let (_, rebuilt_wheel) = generate_wheel_with_files(
        &"checksum-example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("checksum_example/rebuilt.py", "VALUE = 'rebuilt'\n")],
    );
    let backend = formatdoc! {r"
        from pathlib import Path

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            with Path({marker:?}).open('a') as file:
                file.write('built\n')
            wheel = {rebuilt:?} if len(Path({marker:?}).read_text().splitlines()) >= 3 else {wheel:?}
            Path(wheel_directory, {WHEEL:?}).write_bytes(bytes.fromhex(wheel))
            return {WHEEL:?}
        ", marker = marker.path().to_string_lossy(), wheel = hex::encode(wheel()?), rebuilt = hex::encode(rebuilt_wheel),
    };
    let mut bytes = Vec::new();
    write_tar_gz(
        &mut bytes,
        &[
            (
                "checksum_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n[project]\nname = 'checksum-example'\nversion = '1.0.0'\n",
            ),
            ("checksum_example-1.0.0/backend.py", backend.as_str()),
        ],
    )?;
    index(&server, filename, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    context
        .pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")
        .assert()
        .success();
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();
    let builds = fs_err::read_to_string(marker.path())?;
    insta::assert_snapshot!(builds, @"built");

    let authority = Authority::start(vec![record(&index_url, filename, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install().args(["--config-settings", "authority=true"])
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    let verified_builds = fs_err::read_to_string(marker.path())?;
    insta::assert_snapshot!(verified_builds, @"
    built
    built
    ");
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();
    let unavailable = Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .expect(0)
        .mount_as_scoped(&server)
        .await;
    // CI pruning retains the wheel and the receipts required to reuse it without rebuilding.
    context
        .command()
        .args(["cache", "prune", "--ci"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), authority.configure(context.pip_install().args(["--config-settings", "authority=true"])
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    assert_eq!(fs_err::read_to_string(marker.path())?, verified_builds);
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();

    // Repairing a tampered output after pruning needs to retrieve the source again.
    drop(unavailable);

    // A replaced output must not inherit the previous file's receipt or unpacked directory.
    let built_wheel = WalkDir::new(context.cache_dir.path())
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_name() == WHEEL
                && entry.path().components().any(|component| {
                    component
                        .as_os_str()
                        .to_string_lossy()
                        .starts_with("authority-")
                })
        })
        .expect("authority-built wheel");
    fs_err::write(built_wheel.path(), b"incomplete build")?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install().args(["--config-settings", "authority=true"])
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    let repaired_builds = fs_err::read_to_string(marker.path())?;
    insta::assert_snapshot!(repaired_builds, @"
    built
    built
    built
    ");
    uv_snapshot!(context.filters(), context.python_command()
        .args(["-c", "from checksum_example.rebuilt import VALUE; print(VALUE)"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    rebuilt
    ");
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();

    let unknown = Authority::start(vec![]).await?;
    uv_snapshot!(context.filters(), unknown.configure(context.pip_install().args(["--config-settings", "authority=true"])
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `checksum-example==1.0.0`
      cause: Checksum authority has no trusted record for `checksum_example-1.0.0.tar.gz` from `http://[LOCALHOST]/simple`
    ");
    assert_eq!(fs_err::read_to_string(marker.path())?, repaired_builds);
    context.assert_command("import checksum_example").failure();

    // Unversioned authority outputs do not prove that the build used an isolated source tree.
    let namespace = built_wheel
        .path()
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("expected an authority build-settings shard"))?;
    fs_err::rename(
        namespace,
        namespace.with_file_name(format!("authority-{}", authority.public_key)),
    )?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install().args(["--config-settings", "authority=true"])
        .arg("--index-url").arg(&index_url).arg("checksum-example")), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    "#);
    insta::assert_snapshot!(context.read(marker.path()), @"
    built
    built
    built
    built
    ");
    Ok(())
}

/// Authority-only source revisions are pruned even without outer metadata from an ordinary build.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_prunes_authority_only_source_revision() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "checksum_example-1.0.0.tar.gz";
    let backend = formatdoc! {r"
        from pathlib import Path
        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            Path(wheel_directory, {WHEEL:?}).write_bytes(bytes.fromhex({wheel:?}))
            return {WHEEL:?}
    ", wheel = hex::encode(wheel()?)};
    let mut bytes = Vec::new();
    write_tar_gz(
        &mut bytes,
        &[
            (
                "checksum_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n[project]\nname = 'checksum-example'\nversion = '1.0.0'\n",
            ),
            ("checksum_example-1.0.0/backend.py", &backend),
        ],
    )?;
    index(&server, filename, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    let authority = Authority::start(vec![record(&index_url, filename, &bytes)?]).await?;
    authority
        .configure(
            context
                .pip_install()
                .arg("--index-url")
                .arg(&index_url)
                .arg("checksum-example"),
        )
        .assert()
        .success();
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();
    let source = WalkDir::new(context.cache_dir.path())
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|entry| entry.file_name() == "src")
        .ok_or_else(|| anyhow!("expected extracted source tree"))?;
    let source = context
        .cache_dir
        .child(source.path().strip_prefix(context.cache_dir.path())?);
    source.assert(predicates::path::exists());
    context
        .command()
        .args(["cache", "prune", "--ci"])
        .assert()
        .success();
    source.assert(predicates::path::missing());
    Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .expect(0)
        .mount(&server)
        .await;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url").arg(&index_url).arg("checksum-example")), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    "#);
    Ok(())
}

/// A verified source wheel cannot be replaced while installation waits to extract it.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_retains_source_lock_through_extraction() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "checksum_example-1.0.0.tar.gz";
    let backend = formatdoc! {r"
        from pathlib import Path

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            Path(wheel_directory, {WHEEL:?}).write_bytes(bytes.fromhex({wheel:?}))
            return {WHEEL:?}
        ", wheel = hex::encode(wheel()?),
    };
    let mut bytes = Vec::new();
    write_tar_gz(
        &mut bytes,
        &[
            (
                "checksum_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n[project]\nname = 'checksum-example'\nversion = '1.0.0'\n",
            ),
            ("checksum_example-1.0.0/backend.py", backend.as_str()),
        ],
    )?;
    index(&server, filename, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    let authority = Authority::start(vec![record(&index_url, filename, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url").arg(&index_url)
        .arg("--config-settings").arg("mode=custom")
        .arg("checksum-example")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();

    let built_wheel = WalkDir::new(context.cache_dir.path())
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .find(|entry| entry.file_name() == WHEEL)
        .ok_or_else(|| anyhow!("missing cached source wheel"))?;
    let filename = WheelFilename::from_str(WHEEL)?;
    let wheel_entry = uv_cache::CacheEntry::from_path(built_wheel.path());
    let digest = Sha256Digest::from_bytes(Sha256::digest(fs_err::read(built_wheel.path())?).into());
    uv_fs::remove_symlink(
        wheel_entry
            .with_file(format!("{}-{digest}", filename.cache_key()))
            .path(),
    )?;
    let lock_name = format!("{}.lock", filename.cache_key());
    let wheel_lock = wheel_entry.with_file(&lock_name).lock().await?;
    let source_lock_path = built_wheel
        .path()
        .ancestors()
        .map(|path| path.join(".lock"))
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow!("missing source shard lock"))?;
    let source_lock = fs_err::OpenOptions::new()
        .read(true)
        .write(true)
        .open(source_lock_path)?;

    let stderr_path = context.temp_dir.child("install.log");
    let mut command = context.pip_install();
    authority
        .configure(&mut command)
        .arg("--index-url")
        .arg(&index_url)
        .arg("--config-settings")
        .arg("mode=custom")
        .arg("checksum-example")
        .env(EnvVars::RUST_LOG, "uv_fs::locked_file=info")
        .stderr(std::process::Stdio::from(
            fs_err::File::create(stderr_path.path())?.into_file(),
        ));
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .spawn()?;
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let stderr = context.read("install.log");
            if stderr.contains("Waiting to acquire exclusive lock") && stderr.contains(&lock_name) {
                return Ok::<_, anyhow::Error>(());
            }
            if let Some(status) = child.try_wait()? {
                return Err(anyhow!(
                    "install exited before extraction: {status}: {stderr}"
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await??;
    let retained = matches!(
        source_lock.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    );
    drop(source_lock);
    drop(wheel_lock);
    let output = child.wait_with_output().await?;
    output.assert().success();
    assert!(
        retained,
        "the source shard must stay locked while extraction is pending"
    );
    context.assert_command("import checksum_example").success();
    Ok(())
}

/// Exempt local sources retain the ordinary built-wheel cache layout.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_reuses_exempt_local_build_without_building() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "checksum-example"
        version = "1.0.0"
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    project.child("backend.py").write_str(&formatdoc! {r#"
        from pathlib import Path
        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            with Path(__file__).with_name("builds").open("a") as file:
                file.write("built\n")
            Path(wheel_directory, {WHEEL:?}).write_bytes(bytes.fromhex({wheel:?}))
            return {WHEEL:?}
    "#, wheel = hex::encode(wheel()?)})?;
    let url = Url::from_directory_path(project.path())
        .map_err(|()| anyhow!("invalid local source URL"))?;
    let (parent_name, parent_bytes) = generate_wheel(
        &"authority-parent".parse()?,
        &"1.0.0".parse()?,
        &[format!("checksum-example @ {url}").parse()?],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let parent = context.temp_dir.child(parent_name);
    parent.write_binary(&parent_bytes)?;
    let authority = Authority::start(vec![]).await?;
    authority
        .configure(context.pip_install().arg(parent.path()))
        .assert()
        .success();
    context
        .pip_uninstall()
        .args(["checksum-example", "authority-parent"])
        .assert()
        .success();
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg(parent.path()).arg("--no-build")), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + authority-parent==1.0.0 (from file://[TEMP_DIR]/authority_parent-1.0.0-py3-none-any.whl)
     + checksum-example==1.0.0 (from file://[TEMP_DIR]/project)
    "#);
    insta::assert_snapshot!(context.read("project/builds"), @"built");
    context.assert_command("import importlib.metadata; assert importlib.metadata.version('checksum-example') == '1.0.0'").success();
    Ok(())
}

/// A cached HTTP source wheel is reauthorized without permitting a replacement build.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_reauthorizes_cached_source_without_building() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "checksum_example-1.0.0.tar.gz";
    let marker = context.temp_dir.child("builds");
    let backend = formatdoc! {r#"
        from pathlib import Path
        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            with Path({marker:?}).open("a") as file:
                file.write("built\n")
            Path(wheel_directory, {WHEEL:?}).write_bytes(bytes.fromhex({wheel:?}))
            return {WHEEL:?}
    "#, marker = marker.path().to_string_lossy(), wheel = hex::encode(wheel()?)};
    let mut bytes = Vec::new();
    write_tar_gz(
        &mut bytes,
        &[
            (
                "checksum_example-1.0.0/pyproject.toml",
                "[build-system]\nrequires = []\nbuild-backend = 'backend'\nbackend-path = ['.']\n[project]\nname = 'checksum-example'\nversion = '1.0.0'\n",
            ),
            ("checksum_example-1.0.0/backend.py", &backend),
        ],
    )?;
    index(&server, filename, &bytes, false).await;
    let url = format!("{}/files/{filename}", server.uri());
    let authority = Authority::start(vec![record(&url, filename, &bytes)?]).await?;
    authority
        .configure(context.pip_install().arg(&url).arg("--no-deps"))
        .assert()
        .success();
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg(&url).args(["--no-deps", "--no-build"])), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0 (from http://[LOCALHOST]/files/checksum_example-1.0.0.tar.gz)
    "#);
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg(&url).args(["--no-deps", "--no-build", "--config-settings", "changed=true"])), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download and build `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0.tar.gz`
      cause: Building source distributions is disabled
    "#);
    insta::assert_snapshot!(context.read("builds"), @"built");
    Ok(())
}
