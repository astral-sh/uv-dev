use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use nix::errno::Errno;
use nix::libc;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde_json::Value;
use tempfile::TempDir;

use uv_test::{TestContext, get_bin, uv_snapshot};

/// Cargo's jobserver descriptors belong to the test harness, not the command under test. Mark
/// them close-on-exec in the child without changing descriptors used by other test threads.
#[allow(unsafe_code)]
fn isolated_command(mut command: Command) -> Command {
    #[cfg(target_os = "linux")]
    let directory = "/proc/self/fd";
    #[cfg(target_os = "macos")]
    let directory = "/dev/fd";
    let descriptors = fs_err::read_dir(directory)
        .expect("read inherited descriptors")
        .filter_map(|entry| {
            entry
                .expect("read descriptor entry")
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
                .filter(|descriptor| *descriptor > 2)
        })
        .collect::<Vec<_>>();
    // SAFETY: The child closure only calls async-signal-safe fcntl and reads the descriptor list
    // allocated before fork. Standard streams have already been installed by Command.
    unsafe {
        command.pre_exec(move || {
            for &descriptor in &descriptors {
                let flags = libc::fcntl(descriptor, libc::F_GETFD);
                if flags == -1 {
                    if Errno::last() == Errno::EBADF {
                        continue;
                    }
                    return Err(std::io::Error::last_os_error());
                }
                if libc::fcntl(descriptor, libc::F_SETFD, flags | libc::FD_CLOEXEC) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    command
}

struct Daemon {
    directory: TempDir,
    executable: PathBuf,
}

impl Daemon {
    fn start(context: &TestContext) -> Result<Option<Self>> {
        let daemon = Self {
            directory: tempfile::Builder::new().prefix("uvd-").tempdir()?,
            executable: get_bin!(),
        };
        fs_err::set_permissions(
            daemon.directory.path(),
            std::fs::Permissions::from_mode(0o700),
        )?;
        let output = daemon.command(context).arg("--daemon").output()?;
        if output.status.code() == Some(2)
            && [
                b"error: The experimental daemon cannot be enabled: Sandboxed commands must execute in their original process\n".as_slice(),
                b"error: The experimental daemon cannot be enabled: Restricted commands must execute in their original process\n".as_slice(),
            ].contains(&output.stderr.as_slice())
        {
            return Ok(None);
        }
        output.assert().success();
        Ok(Some(daemon))
    }

    fn command(&self, context: &TestContext) -> Command {
        let mut command = context.external_command(&self.executable);
        command
            .current_dir(&context.temp_dir)
            .env("UV_DAEMON_DIR", self.directory.path());
        isolated_command(command)
    }

    fn status(&self, context: &TestContext) -> Result<Value> {
        let output = self.command(context).arg("--daemon-status").output()?;
        self.assert_success(&output)?;
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn assert_success(&self, output: &Output) -> Result<()> {
        if output.status.success() {
            return Ok(());
        }
        let mut logs = String::new();
        for entry in fs_err::read_dir(self.directory.path())? {
            let path = entry?.path();
            if path.extension().is_some_and(|extension| extension == "log") {
                logs.push_str(&fs_err::read_to_string(path)?);
            }
        }
        bail!(
            "{}\nDaemon log:\n{logs}",
            String::from_utf8_lossy(&output.stderr)
        )
    }

    fn export(&self, context: &TestContext, local: bool) -> Result<Output> {
        let mut command = self.command(context);
        if local {
            command.arg("--no-daemon");
        }
        Ok(command
            .args(["export", "--frozen", "--no-header", "--no-hashes"])
            .output()?)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = Command::new(&self.executable)
            .env("UV_DAEMON_DIR", self.directory.path())
            .arg("--no-daemon")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn write_project(context: &TestContext) -> Result<String> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dependency"]
    "#})?;
    let lock = indoc! {r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "dependency"
        version = "1.0.0"
        source = { registry = "https://pypi.org/simple" }

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "dependency" },
        ]
    "#}
    .to_owned();
    context.temp_dir.child("uv.lock").write_str(&lock)?;
    Ok(lock)
}

#[test]
fn daemon_lock_cache_invalidation() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let lock = write_project(&context)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };

    let local = daemon.export(&context, true)?;
    daemon.assert_success(&local)?;
    let cold = daemon.export(&context, false)?;
    daemon.assert_success(&cold)?;
    assert_eq!(local.stdout, cold.stdout);
    assert_eq!(local.stderr, cold.stderr);
    let status = daemon.status(&context)?;
    assert_eq!(status["cached_locks"], 1);
    assert_eq!(status["cache_hits"], 0);

    let warm = daemon.export(&context, false)?;
    daemon.assert_success(&warm)?;
    assert_eq!(local.stdout, warm.stdout);
    assert_eq!(daemon.status(&context)?["cache_hits"], 1);

    // A replacement with the same length and mtime must not reuse the old parsed lock.
    let lock_path = context.temp_dir.child("uv.lock");
    let modified = filetime::FileTime::from_last_modification_time(&fs_err::metadata(&lock_path)?);
    let replacement = context.temp_dir.child("replacement.lock");
    replacement.write_str(&lock.replace("1.0.0", "2.0.0"))?;
    filetime::set_file_mtime(&replacement, modified)?;
    fs_err::rename(&replacement, &lock_path)?;
    let updated = daemon.export(&context, false)?;
    daemon.assert_success(&updated)?;
    assert_eq!(updated.stdout, daemon.export(&context, true)?.stdout);
    assert_ne!(updated.stdout, warm.stdout);
    assert_eq!(daemon.status(&context)?["cached_locks"], 2);

    lock_path.write_str("invalid lockfile\n")?;
    let invalid = daemon.export(&context, false)?;
    let local_invalid = daemon.export(&context, true)?;
    assert!(!invalid.status.success());
    assert_eq!(invalid.status.code(), local_invalid.status.code());
    assert_eq!(invalid.stderr, local_invalid.stderr);
    fs_err::remove_file(&lock_path)?;
    let missing = daemon.export(&context, false)?;
    let local_missing = daemon.export(&context, true)?;
    assert!(!missing.status.success());
    assert_eq!(missing.status.code(), local_missing.status.code());
    assert_eq!(missing.stderr, local_missing.stderr);
    Ok(())
}

#[test]
fn daemon_cache_respects_working_directory() -> Result<()> {
    let first = uv_test::test_context_with_versions!(&[]);
    let first_lock = write_project(&first)?;
    first.temp_dir.child("uv.lock").write_str(&format!(
        "{first_lock}\n[package.metadata]\nrequires-dist = [{{ name = \"dependency\", directory = \"relative\" }}]\n"
    ))?;
    let second = uv_test::test_context_with_versions!(&[]);
    let second_lock = write_project(&second)?;
    second
        .temp_dir
        .child("uv.lock")
        .write_str(&second_lock.replace("1.0.0", "2.0.0"))?;
    let Some(daemon) = Daemon::start(&first)? else {
        return Ok(());
    };
    daemon.assert_success(&daemon.export(&first, false)?)?;
    let second_local = daemon.export(&second, true)?;
    let second_remote = daemon.export(&second, false)?;
    daemon.assert_success(&second_remote)?;
    assert_eq!(second_local.stdout, second_remote.stdout);
    assert_eq!(second_local.stderr, second_remote.stderr);
    assert_eq!(daemon.status(&second)?["cached_locks"], 2);
    Ok(())
}

#[test]
fn daemon_reaps_workers_after_inherited_ignored_sigchld() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    write_project(&context)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };
    daemon.assert_success(&daemon.command(&context).arg("--no-daemon").output()?)?;
    let output = isolated_command(context.external_command("sh"))
        .current_dir(&context.temp_dir)
        .env("UV_DAEMON_DIR", daemon.directory.path())
        .args(["-c", "trap '' CHLD; exec \"$@\"", "sh"])
        .arg(&daemon.executable)
        .arg("--daemon")
        .output()?;
    daemon.assert_success(&output)?;
    daemon.assert_success(&daemon.export(&context, false)?)?;
    assert_eq!(daemon.status(&context)?["completed_requests"], 1);
    Ok(())
}

#[test]
fn daemon_process_context() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_project(&context)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };
    let python = &context.python_versions[0].1;
    let mut command = daemon.command(&context);
    command
        .args(["run", "--no-project", "--"])
        .arg(python)
        .args(["-c", "import os, sys; print(os.environ['DAEMON_VALUE']); print(os.path.basename(os.getcwd())); print(sys.stdin.read(), end=''); print('stderr', file=sys.stderr); sys.exit(37)"])
        .env("DAEMON_VALUE", "first")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .context("child stdin")?
            .write_all(b"stdin\n")?;
    }
    let output = child.wait_with_output()?;
    assert_eq!(output.status.code(), Some(37));
    assert_eq!(output.stdout, b"first\ntemp\nstdin\n");
    assert_eq!(output.stderr, b"stderr\n");

    uv_snapshot!(context.filters(), daemon.command(&context)
        .args(["run", "--no-project", "--"])
        .arg(python)
        .args(["-c", "import os; print(os.environ['DAEMON_VALUE'])"])
        .env("DAEMON_VALUE", "second"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    second
    ");

    let status = daemon.status(&context)?;
    if status["completed_requests"] != 2 {
        bail!("expected both commands to run through the daemon: {status}");
    }
    Ok(())
}

#[test]
fn daemon_concurrent_commands_and_shutdown() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_project(&context)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };
    let python = &context.python_versions[0].1;
    let script = "from pathlib import Path; import sys, time; Path(sys.argv[1]).touch(); deadline = time.monotonic() + 15\nwhile not Path('release').exists() and time.monotonic() < deadline: time.sleep(0.01)\nassert Path('release').exists(); print(sys.argv[1])";
    let spawn = |name: &str| {
        daemon
            .command(&context)
            .args(["run", "--no-project", "--"])
            .arg(python)
            .args(["-c", script, name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    };
    let first = spawn("first")?;
    let second = spawn("second")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(context.temp_dir.child("first").exists() && context.temp_dir.child("second").exists())
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    let both_started =
        context.temp_dir.child("first").exists() && context.temp_dir.child("second").exists();
    let active_workers = daemon.status(&context)?["active_workers"].as_u64();
    let mut stop = daemon
        .command(&context)
        .arg("--no-daemon")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stopping = false;
    while Instant::now() < deadline {
        if daemon.status(&context)?["stopping"] == true {
            stopping = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let drained_early = stop.try_wait()?.is_some();
    context.temp_dir.child("release").write_str("")?;
    let first = first.wait_with_output()?;
    let second = second.wait_with_output()?;
    let stop = stop.wait_with_output()?;
    ensure!(
        both_started,
        "both workers must reach their readiness barrier"
    );
    assert_eq!(active_workers, Some(2));
    assert!(stopping);
    assert!(!drained_early);
    daemon.assert_success(&first)?;
    daemon.assert_success(&second)?;
    daemon.assert_success(&stop)?;
    assert_eq!(first.stdout, b"first\n");
    assert_eq!(second.stdout, b"second\n");
    Ok(())
}

#[test]
fn daemon_forwards_direct_sigterm() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_project(&context)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };
    let python = &context.python_versions[0].1;
    let mut child = daemon.command(&context)
        .args(["run", "--no-project", "--"])
        .arg(python)
        .args(["-c", "from pathlib import Path; import signal, sys; signal.signal(signal.SIGTERM, lambda *_: sys.exit(23)); signal.alarm(15); Path('ready').touch(); signal.pause()"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !context.temp_dir.child("ready").exists() && Instant::now() < deadline {
        if child.try_wait()?.is_some() {
            bail!("signal-test worker exited before becoming ready");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let ready = context.temp_dir.child("ready").exists();
    kill(Pid::from_raw(i32::try_from(child.id())?), Signal::SIGTERM)?;
    let output = child.wait_with_output()?;
    ensure!(ready, "signal-test worker must reach its readiness barrier");
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
    Ok(())
}

#[test]
fn daemon_preserves_extra_descriptors_by_running_locally() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_project(&context)?;
    context
        .temp_dir
        .child("descriptor")
        .write_str("descriptor\n")?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };
    let python = &context.python_versions[0].1;
    let before = daemon.status(&context)?["completed_requests"].clone();
    let output = isolated_command(context.external_command("sh"))
        .current_dir(&context.temp_dir)
        .env("UV_DAEMON_DIR", daemon.directory.path())
        .args(["-c", "exec 3<descriptor; exec \"$@\"", "sh"])
        .arg(&daemon.executable)
        .args(["run", "--no-project", "--"])
        .arg(python)
        .args([
            "-c",
            "import os, sys; sys.stdout.buffer.write(os.read(3, 100))",
        ])
        .output()?;
    daemon.assert_success(&output)?;
    assert_eq!(output.stdout, b"descriptor\n");
    assert_eq!(output.stderr, b"");
    assert_eq!(daemon.status(&context)?["completed_requests"], before);
    Ok(())
}
