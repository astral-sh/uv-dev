use std::collections::BTreeMap;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;
#[cfg(unix)]
use uv_cache_key::cache_digest;
use uv_static::EnvVars;
use uv_test::TestContext;

fn setuptools_fixture(context: &TestContext, name: &str) -> Result<PathBuf> {
    let source = context.temp_dir.child(name);
    source.child("pyproject.toml").write_str(indoc! {r#"
        [build-system]
        requires = []
        build-backend = "setuptools.build_meta"
        backend-path = ["."]
    "#})?;
    let (filename, wheel) = uv_test::packse::generate_wheel(
        &name.parse()?,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    fs_err::write(source.join(&filename), wheel)?;
    source.child("setuptools/__init__.py").write_str("")?;
    source
        .child("setuptools/build_meta.py")
        .write_str(&formatdoc! {r#"
        import contextlib
        import os
        import shutil
        import socket
        from pathlib import Path

        root = Path(__file__).parent.parent

        @contextlib.contextmanager
        def source_mutation(phase):
            active = root / "hook-active"
            active.mkdir()
            try:
                if phase == os.environ.get("UV_TEST_HOOK_PHASE"):
                    host, port = os.environ["UV_TEST_HOOK_ADDRESS"].split(":")
                    with socket.create_connection((host, int(port)), timeout=30) as gate:
                        gate.settimeout(None)
                        gate.sendall((phase + ":{name}:" + str(os.getpid()) + "\n").encode())
                        if gate.recv(1) != b"S":
                            raise RuntimeError("hook gate closed")
                yield
            finally:
                active.rmdir()

        def get_requires_for_build_wheel(config_settings=None):
            with source_mutation("requires"):
                return []

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            with source_mutation("build"):
                shutil.copyfile(root / "{filename}", Path(wheel_directory) / "{filename}")
                return "{filename}"
    "#})?;
    Ok(source.path().to_owned())
}

struct HookGate {
    stream: TcpStream,
    phase: String,
    name: String,
    #[cfg(unix)]
    pid: u32,
}

async fn hook_gate(listener: &TcpListener) -> Result<HookGate> {
    let (stream, _) = tokio::time::timeout(Duration::from_secs(30), listener.accept()).await??;
    let mut reader = tokio::io::BufReader::new(stream);
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(30), reader.read_line(&mut line)).await??;
    let mut fields = line.trim().split(':');
    let phase = fields.next().context("hook phase")?.to_owned();
    let name = fields.next().context("hook name")?.to_owned();
    #[cfg(unix)]
    let pid = fields.next().context("hook process ID")?.parse()?;
    Ok(HookGate {
        stream: reader.into_inner(),
        phase,
        name,
        #[cfg(unix)]
        pid,
    })
}

async fn wait_for_source_lock(
    child: &mut tokio::process::Child,
) -> Result<tokio::task::JoinHandle<std::io::Result<Vec<u8>>>> {
    let mut reader =
        tokio::io::BufReader::new(child.stderr.take().context("captured build stderr")?);
    let mut output = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let start = output.len();
            if reader.read_until(b'\n', &mut output).await? == 0 {
                anyhow::bail!("build exited without waiting for its source lock");
            }
            let line = String::from_utf8_lossy(&output[start..]);
            if line.contains("Waiting to acquire exclusive lock") && line.contains("uv-setuptools-")
            {
                return Ok::<_, anyhow::Error>(());
            }
        }
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(err)) => anyhow::bail!("{err}\n{}", String::from_utf8_lossy(&output)),
        Err(err) => anyhow::bail!(
            "Timed out waiting for source admission: {err}\n{}",
            String::from_utf8_lossy(&output)
        ),
    }
    Ok(tokio::spawn(async move {
        reader.read_to_end(&mut output).await?;
        Ok(output)
    }))
}

#[cfg(unix)]
#[tokio::test]
async fn setuptools_setup_locks_the_canonical_source_only() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = setuptools_fixture(&context, "demo")?;
    let unrelated = setuptools_fixture(&context, "unrelated")?;
    let alias = context.temp_dir.join("alias");
    fs_err::os::unix::fs::symlink(&source, &alias)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?.to_string();
    let mut first = context.build();
    first
        .arg(&source)
        .args(["--wheel", "--offline", "--out-dir"])
        .arg(context.temp_dir.join("first-out"))
        .env("UV_TEST_HOOK_ADDRESS", &address)
        .env("UV_TEST_HOOK_PHASE", "requires");
    let first = tokio::process::Command::from(first)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut first_gate = hook_gate(&listener).await?;
    assert_eq!(first_gate.phase, "requires");

    let second_cache = context.temp_dir.join("second-cache");
    let context = context.with_cache_dir(second_cache);
    let mut second = context.build();
    second
        .arg(alias)
        .args(["--wheel", "--offline", "--out-dir"])
        .arg(context.temp_dir.join("second-out"))
        .env("UV_TEST_HOOK_ADDRESS", &address)
        .env("UV_TEST_HOOK_PHASE", "requires")
        .env(EnvVars::RUST_LOG, "uv_fs=info");
    let mut second = tokio::process::Command::from(second)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let second_stderr = wait_for_source_lock(&mut second).await?;

    // Another source tree can complete while the first setup hook holds admission.
    let unrelated_cache = context.temp_dir.join("unrelated-cache");
    let context = context.with_cache_dir(unrelated_cache);
    let mut third = context.build();
    third
        .arg(unrelated)
        .args(["--wheel", "--offline", "--out-dir"])
        .arg(context.temp_dir.join("unrelated-out"));
    let third = tokio::process::Command::from(third)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    tokio::time::timeout(Duration::from_secs(30), third.wait_with_output())
        .await??
        .assert()
        .success();

    first_gate.stream.write_all(b"S").await?;
    let mut second_gate = hook_gate(&listener).await?;
    assert_eq!(second_gate.phase, "requires");
    second_gate.stream.write_all(b"S").await?;
    tokio::time::timeout(Duration::from_secs(30), first.wait_with_output())
        .await??
        .assert()
        .success();
    let mut output =
        tokio::time::timeout(Duration::from_secs(30), second.wait_with_output()).await??;
    output.stderr = second_stderr.await??;
    output.assert().success();
    assert!(!source.join("hook-active").exists());
    Ok(())
}

#[tokio::test]
async fn setuptools_setup_waits_for_an_active_build_hook() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = setuptools_fixture(&context, "demo")?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?.to_string();
    let mut first = context.build();
    first
        .arg(&source)
        .args(["--wheel", "--offline", "--out-dir"])
        .arg(context.temp_dir.join("first-out"))
        .env("UV_TEST_HOOK_ADDRESS", &address)
        .env("UV_TEST_HOOK_PHASE", "build");
    let first = tokio::process::Command::from(first)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut first_gate = hook_gate(&listener).await?;
    assert_eq!(first_gate.phase, "build");

    let second_cache = context.temp_dir.join("second-cache");
    let context = context.with_cache_dir(second_cache);
    let mut second = context.build();
    second
        .arg(&source)
        .args(["--wheel", "--offline", "--out-dir"])
        .arg(context.temp_dir.join("second-out"))
        .env("UV_TEST_HOOK_ADDRESS", &address)
        .env("UV_TEST_HOOK_PHASE", "requires")
        .env(EnvVars::RUST_LOG, "uv_fs=info");
    let mut second = tokio::process::Command::from(second)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let second_stderr = wait_for_source_lock(&mut second).await?;
    first_gate.stream.write_all(b"S").await?;
    let mut second_gate = hook_gate(&listener).await?;
    assert_eq!(second_gate.phase, "requires");
    second_gate.stream.write_all(b"S").await?;
    tokio::time::timeout(Duration::from_secs(30), first.wait_with_output())
        .await??
        .assert()
        .success();
    let mut output =
        tokio::time::timeout(Duration::from_secs(30), second.wait_with_output()).await??;
    output.stderr = second_stderr.await??;
    output.assert().success();
    Ok(())
}

#[tokio::test]
async fn failed_requirement_discovery_cancels_another_protected_hook() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let blocked = setuptools_fixture(&context, "blocked")?;
    let failing = setuptools_fixture(&context, "failing")?;
    fs_err::OpenOptions::new()
        .append(true)
        .open(failing.join("setuptools/build_meta.py"))?
        .write_all(
            indoc! {r#"

            def get_requires_for_build_wheel(config_settings=None):
                with source_mutation("requires"):
                    raise RuntimeError("intentional hook failure")
        "#}
            .as_bytes(),
        )?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!(
            "blocked @ {}\nfailing @ {}\n",
            Url::from_directory_path(blocked)
                .map_err(|()| anyhow::anyhow!("blocked source URL"))?,
            Url::from_directory_path(failing)
                .map_err(|()| anyhow::anyhow!("failing source URL"))?,
        ))?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let mut command = context.pip_compile();
    command
        .arg("requirements.in")
        .args(["--offline", "--no-index"])
        .env(EnvVars::UV_CONCURRENT_BUILDS, "2")
        .env("UV_TEST_HOOK_ADDRESS", listener.local_addr()?.to_string())
        .env("UV_TEST_HOOK_PHASE", "requires");
    let command = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let first = hook_gate(&listener).await?;
    let second = hook_gate(&listener).await?;
    let (mut blocked, mut failing) = if first.name == "blocked" {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(blocked.name, "blocked");
    assert_eq!(failing.name, "failing");
    failing.stream.write_all(b"S").await?;
    let output = tokio::time::timeout(Duration::from_secs(30), command.wait_with_output()).await;
    if output.is_err() {
        // Unblock a natural-exit-only waiter before reporting its cancellation failure.
        let _ = blocked.stream.write_all(b"S").await;
    }
    output??
        .assert()
        .failure()
        .stderr(predicates::str::contains("intentional hook failure"));
    let mut byte = [0];
    let closed =
        tokio::time::timeout(Duration::from_secs(30), blocked.stream.read(&mut byte)).await?;
    assert!(
        matches!(&closed, Ok(0))
            || matches!(&closed, Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset),
        "the canceled hook must close its connection: {closed:?}"
    );
    #[cfg(unix)]
    assert_process_exited(&context, blocked.pid);
    Ok(())
}

#[cfg(unix)]
fn assert_process_exited(context: &TestContext, pid: u32) {
    context
        .external_command(&context.python_versions[0].1)
        .args([
            "-c",
            &formatdoc! {r"
        import os
        try:
            os.kill({pid}, 0)
        except ProcessLookupError:
            pass
        else:
            raise SystemExit('protected hook is still alive')
    "},
        ])
        .assert()
        .success();
}

#[cfg(unix)]
#[tokio::test]
async fn interrupting_a_protected_build_exits_the_process_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = setuptools_fixture(&context, "demo")?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let mut command = context.build();
    command
        .arg(source)
        .args(["--wheel", "--offline"])
        .env("UV_TEST_HOOK_ADDRESS", listener.local_addr()?.to_string())
        .env("UV_TEST_HOOK_PHASE", "requires")
        .process_group(0);
    let command = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut gate = hook_gate(&listener).await?;
    let pid = command.id().context("uv process group ID")?;
    context
        .external_command(&context.python_versions[0].1)
        .args([
            "-c",
            &format!("import os, signal; os.killpg({pid}, signal.SIGINT)"),
        ])
        .assert()
        .success();
    let output =
        tokio::time::timeout(Duration::from_secs(30), command.wait_with_output()).await??;
    assert!(output.status.signal() == Some(2) || output.status.code() == Some(130));
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(30), gate.stream.read(&mut byte)).await??,
        0
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn setuptools_setup_releases_source_before_building_extra_requirements() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = setuptools_fixture(&context, "parent")?;
    let helper = setuptools_fixture(&context, "helper")?;
    fs_err::OpenOptions::new()
        .append(true)
        .open(source.join("setuptools/build_meta.py"))?
        .write_all(
            indoc! {r#"

            def get_requires_for_build_wheel(config_settings=None):
                with source_mutation("requires"):
                    return ["helper @ " + (root.parent / "helper").as_uri()]
        "#}
            .as_bytes(),
        )?;
    fs_err::OpenOptions::new()
        .append(true)
        .open(helper.join("setuptools/build_meta.py"))?
        .write_all(
            indoc! {r#"

            def get_requires_for_build_wheel(config_settings=None):
                import fcntl
                with source_mutation("requires"):
                    with open(os.environ["UV_TEST_PARENT_SOURCE_LOCK"], "r+") as lock:
                        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        fcntl.flock(lock, fcntl.LOCK_UN)
                return []
        "#}
            .as_bytes(),
        )?;
    let lock = std::env::temp_dir().join(format!(
        "uv-setuptools-{}.lock",
        cache_digest(&fs_err::canonicalize(&source)?)
    ));
    context
        .build()
        .arg(source)
        .args(["--wheel", "--offline", "--no-index"])
        .env("UV_TEST_PARENT_SOURCE_LOCK", lock)
        .assert()
        .success();
    Ok(())
}
