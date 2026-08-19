use std::fmt::Write as _;
use std::process::Command;

use anyhow::{Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild};
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use ring::signature::Ed25519KeyPair;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_checksum_authority::{ArtifactId, ChecksumRecord};
use uv_checksum_authority_service::{AuthorityService, Catalog};
use uv_static::EnvVars;
use uv_test::archive::write_tar_gz;
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
        let public_key = service.public_key().to_owned();
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
    Ok(ChecksumRecord {
        artifact: ArtifactId::new(&Url::parse(source)?, filename)?,
        sha256: hex::encode(Sha256::digest(bytes)),
    })
}

async fn wheel() -> Result<Vec<u8>> {
    let mut writer = ZipFileWriter::new(Vec::new());
    let mut record = String::new();
    for (name, contents) in [
        ("checksum_example/__init__.py", "VALUE = 42\n"),
        (
            "checksum_example-1.0.0.dist-info/METADATA",
            "Metadata-Version: 2.1\nName: checksum-example\nVersion: 1.0.0\n",
        ),
        (
            "checksum_example-1.0.0.dist-info/WHEEL",
            "Wheel-Version: 1.0\nGenerator: uv-test\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
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
    let name = "checksum_example-1.0.0.dist-info/RECORD";
    writeln!(record, "{name},,")?;
    writer
        .write_entry_whole(
            ZipEntryBuilder::new(name.into(), Compression::Stored),
            record.as_bytes(),
        )
        .await?;
    Ok(writer.close().await?)
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
                .set_body_raw(body.to_string(), "application/vnd.pypi.simple.v1+json"),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.to_vec()))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_install_and_authenticated_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel().await?;
    index(&server, WHEEL, &bytes, true).await;
    // The registry's metadata sidecar lies. Authority mode must read the verified wheel instead.
    Mock::given(method("GET")).and(path(format!("/files/{WHEEL}.metadata")))
        .respond_with(ResponseTemplate::new(200).set_body_string("Metadata-Version: 2.1\nName: checksum-example\nVersion: 1.0.0\nRequires-Dist: nonexistent-malicious-dependency\n"))
        .expect(0).mount(&server).await;
    let index_url = format!("{}/simple", server.uri());
    let authority = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url").arg(&index_url).arg("checksum-example")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    context
        .assert_command("from checksum_example import VALUE; assert VALUE == 42")
        .success();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_rejects_replacement_and_old_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel().await?;
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
        Authority::start(vec![record(&index_url, WHEEL, b"different trusted bytes")?]).await?;
    let filters = context
        .filters()
        .into_iter()
        .chain([(r"sha256:[a-f0-9]{64}", "sha256:[HASH]")])
        .collect::<Vec<_>>();
    uv_snapshot!(filters, authority.configure(context.pip_install()
        .arg("--index-url").arg(&index_url).arg("checksum-example")), @"
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
    let bytes = wheel().await?;
    index(&server, WHEEL, &bytes, false).await;
    let index_url = format!("{}/simple", server.uri());
    let unknown = Authority::start(vec![]).await?;
    uv_snapshot!(context.filters(), unknown.configure(context.pip_install()
        .arg("--index-url").arg(&index_url).arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority has no trusted record for `checksum_example-1.0.0-py3-none-any.whl`
    ");
    let authority = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url").arg(&index_url).arg("checksum-example"))
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
    let authority = Authority::start(vec![record(&index_url, filename, b"trusted sdist")?]).await?;
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
        .arg("--index-url").arg(&index_url).arg("requirements.in")), @"
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
    let bytes = wheel().await?;
    index(&server, WHEEL, &bytes, true).await;
    let index_url = format!("{}/simple", server.uri());
    let authority = Authority::start(vec![record(&index_url, WHEEL, &bytes)?]).await?;
    context.temp_dir.child("pyproject.toml").write_str(
        "[project]\nname = 'checksum-project'\nversion = '0.1.0'\nrequires-python = '>=3.12'\ndependencies = ['checksum-example']\n",
    )?;
    uv_snapshot!(context.filters(), authority.configure(context.lock()
        .arg("--index-url").arg(&index_url)), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), authority.configure(context.sync()
        .arg("--frozen")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0
    ");
    context
        .assert_command("from checksum_example import VALUE; assert VALUE == 42")
        .success();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_direct_url_keeps_required_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel().await?;
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
        .arg("--no-index").arg("--require-hashes").arg("-r").arg("requirements.txt")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
      × Failed to download `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl`
      ╰─▶ Hash mismatch for `checksum-example @ http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl`

          Expected:
            sha256:0000000000000000000000000000000000000000000000000000000000000000

          Computed:
            sha256:ffb0d4491308737c7dc01ef2f5ae4748636cf430928b4cdd578fb730fbf3f3ec
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
        .arg("--no-index").arg("--require-hashes").arg("-r").arg("requirements.txt")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + checksum-example==1.0.0 (from http://[LOCALHOST]/files/checksum_example-1.0.0-py3-none-any.whl)
    ");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_build_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let wheel_bytes = wheel().await?;
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
        .arg("--index-url").arg(&index_url).arg("checksum-source")), @"
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
        .arg("--index-url").arg(&index_url).arg("checksum-source")), @"
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
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_authority_unavailable_fails_closed() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let bytes = wheel().await?;
    index(&server, WHEEL, &bytes, false).await;
    let authority = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/checksum"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&authority)
        .await;
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--index-url").arg(format!("{}/simple", server.uri())).arg("checksum-example")
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
    let bytes = wheel().await?;
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
                .set_body_raw(body.to_string(), "application/vnd.pypi.simple.v1+json"),
        )
        .mount(&server)
        .await;
    let authority = Authority::start(vec![]).await?;
    uv_snapshot!(context.filters(), authority.configure(context.pip_install()
        .arg("--index-url").arg(format!("{}/simple", server.uri())).arg("checksum-example")), @"
    exit_code: 1 (failure)
    ----- stderr -----
      × Failed to download `checksum-example==1.0.0`
      ╰─▶ Checksum authority does not support a local archive supplied by a remote index: file://[TEMP_DIR]/checksum_example-1.0.0-py3-none-any.whl
    ");
    context.assert_command("import checksum_example").failure();
    Ok(())
}
