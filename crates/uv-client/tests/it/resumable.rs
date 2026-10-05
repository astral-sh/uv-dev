use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::TryStreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use uv_client::{BaseClientBuilder, RetryState, resumable_bytes_stream};
use uv_redacted::DisplaySafeUrl;

type Headers = BTreeMap<String, String>;

struct Reply {
    head: String,
    body: &'static [u8],
}

fn reply(status: u16, headers: &str, length: usize, body: &'static [u8]) -> Reply {
    Reply {
        head: format!(
            "HTTP/1.1 {status} Response\r\nConnection: close\r\nContent-Length: {length}\r\n{headers}\r\n"
        ),
        body,
    }
}

fn interrupted() -> Reply {
    reply(
        200,
        "Accept-Ranges: bytes\r\nETag: \"one\"\r\n",
        10,
        b"abcd",
    )
}

fn range(headers: &str, length: usize, body: &'static [u8]) -> Reply {
    reply(206, &format!("ETag: \"one\"\r\n{headers}"), length, body)
}

async fn server(replies: Vec<Reply>) -> Result<(DisplaySafeUrl, JoinHandle<Result<Vec<Headers>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = DisplaySafeUrl::parse(&format!("http://{}/archive", listener.local_addr()?))?;
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().await?;
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await?);
                anyhow::ensure!(request.len() < 16384, "Request headers too large");
            }
            let request = String::from_utf8(request)?;
            requests.push(
                request
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
                    .collect(),
            );
            stream.write_all(reply.head.as_bytes()).await?;
            stream.write_all(reply.body).await?;
            stream.shutdown().await?;
        }
        Ok(requests)
    });
    Ok((url, task))
}

async fn download(
    replies: Vec<Reply>,
    retries: u32,
) -> Result<(Vec<u8>, Option<reqwest_middleware::Error>, Vec<Headers>)> {
    let (url, server) = server(replies).await?;
    let client = BaseClientBuilder::default()
        .retries(retries)
        .no_retry_delay(true)
        .build()?;
    let mut retry_state = RetryState::start(client.retry_policy(), url.clone());
    let response = retry_state
        .send(
            client
                .for_host(&url)
                .get(url.as_str())
                .header("accept-encoding", "identity"),
        )
        .await?
        .error_for_status()?;
    let mut stream =
        resumable_bytes_stream(response, client.for_host(&url), &url, &mut retry_state);
    let mut bytes = Vec::new();
    let error = loop {
        match stream.try_next().await {
            Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
            Ok(None) => break None,
            Err(err) => break Some(err),
        }
    };
    let requests = tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .context("Server did not receive the expected requests")???;
    Ok((bytes, error, requests))
}

#[tokio::test]
async fn continues_through_completed_partial_ranges() -> Result<()> {
    let (bytes, error, requests) = download(
        vec![
            interrupted(),
            range("Content-Range: bytes 4-6/10\r\n", 3, b"efg"),
            range("Content-Range: bytes 7-9/*\r\n", 3, b"hij"),
        ],
        1,
    )
    .await?;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(bytes, b"abcdefghij");
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1]["range"], "bytes=4-");
    assert_eq!(requests[2]["range"], "bytes=7-");
    for request in &requests[1..] {
        assert_eq!(request["if-range"], "\"one\"");
        assert_eq!(request["accept-encoding"], "identity");
    }
    Ok(())
}

#[tokio::test]
async fn repeated_interruptions_share_the_retry_budget() -> Result<()> {
    let (bytes, error, requests) = download(
        vec![
            interrupted(),
            range("Content-Range: bytes 4-9/10\r\n", 6, b"ef"),
            range("Content-Range: bytes 6-9/10\r\n", 4, b"ghij"),
        ],
        2,
    )
    .await?;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(bytes, b"abcdefghij");
    assert_eq!(requests[2]["range"], "bytes=6-");

    let (bytes, error, requests) = download(
        vec![
            interrupted(),
            range("Content-Range: bytes 4-9/10\r\n", 6, b"ef"),
        ],
        1,
    )
    .await?;
    assert!(error.is_some());
    assert_eq!(bytes, b"abcdef");
    assert_eq!(requests.len(), 2);

    let (bytes, error, requests) = download(vec![interrupted()], 0).await?;
    assert!(error.is_some());
    assert_eq!(bytes, b"abcd");
    assert_eq!(requests.len(), 1);
    Ok(())
}

#[tokio::test]
async fn middleware_retries_count_toward_body_retries() -> Result<()> {
    let (bytes, error, requests) = download(vec![reply(503, "", 0, b""), interrupted()], 1).await?;
    assert!(error.is_some());
    assert_eq!(bytes, b"abcd");
    assert_eq!(requests.len(), 2);
    Ok(())
}

#[tokio::test]
async fn requires_an_identifiable_range_supported_representation() -> Result<()> {
    for headers in [
        "Accept-Ranges: bytes\r\n",
        "Accept-Ranges: bytes\r\nETag: W/\"one\"\r\n",
        "ETag: \"one\"\r\n",
    ] {
        let (bytes, error, requests) = download(vec![reply(200, headers, 10, b"abcd")], 2).await?;
        assert!(error.is_some());
        assert_eq!(bytes, b"abcd");
        assert_eq!(requests.len(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn rejects_changed_or_invalid_continuations() -> Result<()> {
    for continuation in [
        reply(200, "ETag: \"one\"\r\n", 10, b"abcdefghij"),
        reply(
            206,
            "ETag: \"two\"\r\nContent-Range: bytes 4-9/10\r\n",
            6,
            b"efghij",
        ),
        reply(206, "Content-Range: bytes 4-9/10\r\n", 6, b"efghij"),
        range("Content-Range: bytes 5-9/10\r\n", 5, b"fghij"),
        range("Content-Range: bytes 4-9/11\r\n", 6, b"efghij"),
        range("Content-Range: bytes 4-9/10\r\n", 5, b"efghi"),
    ] {
        let (bytes, error, requests) = download(vec![interrupted(), continuation], 2).await?;
        assert!(error.is_some());
        assert_eq!(bytes, b"abcd");
        assert_eq!(requests.len(), 2);
    }
    Ok(())
}

const LAST_MODIFIED: &str = "Wed, 30 Sep 2026 12:00:00 GMT";
const STRONG_DATES: &str =
    "Last-Modified: Wed, 30 Sep 2026 12:00:00 GMT\r\nDate: Wed, 30 Sep 2026 12:01:00 GMT\r\n";

fn date_interrupted() -> Reply {
    reply(
        200,
        &format!("Accept-Ranges: bytes\r\n{STRONG_DATES}"),
        10,
        b"abcd",
    )
}

#[tokio::test]
async fn continues_with_a_strong_last_modified_date() -> Result<()> {
    let (bytes, error, requests) = download(
        vec![
            date_interrupted(),
            reply(
                206,
                &format!("{STRONG_DATES}Content-Range: bytes 4-6/10\r\n"),
                3,
                b"efg",
            ),
            reply(
                206,
                &format!("Last-Modified: {LAST_MODIFIED}\r\nContent-Range: bytes 7-9/10\r\n"),
                3,
                b"hij",
            ),
        ],
        1,
    )
    .await?;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(bytes, b"abcdefghij");
    assert_eq!(requests.len(), 3);
    for request in &requests[1..] {
        assert_eq!(request["if-range"], LAST_MODIFIED);
    }
    Ok(())
}

#[tokio::test]
async fn prefers_entity_tags_and_rejects_weak_dates() -> Result<()> {
    let (bytes, error, requests) = download(
        vec![
            reply(
                200,
                &format!("Accept-Ranges: bytes\r\nETag: \"one\"\r\n{STRONG_DATES}"),
                10,
                b"abcd",
            ),
            range("Content-Range: bytes 4-9/10\r\n", 6, b"efghij"),
        ],
        1,
    )
    .await?;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(bytes, b"abcdefghij");
    assert_eq!(requests[1]["if-range"], "\"one\"");

    for headers in [
        format!("ETag: W/\"one\"\r\n{STRONG_DATES}"),
        format!("ETag: invalid\r\n{STRONG_DATES}"),
        format!("Last-Modified: {LAST_MODIFIED}\r\n"),
        format!("Last-Modified: {LAST_MODIFIED}\r\nDate: Wed, 30 Sep 2026 12:00:59 GMT\r\n"),
        format!("Last-Modified: {LAST_MODIFIED}\r\nDate: Wed, 30 Sep 2026 11:59:00 GMT\r\n"),
        format!("Last-Modified: {LAST_MODIFIED}\r\nDate: invalid\r\n"),
        "Last-Modified: invalid\r\nDate: Wed, 30 Sep 2026 12:01:00 GMT\r\n".to_owned(),
    ] {
        let (bytes, error, requests) = download(
            vec![reply(
                200,
                &format!("Accept-Ranges: bytes\r\n{headers}"),
                10,
                b"abcd",
            )],
            1,
        )
        .await?;
        assert!(error.is_some(), "{headers:?}");
        assert_eq!(bytes, b"abcd");
        assert_eq!(requests.len(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn rejects_changed_or_missing_last_modified_dates() -> Result<()> {
    for headers in [
        "Last-Modified: Wed, 30 Sep 2026 12:00:01 GMT\r\n",
        "Last-Modified: invalid\r\n",
        "ETag: \"new\"\r\n",
        "",
    ] {
        let (bytes, error, requests) = download(
            vec![
                date_interrupted(),
                reply(
                    206,
                    &format!("{headers}Content-Range: bytes 4-9/10\r\n"),
                    6,
                    b"efghij",
                ),
            ],
            1,
        )
        .await?;
        assert!(error.is_some(), "{headers:?}");
        assert_eq!(bytes, b"abcd");
        assert_eq!(requests.len(), 2);
    }
    Ok(())
}
