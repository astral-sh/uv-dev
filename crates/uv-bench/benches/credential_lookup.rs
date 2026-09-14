//! Look up credentials for the index and artifact URLs used by private package repositories.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use http::Extensions;
use reqwest::{Request, Response};
use reqwest_middleware::{ClientBuilder, Middleware, Next};
use uv_auth::{AuthMiddleware, Credentials, CredentialsCache, TextCredentialStore};
use uv_redacted::DisplaySafeUrl;

/// Terminate an authenticated request before network I/O so the authentication work is measured.
struct AuthenticatedResponse;

#[async_trait::async_trait]
impl Middleware for AuthenticatedResponse {
    async fn handle(
        &self,
        request: Request,
        _extensions: &mut Extensions,
        _next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        assert!(request.headers().contains_key(http::header::AUTHORIZATION));
        Ok(http::Response::new(Vec::<u8>::new()).into())
    }
}

fn credential_lookup(c: &mut Criterion<WallTime>) {
    let mut store = TextCredentialStore::default();
    for service in [
        "https://pypi.org/legacy/",
        "https://test.pypi.org/legacy/",
        "https://packages.example.com/team/simple/",
        "https://packages.example.com/team/releases/",
    ] {
        store.insert(
            service.parse().expect("Invalid credential service"),
            Credentials::basic(Some("__token__".to_owned()), Some("benchmark".to_owned())),
        );
    }
    let mut group = c.benchmark_group("credential_lookup");
    for (name, raw) in [
        ("exact", "https://packages.example.com/team/simple/"),
        ("prefix", "https://packages.example.com/team/simple/flask/"),
    ] {
        let url = DisplaySafeUrl::parse(raw).expect("Invalid package index URL");
        assert!(
            store
                .get_credentials(&url, Some("__token__"))
                .unwrap()
                .is_some()
        );
        group.bench_function(name, |b| {
            b.iter(|| {
                store
                    .get_credentials(black_box(&url), Some("__token__"))
                    .unwrap()
            });
        });
    }

    let cache = Arc::new(CredentialsCache::new());
    let index = DisplaySafeUrl::parse("https://packages.example.com/team/simple/")
        .expect("Invalid package index URL");
    cache.store_credentials(
        &index,
        Credentials::basic(Some("__token__".to_owned()), Some("benchmark".to_owned())),
    );
    let client = ClientBuilder::new(reqwest::Client::new())
        .with(
            AuthMiddleware::new()
                .with_cache_arc(cache)
                .with_only_authenticated(true),
        )
        .with(AuthenticatedResponse)
        .build();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    for (name, url) in [
        (
            "middleware_url",
            "https://packages.example.com/team/simple/flask/",
        ),
        (
            "middleware_realm",
            "https://packages.example.com/files/flask-3.1.2-py3-none-any.whl.metadata",
        ),
    ] {
        group.bench_function(name, |b| {
            b.iter_batched(
                || {
                    client
                        .get(url)
                        .build()
                        .expect("Failed to build metadata request")
                },
                |request| {
                    runtime
                        .block_on(client.execute(request))
                        .expect("Authentication failed")
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(credentials, credential_lookup);
criterion_main!(credentials);
