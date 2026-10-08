use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use anyhow::{Result, bail};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild, PathCreateDir};
use predicates::prelude::{PredicateStrExt, predicate};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStderr};

use uv_fs::{LockedFile, LockedFileMode};
use uv_static::EnvVars;
use uv_test::packse::generate_wheel_with_files;
use uv_test::{TestContext, uv_snapshot};

/// Keep the child alive and retain all diagnostics while waiting for its admission message.
struct WaitingTool {
    child: Child,
    stderr: BufReader<ChildStderr>,
    captured: String,
}

impl WaitingTool {
    async fn start(command: Command) -> Result<Self> {
        let mut child = tokio::process::Command::from(command)
            .env(EnvVars::RUST_LOG, "uv_fs=info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stderr = BufReader::new(child.stderr.take().unwrap());
        let mut waiting = Self {
            child,
            stderr,
            captured: String::new(),
        };
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let mut line = String::new();
                if waiting.stderr.read_line(&mut line).await? == 0 {
                    bail!(
                        "tool exited before destination admission: {}",
                        waiting.captured
                    );
                }
                let admitted = line
                    .contains("Waiting to acquire exclusive lock for `tool executable directory ");
                waiting.captured.push_str(&line);
                if admitted {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        })
        .await??;
        Ok(waiting)
    }

    async fn finish(mut self) -> Result<Output> {
        tokio::time::timeout(Duration::from_secs(30), async {
            self.stderr.read_to_string(&mut self.captured).await?;
            let mut output = self.child.wait_with_output().await?;
            output.stderr = self.captured.into_bytes();
            Ok(output)
        })
        .await?
    }
}

fn wheel(directory: &Path, name: &str, command: &str) -> Result<PathBuf> {
    let normalized = name.replace('-', "_");
    let entrypoints = format!("{normalized}-1.0.0.dist-info/entry_points.txt");
    let module = format!("{normalized}/commands.py");
    let (filename, contents) = generate_wheel_with_files(
        &name.parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::default(),
        None,
        "py3-none-any",
        &[
            (
                &entrypoints,
                &format!("[console_scripts]\n{command} = {normalized}.commands:main\n"),
            ),
            (&module, &format!("def main():\n    print({name:?})\n")),
        ],
    );
    let path = directory.join(filename);
    fs_err::write(&path, contents)?;
    Ok(path)
}

fn install(context: &TestContext, wheel: &Path, root: &Path, bin: &Path) -> Command {
    let mut command = context.tool_install();
    command
        .arg(wheel)
        .arg("--offline")
        .env(EnvVars::UV_TOOL_DIR, root)
        .env(EnvVars::UV_TOOL_BIN_DIR, bin)
        .env(EnvVars::PATH, bin);
    command
}

async fn block_destination(bin: &Path) -> Result<LockedFile> {
    let sidecar = bin.join(".uv-tool-lock");
    fs_err::create_dir_all(&sidecar)?;
    Ok(LockedFile::acquire(
        sidecar.join("lock"),
        LockedFileMode::Exclusive,
        "test tool destination",
    )
    .await?)
}

fn executable(bin: &Path, name: &str) -> PathBuf {
    bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

#[tokio::test]
async fn independent_tool_roots_recheck_shared_command_after_admission() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let bin = context.temp_dir.child("bin");
    let first_root = context.temp_dir.child("first-tools");
    let second_root = context.temp_dir.child("second-tools");
    let first_wheel = wheel(context.temp_dir.path(), "first-owner", "shared-command")?;
    let second_wheel = wheel(context.temp_dir.path(), "second-owner", "shared-command")?;
    let guard = block_destination(bin.path()).await?;
    let first = WaitingTool::start(install(
        &context,
        &first_wheel,
        first_root.path(),
        bin.path(),
    ))
    .await?;
    let second = WaitingTool::start(install(
        &context,
        &second_wheel,
        second_root.path(),
        bin.path(),
    ))
    .await?;
    assert!(!executable(bin.path(), "shared-command").exists());
    assert!(!first_root.child("first-owner/uv-receipt.toml").exists());
    assert!(!second_root.child("second-owner/uv-receipt.toml").exists());
    drop(guard);
    let (first, second) = tokio::try_join!(first.finish(), second.finish())?;
    assert_ne!(
        first.status.success(),
        second.status.success(),
        "first: {first:?}\nsecond: {second:?}"
    );
    let (winner, winner_name, winner_root, loser, loser_name, loser_root) =
        if first.status.success() {
            (
                first,
                "first-owner",
                first_root,
                second,
                "second-owner",
                second_root,
            )
        } else {
            (
                second,
                "second-owner",
                second_root,
                first,
                "first-owner",
                first_root,
            )
        };
    assert!(winner.stdout.is_empty(), "{winner:?}");
    assert!(loser.stdout.is_empty(), "{loser:?}");
    assert_eq!(loser.status.code(), Some(2), "{loser:?}");
    assert!(
        String::from_utf8_lossy(&loser.stderr).contains("already exists"),
        "{loser:?}"
    );
    assert!(
        winner_root
            .child(winner_name)
            .child("uv-receipt.toml")
            .is_file()
    );
    assert!(
        !loser_root
            .child(loser_name)
            .child("uv-receipt.toml")
            .exists()
    );
    Command::new(executable(bin.path(), "shared-command"))
        .assert()
        .success()
        .stdout(predicate::str::diff(format!("{winner_name}\n")).normalize());
    assert!(bin.child(".uv-tool-lock/lock").is_file());
    Ok(())
}

#[tokio::test]
async fn independent_tool_roots_publish_distinct_commands() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let bin = context.temp_dir.child("bin");
    let first_root = context.temp_dir.child("first-tools");
    let second_root = context.temp_dir.child("second-tools");
    let first_wheel = wheel(context.temp_dir.path(), "first-owner", "first-command")?;
    let second_wheel = wheel(context.temp_dir.path(), "second-owner", "second-command")?;
    let guard = block_destination(bin.path()).await?;
    let first = WaitingTool::start(install(
        &context,
        &first_wheel,
        first_root.path(),
        bin.path(),
    ))
    .await?;
    let second = WaitingTool::start(install(
        &context,
        &second_wheel,
        second_root.path(),
        bin.path(),
    ))
    .await?;
    drop(guard);
    let (first, second) = tokio::try_join!(first.finish(), second.finish())?;
    first.assert().success();
    second.assert().success();
    Command::new(executable(bin.path(), "first-command"))
        .assert()
        .success()
        .stdout(predicate::str::diff("first-owner\n").normalize());
    Command::new(executable(bin.path(), "second-command"))
        .assert()
        .success()
        .stdout(predicate::str::diff("second-owner\n").normalize());
    assert!(first_root.child("first-owner/uv-receipt.toml").is_file());
    assert!(second_root.child("second-owner/uv-receipt.toml").is_file());
    Ok(())
}

#[tokio::test]
async fn repair_and_uninstall_wait_for_destination_admission() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let bin = context.temp_dir.child("bin");
    let root = context.temp_dir.child("tools");
    let wheel = wheel(context.temp_dir.path(), "repair-owner", "repair-command")?;
    install(&context, &wheel, root.path(), bin.path())
        .assert()
        .success();
    let exported = executable(bin.path(), "repair-command");
    fs_err::remove_file(&exported)?;
    let guard = block_destination(bin.path()).await?;
    let repair = WaitingTool::start(install(&context, &wheel, root.path(), bin.path())).await?;
    assert!(!exported.exists());
    drop(guard);
    repair.finish().await?.assert().success();
    Command::new(&exported)
        .assert()
        .success()
        .stdout(predicate::str::diff("repair-owner\n").normalize());

    let guard = block_destination(bin.path()).await?;
    let mut command = context.tool_uninstall();
    command.arg("repair-owner").env(EnvVars::PATH, bin.path());
    let uninstall = WaitingTool::start(command).await?;
    assert!(exported.exists());
    assert!(root.child("repair-owner/uv-receipt.toml").is_file());
    drop(guard);
    uninstall.finish().await?.assert().success();
    assert!(!exported.exists());
    assert!(!root.child("repair-owner").exists());
    assert!(bin.child(".uv-tool-lock/lock").is_file());
    Ok(())
}

#[tokio::test]
async fn repair_waits_for_previous_bin_directory() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let previous = context.temp_dir.child("bin");
    let next = context.temp_dir.child("next-bin");
    let root = context.temp_dir.child("tools");
    let wheel = wheel(context.temp_dir.path(), "moved-owner", "moved-command")?;
    install(&context, &wheel, root.path(), previous.path())
        .assert()
        .success();
    let guard = block_destination(previous.path()).await?;
    let repair = WaitingTool::start(install(&context, &wheel, root.path(), next.path())).await?;
    assert!(executable(previous.path(), "moved-command").exists());
    assert!(!executable(next.path(), "moved-command").exists());
    drop(guard);
    repair.finish().await?.assert().success();
    assert!(!executable(previous.path(), "moved-command").exists());
    Command::new(executable(next.path(), "moved-command"))
        .assert()
        .success()
        .stdout(predicate::str::diff("moved-owner\n").normalize());
    assert!(previous.child(".uv-tool-lock/lock").is_file());
    assert!(next.child(".uv-tool-lock/lock").is_file());
    Ok(())
}

#[tokio::test]
async fn forced_transfer_waits_and_previous_store_cannot_remove_export() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let bin = context.temp_dir.child("bin");
    let first_root = context.temp_dir.child("first-tools");
    let second_root = context.temp_dir.child("second-tools");
    let first = wheel(context.temp_dir.path(), "first-owner", "shared-command")?;
    let second = wheel(context.temp_dir.path(), "second-owner", "shared-command")?;
    install(&context, &first, first_root.path(), bin.path())
        .assert()
        .success();
    let guard = block_destination(bin.path()).await?;
    let mut command = install(&context, &second, second_root.path(), bin.path());
    command.arg("--force");
    let transfer = WaitingTool::start(command).await?;
    Command::new(executable(bin.path(), "shared-command"))
        .assert()
        .success()
        .stdout(predicate::str::diff("first-owner\n").normalize());
    drop(guard);
    transfer.finish().await?.assert().success();
    context
        .tool_uninstall()
        .arg("first-owner")
        .env(EnvVars::UV_TOOL_DIR, first_root.path())
        .assert()
        .success();
    Command::new(executable(bin.path(), "shared-command"))
        .assert()
        .success()
        .stdout(predicate::str::diff("second-owner\n").normalize());
    assert!(second_root.child("second-owner/uv-receipt.toml").is_file());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn aliased_bin_directories_share_destination_admission() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let bin = context.temp_dir.child("bin");
    let alias = context.temp_dir.child("bin-alias");
    let root = context.temp_dir.child("tools");
    bin.create_dir_all()?;
    fs_err::os::unix::fs::symlink(bin.path(), alias.path())?;
    let wheel = wheel(context.temp_dir.path(), "alias-owner", "alias-command")?;
    let guard = block_destination(bin.path()).await?;
    let waiting = WaitingTool::start(install(&context, &wheel, root.path(), alias.path())).await?;
    assert!(!executable(bin.path(), "alias-command").exists());
    drop(guard);
    waiting.finish().await?.assert().success();
    Command::new(executable(bin.path(), "alias-command"))
        .assert()
        .success()
        .stdout(predicate::str::diff("alias-owner\n").normalize());
    Ok(())
}

#[test]
fn destination_sidecar_collision_keeps_existing_contents() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let bin = context.temp_dir.child("bin");
    let root = context.temp_dir.child("tools");
    bin.create_dir_all()?;
    bin.child(".uv-tool-lock").write_str("existing contents")?;
    let wheel = wheel(
        context.temp_dir.path(),
        "collision-owner",
        "collision-command",
    )?;
    uv_snapshot!(context.filters(), install(&context, &wheel, root.path(), bin.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + collision-owner==1.0.0 (from file://[TEMP_DIR]/collision_owner-1.0.0-py3-none-any.whl)
    error: tool executable lock directory `bin/.uv-tool-lock` is not a directory
    ");
    assert_eq!(
        fs_err::read_to_string(bin.child(".uv-tool-lock"))?,
        "existing contents"
    );
    assert!(!executable(bin.path(), "collision-command").exists());
    assert!(!root.child("collision-owner/uv-receipt.toml").exists());
    Ok(())
}

#[test]
fn uninstall_does_not_recreate_removed_destination() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let bin = context.temp_dir.child("bin");
    let root = context.temp_dir.child("tools");
    let wheel = wheel(context.temp_dir.path(), "removed-owner", "removed-command")?;
    install(&context, &wheel, root.path(), bin.path())
        .assert()
        .success();
    fs_err::remove_dir_all(bin.path())?;
    uv_snapshot!(context.filters(), context.tool_uninstall().arg("removed-owner"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed environment for `removed-owner`
    ");
    assert!(!bin.exists());
    assert!(!root.child("removed-owner").exists());
    Ok(())
}

#[test]
fn destination_sidecar_is_not_listed_as_a_tool() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let root = context.temp_dir.child("tools");
    let wheel = wheel(context.temp_dir.path(), "listed-owner", "listed-command")?;
    install(&context, &wheel, root.path(), root.path())
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    listed-owner v1.0.0
    - listed-command
    ");
    assert!(root.child(".uv-tool-lock/lock").is_file());
    Ok(())
}
