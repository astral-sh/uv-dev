use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild};
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileMode};
use uv_python::downloads::{ManagedPythonDownloadList, PythonDownloadRequest};
use uv_static::EnvVars;
use uv_test::archive::write_tar_gz;
use uv_test::{TestContext, uv_snapshot, venv_bin_path};

const CPYTHON_RELEASES: &str =
    "https://github.com/astral-sh/python-build-standalone/releases/download";

struct PythonArchive {
    key: String,
    cache_filename: String,
    mirror_path: String,
    sha256: String,
    downloads_json: String,
    contents: Vec<u8>,
}

/// Select a native embedded distribution and reuse the shared managed-Python archive cache.
fn python_archive() -> Result<PythonArchive> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let downloads = ManagedPythonDownloadList::new_only_embedded()?;
    let request = "cpython-3.13.1"
        .parse::<PythonDownloadRequest>()?
        .fill_platform()?;
    let download = downloads.find(&request)?;
    let key = download.key().to_string();
    let url = download
        .download_urls(Some(CPYTHON_RELEASES), None)?
        .into_iter()
        .next()
        .context("CPython fixture has no download URL")?;
    let filename = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .context("CPython fixture URL has no filename")?
        .replace("%2B", "-");
    let mirror_path = format!(
        "/{}",
        url.as_str()
            .strip_prefix(CPYTHON_RELEASES)
            .context("CPython fixture is not a standalone release")?
            .trim_start_matches('/')
    );

    let catalog: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&fs_err::read(
            context
                .workspace_root
                .join("crates/uv-python/download-metadata.json"),
        )?)?;
    let (catalog_key, metadata) = catalog
        .into_iter()
        .find(|(_, metadata)| metadata["url"].as_str() == Some(url.as_str()))
        .context("CPython fixture is missing from the download metadata")?;
    let sha256 = metadata["sha256"]
        .as_str()
        .context("CPython fixture has no SHA-256 digest")?
        .to_owned();
    let cache_filename = format!(
        "{}-{filename}",
        sha256
            .get(..9)
            .context("CPython fixture has an invalid SHA-256 digest")?
    );
    let mut manifest = serde_json::Map::new();
    manifest.insert(catalog_key, metadata);

    let mut prepare = context.python_install();
    let archive_cache = prepare
        .get_envs()
        .find_map(|(name, value)| (name == EnvVars::UV_PYTHON_CACHE_DIR).then_some(value))
        .flatten()
        .filter(|value| !value.is_empty())
        .map_or_else(
            || context.cache_dir.join("python-archive-fixture"),
            PathBuf::from,
        );
    let archive_cache = if archive_cache.is_absolute() {
        archive_cache
    } else {
        context.temp_dir.join(archive_cache)
    };
    let archive_path = archive_cache.join(&cache_filename);
    if !archive_path.try_exists()? {
        prepare
            .arg(&key)
            .arg("--no-bin")
            .arg("--no-registry")
            .env(EnvVars::UV_PYTHON_CACHE_DIR, &archive_cache)
            .assert()
            .success();
    }
    let contents = fs_err::read(&archive_path)?;
    assert_eq!(hex::encode(Sha256::digest(&contents)), sha256);

    Ok(PythonArchive {
        key,
        cache_filename,
        mirror_path,
        sha256,
        downloads_json: serde_json::to_string(&manifest)?,
        contents,
    })
}

/// A checksum failure must leave the next invocation able to install a working Python.
#[tokio::test]
async fn python_install_recovers_corrupt_archive_cache() -> Result<()> {
    let archive = python_archive()?;
    let mut rejected = Vec::new();
    write_tar_gz(
        &mut rejected,
        &[("python/README", "inert Python distribution fixture\n")],
    )?;
    let rejected_hash = hex::encode(Sha256::digest(&rejected));
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs()
        .with_filter((archive.sha256.clone(), "[EXPECTED_HASH]"))
        .with_filter((rejected_hash, "[ACTUAL_HASH]"));
    let archive_cache = context.temp_dir.child("archive-cache");
    let cached_archive = archive_cache.child(&archive.cache_filename);
    let downloads_path = context.temp_dir.child("python-downloads.json");
    downloads_path.write_str(&archive.downloads_json)?;

    let server = MockServer::start().await;
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = Arc::clone(&requests);
    Mock::given(method("GET"))
        .and(path(&archive.mirror_path))
        .respond_with(move |_request: &Request| {
            if request_count.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(200).set_body_bytes(rejected.clone())
            } else {
                ResponseTemplate::new(200).set_body_bytes(archive.contents.clone())
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let install = |context: &TestContext| {
        let mut command = context.python_install();
        command
            .arg(&archive.key)
            .arg("--python-downloads-json-url")
            .arg(downloads_path.path())
            .env(EnvVars::UV_PYTHON_CACHE_DIR, archive_cache.path())
            .env(EnvVars::UV_PYTHON_INSTALL_MIRROR, server.uri());
        command
    };

    uv_snapshot!(context.filters(), install(&context), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to install cpython-3.13.1-[PLATFORM]
      cause: Hash mismatch for `cpython-3.13.1-[PLATFORM]`

             Expected:
             [EXPECTED_HASH]

             Computed:
             [ACTUAL_HASH]
    ");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    for unpublished in [
        cached_archive.path().to_path_buf(),
        context.temp_dir.join("managed").join(&archive.key),
        context
            .bin_dir
            .join(format!("python3.13{}", std::env::consts::EXE_SUFFIX)),
    ] {
        assert_eq!(
            fs_err::symlink_metadata(&unpublished)
                .expect_err("a rejected archive must not be published")
                .kind(),
            io::ErrorKind::NotFound,
        );
    }
    assert!(
        fs_err::read_dir(context.temp_dir.join("managed/.temp"))?
            .next()
            .is_none()
    );

    uv_snapshot!(context.filters(), install(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.1 in [TIME]
     + cpython-3.13.1-[PLATFORM] (python3.13)
    ");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        hex::encode(Sha256::digest(fs_err::read(cached_archive.path())?)),
        archive.sha256,
    );

    // A valid cache hit needs no publication or repair lock, even in a fresh installation root.
    let offline = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs();
    let lock = LockedFile::acquire(
        archive_cache.join(format!(".{}.lock", cache_digest(&archive.cache_filename))),
        LockedFileMode::Exclusive,
        "Python archive fixture",
    )
    .await?;
    uv_snapshot!(offline.filters(), install(&offline)
        .arg("--offline")
        .env(EnvVars::UV_LOCK_TIMEOUT, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.1 in [TIME]
     + cpython-3.13.1-[PLATFORM] (python3.13)
    ");
    drop(lock);
    assert_eq!(requests.load(Ordering::SeqCst), 2);

    insta::allow_duplicates! {
        for context in [&context, &offline] {
            let python = context
                .bin_dir
                .join(format!("python3.13{}", std::env::consts::EXE_SUFFIX));
            uv_snapshot!(context.filters(), context.external_command(&python)
                .args(["-I", "-c", "import platform; print(platform.python_version())"]), @"
            exit_code: 0 (success)
            ----- stdout -----
            3.13.1
            ");
            uv_snapshot!(context.filters(), context.venv()
                .arg(context.venv.path())
                .arg("--python").arg(&python)
                .arg("--offline")
                .arg("--quiet"), @"
            exit_code: 0 (success)
            ");
            uv_snapshot!(context.filters(), context.external_command(
                venv_bin_path(&context.venv).join(format!("python{}", std::env::consts::EXE_SUFFIX)))
                .args(["-I", "-c", "import platform, sys; print(platform.python_version(), sys.prefix != sys.base_prefix)"]), @"
            exit_code: 0 (success)
            ----- stdout -----
            3.13.1 True
            ");
        }
    }
    server.verify().await;

    Ok(())
}
