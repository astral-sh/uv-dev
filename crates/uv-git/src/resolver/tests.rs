use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

use http::{Extensions, StatusCode};
use reqwest::{Request, Response};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware, Middleware, Next};
use tokio::sync::{Semaphore, mpsc};

use uv_git_types::{GitLfs, GitOid, GitReference, GitUrl};
use uv_redacted::DisplaySafeUrl;

use super::{GitResolver, RepositoryReference};

#[cfg(feature = "test-git")]
mod git;

const FIRST: &str = "1111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222";

#[derive(Clone)]
struct MockGithub {
    calls: Arc<AtomicUsize>,
    entered: mpsc::UnboundedSender<usize>,
    release: Arc<Semaphore>,
    responses: Arc<[(StatusCode, String)]>,
}

#[async_trait::async_trait]
impl Middleware for MockGithub {
    async fn handle(
        &self,
        _request: Request,
        _extensions: &mut Extensions,
        _next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.entered.send(index);
        self.release
            .acquire()
            .await
            .map_err(|error| reqwest_middleware::Error::Middleware(error.into()))?
            .forget();
        let (status, body) = self.responses.get(index).ok_or_else(|| {
            reqwest_middleware::Error::Middleware(anyhow::anyhow!("unexpected request {index}"))
        })?;
        let response = http::Response::builder()
            .status(*status)
            .body(body.clone())
            .map_err(|error| reqwest_middleware::Error::Middleware(error.into()))?;
        Ok(Response::from(response))
    }
}

fn client(
    responses: Vec<(StatusCode, String)>,
) -> (
    ClientWithMiddleware,
    MockGithub,
    mpsc::UnboundedReceiver<usize>,
) {
    let (entered, received) = mpsc::unbounded_channel();
    let mock = MockGithub {
        calls: Arc::new(AtomicUsize::new(0)),
        entered,
        release: Arc::new(Semaphore::new(0)),
        responses: responses.into(),
    };
    (
        ClientBuilder::new(reqwest::Client::new())
            .with(mock.clone())
            .build(),
        mock,
        received,
    )
}

fn url(reference: &str) -> anyhow::Result<GitUrl> {
    Ok(GitUrl::from_fields(
        DisplaySafeUrl::parse("https://github.com/uv-test/reference-claims")?,
        GitReference::Branch(reference.to_owned()),
        None,
        GitLfs::Disabled,
    )?)
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[tokio::test]
async fn concurrent_http_resolutions_publish_one_commit() -> anyhow::Result<()> {
    let resolver = GitResolver::default();
    let url = url("main")?;
    let (client, mock, mut entered) = client(vec![
        (StatusCode::OK, FIRST.to_owned()),
        (StatusCode::OK, SECOND.to_owned()),
    ]);
    let first = {
        let resolver = resolver.clone();
        let url = url.clone();
        let client = client.clone();
        tokio::spawn(async move { resolver.github_fast_path(&url, &client).await })
    };
    assert_eq!(entered.recv().await, Some(0));
    let mut second = Box::pin(resolver.github_fast_path(&url, &client));
    assert!(poll_once(second.as_mut()).is_pending());
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    mock.release.add_permits(1);
    let first = first.await??;
    assert_eq!(first, Some(GitOid::from_str(FIRST)?));
    assert_eq!(second.await?, first);
    assert_eq!(resolver.get_precise(&url), first);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn independent_references_can_resolve_together() -> anyhow::Result<()> {
    let resolver = GitResolver::default();
    let main = url("main")?;
    let other = url("other")?;
    let (client, mock, mut entered) = client(vec![
        (StatusCode::OK, FIRST.to_owned()),
        (StatusCode::OK, SECOND.to_owned()),
    ]);
    let mut first = Box::pin(resolver.github_fast_path(&main, &client));
    let mut second = Box::pin(resolver.github_fast_path(&other, &client));
    assert!(poll_once(first.as_mut()).is_pending());
    assert!(poll_once(second.as_mut()).is_pending());
    assert_eq!(entered.recv().await, Some(0));
    assert_eq!(entered.recv().await, Some(1));
    mock.release.add_permits(2);
    assert_eq!(first.await?, Some(GitOid::from_str(FIRST)?));
    assert_eq!(second.await?, Some(GitOid::from_str(SECOND)?));
    Ok(())
}

#[tokio::test]
async fn seeded_and_explicit_pins_remain_authoritative() -> anyhow::Result<()> {
    let resolver = GitResolver::default();
    let url = url("main")?;
    let (client, mock, mut entered) = client(vec![(StatusCode::OK, FIRST.to_owned())]);
    let mut pending = Box::pin(resolver.github_fast_path(&url, &client));
    assert!(poll_once(pending.as_mut()).is_pending());
    assert_eq!(entered.recv().await, Some(0));
    resolver.insert(RepositoryReference::from(&url), GitOid::from_str(SECOND)?);
    mock.release.add_permits(1);
    assert_eq!(pending.await?, Some(GitOid::from_str(SECOND)?));

    let precise = url.clone().with_precise(GitOid::from_str(FIRST)?)?;
    assert_eq!(
        resolver.github_fast_path(&precise, &client).await?,
        precise.precise()
    );
    assert_eq!(
        resolver
            .precise(precise.clone())
            .and_then(|url| url.precise()),
        precise.precise()
    );
    assert_eq!(resolver.get_precise(&url), Some(GitOid::from_str(SECOND)?));
    // Seeding is deliberately replacing, even after a reference was resolved.
    resolver.insert(RepositoryReference::from(&url), GitOid::from_str(FIRST)?);
    assert_eq!(resolver.get_precise(&url), Some(GitOid::from_str(FIRST)?));
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn unsuccessful_http_resolution_releases_its_claim() -> anyhow::Result<()> {
    let resolver = GitResolver::default();
    let url = url("main")?;
    let (client, mock, _) = client(vec![
        (StatusCode::NOT_FOUND, String::new()),
        (StatusCode::OK, SECOND.to_owned()),
    ]);
    mock.release.add_permits(2);
    assert_eq!(resolver.github_fast_path(&url, &client).await?, None);
    assert_eq!(
        resolver.github_fast_path(&url, &client).await?,
        Some(GitOid::from_str(SECOND)?)
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn cancelled_http_resolution_releases_its_claim() -> anyhow::Result<()> {
    let resolver = GitResolver::default();
    let url = url("main")?;
    let (client, mock, mut entered) = client(vec![
        (StatusCode::OK, FIRST.to_owned()),
        (StatusCode::OK, SECOND.to_owned()),
    ]);
    let first = {
        let resolver = resolver.clone();
        let url = url.clone();
        let client = client.clone();
        tokio::spawn(async move { resolver.github_fast_path(&url, &client).await })
    };
    assert_eq!(entered.recv().await, Some(0));
    first.abort();
    assert!(
        first
            .await
            .expect_err("request should be cancelled")
            .is_cancelled()
    );
    mock.release.add_permits(1);
    assert_eq!(
        resolver.github_fast_path(&url, &client).await?,
        Some(GitOid::from_str(SECOND)?)
    );
    Ok(())
}
