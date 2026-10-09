use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uv_cache::Cache;
use uv_fs::Simplified;

use super::{
    EnvironmentLock, EnvironmentLockError, Key, coordination_directory, initialize_registry,
    validate_destinations,
};

const HOLDER: &str = "environment_lock::process_tests::lock_holder";

/// The direct child owns both its destination and its blocking release gate, including under
/// panic-abort test execution. Killing it cannot leave a grandchild holding the lock.
#[test]
#[expect(clippy::exit, reason = "This fixture owns its gated subprocess")]
fn lock_holder() -> Result<()> {
    let Some(path) = std::env::var_os("UV_TEST_LOCK_DESTINATION") else {
        return Ok(());
    };
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result: Result<()> = runtime.block_on(async {
        let cache = Cache::temp()?;
        let path = std::path::PathBuf::from(path);
        let mut guard = EnvironmentLock::acquire(std::slice::from_ref(&path), &cache).await?;
        if std::env::var_os("UV_TEST_LOCK_REPLACE").is_some() {
            fs_err::remove_dir_all(&path)?;
            signal("removed")?;
            release()?;
            fs_err::create_dir(&path)?;
            guard.finish_creation()?;
            signal("created")?;
        } else {
            guard.finish_creation()?;
            signal("ready")?;
        }
        release()?;
        drop(guard);
        Ok(())
    });
    drop(runtime);
    result?;
    // This fixture owns the subprocess. Report success after cleanup without using libtest's
    // private exit-status protocol for panic-abort children.
    std::process::exit(0)
}

fn signal(state: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "lock-holder:{state}")?;
    stdout.flush()?;
    Ok(())
}

fn release() -> Result<()> {
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte)?;
    anyhow::ensure!(byte == *b"x", "invalid release gate");
    Ok(())
}

enum Event<'a> {
    State(&'a str),
    Waiting(&'a Path),
}

struct Holder {
    child: tokio::process::Child,
    stdout: BufReader<tokio::process::ChildStdout>,
    stderr: BufReader<tokio::process::ChildStderr>,
    output: String,
}

impl Holder {
    fn spawn(path: &Path, settings: &Path, replace: bool) -> Result<Self> {
        fs_err::create_dir_all(settings)?;
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        command
            .args(["--exact", HOLDER, "--nocapture"])
            .env("__RUST_TEST_INVOKE", HOLDER)
            .env("UV_TEST_LOCK_DESTINATION", path)
            .env_remove("UV_TEST_LOCK_REPLACE")
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in [
            "TMPDIR",
            "TMP",
            "TEMP",
            "HOME",
            "USERPROFILE",
            "LOCALAPPDATA",
            "XDG_CACHE_HOME",
            "XDG_DATA_HOME",
            "UV_CACHE_DIR",
        ] {
            command.env(name, settings);
        }
        if replace {
            command.env("UV_TEST_LOCK_REPLACE", "1");
        }
        let mut child = command.spawn()?;
        Ok(Self {
            stdout: BufReader::new(child.stdout.take().context("captured stdout")?),
            stderr: BufReader::new(child.stderr.take().context("captured stderr")?),
            child,
            output: String::new(),
        })
    }

    async fn wait(&mut self, event: Event<'_>) -> Result<()> {
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let mut stdout = String::new();
                let mut stderr = String::new();
                tokio::select! {
                    count = self.stdout.read_line(&mut stdout) => {
                        anyhow::ensure!(count? != 0, "holder exited before expected event");
                        self.output.push_str(&stdout);
                        if stdout.starts_with("lock-holder:") {
                            if let Event::State(expected) = event {
                                anyhow::ensure!(stdout.trim() == format!("lock-holder:{expected}"), "unexpected holder state");
                                return Ok(());
                            }
                            anyhow::bail!("holder entered destination before its owner released admission");
                        }
                    }
                    count = self.stderr.read_line(&mut stderr) => {
                        anyhow::ensure!(count? != 0, "holder stderr closed before expected event");
                        self.output.push_str(&stderr);
                        if let Event::Waiting(resource) = event
                            && stderr.contains("Waiting to acquire exclusive lock")
                            && stderr.contains("uv-environment-")
                            && stderr.contains(&resource.simplified_display().to_string()) {
                            return Ok(());
                        }
                    }
                }
            }
        }).await;
        match result {
            Ok(Ok(())) => Ok(()),
            result => anyhow::bail!("holder event failed: {result:?}\n{}", self.output),
        }
    }

    async fn advance(&mut self) -> Result<()> {
        self.child
            .stdin
            .as_mut()
            .context("release gate")?
            .write_all(b"x")
            .await?;
        Ok(())
    }

    async fn finish(mut self) -> Result<()> {
        self.advance().await?;
        let status = tokio::time::timeout(Duration::from_secs(30), self.child.wait()).await??;
        anyhow::ensure!(status.success(), "holder failed: {status}\n{}", self.output);
        Ok(())
    }
}

#[tokio::test]
async fn existing_case_replacement_keeps_admission_while_absent() -> Result<()> {
    let parent = tempfile::tempdir()?;
    fs_err::write(parent.path().join("CaseProbe"), "")?;
    if !parent.path().join("caseprobe").exists() {
        // Missing case aliases collide only on a case-insensitive filesystem.
        return Ok(());
    }
    let destination = parent.path().join("NewEnv");
    fs_err::create_dir(&destination)?;
    let destination = fs_err::canonicalize(destination)?;
    let alias = destination.with_file_name("newenv");
    let mut owner = Holder::spawn(&destination, &parent.path().join("owner"), true)?;
    owner.wait(Event::State("removed")).await?;
    assert!(!destination.exists());
    let mut waiter = Holder::spawn(&alias, &parent.path().join("waiter"), false)?;
    waiter
        .wait(Event::Waiting(
            destination.parent().context("environment parent")?,
        ))
        .await?;
    owner.advance().await?;
    owner.wait(Event::State("created")).await?;
    waiter.wait(Event::Waiting(&destination)).await?;
    owner.finish().await?;
    waiter.wait(Event::State("ready")).await?;
    waiter.finish().await
}

#[tokio::test]
async fn process_configuration_does_not_split_destination_admission() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let destination = parent.path().join("environment");
    fs_err::create_dir(&destination)?;
    let destination = fs_err::canonicalize(destination)?;
    let mut owner = Holder::spawn(&destination, &parent.path().join("first-settings"), false)?;
    owner.wait(Event::State("ready")).await?;
    let mut waiter = Holder::spawn(&destination, &parent.path().join("second-settings"), false)?;
    waiter.wait(Event::Waiting(&destination)).await?;
    owner.finish().await?;
    waiter.wait(Event::State("ready")).await?;
    waiter.finish().await
}

#[tokio::test]
async fn replacing_ancestor_waits_for_descendant_worker() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let destination = parent.path().join("outer");
    let descendant = destination.join("inner");
    fs_err::create_dir_all(&descendant)?;
    let destination = fs_err::canonicalize(destination)?;
    let mut owner = Holder::spawn(&descendant, &parent.path().join("owner"), false)?;
    owner.wait(Event::State("ready")).await?;
    let mut waiter = Holder::spawn(&destination, &parent.path().join("waiter"), true)?;
    waiter.wait(Event::Waiting(&destination)).await?;
    assert!(descendant.is_dir());
    owner.finish().await?;
    waiter.wait(Event::State("removed")).await?;
    waiter.advance().await?;
    waiter.wait(Event::State("created")).await?;
    waiter.finish().await
}

#[test]
fn registry_overlap_is_rejected_before_initialization() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let registry = parent.path().join("registry");
    assert!(matches!(
        validate_destinations(&[parent.path().to_path_buf()], &registry),
        Err(EnvironmentLockError::ProtectedRegistry { .. })
    ));
    assert!(!registry.exists());
    fs_err::write(&registry, "unrelated")?;
    assert!(matches!(
        initialize_registry(&registry),
        Err(EnvironmentLockError::Registry { .. })
    ));
    assert_eq!(fs_err::read_to_string(registry)?, "unrelated");
    Ok(())
}

#[tokio::test]
async fn registry_file_failure_cannot_disable_admission() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let destination = fs_err::canonicalize(parent.path())?;
    let registry = coordination_directory()?;
    initialize_registry(&registry)?;
    let lock_path = Key::Destination(destination.clone()).lock_path(&registry);
    // This unique destination has no lock file yet. An owned directory obstructs its creation
    // without changing any shared lock inode or depending on account-specific permissions.
    fs_err::create_dir(&lock_path)?;
    let cache = Cache::temp()?;
    let result = EnvironmentLock::acquire_optional(&[destination], &cache).await;
    fs_err::remove_dir(&lock_path)?;
    assert!(matches!(result, Err(EnvironmentLockError::Registry { .. })));
    Ok(())
}

#[test]
#[cfg(unix)]
fn new_registry_directory_is_private() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let parent = tempfile::tempdir()?;
    let registry = parent.path().join("registry");
    initialize_registry(&registry)?;
    assert_eq!(fs_err::metadata(registry)?.permissions().mode() & 0o077, 0);
    Ok(())
}

#[test]
#[cfg(unix)]
fn registry_entry_cannot_escape_protection_through_symlink() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let registry = parent.path().join("registry");
    initialize_registry(&registry)?;
    let outside = parent.path().join("outside");
    fs_err::create_dir(&outside)?;
    let entry = registry.join("environment");
    fs_err::os::unix::fs::symlink(&outside, &entry)?;
    assert!(matches!(
        validate_destinations(std::slice::from_ref(&entry), &registry),
        Err(EnvironmentLockError::ProtectedRegistry { .. })
    ));
    assert!(entry.is_symlink());
    assert!(outside.is_dir());
    Ok(())
}
