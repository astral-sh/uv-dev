use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use anyhow::Context;
use http::StatusCode;
use tempfile::TempDir;
use tokio::sync::Notify;

use uv_cache_key::cache_digest;
use uv_git_types::GitOid;
use uv_redacted::DisplaySafeUrl;

use crate::{GitHttpSettings, GitResolver, Reporter, RepositoryReference};

use super::{client, poll_once, url};

struct Repository {
    _root: TempDir,
    cache: PathBuf,
    first: GitOid,
    second: GitOid,
}

fn git(path: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git").current_dir(path).args(args).output()?;
    anyhow::ensure!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
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
        fs_err::write(source.join("value"), "first")?;
        git(&source, &["add", "value"])?;
        git(&source, &["commit", "-m", "first"])?;
        let first = GitOid::from_str(&git(&source, &["rev-parse", "HEAD"])?)?;
        fs_err::write(source.join("value"), "second")?;
        git(&source, &["commit", "-am", "second"])?;
        let second = GitOid::from_str(&git(&source, &["rev-parse", "HEAD"])?)?;

        let cache = root.path().join("cache");
        let url = url("main")?;
        let database = cache.join("db").join(cache_digest(url.repository()));
        fs_err::create_dir_all(&database)?;
        git(&database, &["init", "--bare"])?;
        let source_url = DisplaySafeUrl::from_file_path(source)
            .map_err(|()| anyhow::anyhow!("invalid fixture URL"))?;
        git(
            &database,
            &[
                "config",
                &format!("url.{source_url}.insteadOf"),
                url.url().as_str(),
            ],
        )?;
        Ok(Self {
            _root: root,
            cache,
            first,
            second,
        })
    }
}

struct GatedReporter {
    claimed: AtomicBool,
    started: Arc<Notify>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Reporter for GatedReporter {
    fn on_checkout_start(&self, _url: &DisplaySafeUrl, _revision: &str) -> usize {
        if !self.claimed.swap(true, Ordering::SeqCst) {
            self.started.notify_one();
            // Dropping the test sender also releases a worker if an assertion fails.
            if let Ok(release) = self.release.lock() {
                let _ = release.recv();
            }
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
            claimed: AtomicBool::new(false),
            started: started.clone(),
            release: Mutex::new(receiver),
        }),
        started,
        release,
    )
}

#[tokio::test]
async fn http_resolution_waits_for_a_running_checkout() -> anyhow::Result<()> {
    let repository = Repository::new()?;
    let resolver = GitResolver::default();
    let url = url("main")?;
    let (client, mock, _) = client(vec![(StatusCode::OK, repository.first.to_string())]);
    mock.release.add_permits(1);
    let (reporter, started, release) = reporter();
    let checkout = {
        let resolver = resolver.clone();
        let url = url.clone();
        let cache = repository.cache.clone();
        tokio::spawn(async move {
            resolver
                .fetch(
                    &url,
                    GitHttpSettings::default().with_offline(true),
                    cache,
                    Some(reporter),
                )
                .await
        })
    };
    started.notified().await;
    let mut http = Box::pin(resolver.github_fast_path(&url, &client));
    assert!(poll_once(http.as_mut()).is_pending());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    release.send(())?;
    let checkout = checkout.await??;
    assert_eq!(checkout.git().precise(), Some(repository.second));
    assert_eq!(
        fs_err::read_to_string(checkout.path().join("value"))?,
        "second"
    );
    assert_eq!(http.await?, Some(repository.second));
    Ok(())
}

#[tokio::test]
async fn checkout_waits_for_a_running_http_resolution() -> anyhow::Result<()> {
    let repository = Repository::new()?;
    let resolver = GitResolver::default();
    let url = url("main")?;
    let (client, mock, mut entered) = client(vec![(StatusCode::OK, repository.first.to_string())]);
    let mut http = Box::pin(resolver.github_fast_path(&url, &client));
    assert!(poll_once(http.as_mut()).is_pending());
    assert_eq!(entered.recv().await, Some(0));
    let mut checkout = Box::pin(resolver.fetch(
        &url,
        GitHttpSettings::default().with_offline(true),
        repository.cache.clone(),
        None,
    ));
    assert!(poll_once(checkout.as_mut()).is_pending());
    mock.release.add_permits(1);
    assert_eq!(http.await?, Some(repository.first));
    let checkout = checkout.await?;
    assert_eq!(checkout.git().precise(), Some(repository.first));
    assert_eq!(
        fs_err::read_to_string(checkout.path().join("value"))?,
        "first"
    );
    Ok(())
}

#[tokio::test]
async fn a_pin_seeded_during_checkout_selects_the_returned_tree() -> anyhow::Result<()> {
    let repository = Repository::new()?;
    let resolver = GitResolver::default();
    let url = url("main")?;
    let (reporter, started, release) = reporter();
    let checkout = {
        let resolver = resolver.clone();
        let url = url.clone();
        let cache = repository.cache.clone();
        tokio::spawn(async move {
            resolver
                .fetch(
                    &url,
                    GitHttpSettings::default().with_offline(true),
                    cache,
                    Some(reporter),
                )
                .await
        })
    };
    started.notified().await;
    resolver.insert(RepositoryReference::from(&url), repository.first);
    release.send(())?;
    let checkout = checkout.await??;
    assert_eq!(checkout.git().precise(), Some(repository.first));
    assert_eq!(
        fs_err::read_to_string(checkout.path().join("value"))?,
        "first"
    );
    assert_eq!(resolver.get_precise(&url), Some(repository.first));
    Ok(())
}

#[tokio::test]
async fn an_explicit_checkout_does_not_replace_a_seeded_pin() -> anyhow::Result<()> {
    let repository = Repository::new()?;
    let resolver = GitResolver::default();
    let url = url("main")?;
    resolver.insert(RepositoryReference::from(&url), repository.second);
    let explicit = url.clone().with_precise(repository.first)?;
    let checkout = resolver
        .fetch(
            &explicit,
            GitHttpSettings::default().with_offline(true),
            repository.cache.clone(),
            None,
        )
        .await?;
    assert_eq!(checkout.git().precise(), Some(repository.first));
    assert_eq!(resolver.get_precise(&url), Some(repository.second));
    assert_eq!(
        resolver.precise(explicit).and_then(|url| url.precise()),
        Some(repository.first)
    );
    Ok(())
}
