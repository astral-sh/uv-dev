use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use nix::errno::Errno;
use nix::libc;
use nix::sys::signal::{self, SigSet, SigmaskHow, Signal, kill};
use nix::unistd::Pid;
use serde::Serialize;
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

#[allow(unsafe_code)]
fn block_sigchld(mut command: Command) -> Command {
    let signals = SigSet::from(Signal::SIGCHLD);
    // SAFETY: The child closure only changes its signal mask with async-signal-safe sigprocmask.
    // The signal set is prepared before fork, and the test harness's mask is not modified.
    unsafe {
        command.pre_exec(move || {
            signal::sigprocmask(SigmaskHow::SIG_BLOCK, Some(&signals), None)
                .map_err(io::Error::from)
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

    fn workspace_list(&self, context: &TestContext, local: bool) -> Command {
        let mut command = self.command(context);
        if local {
            command.arg("--no-daemon");
        }
        command.args(["--offline", "workspace", "list"]);
        command
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

/// Wait without sending status requests, which could conceal a missed child notification by
/// waking the daemon's listener. A failed wait wakes the server only to clean up the test.
fn wait_without_daemon_requests(
    daemon: &Daemon,
    context: &TestContext,
    mut children: Vec<Child>,
) -> Result<Vec<Output>> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let finished = loop {
        let mut finished = true;
        for child in &mut children {
            if child.try_wait()?.is_none() {
                finished = false;
            }
        }
        if finished || Instant::now() >= deadline {
            break finished;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !finished {
        let _ = daemon.status(context);
        for child in &mut children {
            let _ = child.kill();
        }
    }
    let outputs = children
        .into_iter()
        .map(Child::wait_with_output)
        .collect::<io::Result<Vec<_>>>()?;
    ensure!(
        finished,
        "Daemon commands did not finish without additional requests"
    );
    Ok(outputs)
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

fn assert_output_eq(local: &Output, delegated: &Output) {
    assert_eq!(local.status.code(), delegated.status.code());
    assert_eq!(local.stdout, delegated.stdout);
    assert_eq!(local.stderr, delegated.stderr);
}

fn manifest_cache_hits(status: &Value) -> Result<u64> {
    status["manifest_cache_hits"]
        .as_u64()
        .context("daemon manifest-cache hit counter")
}

#[test]
fn daemon_manifest_cache_rejects_non_worker_observations() -> Result<()> {
    #[derive(Serialize)]
    enum CacheRequest {
        Manifests,
    }

    let context = uv_test::test_context_with_versions!(&[]);
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };
    let before = daemon.status(&context)?;
    let socket = fs_err::read_dir(daemon.directory.path())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "sock")
        })
        .context("daemon socket")?;
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut encoded = Vec::new();
    for frame in [
        rmp_serde::to_vec(&CacheRequest::Manifests)?,
        rmp_serde::to_vec(&(
            vec!["[project]\nname = 'untrusted'\nversion = '1'\n"],
            u64::MAX,
            u64::MAX,
        ))?,
    ] {
        encoded.extend(u32::try_from(frame.len())?.to_be_bytes());
        encoded.extend(frame);
    }
    match stream.write_all(&encoded) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
            ) => {}
        Err(error) => return Err(error.into()),
    }
    let mut response = [0; 4];
    match stream.read(&mut response) {
        Ok(0) => {}
        Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
        Ok(_) => bail!("daemon acknowledged a cache batch from a non-worker"),
        Err(error) => return Err(error.into()),
    }
    let after = daemon.status(&context)?;
    for field in [
        "completed_requests",
        "cached_manifests",
        "cached_manifest_source_bytes",
        "manifest_cache_hits",
        "manifest_cache_dropped",
    ] {
        assert_eq!(before[field], after[field]);
    }
    Ok(())
}

#[test]
fn daemon_manifest_cache_invalidation() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let source = indoc! {r#"
        [project]
        name = "first"
        version = "0.1.0"
    "#};
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(source)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };

    let local = daemon.workspace_list(&context, true).output()?;
    daemon.assert_success(&local)?;
    assert_eq!(local.stdout, b"first\n");
    let cold = daemon.workspace_list(&context, false).output()?;
    daemon.assert_success(&cold)?;
    assert_output_eq(&local, &cold);
    let cold_status = daemon.status(&context)?;
    assert_eq!(cold_status["cached_manifests"], 1);
    assert_eq!(cold_status["cached_manifest_source_bytes"], source.len());
    assert_eq!(cold_status["manifest_cache_dropped"], 0);

    let warm = daemon.workspace_list(&context, false).output()?;
    daemon.assert_success(&warm)?;
    assert_output_eq(&local, &warm);
    assert!(manifest_cache_hits(&daemon.status(&context)?)? > manifest_cache_hits(&cold_status)?);

    // File identity, size, and timestamps are not substitutes for the bytes that were read.
    let replacement_source = source.replace("\"first\"", "\"other\"");
    assert_eq!(replacement_source.len(), source.len());
    let modified = filetime::FileTime::from_last_modification_time(&fs_err::metadata(&manifest)?);
    let replacement = context.temp_dir.child("replacement.toml");
    replacement.write_str(&replacement_source)?;
    filetime::set_file_mtime(&replacement, modified)?;
    fs_err::rename(&replacement, &manifest)?;
    let updated = daemon.workspace_list(&context, false).output()?;
    daemon.assert_success(&updated)?;
    assert_eq!(updated.stdout, b"other\n");
    assert_output_eq(&daemon.workspace_list(&context, true).output()?, &updated);
    let updated_status = daemon.status(&context)?;
    assert_eq!(updated_status["cached_manifests"], 2);
    assert_eq!(
        updated_status["cached_manifest_source_bytes"],
        source.len() + replacement_source.len()
    );

    manifest.write_str("[project\n")?;
    let invalid = daemon.workspace_list(&context, false).output()?;
    let local_invalid = daemon.workspace_list(&context, true).output()?;
    assert!(!invalid.status.success());
    assert_output_eq(&local_invalid, &invalid);
    assert_eq!(daemon.status(&context)?["cached_manifests"], 2);

    fs_err::remove_file(&manifest)?;
    let missing = daemon.workspace_list(&context, false).output()?;
    let local_missing = daemon.workspace_list(&context, true).output()?;
    assert!(!missing.status.success());
    assert_output_eq(&local_missing, &missing);
    assert_eq!(daemon.status(&context)?["cached_manifests"], 2);
    Ok(())
}

#[test]
fn daemon_manifest_cache_flushes_after_config_bypass() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let source = indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
    "#};
    context.temp_dir.child("pyproject.toml").write_str(source)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };
    let run = |local| {
        daemon
            .workspace_list(&context, local)
            .arg("--no-config")
            .output()
    };
    let local = run(true)?;
    daemon.assert_success(&local)?;
    assert_eq!(daemon.status(&context)?["cached_manifests"], 0);

    // Configuration discovery is disabled, so only the invocation's final flush can publish
    // the manifest read by `workspace list`.
    let cold = run(false)?;
    daemon.assert_success(&cold)?;
    assert_output_eq(&local, &cold);
    let cold_status = daemon.status(&context)?;
    assert_eq!(cold_status["cached_manifests"], 1);
    assert_eq!(cold_status["cached_manifest_source_bytes"], source.len());
    assert_eq!(cold_status["manifest_cache_dropped"], 0);

    let warm = run(false)?;
    daemon.assert_success(&warm)?;
    assert_output_eq(&local, &warm);
    assert!(manifest_cache_hits(&daemon.status(&context)?)? > manifest_cache_hits(&cold_status)?);
    Ok(())
}

#[test]
fn daemon_manifest_cache_lowers_in_request_context() -> Result<()> {
    let first = uv_test::test_context_with_versions!(&[]);
    let second = uv_test::test_context_with_versions!(&[]);
    let source = indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"

        [tool.uv]
        constraint-dependencies = [
            "dependency @ ${DAEMON_MANIFEST_URL}",
            "archive @ file://${PROJECT_ROOT}/archive",
        ]
    "#};
    first.temp_dir.child("pyproject.toml").write_str(source)?;
    second.temp_dir.child("pyproject.toml").write_str(source)?;
    first.temp_dir.child("archive").create_dir_all()?;
    second
        .temp_dir
        .child("archive")
        .write_str("not an archive")?;
    let Some(daemon) = Daemon::start(&first)? else {
        return Ok(());
    };
    let run = |context: &TestContext, local, url| {
        daemon
            .workspace_list(context, local)
            .env_remove("PROJECT_ROOT")
            .env("DAEMON_MANIFEST_URL", url)
            .output()
    };
    let valid_url = "https://example.org/dependency-1.0.0.tar.gz";

    let local = run(&first, true, valid_url)?;
    daemon.assert_success(&local)?;
    let cold = run(&first, false, valid_url)?;
    daemon.assert_success(&cold)?;
    assert_output_eq(&local, &cold);
    let before = daemon.status(&first)?;
    assert_eq!(before["cached_manifests"], 1);

    // The same source classifies the second directory's regular file independently.
    let local_file = run(&second, true, valid_url)?;
    let delegated_file = run(&second, false, valid_url)?;
    assert!(!local_file.status.success());
    assert_output_eq(&local_file, &delegated_file);

    fs_err::remove_file(second.temp_dir.child("archive"))?;
    second.temp_dir.child("archive").create_dir_all()?;
    let local_directory = run(&second, true, valid_url)?;
    let delegated_directory = run(&second, false, valid_url)?;
    daemon.assert_success(&delegated_directory)?;
    assert_output_eq(&local_directory, &delegated_directory);

    // A syntax-cache hit must still expand the current request's environment.
    let invalid_url = "hg+https://example.org/dependency";
    let local_environment = run(&second, true, invalid_url)?;
    let delegated_environment = run(&second, false, invalid_url)?;
    assert!(!local_environment.status.success());
    assert_output_eq(&local_environment, &delegated_environment);
    let after = daemon.status(&second)?;
    assert_eq!(after["cached_manifests"], 1);
    assert_eq!(after["cached_manifest_source_bytes"], source.len());
    assert!(manifest_cache_hits(&after)? > manifest_cache_hits(&before)?);
    Ok(())
}

#[test]
fn daemon_manifest_cache_replays_diagnostics() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"

        [tool.uv]
        dev-dependencies = []
    "#})?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };

    let local = daemon.workspace_list(&context, true).output()?;
    daemon.assert_success(&local)?;
    let cold = daemon.workspace_list(&context, false).output()?;
    daemon.assert_success(&cold)?;
    assert_output_eq(&local, &cold);
    uv_snapshot!(context.filters(), daemon.workspace_list(&context, false), @r"
    exit_code: 0 (success)
    ----- stdout -----
    project

    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    ");

    let local_quiet = daemon
        .workspace_list(&context, true)
        .arg("--quiet")
        .output()?;
    let delegated_quiet = daemon
        .workspace_list(&context, false)
        .arg("--quiet")
        .output()?;
    daemon.assert_success(&delegated_quiet)?;
    assert_output_eq(&local_quiet, &delegated_quiet);
    assert!(delegated_quiet.stderr.is_empty());
    assert_output_eq(&local, &daemon.workspace_list(&context, false).output()?);

    // Successful syntax parsing can be cached even when typed project validation fails.
    manifest.write_str("[project]\nname = \"project\"\n")?;
    let local_error = daemon.workspace_list(&context, true).output()?;
    let cold_error = daemon.workspace_list(&context, false).output()?;
    assert!(!local_error.status.success());
    assert_output_eq(&local_error, &cold_error);
    let before = daemon.status(&context)?;
    assert_eq!(before["cached_manifests"], 2);
    let warm_error = daemon.workspace_list(&context, false).output()?;
    assert_output_eq(&local_error, &warm_error);
    assert!(manifest_cache_hits(&daemon.status(&context)?)? > manifest_cache_hits(&before)?);
    Ok(())
}

#[test]
fn daemon_manifest_cache_allows_project_edits() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let control = uv_test::test_context_with_versions!(&[]);
    write_project(&context)?;
    write_project(&control)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };

    daemon.assert_success(&daemon.workspace_list(&context, false).output()?)?;
    daemon.assert_success(&daemon.workspace_list(&context, false).output()?)?;
    let before = daemon.status(&context)?;
    assert_eq!(before["cached_manifests"], 1);

    for expected in [b"0.1.1\n".as_slice(), b"0.1.2\n".as_slice()] {
        let arguments = [
            "--offline",
            "version",
            "--package",
            "project",
            "--bump",
            "patch",
            "--frozen",
        ];
        let local = daemon
            .command(&control)
            .arg("--no-daemon")
            .args(arguments)
            .output()?;
        daemon.assert_success(&local)?;
        let delegated = daemon.command(&context).args(arguments).output()?;
        daemon.assert_success(&delegated)?;
        assert_output_eq(&local, &delegated);
        assert_eq!(
            fs_err::read(context.temp_dir.child("pyproject.toml"))?,
            fs_err::read(control.temp_dir.child("pyproject.toml"))?
        );

        let version = daemon
            .command(&context)
            .args(["--offline", "version", "--short"])
            .output()?;
        daemon.assert_success(&version)?;
        assert_eq!(version.stdout, expected);
    }

    let after = daemon.status(&context)?;
    assert!(manifest_cache_hits(&after)? > manifest_cache_hits(&before)?);
    assert_eq!(after["manifest_cache_dropped"], 0);
    Ok(())
}

#[test]
fn daemon_manifest_cache_skips_large_sources() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let source = format!(
        "[project]\nname = \"project\"\nversion = \"0.1.0\"\n#{}\n",
        "x".repeat(256 * 1024)
    );
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&source)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };

    let local = daemon.workspace_list(&context, true).output()?;
    daemon.assert_success(&local)?;
    for _ in 0..2 {
        let delegated = daemon.workspace_list(&context, false).output()?;
        daemon.assert_success(&delegated)?;
        assert_output_eq(&local, &delegated);
    }
    let status = daemon.status(&context)?;
    assert_eq!(status["cached_manifests"], 0);
    assert_eq!(status["cached_manifest_source_bytes"], 0);
    Ok(())
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
fn daemon_reuses_lock_for_read_only_commands() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    context.temp_dir.child("uv.lock").write_str(indoc! {r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
    "#})?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };

    daemon.assert_success(&daemon.export(&context, false)?)?;
    assert_eq!(daemon.status(&context)?["cached_locks"], 1);
    assert_eq!(daemon.status(&context)?["cache_hits"], 0);

    for (index, arguments) in [
        ["tree", "--frozen", "--universal", "--offline"].as_slice(),
        ["--quiet", "sync", "--frozen", "--offline"].as_slice(),
        [
            "--quiet",
            "run",
            "--frozen",
            "--offline",
            "--",
            "python",
            "-c",
            "print('shared lock')",
        ]
        .as_slice(),
    ]
    .into_iter()
    .enumerate()
    {
        let local = daemon
            .command(&context)
            .arg("--no-daemon")
            .args(arguments)
            .output()?;
        daemon.assert_success(&local)?;
        let shared = daemon.command(&context).args(arguments).output()?;
        daemon.assert_success(&shared)?;
        assert_eq!(local.stdout, shared.stdout);
        assert_eq!(local.stderr, shared.stderr);
        assert_eq!(daemon.status(&context)?["cache_hits"], index + 1);
    }
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
    let child = daemon
        .command(&context)
        .args(["export", "--frozen", "--no-header", "--no-hashes"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    for output in wait_without_daemon_requests(&daemon, &context, vec![child])? {
        daemon.assert_success(&output)?;
    }
    assert_eq!(daemon.status(&context)?["completed_requests"], 1);
    Ok(())
}

#[test]
fn daemon_reaps_workers_after_inherited_blocked_sigchld() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    write_project(&context)?;
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };
    daemon.assert_success(&daemon.command(&context).arg("--no-daemon").output()?)?;
    let output = block_sigchld(daemon.command(&context))
        .arg("--daemon")
        .output()?;
    daemon.assert_success(&output)?;

    let child = daemon
        .command(&context)
        .args(["export", "--frozen", "--no-header", "--no-hashes"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    for output in wait_without_daemon_requests(&daemon, &context, vec![child])? {
        daemon.assert_success(&output)?;
    }
    let status = daemon.status(&context)?;
    assert_eq!(status["active_workers"], 0);
    assert_eq!(status["completed_requests"], 1);
    Ok(())
}

#[test]
fn daemon_reaps_short_command_bursts() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let Some(daemon) = Daemon::start(&context)? else {
        return Ok(());
    };
    let local = daemon
        .command(&context)
        .args(["--no-daemon", "cache", "dir"])
        .output()?;
    daemon.assert_success(&local)?;

    let mut children = Vec::new();
    for _ in 0..16 {
        children.push(
            daemon
                .command(&context)
                .args(["cache", "dir"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?,
        );
    }
    for output in wait_without_daemon_requests(&daemon, &context, children)? {
        daemon.assert_success(&output)?;
        assert_eq!(output.stdout, local.stdout);
        assert_eq!(output.stderr, local.stderr);
    }
    let status = daemon.status(&context)?;
    assert_eq!(status["active_workers"], 0);
    assert_eq!(status["completed_requests"], 16);
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
    let mut outputs =
        wait_without_daemon_requests(&daemon, &context, vec![first, second, stop])?.into_iter();
    let first = outputs.next().context("first worker output")?;
    let second = outputs.next().context("second worker output")?;
    let stop = outputs.next().context("drain output")?;
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
    let status: Value = serde_json::from_slice(&stop.stdout)?;
    assert_eq!(status["active_workers"], 0);
    assert_eq!(status["completed_requests"], 2);
    assert_eq!(status["stopping"], true);
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
