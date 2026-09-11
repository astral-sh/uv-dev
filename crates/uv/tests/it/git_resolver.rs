use std::future::{Future, poll_fn};
use std::io::Write;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::Poll;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::time::timeout;

use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileMode};
use uv_git::{
    Fetch, GitHttpSettings, GitResolver, GitResolverError, Reporter, RepositoryReference,
};
use uv_git_types::{GitLfs, GitOid, GitReference, GitUrl, GitUrlParseError};
use uv_redacted::DisplaySafeUrl;

const GATE_REACHED_TIMEOUT: Duration = Duration::from_secs(30);
const REPORTER_HOLD_TIMEOUT: Duration = Duration::from_secs(60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// A bare, authored repository containing only two versions of a marker file.
struct LocalRepository {
    _root: TempDir,
    origin: PathBuf,
    cache: PathBuf,
    empty_config: PathBuf,
    empty_hooks: PathBuf,
    first: GitOid,
    second: GitOid,
}

impl LocalRepository {
    fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let root_path = fs_err::canonicalize(root.path())?;
        let origin = root_path.join("origin");
        let cache = root_path.join("cache");
        let empty_config = root_path.join("gitconfig");
        let empty_hooks = root_path.join("hooks");
        fs_err::create_dir(&origin)?;
        fs_err::create_dir(&empty_hooks)?;
        fs_err::write(&empty_config, "")?;

        let git = |args: &[&str], input: Option<&[u8]>| {
            local_git(&origin, &empty_config, &empty_hooks, args, input)
        };
        git(
            &["init", "--bare", "--object-format=sha1", "--template=", "."],
            None,
        )?;
        let first = commit(&git, "first\n", None)?;
        let second = commit(&git, "second\n", Some(first))?;
        git(&["symbolic-ref", "HEAD", "refs/heads/main"], None)?;
        git(&["update-ref", "refs/heads/main", first.as_str()], None)?;

        Ok(Self {
            _root: root,
            origin,
            cache,
            empty_config,
            empty_hooks,
            first,
            second,
        })
    }

    fn url(&self, reference: GitReference) -> Result<GitUrl> {
        let url = DisplaySafeUrl::from_file_path(&self.origin)
            .map_err(|()| anyhow::anyhow!("could not create local repository URL"))?;
        Ok(GitUrl::from_fields(url, reference, None, GitLfs::Disabled)?)
    }

    fn branch_url(&self) -> Result<GitUrl> {
        self.url(GitReference::Branch("main".to_owned()))
    }

    fn advance(&self) -> Result<()> {
        local_git(
            &self.origin,
            &self.empty_config,
            &self.empty_hooks,
            &["update-ref", "refs/heads/main", self.second.as_str()],
            None,
        )?;
        Ok(())
    }

    async fn lock(&self, url: &GitUrl) -> Result<LockedFile> {
        let directory = self.cache.join("locks");
        fs_err::create_dir_all(&directory)?;
        Ok(timeout(
            FETCH_TIMEOUT,
            LockedFile::acquire(
                directory.join(cache_digest(url.repository())),
                LockedFileMode::Exclusive,
                url.repository(),
            ),
        )
        .await
        .context("timed out acquiring the authored repository lock")??)
    }
}

fn local_git(
    origin: &std::path::Path,
    empty_config: &std::path::Path,
    empty_hooks: &std::path::Path,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<String> {
    let mut hooks = std::ffi::OsString::from("core.hooksPath=");
    hooks.push(empty_hooks);
    let mut command = Command::new("git");
    command
        .current_dir(origin)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_DEFAULT_HASH")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", empty_config)
        .env("GIT_CONFIG_GLOBAL", empty_config)
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "file")
        .env("GIT_AUTHOR_NAME", "uv test")
        .env("GIT_AUTHOR_EMAIL", "uv-test@example.invalid")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "uv test")
        .env("GIT_COMMITTER_EMAIL", "uv-test@example.invalid")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .arg("-c")
        .arg(hooks)
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("failed to start local Git")?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .context("local Git stdin was not piped")?
            .write_all(input)?;
    }
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "local Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
}

fn commit(
    git: &impl Fn(&[&str], Option<&[u8]>) -> Result<String>,
    marker: &str,
    parent: Option<GitOid>,
) -> Result<GitOid> {
    let blob = git(&["hash-object", "-w", "--stdin"], Some(marker.as_bytes()))?;
    let tree = git(
        &["mktree"],
        Some(format!("100644 blob {blob}\tmarker.txt\n").as_bytes()),
    )?;
    let mut args = vec!["commit-tree", tree.as_str(), "-m", "authored marker"];
    if let Some(parent) = &parent {
        args.extend(["-p", parent.as_str()]);
    }
    Ok(git(&args, None)?.parse()?)
}

struct CheckoutGate {
    reached: oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
}

struct ReleaseOnDrop(Option<mpsc::Sender<()>>);

impl ReleaseOnDrop {
    fn release(&mut self) {
        drop(self.0.take());
    }
}

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Default)]
struct FetchReporter {
    checkouts: AtomicUsize,
    gate: Mutex<Option<CheckoutGate>>,
    timed_out: AtomicBool,
}

impl FetchReporter {
    fn gated() -> (Arc<Self>, oneshot::Receiver<()>, ReleaseOnDrop) {
        let (reached, wait_for_checkout) = oneshot::channel();
        let (release, wait_for_release) = mpsc::channel();
        (
            Arc::new(Self {
                gate: Mutex::new(Some(CheckoutGate {
                    reached,
                    release: wait_for_release,
                })),
                ..Self::default()
            }),
            wait_for_checkout,
            ReleaseOnDrop(Some(release)),
        )
    }

    fn checkouts(&self) -> usize {
        self.checkouts.load(Ordering::SeqCst)
    }
}

impl Reporter for FetchReporter {
    fn on_checkout_start(&self, _url: &DisplaySafeUrl, _revision: &str) -> usize {
        self.checkouts.fetch_add(1, Ordering::SeqCst)
    }

    fn on_checkout_complete(&self, _url: &DisplaySafeUrl, _revision: &str, index: usize) {
        if index != 0 {
            return;
        }
        let gate = match self.gate.lock() {
            Ok(mut gate) => gate.take(),
            Err(error) => error.into_inner().take(),
        };
        if let Some(CheckoutGate { reached, release }) = gate {
            let _ = reached.send(());
            match release.recv_timeout(REPORTER_HOLD_TIMEOUT) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.timed_out.store(true, Ordering::SeqCst);
                }
            }
        }
    }
}

fn settings() -> GitHttpSettings {
    GitHttpSettings::default().with_offline(true)
}

async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> bool {
    poll_fn(|context| Poll::Ready(future.as_mut().poll(context).is_pending())).await
}

fn assert_fetch(fetch: &Fetch, expected: GitOid, marker: &str) -> Result<()> {
    assert_eq!(fetch.git().precise(), Some(expected));
    assert_eq!(
        fs_err::read_to_string(fetch.path().join("marker.txt"))?,
        marker
    );
    Ok(())
}

fn assert_mismatch(
    result: Result<Fetch, GitResolverError>,
    requested: GitOid,
    expected_precise: GitOid,
) -> Result<()> {
    let Err(error) = result else {
        bail!("mismatched exact revision unexpectedly fetched");
    };
    let GitResolverError::Git(source) = error else {
        bail!("expected a Git revision error, got {error}");
    };
    let Some(GitUrlParseError::MismatchedRevision {
        revision, precise, ..
    }) = source.downcast_ref::<GitUrlParseError>()
    else {
        bail!("expected a mismatched revision, got {source}");
    };
    assert_eq!(revision, requested.as_str());
    assert_eq!(*precise, expected_precise);
    Ok(())
}

#[tokio::test]
async fn waiting_fetch_reuses_preceding_resolution() -> Result<()> {
    let repository = LocalRepository::new()?;
    let url = repository.branch_url()?;
    let resolver = GitResolver::default();
    let (reporter, reached, mut release) = FetchReporter::gated();
    let first = {
        let resolver = resolver.clone();
        let url = url.clone();
        let cache = repository.cache.clone();
        let reporter = reporter.clone();
        tokio::spawn(async move {
            resolver
                .fetch(&url, settings(), cache, Some(reporter))
                .await
        })
    };
    let mut waiter = Box::pin(resolver.fetch(
        &url,
        settings(),
        repository.cache.clone(),
        Some(reporter.clone()),
    ));

    let schedule = async {
        timeout(GATE_REACHED_TIMEOUT, reached)
            .await
            .context("first checkout did not reach the reporter")?
            .context("first checkout reporter closed early")?;
        ensure!(
            resolver.get_precise(&url).is_none(),
            "the held first checkout already populated the resolver"
        );
        ensure!(
            poll_once(waiter.as_mut()).await,
            "waiter did not suspend behind the first fetch"
        );
        repository.advance()?;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    // Release before propagating a schedule failure or awaiting the first task.
    release.release();
    let first_result = timeout(FETCH_TIMEOUT, first).await;
    schedule?;
    let first = first_result
        .context("first fetch did not finish after release")?
        .context("first fetch task failed")??;
    let waiter = timeout(FETCH_TIMEOUT, waiter)
        .await
        .context("waiting fetch did not finish")??;
    ensure!(
        !reporter.timed_out.load(Ordering::SeqCst),
        "checkout reporter timed out before release"
    );
    assert_fetch(&first, repository.first, "first\n")?;
    assert_fetch(&waiter, repository.first, "first\n")?;
    assert_eq!(reporter.checkouts(), 1);
    Ok(())
}

#[tokio::test]
async fn waiting_fetch_preserves_initial_cache_hit() -> Result<()> {
    let repository = LocalRepository::new()?;
    let url = repository.branch_url()?;
    let resolver = GitResolver::default();
    let first = timeout(
        FETCH_TIMEOUT,
        resolver.fetch(&url, settings(), repository.cache.clone(), None),
    )
    .await??;
    assert_fetch(&first, repository.first, "first\n")?;
    repository.advance()?;

    let lock = repository.lock(&url).await?;
    let mut waiter = Box::pin(resolver.fetch(&url, settings(), repository.cache.clone(), None));
    let pending = poll_once(waiter.as_mut()).await;
    resolver.insert(RepositoryReference::from(&url), repository.second);
    drop(lock);
    ensure!(pending, "cache-hit fetch did not suspend behind the lock");
    let fetch = timeout(FETCH_TIMEOUT, waiter).await??;
    assert_fetch(&fetch, repository.first, "first\n")?;
    Ok(())
}

#[tokio::test]
async fn cached_full_revision_mismatch_precedes_lock_io() -> Result<()> {
    let repository = LocalRepository::new()?;
    let url = repository.url(GitReference::BranchOrTagOrCommit(
        repository.first.as_str().to_owned(),
    ))?;
    let resolver = GitResolver::default();
    let reporter = Arc::new(FetchReporter::default());
    resolver.insert(RepositoryReference::from(&url), repository.second);
    fs_err::write(&repository.cache, "not a directory")?;

    let result = timeout(
        FETCH_TIMEOUT,
        resolver.fetch(
            &url,
            settings(),
            repository.cache.clone(),
            Some(reporter.clone()),
        ),
    )
    .await?;
    assert_mismatch(result, repository.first, repository.second)?;
    assert!(fs_err::metadata(&repository.cache)?.is_file());
    assert_eq!(reporter.checkouts(), 0);
    Ok(())
}

#[tokio::test]
async fn waiting_fetch_rejects_new_full_revision_mismatch() -> Result<()> {
    let repository = LocalRepository::new()?;
    let url = repository.url(GitReference::BranchOrTagOrCommit(
        repository.first.as_str().to_owned(),
    ))?;
    let resolver = GitResolver::default();
    let reporter = Arc::new(FetchReporter::default());
    let lock = repository.lock(&url).await?;
    let mut waiter = Box::pin(resolver.fetch(
        &url,
        settings(),
        repository.cache.clone(),
        Some(reporter.clone()),
    ));
    let pending = poll_once(waiter.as_mut()).await;
    resolver.insert(RepositoryReference::from(&url), repository.second);
    drop(lock);
    ensure!(
        pending,
        "initial-miss fetch did not suspend behind the lock"
    );
    let result = timeout(FETCH_TIMEOUT, waiter).await?;
    assert_mismatch(result, repository.first, repository.second)?;
    assert_eq!(reporter.checkouts(), 0);
    Ok(())
}
