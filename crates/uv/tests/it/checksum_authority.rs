use std::collections::BTreeMap;
use std::io::Cursor;
use std::process::Command;
use std::str::FromStr;

use anyhow::{Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::formatdoc;
use ring::signature::Ed25519KeyPair;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;
use walkdir::WalkDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_cache::{Cache, CacheBucket, WheelCache};
use uv_checksum_authority::{ArtifactId, ChecksumRecord, Sha256Digest};
use uv_checksum_authority_service::{AuthorityService, Catalog};
use uv_client::DataWithCachePolicy;
use uv_distribution::HttpArchivePointer;
use uv_distribution_filename::WheelFilename;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pypi_types::HashDigests;
use uv_redacted::DisplaySafeUrl;
use uv_static::EnvVars;
use uv_test::archive::write_tar_gz;
use uv_test::packse::generate_wheel;
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
    );
    assert_eq!(filename, WHEEL);
    Ok(bytes)
}

async fn index(server: &MockServer, filename: &str, bytes: &[u8], metadata: bool) {
    let name = filename
        .split('-')
        .next()
        .unwrap_or_default()
        .replace('_', "-");
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
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority mismatch for `checksum_example-1.0.0-py3-none-any.whl`: expected sha256:[HASH], received sha256:[HASH]
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
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority has no trusted record for `checksum_example-1.0.0-py3-none-any.whl`
    ");
    let authority = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example"))
        .env(EnvVars::UV_CHECKSUM_AUTHORITY_KEY, "00".repeat(32)), @"
    exit_code: 1 (failure)
    ----- stderr -----
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority signature verification failed
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
      × Failed to download and build `checksum-example==1.0.0`
      ╰─▶ Checksum authority mismatch for `checksum_example-1.0.0.tar.gz`: expected sha256:[HASH], received sha256:[HASH]
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
    Resolved 1 package in [TIME]
      × Failed to download `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl`
      ╰─▶ Hash mismatch for `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl`

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
      × Failed to download and build `checksum-source==1.0.0`
      ├─▶ Failed to resolve requirements from `build-system.requires`
      ├─▶ No solution found when resolving: `checksum-example==1.0.0`
      ├─▶ Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority has no trusted record for `checksum_example-1.0.0-py3-none-any.whl`
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
      × Failed to download and build `checksum-source==1.0.0`
      ╰─▶ Checksum authority has no trusted record for `checksum_example-1.0.0-py3-none-any.whl`
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
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority returned HTTP 503 Service Unavailable
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
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority does not support a local archive supplied by a remote index: file://[TEMP_DIR]/checksum_example-1.0.0-py3-none-any.whl
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
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority verification is unavailable in offline mode
    ");

    // An earlier approval must not authorize a later invocation with a different catalog or key.
    let unknown = Authority::start(vec![]).await?;
    uv_snapshot!(context.filters(), unknown.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority has no trusted record for `checksum_example-1.0.0-py3-none-any.whl`
    ");
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example"))
        .env(EnvVars::UV_CHECKSUM_AUTHORITY_KEY, "00".repeat(32)), @"
    exit_code: 1 (failure)
    ----- stderr -----
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority signature verification failed
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
    let data = DataWithCachePolicy::from_reader(Cursor::new(&original))?;
    let mut archive = HttpArchivePointer::read_from(entry.path())?
        .expect("cached wheel pointer")
        .into_archive();
    archive.hashes = HashDigests::empty();
    archive.size = None;
    let mut legacy = rmp_serde::to_vec(&archive)?;
    legacy.extend_from_slice(&original[data.data.len()..]);
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

/// A cached build is reusable only after its original source archive is approved.
#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_reuses_source_revision() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "checksum_example-1.0.0.tar.gz";
    let marker = context.temp_dir.child("builds");
    let backend = formatdoc! {r"
        from pathlib import Path

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            with Path({marker:?}).open('a') as file:
                file.write('built\n')
            Path(wheel_directory, {WHEEL:?}).write_bytes(bytes.fromhex({wheel:?}))
            return {WHEEL:?}
        ", marker = marker.path().to_string_lossy(), wheel = hex::encode(wheel()?),
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

    Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .expect(0)
        .mount(&server)
        .await;
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
    assert_eq!(fs_err::read_to_string(marker.path())?, verified_builds);
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();

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
    let repaired_builds = fs_err::read_to_string(marker.path())?;
    insta::assert_snapshot!(repaired_builds, @"
    built
    built
    built
    ");
    context
        .assert_command("from checksum_example import __version__; assert __version__ == '1.0.0'")
        .success();
    context
        .pip_uninstall()
        .arg("checksum-example")
        .assert()
        .success();

    let unknown = Authority::start(vec![]).await?;
    uv_snapshot!(context.filters(), unknown.configure(context.pip_install()
        .arg("--index-url")
        .arg(&index_url)
        .arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
      × Failed to download and build `checksum-example==1.0.0`
      ╰─▶ Checksum authority has no trusted record for `checksum_example-1.0.0.tar.gz`
    ");
    assert_eq!(fs_err::read_to_string(marker.path())?, repaired_builds);
    context.assert_command("import checksum_example").failure();
    Ok(())
}
