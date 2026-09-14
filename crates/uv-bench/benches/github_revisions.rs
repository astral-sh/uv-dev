//! Resolve GitHub references while retaining precise and in-process cache-hit paths.

mod common;

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use http::Extensions;
use reqwest::{Request, Response};
use reqwest_middleware::{ClientBuilder, Middleware, Next};
use uv_bench::{FixtureServer, git_fixtures, is_codspeed_simulation};
use uv_git::GitResolver;
use uv_git_types::{GitLfs, GitOid, GitReference, GitUrl};
use uv_redacted::DisplaySafeUrl;

/// Keep the production request construction while directing its transport to the replay server.
struct GitHubReplay {
    source: String,
    destination: String,
}

#[async_trait::async_trait]
impl Middleware for GitHubReplay {
    async fn handle(
        &self,
        mut request: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        let path = request
            .url()
            .as_str()
            .strip_prefix(&self.source)
            .expect("Unexpected GitHub request");
        *request.url_mut() = format!("{}{path}", self.destination)
            .parse()
            .expect("Invalid replay URL");
        next.run(request, extensions).await
    }
}

fn github_revisions(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    assert!(
        std::env::var_os("UV_NO_GITHUB_FAST_PATH").is_none(),
        "The GitHub fast path must be enabled for this benchmark"
    );
    let fixture = git_fixtures()
        .into_iter()
        .find(|fixture| fixture.name == "sampleproject")
        .expect("Missing sampleproject Git fixture");
    let commit: GitOid = fixture.commit.parse().expect("Invalid Git commit");
    let reference = GitUrl::from_fields(
        DisplaySafeUrl::parse(&fixture.repository).expect("Invalid Git repository URL"),
        GitReference::Branch(
            fixture
                .reference
                .trim_start_matches("refs/heads/")
                .to_owned(),
        ),
        None,
        GitLfs::Disabled,
    )
    .expect("Invalid Git URL");
    let precise = reference
        .clone()
        .with_precise(commit)
        .expect("Invalid precise Git URL");
    let server = FixtureServer::start_git();
    let client = ClientBuilder::new(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("Failed to create HTTP client"),
    )
    .with(GitHubReplay {
        source: format!(
            "{}/",
            std::env::var("UV_GITHUB_FAST_PATH_URL")
                .unwrap_or_else(|_| "https://api.github.com/repos".to_owned())
        ),
        destination: server.url("/github/repos/"),
    })
    .build();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let mut group = c.benchmark_group("github_revisions");
    group.bench_function("cold_reference", |b| {
        b.iter_batched(
            GitResolver::default,
            |resolver| {
                black_box(
                    runtime
                        .block_on(resolver.github_fast_path(&reference, &client))
                        .expect("Failed to resolve GitHub reference"),
                )
            },
            BatchSize::SmallInput,
        );
    });
    let resolver = GitResolver::default();
    assert_eq!(
        runtime
            .block_on(resolver.github_fast_path(&reference, &client))
            .expect("Failed to resolve GitHub reference"),
        Some(commit)
    );
    for (name, git) in [("known_commit", &precise), ("cached_reference", &reference)] {
        group.bench_function(name, |b| {
            b.iter(|| {
                black_box(
                    runtime
                        .block_on(resolver.github_fast_path(git, &client))
                        .expect("Failed to resolve GitHub reference"),
                )
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = revisions;
    config = common::walltime_criterion();
    targets = github_revisions
}
criterion_main!(revisions);
