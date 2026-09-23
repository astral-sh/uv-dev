use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use assert_fs::prelude::*;
use serde_json::json;
use uv_fs::{LockedFile, LockedFileMode};
use uv_test::uv_snapshot;

#[test]
fn requires_preview() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.command().args(["self", "install"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Native self-installation is experimental; pass `--preview-features self-management` to enable it
    ");
}

#[test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
fn installs_running_distribution() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    let mut command = context.command();
    command
        .args([
            "self",
            "install",
            "--preview-features",
            "self-management",
            "--no-modify-path",
            "--install-dir",
        ])
        .arg(bin.path());
    let output = command.output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let installed = bin.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
    assert!(
        std::process::Command::new(installed.path())
            .arg("--version")
            .status()?
            .success()
    );
    bin.child(format!("uvx{}", std::env::consts::EXE_SUFFIX))
        .assert(predicates::path::is_file());
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs_err::read(bin.child(".uv-receipt.json"))?)?;
    assert_eq!(receipt["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(receipt["modify_path"], false);
    assert_eq!(receipt["provider"]["source"], "uv");
    assert_eq!(
        receipt["install_prefix"],
        fs_err::canonicalize(bin.path())?.to_string_lossy().as_ref()
    );
    assert!(command.status()?.success());
    Ok(())
}

#[test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
fn reinstalls_missing_executable_with_matching_receipt() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    let mut command = context.command();
    command
        .args([
            "self",
            "install",
            "--preview-features",
            "self-management",
            "--no-modify-path",
            "--install-dir",
        ])
        .arg(bin.path());
    assert!(command.status()?.success());
    let executable = bin.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
    let receipt = bin.child(".uv-receipt.json");
    let mut data: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
    data["source"]["owner"] = serde_json::json!("example");
    fs_err::write(receipt.path(), serde_json::to_vec(&data)?)?;
    fs_err::remove_file(executable.path())?;
    assert!(command.status()?.success());
    executable.assert(predicates::path::is_file());
    let repaired: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
    assert_eq!(repaired["source"]["owner"], "example");

    command.args(["--source-repository", "astral-sh/uv"]);
    fs_err::remove_file(executable.path())?;
    assert!(command.status()?.success());
    let repaired: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
    assert_eq!(repaired["source"]["owner"], "astral-sh");

    let foreign = context.temp_dir.child("foreign");
    foreign.create_dir_all()?;
    data["install_prefix"] = serde_json::json!(foreign.path());
    fs_err::write(receipt.path(), serde_json::to_vec(&data)?)?;
    fs_err::remove_file(executable.path())?;
    uv_snapshot!(context.filters(), command, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The install receipt at `[TEMP_DIR]/bin/.uv-receipt.json` belongs to a different installation at `[TEMP_DIR]/foreign`
    ");
    executable.assert(predicates::path::missing());
    assert_eq!(fs_err::read(receipt.path())?, serde_json::to_vec(&data)?);
    Ok(())
}

#[test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
fn unmanaged_install_has_no_receipt() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    let github_path = context.temp_dir.child("github-path");
    assert!(
        context
            .command()
            .args([
                "self",
                "install",
                "--preview-features",
                "self-management",
                "--unmanaged"
            ])
            .arg(bin.path())
            .env("GITHUB_PATH", github_path.path())
            .status()?
            .success()
    );
    bin.child(".uv-receipt.json")
        .assert(predicates::path::missing());
    github_path.assert(predicates::path::missing());
    Ok(())
}

#[test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
fn uninstall_removes_only_owned_files() -> Result<()> {
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
    bin.child("another-tool").write_str("keep")?;
    let executable = bin.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
    uv_snapshot!(context.filters(), context.external_command(executable.path()).args(["self", "uninstall", "--dry-run"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would uninstall uv from [TEMP_DIR]/bin
    ");
    executable.assert(predicates::path::is_file());
    uv_snapshot!(context.filters(), context.external_command(executable.path()).args(["self", "uninstall"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled uv from [TEMP_DIR]/bin
    ");
    #[cfg(windows)]
    for _ in 0..100 {
        if !executable.path().try_exists()? {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    executable.assert(predicates::path::missing());
    bin.child(format!("uvx{}", std::env::consts::EXE_SUFFIX))
        .assert(predicates::path::missing());
    bin.child(".uv-receipt.json")
        .assert(predicates::path::missing());
    bin.child("another-tool").assert("keep");
    Ok(())
}

#[tokio::test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
async fn uninstall_reads_receipt_after_waiting_for_installation() -> Result<()> {
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
    let receipt = bin.child(".uv-receipt.json");
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
            "uninstall",
            "--preview-features",
            "self-management",
            "--verbose",
        ])
        .env(uv_static::EnvVars::RUST_LOG, "uv_fs=info")
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = child
        .stderr
        .take()
        .context("Uninstall stderr was not captured")?;
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
    // Model another installation committing a new ownership inventory while holding the lock.
    let mut data: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
    data["binaries"] = json!([format!("uv{}", std::env::consts::EXE_SUFFIX)]);
    fs_err::write(receipt.path(), serde_json::to_vec(&data)?)?;
    drop(lock);
    let status = tokio::task::spawn_blocking(move || child.wait()).await??;
    let stderr = stderr.await??;
    waiting.with_context(|| format!("Uninstall did not wait for the lock:\n{stderr}"))??;
    assert!(status.success(), "{stderr}");
    #[cfg(windows)]
    for _ in 0..100 {
        if !executable.path().try_exists()? {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    executable.assert(predicates::path::missing());
    bin.child(format!("uvx{}", std::env::consts::EXE_SUFFIX))
        .assert(predicates::path::is_file());
    receipt.assert(predicates::path::missing());
    Ok(())
}

#[tokio::test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
async fn uninstall_rechecks_global_receipt_after_waiting() -> Result<()> {
    check_global_receipt_cleanup(false, true).await
}

#[tokio::test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
async fn unmanaged_install_rechecks_global_receipt_after_waiting() -> Result<()> {
    check_global_receipt_cleanup(true, true).await
}

#[tokio::test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
async fn legacy_uninstall_rechecks_global_receipt_after_waiting() -> Result<()> {
    check_global_receipt_cleanup(false, false).await
}

#[cfg(any(not(windows), feature = "windows-gui-bin"))]
async fn check_global_receipt_cleanup(unmanaged: bool, native_receipt: bool) -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let first = context.temp_dir.child("first");
    let second = context.temp_dir.child("second");
    for bin in [&first, &second] {
        assert!(
            context
                .command()
                .args([
                    "self",
                    "install",
                    "--preview-features",
                    "self-management",
                    "--no-modify-path",
                    "--install-dir",
                ])
                .arg(bin.path())
                .status()?
                .success()
        );
    }
    let legacy = context.temp_dir.child("legacy");
    legacy.create_dir_all()?;
    let receipt = legacy.child("uv-receipt.json");
    fs_err::copy(first.child(".uv-receipt.json"), receipt.path())?;
    let foreign = fs_err::read(second.child(".uv-receipt.json"))?;
    if !native_receipt {
        fs_err::remove_file(first.child(".uv-receipt.json"))?;
    }
    let lock = LockedFile::acquire(
        receipt.path().with_extension("lock"),
        LockedFileMode::Exclusive,
        "legacy uv installation receipt",
    )
    .await?;
    let executable = first.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
    let mut command = if unmanaged {
        let mut command = context.command();
        command
            .args(["self", "install", "--unmanaged"])
            .arg(first.path());
        command
    } else {
        let mut command = context.external_command(executable.path());
        command.args(["self", "uninstall"]);
        command
    };
    let mut child = command
        .args(["--preview-features", "self-management", "--verbose"])
        .env("AXOUPDATER_CONFIG_PATH", legacy.path())
        .env(uv_static::EnvVars::RUST_LOG, "uv_fs=info")
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = child
        .stderr
        .take()
        .context("Self-management stderr was not captured")?;
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let stderr = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut ready = Some(ready);
        let mut output = String::new();
        for line in BufReader::new(stderr).lines() {
            let line = line?;
            if line
                .contains("Waiting to acquire exclusive lock for `legacy uv installation receipt`")
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
    uv_fs::write_atomic_sync(receipt.path(), &foreign)?;
    drop(lock);
    let status = tokio::task::spawn_blocking(move || child.wait()).await??;
    let stderr = stderr.await??;
    waiting.with_context(|| {
        format!("Self-management did not wait for the global lock:\n{stderr}")
    })??;
    if native_receipt || unmanaged {
        assert!(status.success(), "{stderr}");
    } else {
        assert_eq!(status.code(), Some(2), "{stderr}");
        executable.assert(predicates::path::is_file());
    }
    assert_eq!(fs_err::read(receipt.path())?, foreign);
    first
        .child(".uv-receipt.json")
        .assert(predicates::path::missing());
    second
        .child(format!("uv{}", std::env::consts::EXE_SUFFIX))
        .assert(predicates::path::is_file());
    Ok(())
}

#[test]
#[cfg(unix)]
fn native_install_configures_standalone_profiles() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    let github_path = context.temp_dir.child("github-path");
    let zsh = context.temp_dir.child("zsh");
    zsh.create_dir_all()?;
    zsh.child(".zshenv").write_str("# existing zsh\n")?;
    context
        .home_dir
        .child(".bashrc")
        .write_str("# existing bash\n")?;
    let mut command = context.command();
    command
        .args([
            "self",
            "install",
            "--preview-features",
            "self-management",
            "--install-dir",
        ])
        .arg(bin.path())
        .env("GITHUB_PATH", github_path.path())
        .env("ZDOTDIR", zsh.path())
        .env(uv_static::EnvVars::UV_NO_MODIFY_PATH, "0")
        .env("INSTALLER_NO_MODIFY_PATH", "1");
    assert!(command.status()?.success());
    assert_eq!(
        fs_err::read_to_string(github_path.path())?,
        format!("{}\n", bin.path().display())
    );
    let profile = fs_err::read_to_string(context.home_dir.child(".profile"))?;
    assert!(profile.contains("/bin/env"));
    assert!(fs_err::read_to_string(context.home_dir.child(".bashrc"))?.contains(&profile));
    assert!(fs_err::read_to_string(zsh.child(".zshenv"))?.contains(&profile));
    bin.child("env").assert(predicates::path::is_file());
    bin.child("env.fish").assert(predicates::path::is_file());
    context
        .home_dir
        .child(".config/fish/conf.d/uv.env.fish")
        .assert(predicates::path::is_file());
    assert!(command.status()?.success());
    assert_eq!(
        profile,
        fs_err::read_to_string(context.home_dir.child(".profile"))?
    );
    Ok(())
}

#[test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
fn disable_update_omits_receipt() -> Result<()> {
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
                "--install-dir"
            ])
            .arg(bin.path())
            .env(uv_static::EnvVars::UV_DISABLE_UPDATE, "1")
            .env(uv_static::EnvVars::UV_NO_MODIFY_PATH, "1")
            .status()?
            .success()
    );
    bin.child(".uv-receipt.json")
        .assert(predicates::path::missing());
    Ok(())
}
