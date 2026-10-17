use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use anyhow::Context;
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio::time::timeout;

use uv_cache::{Cache, CacheBucket};
use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileMode};
use uv_git_types::{GitLfs, GitReference, GitUrl};
use uv_redacted::DisplaySafeUrl;

use crate::{GitHttpSettings, GitResolver, Reporter};

const DEADLINE: Duration = Duration::from_secs(10);

struct Repository {
    _root: TempDir,
    url: GitUrl,
}

fn git(path: &Path, args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new("git").current_dir(path).args(args).output()?;
    anyhow::ensure!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

impl Repository {
    fn new() -> anyhow::Result<Self> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        fs_err::create_dir(&source)?;
        git(&source, &["init", "--initial-branch=main"])?;
        git(&source, &["config", "user.name", "uv test"])?;
        git(
            &source,
            &["config", "user.email", "uv-test@example.invalid"],
        )?;
        git(&source, &["config", "commit.gpgsign", "false"])?;
        let hooks = root.path().join("empty-hooks");
        fs_err::create_dir(&hooks)?;
        git(
            &source,
            &[
                "config",
                "core.hooksPath",
                hooks.to_str().context("fixture path must be UTF-8")?,
            ],
        )?;
        fs_err::write(source.join("value"), "checked out")?;
        git(&source, &["add", "value"])?;
        git(&source, &["commit", "-m", "fixture"])?;
        let url = GitUrl::from_fields(
            DisplaySafeUrl::from_file_path(source)
                .map_err(|()| anyhow::anyhow!("invalid fixture URL"))?,
            GitReference::Branch("main".to_owned()),
            None,
            GitLfs::Disabled,
        )?;
        Ok(Self { _root: root, url })
    }
}

struct GatedReporter {
    started: Arc<Notify>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Reporter for GatedReporter {
    fn on_checkout_start(&self, _url: &DisplaySafeUrl, _revision: &str) -> usize {
        self.started.notify_one();
        // Dropping the sender releases the worker if an assertion fails.
        if let Ok(release) = self.release.lock() {
            let _ = release.recv();
        }
        0
    }

    fn on_checkout_complete(&self, _url: &DisplaySafeUrl, _revision: &str, _index: usize) {}
}

fn reporter() -> (Arc<GatedReporter>, Arc<Notify>, mpsc::Sender<()>) {
    let started = Arc::new(Notify::new());
    let (release, receiver) = mpsc::channel();
    (
        Arc::new(GatedReporter {
            started: started.clone(),
            release: Mutex::new(receiver),
        }),
        started,
        release,
    )
}

// Also invoked in a separate process to exercise OS file-lock ownership.
#[test]
fn probe_locks() -> anyhow::Result<()> {
    let Some(root_lock) = std::env::var_os("UV_TEST_GIT_ROOT_LOCK") else {
        return Ok(());
    };
    let repository_lock = std::env::var_os("UV_TEST_GIT_REPOSITORY_LOCK")
        .context("repository lock path must be provided")?;
    let held = std::env::var("UV_TEST_GIT_LOCKS_HELD")? == "true";
    for path in [PathBuf::from(root_lock), PathBuf::from(repository_lock)] {
        assert!(path.is_file());
        let lock = LockedFile::acquire_no_wait(&path, LockedFileMode::Exclusive, "Git test");
        assert_eq!(lock.is_none(), held, "{}", path.display());
    }
    Ok(())
}

fn probe(cache: &Path, url: &GitUrl, held: bool) -> anyhow::Result<()> {
    let repository_lock = Cache::from_path(cache)
        .bucket(CacheBucket::Git)
        .join("locks")
        .join(cache_digest(url.repository()));
    let output = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "resolver::worker_tests::probe_locks",
            "--nocapture",
        ])
        .env("UV_TEST_GIT_ROOT_LOCK", cache.join(".lock"))
        .env("UV_TEST_GIT_REPOSITORY_LOCK", repository_lock)
        .env("UV_TEST_GIT_LOCKS_HELD", held.to_string())
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "lock probe failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[tokio::test]
async fn cancelled_checkout_keeps_the_next_checkout_waiting() -> anyhow::Result<()> {
    let repository = Repository::new()?;
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path()).init().await?;
    let resolver = GitResolver::default();
    let (first_reporter, first_started, first_release) = reporter();
    let first = {
        let cache = cache.clone();
        let resolver = resolver.clone();
        let url = repository.url.clone();
        tokio::spawn(async move {
            resolver
                .fetch(
                    &url,
                    GitHttpSettings::default().with_offline(true),
                    &cache,
                    Some(first_reporter),
                )
                .await
        })
    };
    timeout(DEADLINE, first_started.notified()).await?;
    first.abort();
    assert!(
        first
            .await
            .err()
            .context("cancelled fetch must not complete")?
            .is_cancelled()
    );
    probe(cache.root(), &repository.url, true)?;

    let (second_reporter, second_started, second_release) = reporter();
    let second = {
        let cache = cache.clone();
        let resolver = resolver.clone();
        let url = repository.url.clone();
        tokio::spawn(async move {
            resolver
                .fetch(
                    &url,
                    GitHttpSettings::default().with_offline(true),
                    &cache,
                    Some(second_reporter),
                )
                .await
        })
    };
    assert!(
        timeout(Duration::from_millis(50), second_started.notified())
            .await
            .is_err()
    );
    first_release.send(())?;
    timeout(DEADLINE, second_started.notified()).await?;
    assert_eq!(resolver.get_precise(&repository.url), None);
    second_release.send(())?;
    let checkout = timeout(DEADLINE, second).await???;
    assert_eq!(
        fs_err::read_to_string(checkout.path().join("value"))?,
        "checked out"
    );
    assert_eq!(
        resolver.get_precise(&repository.url),
        checkout.git().precise()
    );
    let cache_root = cache.root().to_owned();
    drop(cache);
    probe(&cache_root, &repository.url, false)?;
    Ok(())
}

async fn cancelled_checkout_retains_temporary_cache(reference: &str) -> anyhow::Result<()> {
    let repository = Repository::new()?;
    let cache = Cache::temp()?.init().await?;
    let root = cache.root().to_owned();
    let resolver = GitResolver::default();
    let url = repository
        .url
        .clone()
        .with_reference(GitReference::Branch(reference.to_owned()));
    let (reporter, started, release) = reporter();
    let checkout = {
        let cache = cache.clone();
        let resolver = resolver.clone();
        let url = url.clone();
        tokio::spawn(async move {
            resolver
                .fetch(
                    &url,
                    GitHttpSettings::default().with_offline(true),
                    &cache,
                    Some(reporter),
                )
                .await
        })
    };
    timeout(DEADLINE, started.notified()).await?;
    checkout.abort();
    assert!(
        checkout
            .await
            .err()
            .context("cancelled fetch must not complete")?
            .is_cancelled()
    );
    drop(cache);
    assert!(root.is_dir());
    probe(&root, &url, true)?;
    release.send(())?;
    timeout(DEADLINE, async {
        while root.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(resolver.get_precise(&url), None);
    Ok(())
}

#[tokio::test]
async fn cancelled_successful_checkout_retains_temporary_cache() -> anyhow::Result<()> {
    cancelled_checkout_retains_temporary_cache("main").await
}

#[tokio::test]
async fn cancelled_failed_checkout_retains_temporary_cache() -> anyhow::Result<()> {
    cancelled_checkout_retains_temporary_cache("missing").await
}

#[tokio::test]
async fn failed_checkout_releases_the_repository_lock() -> anyhow::Result<()> {
    let repository = Repository::new()?;
    let cache = Cache::temp()?.init().await?;
    let resolver = GitResolver::default();
    let url = repository
        .url
        .with_reference(GitReference::Branch("missing".to_owned()));
    let result = resolver
        .fetch(
            &url,
            GitHttpSettings::default().with_offline(true),
            &cache,
            None,
        )
        .await;
    assert!(result.is_err());
    assert_eq!(resolver.get_precise(&url), None);
    let lock = cache
        .bucket(CacheBucket::Git)
        .join("locks")
        .join(cache_digest(url.repository()));
    assert!(
        LockedFile::acquire_no_wait(lock, LockedFileMode::Exclusive, "failed checkout").is_some()
    );
    Ok(())
}
