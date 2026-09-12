use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use std::{assert_matches, io};

use anyhow::{Result, anyhow, ensure};
use reqwest::header::IF_NONE_MATCH;
use reqwest::{Method, Request as HttpRequest, Response, Url};
use serde::de::DeserializeOwned;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use wiremock::matchers::{any, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use uv_cache::CacheEntry;
use uv_client::{
    AuthIntegration, BaseClientBuilder, CacheControl, CachedClient, CachedClientError,
    DataWithCachePolicy, ErrorKind, RetryState,
};

#[test]
fn reject_overflowing_cache_policy_length() {
    let error = DataWithCachePolicy::from_reader(&[u8::MAX; 8][..]).unwrap_err();

    assert_matches!(error.kind(), ErrorKind::ArchiveRead(_));
}

/// Exercise the shared budget through both cached and forced-refresh requests.
async fn assert_retry_budget(
    middleware_failures: usize,
    retry_in_callback: bool,
    expected_callback_retries: &[bool],
) -> Result<()> {
    for skip_cache in [false, true] {
        let server = MockServer::start().await;
        let requests = AtomicUsize::new(0);
        Mock::given(any())
            .respond_with(move |_: &Request| {
                if requests.fetch_add(1, Ordering::Relaxed) < middleware_failures {
                    ResponseTemplate::new(503)
                } else {
                    ResponseTemplate::new(200).set_body_string("response")
                }
            })
            .expect((middleware_failures + expected_callback_retries.len()) as u64)
            .mount(&server)
            .await;

        let cache = tempfile::tempdir()?;
        let entry = CacheEntry::new(cache.path(), "response.msgpack");
        let client = CachedClient::new(
            BaseClientBuilder::default()
                .retries(2)
                .no_retry_delay(true)
                .build()?,
        );
        let url = server.uri().parse()?;
        let request = client.uncached().for_host(&url).get(server.uri()).build()?;
        let callback_retries = RefCell::new(Vec::new());
        let callback = async |response: Response, retry_state: &mut RetryState| {
            response.bytes().await.map_err(io::Error::other)?;
            let error = io::Error::new(io::ErrorKind::TimedOut, "interrupted response");
            callback_retries
                .borrow_mut()
                .push(retry_in_callback && retry_state.should_retry(&error, 0).is_some());
            Err::<String, _>(error)
        };
        let result = if skip_cache {
            client
                .skip_cache_with_retry(request, &entry, CacheControl::None, callback)
                .await
        } else {
            client
                .get_serde_with_retry(request, &entry, CacheControl::None, callback)
                .await
        };

        assert_matches!(result, Err(CachedClientError::Callback { retries: 2, .. }));
        assert_eq!(callback_retries.into_inner(), expected_callback_retries);
        server.verify().await;
    }
    Ok(())
}

fn cached_client() -> Result<CachedClient> {
    cached_client_with_retries(0)
}

fn cached_client_with_retries(retries: u32) -> Result<CachedClient> {
    let client = reqwest::Client::builder().no_proxy().build()?;
    Ok(CachedClient::new(
        BaseClientBuilder::default()
            .custom_client(client)
            .auth_integration(AuthIntegration::NoAuthMiddleware)
            .retries(retries)
            .no_retry_delay(true)
            .build()?,
    ))
}

async fn get_text(
    client: &CachedClient,
    url: &Url,
    cache_entry: &CacheEntry,
    cache_control: CacheControl,
) -> Result<String> {
    client
        .get_serde_with_retry(
            HttpRequest::new(Method::GET, url.clone()),
            cache_entry,
            cache_control,
            async |response, _retry_state| response.text().await,
        )
        .await
        .map_err(|error| anyhow!("{error:?}"))
}

/// Store a valid policy with a payload whose schema is incompatible with `String`.
async fn seed_numeric_cache(
    client: &CachedClient,
    url: &Url,
    cache_entry: &CacheEntry,
) -> Result<()> {
    let value = client
        .get_serde_with_retry(
            HttpRequest::new(Method::GET, url.clone()),
            cache_entry,
            CacheControl::None,
            async |response, _retry_state| response.text().await.map(|_| 42_u64),
        )
        .await
        .map_err(|error| anyhow!("{error:?}"))?;
    assert_eq!(value, 42);
    assert_eq!(cached_payload::<u64>(cache_entry)?, 42);
    Ok(())
}

pub(super) fn cached_payload<T: DeserializeOwned>(cache_entry: &CacheEntry) -> Result<T> {
    let cached = DataWithCachePolicy::from_reader(fs_err::File::open(cache_entry.path())?)?;
    Ok(rmp_serde::from_slice(&cached.data)?)
}

/// Serve bodies with a declared length of nine bytes, including intentionally truncated bodies.
async fn body_server(
    bodies: &'static [&'static str],
) -> Result<(Url, JoinHandle<Result<Vec<String>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = Url::parse(&format!("http://{}/metadata", listener.local_addr()?))?;
    let task = tokio::spawn(async move {
        timeout(Duration::from_secs(10), async move {
            let mut requests = Vec::new();
            for body in bodies {
                let (mut stream, _) = listener.accept().await?;
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).await?;
                    ensure!(count > 0, "request ended before its headers");
                    request.extend_from_slice(&buffer[..count]);
                    ensure!(request.len() <= 8192, "request headers are too large");
                }
                let end = request
                    .windows(2)
                    .position(|bytes| bytes == b"\r\n")
                    .ok_or_else(|| anyhow!("missing request line"))?;
                requests.push(str::from_utf8(&request[..end])?.to_owned());

                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nCache-Control: public, max-age=31536000\r\nConnection: close\r\n\r\n",
                    )
                    .await?;
                stream.write_all(body.as_bytes()).await?;
                stream.shutdown().await?;
            }
            Ok(requests)
        })
        .await?
    });
    Ok((url, task))
}

#[tokio::test]
async fn retry_truncated_response_body() -> Result<()> {
    let (url, server) = body_server(&["short", "recovered"]).await?;
    let directory = tempfile::tempdir()?;
    let cache_entry = CacheEntry::new(directory.path(), "metadata.http");
    let client = cached_client_with_retries(1)?;

    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::None).await?,
        "recovered",
    );
    assert_eq!(
        server.await??,
        ["GET /metadata HTTP/1.1", "GET /metadata HTTP/1.1"],
    );
    assert_eq!(cached_payload::<String>(&cache_entry)?, "recovered");

    // The listener is now closed, so this must be a cache hit.
    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::None).await?,
        "recovered",
    );
    Ok(())
}

#[tokio::test]
async fn reject_truncated_response_body_without_retries() -> Result<()> {
    let (url, server) = body_server(&["short"]).await?;
    let directory = tempfile::tempdir()?;
    let cache_entry = CacheEntry::new(directory.path(), "metadata.http");
    let client = cached_client()?;

    let error = client
        .get_serde_with_retry(
            HttpRequest::new(Method::GET, url),
            &cache_entry,
            CacheControl::None,
            async |response, _retry_state| response.text().await,
        )
        .await
        .expect_err("the body is shorter than its declared length");
    assert_matches!(error, CachedClientError::Callback { retries: 0, err, .. } if err.is_body() || err.is_decode());
    assert_eq!(server.await??, ["GET /metadata HTTP/1.1"]);
    assert!(!cache_entry.path().exists());
    Ok(())
}

#[tokio::test]
async fn heal_malformed_cache_policy() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/metadata"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=31536000")
                .set_body_string("recovered"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let directory = tempfile::tempdir()?;
    let cache_entry = CacheEntry::new(directory.path(), "metadata.http");
    let client = cached_client()?;
    let url = Url::parse(&format!("{}/metadata", server.uri()))?;

    // The cache envelope ends in an eight-byte policy length, so this cannot contain a policy.
    let invalid = [0_u8; 7];
    let error = DataWithCachePolicy::from_reader(&invalid[..]).expect_err("invalid cache policy");
    assert_matches!(error.kind(), ErrorKind::ArchiveRead(_));
    fs_err::write(cache_entry.path(), invalid)?;

    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::None).await?,
        "recovered",
    );
    assert_eq!(cached_payload::<String>(&cache_entry)?, "recovered");
    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::None).await?,
        "recovered",
    );
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn callback_and_outer_retries_share_budget() -> Result<()> {
    // One retry in the callback leaves one full restart, whose callback has no budget left.
    assert_retry_budget(0, true, &[true, false]).await
}

#[tokio::test]
async fn middleware_retries_are_counted_before_callback() -> Result<()> {
    // The middleware exhausts the budget before delivering a response to the callback.
    assert_retry_budget(2, true, &[false]).await
}

#[tokio::test]
async fn middleware_retries_are_not_counted_twice() -> Result<()> {
    // One middleware retry leaves one full restart after the callback fails.
    assert_retry_budget(1, false, &[false, false]).await
}

#[tokio::test]
async fn send_counts_middleware_retries() -> Result<()> {
    for network_error in [false, true] {
        let server = MockServer::start().await;
        let mock = if network_error {
            Mock::given(any())
                .respond_with_err(|_: &Request| {
                    io::Error::new(io::ErrorKind::ConnectionReset, "connection reset")
                })
                .expect(3)
        } else {
            let requests = AtomicUsize::new(0);
            Mock::given(any())
                .respond_with(move |_: &Request| {
                    if requests.fetch_add(1, Ordering::Relaxed) == 0 {
                        ResponseTemplate::new(503)
                    } else {
                        ResponseTemplate::new(200)
                    }
                })
                .expect(2)
        };
        mock.mount(&server).await;

        let client = BaseClientBuilder::default()
            .retries(2)
            .no_retry_delay(true)
            .build()?;
        let url = server.uri().parse()?;
        let request = client.for_host(&url).get(server.uri());
        let mut retry_state = RetryState::start(client.retry_policy(), url);
        let result = retry_state.send(request).await;
        assert_eq!(result.is_err(), network_error);

        let error = io::Error::new(io::ErrorKind::TimedOut, "interrupted response");
        if !network_error {
            // The successful request used one retry, leaving one for a body failure.
            assert!(retry_state.should_retry(&error, 0).is_some());
        }
        assert!(retry_state.should_retry(&error, 0).is_none());
        server.verify().await;
    }
    Ok(())
}

#[tokio::test]
async fn heal_fresh_cache_payload() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/metadata"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=31536000")
                .set_body_string("recovered"),
        )
        .expect(2)
        .mount(&server)
        .await;
    let directory = tempfile::tempdir()?;
    let cache_entry = CacheEntry::new(directory.path(), "metadata.http");
    let client = cached_client()?;
    let url = Url::parse(&format!("{}/metadata", server.uri()))?;

    seed_numeric_cache(&client, &url, &cache_entry).await?;
    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::None).await?,
        "recovered",
    );
    assert_eq!(cached_payload::<String>(&cache_entry)?, "recovered");
    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::None).await?,
        "recovered",
    );
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn heal_allowed_stale_cache_payload() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/metadata"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=0")
                .set_body_string("recovered"),
        )
        .expect(2)
        .mount(&server)
        .await;
    let directory = tempfile::tempdir()?;
    let cache_entry = CacheEntry::new(directory.path(), "metadata.http");
    let client = cached_client()?;
    let url = Url::parse(&format!("{}/metadata", server.uri()))?;

    seed_numeric_cache(&client, &url, &cache_entry).await?;
    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::AllowStale).await?,
        "recovered",
    );
    assert_eq!(cached_payload::<String>(&cache_entry)?, "recovered");
    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::AllowStale).await?,
        "recovered",
    );
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn heal_revalidated_cache_payload() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/metadata"))
        .and(|request: &wiremock::Request| !request.headers.contains_key(IF_NONE_MATCH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "public, max-age=31536000")
                .insert_header("ETag", "\"v1\"")
                .set_body_string("recovered"),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/metadata"))
        .and(header("If-None-Match", "\"v1\""))
        .respond_with(
            ResponseTemplate::new(304)
                .insert_header("Cache-Control", "public, max-age=31536000")
                .insert_header("ETag", "\"v1\""),
        )
        .expect(1)
        .mount(&server)
        .await;
    let directory = tempfile::tempdir()?;
    let cache_entry = CacheEntry::new(directory.path(), "metadata.http");
    let client = cached_client()?;
    let url = Url::parse(&format!("{}/metadata", server.uri()))?;

    seed_numeric_cache(&client, &url, &cache_entry).await?;
    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::MustRevalidate).await?,
        "recovered",
    );
    assert_eq!(cached_payload::<String>(&cache_entry)?, "recovered");
    assert_eq!(
        get_text(&client, &url, &cache_entry, CacheControl::None).await?,
        "recovered",
    );
    server.verify().await;
    Ok(())
}
