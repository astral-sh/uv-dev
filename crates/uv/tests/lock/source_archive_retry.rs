use std::process::Command;
use std::time::Duration;

use anyhow::{Result, anyhow, ensure};
use assert_fs::prelude::*;
use indoc::formatdoc;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;
use walkdir::WalkDir;

use uv_cache::{Cache, CacheBucket};
use uv_static::EnvVars;
use uv_test::archive::write_tar_gz;
use uv_test::{TestContext, uv_snapshot};

fn lock_command(context: &TestContext) -> Command {
    let mut command = context.lock();
    command
        .arg("--no-index")
        .arg("--no-build")
        .arg("--no-python-downloads")
        .env(EnvVars::UV_HTTP_RETRIES, "1")
        .env(EnvVars::UV_HTTP_TIMEOUT, "5")
        .env(EnvVars::UV_TEST_NO_HTTP_RETRY_DELAY, "1")
        .env("NO_PROXY", "*")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_ENTERPRISE_TOKEN")
        .env_remove("GITHUB_ENTERPRISE_TOKEN");
    command
}

async fn accept_request(listener: &TcpListener) -> Result<(TcpStream, String)> {
    let (mut stream, _) = listener.accept().await?;
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let count = stream.read(&mut buffer).await?;
        ensure!(count > 0, "request ended before its headers");
        request.extend_from_slice(&buffer[..count]);
        ensure!(request.len() <= 8192, "request headers are too large");
    }
    let end = request
        .windows(2)
        .position(|bytes| bytes == b"\r\n")
        .ok_or_else(|| anyhow!("missing request line"))?;
    let request = str::from_utf8(&request[..end])?.to_owned();
    Ok((stream, request))
}

async fn send_response(mut stream: TcpStream, body: &[u8], length: usize) -> Result<()> {
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nCache-Control: public, max-age=31536000\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await?;
    stream.write_all(body).await?;
    stream.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn lock_retries_truncated_source_archive() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let metadata = "Metadata-Version: 2.3\nName: source-archive-retry\nVersion: 1.0.0\nRequires-Python: >=3.12\n";
    let pyproject =
        "[project]\nname = 'source-archive-retry'\nversion = '1.0.0'\nrequires-python = '>=3.12'\n";
    let payload: Vec<u8> = (0_u32..4096)
        .flat_map(|value| value.wrapping_mul(0x9e37_79b9).to_le_bytes())
        .collect();
    let mut archive = Vec::new();
    write_tar_gz(
        &mut archive,
        &[
            ("source_archive_retry-1.0.0/PKG-INFO", metadata.as_bytes()),
            (
                "source_archive_retry-1.0.0/pyproject.toml",
                pyproject.as_bytes(),
            ),
            ("source_archive_retry-1.0.0/payload.bin", payload.as_slice()),
        ],
    )?;
    let archive_size = archive.len();
    let archive_hash = hex::encode(Sha256::digest(&archive));

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!(
        "http://{}/source_archive_retry-1.0.0.tar.gz",
        listener.local_addr()?
    );
    let (retry_started, retry_waiter) = oneshot::channel();
    let (continue_retry, continue_waiter) = oneshot::channel();
    let server = tokio::spawn(async move {
        timeout(Duration::from_secs(30), async move {
            let (stream, first_request) = accept_request(&listener).await?;
            send_response(stream, &archive[..archive_size / 2], archive_size).await?;

            let (stream, second_request) = accept_request(&listener).await?;
            retry_started
                .send(())
                .map_err(|()| anyhow!("retry observer was dropped"))?;
            continue_waiter.await?;
            send_response(stream, &archive, archive_size).await?;
            Ok::<_, anyhow::Error>([first_request, second_request])
        })
        .await?
    });

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["source-archive-retry @ {url}"]
        "#})?;

    let command = lock_command(&context);
    let filters = context
        .filters()
        .into_iter()
        .map(|(pattern, replacement)| (pattern.to_owned(), replacement.to_owned()))
        .collect::<Vec<_>>();
    let lock = tokio::task::spawn_blocking(move || {
        uv_snapshot!(filters, command, @"
        exit_code: 0 (success)
        ----- stderr -----
        Resolved 2 packages in [TIME]
        ")
    });

    // The retry cannot begin until the failed extraction has returned. Do not let the second
    // response complete until we have checked that no source revision was published.
    timeout(Duration::from_secs(30), retry_waiter).await??;
    let source_cache =
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::SourceDistributions);
    for entry in WalkDir::new(&source_cache) {
        let entry = entry?;
        assert_ne!(entry.file_name(), "revision.http");
        assert_ne!(entry.file_name(), "PKG-INFO");
        assert_ne!(entry.file_name(), "pyproject.toml");
        assert_ne!(entry.file_name(), "src");
    }
    continue_retry
        .send(())
        .map_err(|()| anyhow!("retry server was dropped"))?;
    lock.await?;
    assert_eq!(
        server.await??,
        [
            "GET /source_archive_retry-1.0.0.tar.gz HTTP/1.1",
            "GET /source_archive_retry-1.0.0.tar.gz HTTP/1.1",
        ]
    );

    let sources = WalkDir::new(&source_cache)
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry) if entry.file_type().is_dir() && entry.file_name() == "src" => {
                Some(Ok(entry.into_path()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let [source] = sources.as_slice() else {
        anyhow::bail!("expected one published source tree, got {sources:?}");
    };
    assert_eq!(fs_err::read_to_string(source.join("PKG-INFO"))?, metadata);
    assert_eq!(fs_err::read(source.join("payload.bin"))?, payload);

    let original_lock = context.read("uv.lock");
    let lock: toml::Value = toml::from_str(&original_lock)?;
    let packages = lock["package"]
        .as_array()
        .ok_or_else(|| anyhow!("missing locked packages"))?;
    let distribution = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("source-archive-retry"))
        .ok_or_else(|| anyhow!("missing locked source distribution"))?;
    assert_eq!(distribution["version"].as_str(), Some("1.0.0"));
    assert_eq!(distribution["source"]["url"].as_str(), Some(url.as_str()));
    assert_eq!(
        distribution["sdist"]["hash"].as_str(),
        Some(format!("sha256:{archive_hash}").as_str())
    );
    // Resolve again without the lockfile or a live server, using only the successful cache entry.
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    uv_snapshot!(context.filters(), lock_command(&context).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original_lock);
    Ok(())
}
