use std::io::Write;
#[cfg(unix)]
use std::io::{BufRead, BufReader};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;
use std::time::Duration;

#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use flate2::write::GzEncoder;
use serde_json::json;
use sha2::{Digest, Sha256};
use tar_codec::{ArchiveBuilder, EntryMetadata, TarEncoder};
use uv_fs::{LockedFile, LockedFileMode};
use uv_static::EnvVars;
use uv_test::uv_snapshot;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn all_binaries() -> &'static [&'static str] {
    if cfg!(windows) {
        &["uv", "uvx", "uvw"]
    } else {
        &["uv", "uvx"]
    }
}

async fn archive(target: &str, binaries: &[&str]) -> Result<(String, Vec<u8>)> {
    if cfg!(windows) {
        let mut writer = ZipFileWriter::new(Vec::new());
        for name in binaries {
            let name = format!("{name}.exe");
            writer
                .write_entry_whole(
                    ZipEntryBuilder::new(name.clone().into(), Compression::Stored),
                    name.as_bytes(),
                )
                .await?;
        }
        Ok((format!("uv-{target}.zip"), writer.close().await?))
    } else {
        let mut writer = TarEncoder::new(Vec::new()).builder();
        for name in binaries {
            writer
                .add_file(
                    format!("uv-{target}/{name}"),
                    name.as_bytes(),
                    EntryMetadata::default().executable(true),
                )
                .await?;
        }
        let bytes = writer.finish_into_inner().await?.into_inner();
        let mut compressed = GzEncoder::new(Vec::new(), flate2::Compression::fast());
        compressed.write_all(&bytes)?;
        Ok((format!("uv-{target}.tar.gz"), compressed.finish()?))
    }
}

#[tokio::test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
async fn native_update_verifies_archive_and_migrates_receipt() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filter((
        regex::escape(env!("CARGO_PKG_VERSION")),
        "[CURRENT_VERSION]",
    ));
    let bin = context.temp_dir.child("bin");
    assert!(
        context
            .command()
            .args([
                "self",
                "install",
                "--preview-features",
                "self-management",
                "--no-modify-path",
                "--install-dir"
            ])
            .arg(bin.path())
            .status()?
            .success()
    );
    let executable = bin.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
    let receipt = bin.child(".uv-receipt.json");
    let legacy = context.temp_dir.child("legacy");
    legacy.create_dir_all()?;
    let mut data: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
    data["provider"]["source"] = json!("cargo-dist");
    data["version"] = json!("0.1.0");
    legacy
        .child("uv-receipt.json")
        .write_str(&serde_json::to_string(&data)?)?;
    fs_err::remove_file(receipt.path())?;

    let server = MockServer::start().await;
    let (filename, bytes) = archive(&uv_platform::build_target(), all_binaries()).await?;
    let digest = hex::encode(Sha256::digest(&bytes));
    Mock::given(method("GET")).and(path("/api/v3/repos/astral-sh/uv/releases/tags/9.9.9"))
        .and(header("Authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tag_name": "9.9.9",
            "assets": [
                {"name": filename, "url": format!("{}/api/v3/assets/1", server.uri())},
                {"name": format!("{filename}.sha256"), "url": format!("{}/api/v3/assets/2", server.uri())}
            ]
        }))).expect(2).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v3/assets/2"))
        .and(header("Authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("{digest}  {filename}\n")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/assets/1"))
        .and(header("Authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .expect(1)
        .mount(&server)
        .await;
    let command = || {
        let mut command = context.external_command(executable.path());
        command
            .args([
                "self",
                "update",
                "9.9.9",
                "--preview-features",
                "self-management",
                "--token",
                "test-token",
            ])
            .env("AXOUPDATER_CONFIG_PATH", legacy.path())
            .env(EnvVars::UV_INSTALLER_GHE_BASE_URL, server.uri());
        command
    };
    uv_snapshot!(context.filters(), command().arg("--dry-run"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checking for updates...
    Would update uv from v[CURRENT_VERSION] to v9.9.9
    ");
    uv_snapshot!(context.filters(), command(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checking for updates...
    Updated uv from [CURRENT_VERSION] to 9.9.9
    ");
    assert_eq!(
        fs_err::read(executable.path())?,
        format!("uv{}", std::env::consts::EXE_SUFFIX).as_bytes()
    );
    let updated: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
    assert_eq!(updated["version"], "9.9.9");
    assert_eq!(updated["modify_path"], false);
    assert_eq!(updated["provider"]["source"], "uv");
    Ok(())
}

async fn release_server(binaries: &[&str], delay: Duration) -> Result<MockServer> {
    let server = MockServer::start().await;
    let (filename, bytes) = archive(&uv_platform::build_target(), binaries).await?;
    let digest = hex::encode(Sha256::digest(&bytes));
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/astral-sh/uv/releases/tags/9.9.9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tag_name": "9.9.9",
            "assets": [
                {"name": filename, "url": format!("{}/api/v3/assets/1", server.uri())},
                {"name": format!("{filename}.sha256"), "url": format!("{}/api/v3/assets/2", server.uri())}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/assets/2"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("{digest}  {filename}\n")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/assets/1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(bytes)
                .set_delay(delay),
        )
        .mount(&server)
        .await;
    Ok(server)
}

#[tokio::test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
async fn native_update_locks_installation_while_downloading() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    assert!(
        context
            .command()
            .args([
                "self",
                "install",
                "--preview-features",
                "self-management",
                "--no-modify-path",
                "--install-dir"
            ])
            .arg(bin.path())
            .status()?
            .success()
    );
    let executable = bin.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
    let server = release_server(all_binaries(), Duration::from_secs(2)).await?;
    let update = context
        .external_command(executable.path())
        .args([
            "self",
            "update",
            "9.9.9",
            "--preview-features",
            "self-management",
        ])
        .env(EnvVars::UV_INSTALLER_GHE_BASE_URL, server.uri())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if server.received_requests().await.is_some_and(|requests| {
                requests
                    .iter()
                    .any(|request| request.url.path() == "/api/v3/assets/1")
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert!(
        LockedFile::acquire_no_wait(
            bin.child(".uv-install.lock").path(),
            LockedFileMode::Exclusive,
            "uv installation",
        )
        .is_none()
    );
    let output = tokio::task::spawn_blocking(move || update.wait_with_output()).await??;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[tokio::test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
async fn native_update_removes_only_owned_companions() -> Result<()> {
    let server = release_server(&["uv"], Duration::ZERO).await?;
    for owned in [true, false] {
        let context = uv_test::test_context_with_versions!(&[]).with_filter((
            regex::escape(env!("CARGO_PKG_VERSION")),
            "[CURRENT_VERSION]",
        ));
        let bin = context.temp_dir.child("bin");
        assert!(
            context
                .command()
                .args([
                    "self",
                    "install",
                    "--preview-features",
                    "self-management",
                    "--no-modify-path",
                    "--install-dir"
                ])
                .arg(bin.path())
                .status()?
                .success()
        );
        let executable = bin.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
        let receipt = bin.child(".uv-receipt.json");
        let mut data: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
        data["version"] = json!("0.1.0");
        if !owned {
            data["binaries"] = json!([format!("uv{}", std::env::consts::EXE_SUFFIX)]);
        }
        fs_err::write(receipt.path(), serde_json::to_vec(&data)?)?;
        bin.child("another-tool").write_str("keep")?;
        let mut command = context.external_command(executable.path());
        command
            .args([
                "self",
                "update",
                "9.9.9",
                "--preview-features",
                "self-management",
            ])
            .env(EnvVars::UV_INSTALLER_GHE_BASE_URL, server.uri());
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), command, @"
            exit_code: 0 (success)
            ----- stderr -----
            Checking for updates...
            Updated uv from [CURRENT_VERSION] to 9.9.9
            ");
        }
        for name in all_binaries().iter().filter(|name| **name != "uv") {
            assert_eq!(
                bin.child(format!("{name}{}", std::env::consts::EXE_SUFFIX))
                    .path()
                    .try_exists()?,
                !owned
            );
        }
        bin.child("another-tool").assert("keep");
        let updated: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
        assert_eq!(
            updated["binaries"],
            json!([format!("uv{}", std::env::consts::EXE_SUFFIX)])
        );
    }
    Ok(())
}

#[tokio::test]
#[cfg(unix)]
async fn native_update_rejects_a_replaced_executable() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    assert!(
        context
            .command()
            .args([
                "self",
                "install",
                "--preview-features",
                "self-management",
                "--no-modify-path",
                "--install-dir"
            ])
            .arg(bin.path())
            .status()?
            .success()
    );
    let executable = bin.child("uv");
    let lock = LockedFile::acquire(
        bin.child(".uv-install.lock").path(),
        LockedFileMode::Exclusive,
        "uv installation",
    )
    .await?;
    let mut child = context
        .external_command(executable.path())
        .args([
            "self",
            "update",
            "--preview-features",
            "self-management",
            "--verbose",
        ])
        .env(EnvVars::RUST_LOG, "uv_fs=info")
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = child
        .stderr
        .take()
        .context("Update stderr was not captured")?;
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let stderr = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut ready = Some(ready);
        let mut output = String::new();
        for line in BufReader::new(stderr).lines() {
            let line = line?;
            if line.contains("Waiting to acquire exclusive lock for `uv installation`")
                && let Some(ready) = ready.take()
            {
                let _ = ready.send(());
            }
            output.push_str(&line);
            output.push('\n');
        }
        Ok(output)
    });
    let waiting = tokio::time::timeout(Duration::from_secs(60), waiting).await;
    // Replace the executable while leaving the receipt unchanged, as a manual install can do.
    let replacement = context.temp_dir.child("replacement");
    replacement.write_str("#!/bin/sh\nprintf 'uv 99.0.0\\n'\n")?;
    fs_err::set_permissions(replacement.path(), std::fs::Permissions::from_mode(0o755))?;
    uv_fs::copy_atomic_sync(replacement.path(), executable.path())?;
    drop(lock);
    let status = tokio::task::spawn_blocking(move || child.wait()).await??;
    let stderr = stderr.await??;
    waiting.with_context(|| format!("Update did not wait for the lock:\n{stderr}"))??;
    assert_eq!(status.code(), Some(2), "{stderr}");
    insta::assert_snapshot!(stderr.lines().last().context("Update stderr was empty")?, @"error: The installed uv version changed; run `uv self update` again");
    assert_eq!(
        fs_err::read(executable.path())?,
        fs_err::read(replacement.path())?
    );
    Ok(())
}
