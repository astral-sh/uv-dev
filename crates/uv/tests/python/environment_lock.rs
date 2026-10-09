use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use uv_cache::Cache;
use uv_fs::Simplified;
use uv_python_interpreter::{EnvironmentLock, PythonEnvironment};
use uv_static::EnvVars;
use uv_test::packse::generate_wheel;

struct QueuedCommand {
    child: tokio::process::Child,
    stderr: tokio::io::BufReader<tokio::process::ChildStderr>,
    output: Vec<u8>,
}

impl QueuedCommand {
    fn spawn(mut command: Command) -> Result<Self> {
        command.env(EnvVars::RUST_LOG, "uv_fs=info");
        let mut child = tokio::process::Command::from(command)
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = child.stderr.take().context("captured command stderr")?;
        Ok(Self {
            child,
            stderr: tokio::io::BufReader::new(stderr),
            output: Vec::new(),
        })
    }

    async fn wait_for_destination(&mut self, resource: &Path) -> Result<()> {
        let resource = resource.simplified_display().to_string();
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let start = self.output.len();
                if self.stderr.read_until(b'\n', &mut self.output).await? == 0 {
                    anyhow::bail!("command exited before destination admission");
                }
                let line = String::from_utf8_lossy(&self.output[start..]);
                if line.contains("Waiting to acquire exclusive lock")
                    && line.contains("uv-environment-")
                    && line.contains(&resource)
                {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            result => anyhow::bail!(
                "Waiting for destination {resource}: {result:?}\nCaptured stderr:\n{}",
                String::from_utf8_lossy(&self.output)
            ),
        }
    }

    async fn finish(mut self) -> Result<()> {
        let stderr = tokio::spawn(async move {
            self.stderr.read_to_end(&mut self.output).await?;
            Ok::<_, std::io::Error>(self.output)
        });
        let mut output =
            tokio::time::timeout(Duration::from_secs(30), self.child.wait_with_output()).await??;
        output.stderr = stderr.await??;
        output.assert().success();
        Ok(())
    }
}

#[tokio::test]
#[cfg(unix)]
async fn explicit_venv_replacement_waits_through_parent_alias() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context.venv().assert().success();
    let cache = Cache::from_path(context.cache_dir.path().to_path_buf());
    let destination = fs_err::canonicalize(context.venv.path())?;
    let marker = destination.join("owner");
    fs_err::write(&marker, "owned")?;
    let alias = context.root.child("alias");
    fs_err::os::unix::fs::symlink(context.temp_dir.path(), alias.path())?;
    let guard = EnvironmentLock::acquire(std::slice::from_ref(&destination), &cache).await?;

    let mut command = context.venv();
    command
        .arg(alias.child(".venv").path())
        .args(["--clear", "--no-project"]);
    let mut replacement = QueuedCommand::spawn(command)?;
    replacement.wait_for_destination(&destination).await?;
    assert!(marker.is_file());
    assert!(replacement.child.try_wait()?.is_none());

    drop(guard);
    replacement.finish().await?;
    assert!(!marker.exists());
    assert!(destination.join("pyvenv.cfg").is_file());
    Ok(())
}

#[tokio::test]
async fn separate_workspaces_wait_for_shared_environment_destination() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let first = context.temp_dir.child("first");
    first.create_dir_all()?;
    first.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "first"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let second = context.temp_dir.child("second");
    second.create_dir_all()?;
    second.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "second"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let destination = context.temp_dir.join("shared");
    context.venv().arg(&destination).assert().success();
    let destination = fs_err::canonicalize(&destination)?;
    let cache = Cache::from_path(context.cache_dir.path().to_path_buf());
    let guard = EnvironmentLock::acquire(std::slice::from_ref(&destination), &cache).await?;

    let mut command = context.sync();
    command
        .current_dir(first.path())
        .arg("--offline")
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, &destination);
    let mut first_sync = QueuedCommand::spawn(command)?;
    first_sync.wait_for_destination(&destination).await?;

    let mut command = context.sync();
    command
        .current_dir(second.path())
        .arg("--offline")
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, &destination);
    let mut second_sync = QueuedCommand::spawn(command)?;
    second_sync.wait_for_destination(&destination).await?;

    assert!(first_sync.child.try_wait()?.is_none());
    assert!(second_sync.child.try_wait()?.is_none());
    drop(guard);
    first_sync.finish().await?;
    second_sync.finish().await?;
    assert!(first.child("uv.lock").is_file());
    assert!(second.child("uv.lock").is_file());
    Ok(())
}

#[tokio::test]
async fn queued_sync_reclaims_changed_centralized_reference() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []
    "#})?;
    context
        .venv()
        .args([
            "--preview-features",
            "centralized-project-envs",
            "--python",
            "3.11",
        ])
        .assert()
        .success();
    let reference = context.temp_dir.join(".venv");
    let original = fs_err::read_link(&reference)?;
    context
        .venv()
        .args([
            "--preview-features",
            "centralized-project-envs",
            "--python",
            "3.12",
        ])
        .assert()
        .success();
    let updated = fs_err::read_link(&reference)?;
    uv_fs::remove_virtualenv(&reference)?;
    fs_err::write(&reference, original.to_string_lossy().as_bytes())?;

    let cache = Cache::from_path(context.cache_dir.path().to_path_buf());
    let original_guard =
        EnvironmentLock::acquire(&[reference.clone(), original.clone()], &cache).await?;
    let updated_guard = EnvironmentLock::acquire(std::slice::from_ref(&updated), &cache).await?;
    let mut command = context.sync();
    command.args([
        "--preview-features",
        "centralized-project-envs",
        "--offline",
    ]);
    let mut sync = QueuedCommand::spawn(command)?;
    sync.wait_for_destination(&original).await?;

    fs_err::write(&reference, updated.to_string_lossy().as_bytes())?;
    drop(original_guard);
    sync.wait_for_destination(&updated).await?;
    assert!(sync.child.try_wait()?.is_none());
    drop(updated_guard);
    sync.finish().await?;
    assert_eq!(
        fs_err::canonicalize(&reference)?,
        fs_err::canonicalize(&updated)?
    );
    Ok(())
}

#[tokio::test]
#[cfg(unix)]
async fn queued_pip_install_rediscovers_retargeted_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    let original = context.temp_dir.join("original");
    let updated = context.temp_dir.join("updated");
    context
        .venv()
        .arg(&original)
        .args(["--python", "3.11"])
        .assert()
        .success();
    context
        .venv()
        .arg(&updated)
        .args(["--python", "3.12"])
        .assert()
        .success();
    let reference = context.temp_dir.join("selected");
    fs_err::os::unix::fs::symlink(&original, &reference)?;
    let cache = Cache::from_path(context.cache_dir.path().to_path_buf());
    let guard = EnvironmentLock::acquire(std::slice::from_ref(&original), &cache).await?;

    let (filename, bytes) = generate_wheel(
        &"example".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.join(filename);
    fs_err::write(&wheel, bytes)?;
    let mut command = context.pip_install();
    command
        .arg(&wheel)
        .arg("--no-index")
        .env(EnvVars::VIRTUAL_ENV, &reference);
    let mut install = QueuedCommand::spawn(command)?;
    install
        .wait_for_destination(&fs_err::canonicalize(&original)?)
        .await?;

    fs_err::remove_file(&reference)?;
    fs_err::os::unix::fs::symlink(&updated, &reference)?;
    drop(guard);
    install.finish().await?;
    let original = PythonEnvironment::from_root(&original, &cache)?;
    let updated = PythonEnvironment::from_root(&updated, &cache)?;
    assert!(
        !original
            .site_packages()
            .any(|path| path.join("example").is_dir())
    );
    assert!(
        updated
            .site_packages()
            .any(|path| path.join("example").is_dir())
    );
    Ok(())
}

#[tokio::test]
async fn run_releases_destination_before_user_program() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
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
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let script = formatdoc! {r#"
        import socket
        with socket.create_connection(("127.0.0.1", {port})) as connection:
            connection.sendall(b"ready")
            assert connection.recv(1) == b"x"
    "#};
    let mut command = context.run();
    command.args(["--offline", "python", "-c", &script]);
    let run = QueuedCommand::spawn(command)?;
    let (mut connection, _) =
        tokio::time::timeout(Duration::from_secs(30), listener.accept()).await??;
    let mut ready = [0; 5];
    tokio::time::timeout(Duration::from_secs(30), connection.read_exact(&mut ready)).await??;
    assert_eq!(&ready, b"ready");
    let cache = Cache::from_path(context.cache_dir.path().to_path_buf());
    let guard = tokio::time::timeout(
        Duration::from_secs(30),
        EnvironmentLock::acquire(&[context.venv.path().to_path_buf()], &cache),
    )
    .await??;
    connection.write_all(b"x").await?;
    run.finish().await?;
    drop(guard);
    Ok(())
}

#[tokio::test]
async fn venv_replacement_waits_for_running_project_install() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let (filename, wheel) = generate_wheel(
        &"project".parse()?,
        &"0.1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    context.temp_dir.child(&filename).write_binary(&wheel)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context
        .temp_dir
        .child("backend.py")
        .write_str(&formatdoc! {r#"
        import shutil
        import socket
        from pathlib import Path
        from zipfile import ZipFile

        WHEEL = Path(__file__).with_name("{filename}")
        DIST_INFO = "project-0.1.0.dist-info"

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            dist_info = Path(metadata_directory) / DIST_INFO
            dist_info.mkdir()
            with ZipFile(WHEEL) as wheel:
                (dist_info / "METADATA").write_bytes(wheel.read(f"{{DIST_INFO}}/METADATA"))
            return dist_info.name

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            with socket.create_connection(("127.0.0.1", {port}), timeout=30) as connection:
                connection.sendall(b"ready")
                assert connection.recv(1) == b"x"
            shutil.copyfile(WHEEL, Path(wheel_directory) / WHEEL.name)
            return WHEEL.name
    "#})?;

    let mut command = context.sync();
    command.args(["--offline", "--no-editable"]);
    let sync = QueuedCommand::spawn(command)?;
    let (mut connection, _) =
        tokio::time::timeout(Duration::from_secs(30), listener.accept()).await??;
    let mut ready = [0; 5];
    tokio::time::timeout(Duration::from_secs(30), connection.read_exact(&mut ready)).await??;
    assert_eq!(&ready, b"ready");

    let destination = fs_err::canonicalize(context.venv.path())?;
    let mut command = context.venv();
    command.arg(&destination).args(["--clear", "--no-project"]);
    let mut replacement = QueuedCommand::spawn(command)?;
    replacement.wait_for_destination(&destination).await?;
    assert!(replacement.child.try_wait()?.is_none());

    connection.write_all(b"x").await?;
    sync.finish().await?;
    replacement.finish().await?;
    let cache = Cache::from_path(context.cache_dir.path().to_path_buf());
    let environment = PythonEnvironment::from_root(&destination, &cache)?;
    assert!(
        !environment
            .site_packages()
            .any(|path| path.join("project").is_dir())
    );
    Ok(())
}

#[tokio::test]
async fn active_script_waits_for_environment_destination() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context.venv().assert().success();
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # ///
        print("ready")
    "#})?;
    let destination = fs_err::canonicalize(context.venv.path())?;
    let cache = Cache::from_path(context.cache_dir.path().to_path_buf());
    let guard = EnvironmentLock::acquire(std::slice::from_ref(&destination), &cache).await?;
    let mut command = context.run();
    command.args(["--active", "--offline"]).arg(script.path());
    let mut run = QueuedCommand::spawn(command)?;
    run.wait_for_destination(&destination).await?;
    assert!(run.child.try_wait()?.is_none());
    drop(guard);
    run.finish().await?;
    Ok(())
}

#[tokio::test]
async fn venv_creation_waits_through_missing_parent_component() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let parent = fs_err::canonicalize(context.temp_dir.path())?;
    let destination = parent.join("environment");
    let indirect = parent.join("absent").join("..").join("environment");
    assert!(!destination.exists());
    let cache = Cache::from_path(context.cache_dir.path().to_path_buf());
    let guard = EnvironmentLock::acquire(std::slice::from_ref(&destination), &cache).await?;
    let mut command = context.venv();
    command.arg(&indirect).arg("--no-project");
    let mut creation = QueuedCommand::spawn(command)?;
    creation.wait_for_destination(&destination).await?;
    assert!(!destination.exists());
    assert!(creation.child.try_wait()?.is_none());
    drop(guard);
    creation.finish().await?;
    assert!(destination.join("pyvenv.cfg").is_file());
    Ok(())
}
