use std::io::{BufRead, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};

use super::*;

const FIRST: &str = concat!(
    r#"{"version":"3.13.2","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/python-3.13.2.tar.gz"}]}"#,
    "\n"
);
const SECOND: &str = concat!(
    r#"{"version":"3.14.1","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/python-3.14.1.tar.gz"}]}"#,
    "\n"
);
const PREPENDED: &str = concat!(
    r#"{"version":"3.12.9","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/python-3.12.9.tar.gz"}]}"#,
    "\n"
);

struct Exchange {
    method: &'static str,
    range: Option<String>,
    if_match: Option<&'static str>,
    response: String,
}

struct Interruption {
    exchange: usize,
    record_read: Receiver<()>,
    remaining_bytes: String,
}

struct ObservedSearch<'a> {
    search: DownloadSearch<'a>,
    record_read: &'a Sender<()>,
}

impl DownloadQuery for ObservedSearch<'_> {
    type Output = Option<ManagedPythonDownload>;

    fn visit(&mut self, download: ManagedPythonDownload) -> ControlFlow<()> {
        let result = DownloadQuery::visit(&mut self.search, download);
        if result.is_break() {
            self.record_read
                .send(())
                .expect("server is waiting for the matching record");
        }
        result
    }

    fn finish(self) -> Self::Output {
        DownloadQuery::finish(self.search)
    }
}

struct MetadataServer {
    url: SafeUrl,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<()>>>,
}

impl MetadataServer {
    fn start(exchanges: Vec<Exchange>) -> Result<Self> {
        Self::start_with_interruption(exchanges, None)
    }

    fn start_with_interruption(
        exchanges: Vec<Exchange>,
        interruption: Option<Interruption>,
    ) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let url = SafeUrl::parse(&format!(
            "http://{}/versions.ndjson",
            listener.local_addr()?
        ))?;
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut received = 0;
            while !server_stop.load(Ordering::SeqCst) {
                ensure!(Instant::now() < deadline, "metadata server timed out");
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                };
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                let request = read_request(&mut stream)?;
                let exchange = exchanges.get(received).context("unexpected HTTP request")?;
                ensure!(
                    request.starts_with(&format!(
                        "{} /versions.ndjson HTTP/1.1\r\n",
                        exchange.method
                    )),
                    "unexpected request: {request}"
                );
                let header = |name: &str| {
                    request.lines().find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case(name).then(|| value.trim())
                    })
                };
                ensure!(
                    header("range") == exchange.range.as_deref(),
                    "wrong range: {request}"
                );
                ensure!(
                    header("if-match") == exchange.if_match,
                    "wrong validator: {request}"
                );
                ensure!(
                    header("accept-encoding") == Some("identity"),
                    "encoded byte range: {request}"
                );
                stream.write_all(exchange.response.as_bytes())?;
                if let Some(interruption) = &interruption
                    && interruption.exchange == received
                {
                    interruption
                        .record_read
                        .recv_timeout(Duration::from_secs(2))?;
                    stream.write_all(interruption.remaining_bytes.as_bytes())?;
                }
                received += 1;
            }
            ensure!(
                received == exchanges.len(),
                "received {received} of {} requests",
                exchanges.len()
            );
            Ok(())
        });
        Ok(Self {
            url,
            stop,
            thread: Some(thread),
        })
    }

    fn finish(mut self) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread
            .take()
            .context("missing server thread")?
            .join()
            .map_err(|_| anyhow::anyhow!("metadata server panicked"))?
    }
}

impl Drop for MetadataServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn read_request(stream: &mut TcpStream) -> Result<String> {
    let mut reader = std::io::BufReader::new(stream);
    let mut request = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            bail!("request ended before its headers");
        }
        request.push_str(&line);
        if line == "\r\n" {
            return Ok(request);
        }
    }
}

fn full_response(content: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{content}",
        content.len()
    )
}

async fn seed_prefix(cache: &Cache, url: &SafeUrl) -> Result<()> {
    let shard = versions_cache_shard(cache, url);
    let (content_entry, meta_entry) = versions_cache_entries(&shard);
    write_versions_cache(
        &content_entry,
        &meta_entry,
        FIRST.as_bytes(),
        &VersionsCacheMeta {
            content_length: FIRST.len() as u64,
            representation_length: Some((FIRST.len() + SECOND.len()) as u64),
            complete: false,
            etag: Some("\"v1\"".to_owned()),
            checked_at: SystemTime::now(),
        },
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn exact_lookup_persists_and_reuses_a_complete_record_prefix() -> Result<()> {
    let full = format!("{FIRST}{SECOND}");
    let server = MetadataServer::start(vec![Exchange {
        method: "GET",
        range: None,
        if_match: None,
        response: full_response(&full),
    }])?;
    let cache = Cache::temp()?.init().await?;
    let client = BaseClientBuilder::default().retries(0).build()?;
    let request = PythonDownloadRequest::from_str("cpython-3.13.2-linux-x86_64-gnu")?;

    let first = fetch_ndjson_find_cached(&client, &server.url, &cache, &request)
        .await?
        .context("matching download")?;
    assert_eq!(first.key().version().to_string(), "3.13.2");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("cached prefix")?;
    assert_eq!(content, FIRST.as_bytes());
    assert_eq!(meta.content_length, FIRST.len() as u64);
    assert_eq!(meta.representation_length, Some(full.len() as u64));
    assert!(!meta.complete);
    assert_eq!(meta.etag.as_deref(), Some("\"v1\""));

    assert_eq!(
        fetch_ndjson_find_cached(&client, &server.url, &cache, &request).await?,
        Some(first)
    );
    server.finish()?;
    Ok(())
}

/// A full `GET` that stops at an exact match must retain a longer cached prefix of the same
/// representation.
#[tokio::test]
async fn exact_refresh_retains_a_longer_prefix_of_the_same_representation() -> Result<()> {
    let prefix = format!("{FIRST}{SECOND}");
    let full = format!("{prefix}{PREPENDED}");
    let server = MetadataServer::start(vec![
        Exchange {
            method: "HEAD",
            range: None,
            if_match: None,
            response:
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned(),
        },
        Exchange {
            method: "GET",
            range: None,
            if_match: None,
            response: full_response(&full),
        },
    ])?;
    let cache = Cache::temp()?.init().await?;
    let shard = versions_cache_shard(&cache, &server.url);
    let (content_entry, meta_entry) = versions_cache_entries(&shard);
    let previous_meta = VersionsCacheMeta {
        content_length: prefix.len() as u64,
        representation_length: Some(full.len() as u64),
        complete: false,
        etag: Some("\"v1\"".to_owned()),
        checked_at: SystemTime::UNIX_EPOCH,
    };
    write_versions_cache(
        &content_entry,
        &meta_entry,
        prefix.as_bytes(),
        &previous_meta,
    )
    .await?;
    assert!(!versions_cache_is_fresh(
        &cache,
        &server.url,
        &previous_meta
    ));
    let client = BaseClientBuilder::default().retries(0).build()?;
    let request = PythonDownloadRequest::from_str("cpython-3.13.2-linux-x86_64-gnu")?;

    let download = fetch_ndjson_find_cached(&client, &server.url, &cache, &request)
        .await?
        .context("matching first record")?;
    assert_eq!(download.key().version().to_string(), "3.13.2");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("longer cached prefix")?;
    assert_eq!(content, prefix.as_bytes());
    assert_eq!(
        meta,
        VersionsCacheMeta {
            checked_at: meta.checked_at,
            ..previous_meta
        }
    );
    assert!(versions_cache_is_fresh(&cache, &server.url, &meta));

    let offline_client = BaseClientBuilder::default()
        .connectivity(Connectivity::Offline)
        .build()?;
    let request = PythonDownloadRequest::from_str("cpython-3.14.1-linux-x86_64-gnu")?;
    let download = fetch_ndjson_find_cached(&offline_client, &server.url, &cache, &request)
        .await?
        .context("matching second record retained for offline use")?;
    assert_eq!(download.key().version().to_string(), "3.14.1");
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn deeper_exact_lookup_resumes_from_the_cached_offset() -> Result<()> {
    let full = format!("{FIRST}{SECOND}");
    let server = MetadataServer::start(vec![Exchange {
        method: "GET",
        range: Some(format!("bytes={}-", FIRST.len())),
        if_match: Some("\"v1\""),
        response: format!(
            "HTTP/1.1 206 Partial Content\r\nETag: \"v1\"\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n{SECOND}",
            SECOND.len(),
            FIRST.len(),
            full.len() - 1,
            full.len()
        ),
    }])?;
    let cache = Cache::temp()?.init().await?;
    seed_prefix(&cache, &server.url).await?;
    let client = BaseClientBuilder::default().retries(0).build()?;
    let request = PythonDownloadRequest::from_str("cpython-3.14.1-linux-x86_64-gnu")?;

    let download = fetch_ndjson_find_cached(&client, &server.url, &cache, &request)
        .await?
        .context("matching download in resumed response")?;
    assert_eq!(download.key().version().to_string(), "3.14.1");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("complete cache")?;
    assert_eq!(content, full.as_bytes());
    assert!(meta.complete);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn broad_lookup_resumes_before_selecting_the_highest_version() -> Result<()> {
    let full = format!("{FIRST}{SECOND}");
    let server = MetadataServer::start(vec![Exchange {
        method: "GET",
        range: Some(format!("bytes={}-", FIRST.len())),
        if_match: Some("\"v1\""),
        response: format!(
            "HTTP/1.1 206 Partial Content\r\nETag: \"v1\"\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n{SECOND}",
            SECOND.len(),
            FIRST.len(),
            full.len() - 1,
            full.len()
        ),
    }])?;
    let cache = Cache::temp()?.init().await?;
    seed_prefix(&cache, &server.url).await?;
    let client = BaseClientBuilder::default().retries(0).build()?;
    let request = PythonDownloadRequest::from_str("cpython-3-linux-x86_64-gnu")?;

    let download = fetch_ndjson_find_cached(&client, &server.url, &cache, &request)
        .await?
        .context("highest compatible download")?;
    assert_eq!(download.key().version().to_string(), "3.14.1");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("complete cache")?;
    assert_eq!(content, full.as_bytes());
    assert!(meta.complete);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn prepend_to_partial_cache_uses_representation_length_then_resumes() -> Result<()> {
    let full = format!("{PREPENDED}{FIRST}{SECOND}");
    let offset = PREPENDED.len() + FIRST.len();
    let server = MetadataServer::start(vec![
        Exchange {
            method: "HEAD",
            range: None,
            if_match: None,
            response: format!(
                "HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                full.len()
            ),
        },
        Exchange {
            method: "GET",
            range: Some(format!("bytes=0-{}", PREPENDED.len() - 1)),
            if_match: Some("\"v2\""),
            response: format!(
                "HTTP/1.1 206 Partial Content\r\nETag: \"v2\"\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n{PREPENDED}",
                PREPENDED.len(),
                PREPENDED.len() - 1,
                full.len()
            ),
        },
        Exchange {
            method: "GET",
            range: Some(format!("bytes={offset}-")),
            if_match: Some("\"v2\""),
            response: format!(
                "HTTP/1.1 206 Partial Content\r\nETag: \"v2\"\r\nContent-Length: {}\r\nContent-Range: bytes {offset}-{}/{}\r\nConnection: close\r\n\r\n{SECOND}",
                SECOND.len(),
                full.len() - 1,
                full.len()
            ),
        },
    ])?;
    let cache = Cache::temp()?.init().await?;
    let shard = versions_cache_shard(&cache, &server.url);
    let (content_entry, meta_entry) = versions_cache_entries(&shard);
    write_versions_cache(
        &content_entry,
        &meta_entry,
        FIRST.as_bytes(),
        &VersionsCacheMeta {
            content_length: FIRST.len() as u64,
            representation_length: Some((FIRST.len() + SECOND.len()) as u64),
            complete: false,
            etag: Some("\"v1\"".to_owned()),
            checked_at: SystemTime::UNIX_EPOCH,
        },
    )
    .await?;
    let client = BaseClientBuilder::default().retries(0).build()?;
    let predicate = |_: &ManagedPythonDownload| true;

    let downloads = fetch_ndjson_cached_query(&client, &server.url, &cache, true, || {
        DownloadCollector::new(&predicate)
    })
    .await?;
    assert_eq!(downloads.len(), 3);
    let (cached, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("complete cache")?;
    assert_eq!(cached, full.as_bytes());
    assert_eq!(meta.content_length, full.len() as u64);
    assert_eq!(meta.representation_length, Some(full.len() as u64));
    assert_eq!(meta.etag.as_deref(), Some("\"v2\""));
    assert!(meta.complete);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn ignored_resume_range_replaces_the_cached_prefix() -> Result<()> {
    let server = MetadataServer::start(vec![Exchange {
        method: "GET",
        range: Some(format!("bytes={}-", FIRST.len())),
        if_match: Some("\"v1\""),
        response: full_response(SECOND),
    }])?;
    let cache = Cache::temp()?.init().await?;
    seed_prefix(&cache, &server.url).await?;
    let client = BaseClientBuilder::default().retries(0).build()?;

    let downloads =
        fetch_ndjson_collect_streaming_cached(&client, &server.url, &cache, |_| true).await?;
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].key().version().to_string(), "3.14.1");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("replacement cache")?;
    assert_eq!(content, SECOND.as_bytes());
    assert_eq!(meta.representation_length, Some(SECOND.len() as u64));
    assert!(meta.complete);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn mismatched_resume_range_retries_a_full_download() -> Result<()> {
    let full = format!("{FIRST}{SECOND}");
    let server = MetadataServer::start(vec![
        Exchange {
            method: "GET",
            range: Some(format!("bytes={}-", FIRST.len())),
            if_match: Some("\"v1\""),
            response: format!(
                "HTTP/1.1 206 Partial Content\r\nETag: \"v1\"\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n{SECOND}",
                SECOND.len(),
                SECOND.len() - 1,
                full.len()
            ),
        },
        Exchange {
            method: "GET",
            range: None,
            if_match: None,
            response: full_response(&full),
        },
    ])?;
    let cache = Cache::temp()?.init().await?;
    seed_prefix(&cache, &server.url).await?;
    let client = BaseClientBuilder::default().retries(0).build()?;

    let downloads =
        fetch_ndjson_collect_streaming_cached(&client, &server.url, &cache, |_| true).await?;
    assert_eq!(downloads.len(), 2);
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("complete cache")?;
    assert_eq!(content, full.as_bytes());
    assert!(meta.complete);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn failed_resume_precondition_retries_a_full_download() -> Result<()> {
    let full = format!("{FIRST}{SECOND}");
    let server = MetadataServer::start(vec![
        Exchange {
            method: "GET",
            range: Some(format!("bytes={}-", FIRST.len())),
            if_match: Some("\"v1\""),
            response:
                "HTTP/1.1 412 Precondition Failed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned(),
        },
        Exchange {
            method: "GET",
            range: None,
            if_match: None,
            response: full_response(&full),
        },
    ])?;
    let cache = Cache::temp()?.init().await?;
    seed_prefix(&cache, &server.url).await?;
    let client = BaseClientBuilder::default().retries(0).build()?;

    let downloads =
        fetch_ndjson_collect_streaming_cached(&client, &server.url, &cache, |_| true).await?;
    assert_eq!(downloads.len(), 2);
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("complete cache")?;
    assert_eq!(content, full.as_bytes());
    assert!(meta.complete);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn offline_partial_cache_cannot_prove_a_broad_match_or_absence() -> Result<()> {
    let cache = Cache::temp()?.init().await?;
    let url = SafeUrl::parse("http://127.0.0.1:9/versions.ndjson")?;
    seed_prefix(&cache, &url).await?;
    let client = BaseClientBuilder::default()
        .connectivity(Connectivity::Offline)
        .retries(0)
        .build()?;
    let exact = PythonDownloadRequest::from_str("cpython-3.13.2-linux-x86_64-gnu")?;
    let broad = PythonDownloadRequest::from_str("cpython-3-linux-x86_64-gnu")?;
    let absent = PythonDownloadRequest::from_str("cpython-3.99.0-linux-x86_64-gnu")?;

    let download = fetch_ndjson_find_cached(&client, &url, &cache, &exact)
        .await?
        .context("proven exact match")?;
    assert_eq!(download.key().version().to_string(), "3.13.2");
    assert!(
        fetch_ndjson_find_cached(&client, &url, &cache, &broad)
            .await
            .is_err()
    );
    assert!(
        fetch_ndjson_find_cached(&client, &url, &cache, &absent)
            .await
            .is_err()
    );
    assert!(
        fetch_ndjson_collect_streaming_cached(&client, &url, &cache, |_| true)
            .await
            .is_err()
    );
    let (content, meta) = read_versions_cache_content(&cache, &url)
        .await
        .context("unchanged prefix")?;
    assert_eq!(content, FIRST.as_bytes());
    assert!(!meta.complete);
    Ok(())
}

#[tokio::test]
async fn interrupted_cold_download_caches_only_complete_records() -> Result<()> {
    let full = format!("{FIRST}{SECOND}");
    let partial = &SECOND[..SECOND.len() / 2];
    let (record_read, received_record) = channel();
    let server = MetadataServer::start_with_interruption(
        vec![Exchange {
            method: "GET",
            range: None,
            if_match: None,
            response: format!(
                "HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{FIRST}",
                full.len()
            ),
        }],
        Some(Interruption {
            exchange: 0,
            record_read: received_record,
            remaining_bytes: partial.to_owned(),
        }),
    )?;
    let cache = Cache::temp()?.init().await?;
    let client = BaseClientBuilder::default().retries(0).build()?;

    assert!(
        fetch_ndjson_collect_streaming_cached(&client, &server.url, &cache, |_| {
            record_read
                .send(())
                .expect("server is waiting for the first record");
            true
        })
        .await
        .is_err()
    );
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("prefix from interrupted response")?;
    assert_eq!(content, FIRST.as_bytes());
    assert_eq!(meta.content_length, FIRST.len() as u64);
    assert_eq!(meta.representation_length, Some(full.len() as u64));
    assert_eq!(meta.etag.as_deref(), Some("\"v1\""));
    assert!(!meta.complete);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn interrupted_resume_retains_the_longer_complete_record_prefix() -> Result<()> {
    let full = format!("{FIRST}{SECOND}{PREPENDED}");
    let partial = &PREPENDED[..PREPENDED.len() / 2];
    let (record_read, received_record) = channel();
    let server = MetadataServer::start_with_interruption(
        vec![Exchange {
            method: "GET",
            range: Some(format!("bytes={}-", FIRST.len())),
            if_match: Some("\"v1\""),
            response: format!(
                "HTTP/1.1 206 Partial Content\r\nETag: \"v1\"\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n{SECOND}",
                SECOND.len() + PREPENDED.len(),
                FIRST.len(),
                full.len() - 1,
                full.len()
            ),
        }],
        Some(Interruption {
            exchange: 0,
            record_read: received_record,
            remaining_bytes: partial.to_owned(),
        }),
    )?;
    let cache = Cache::temp()?.init().await?;
    let shard = versions_cache_shard(&cache, &server.url);
    let (content_entry, meta_entry) = versions_cache_entries(&shard);
    write_versions_cache(
        &content_entry,
        &meta_entry,
        FIRST.as_bytes(),
        &VersionsCacheMeta {
            content_length: FIRST.len() as u64,
            representation_length: Some(full.len() as u64),
            complete: false,
            etag: Some("\"v1\"".to_owned()),
            checked_at: SystemTime::now(),
        },
    )
    .await?;
    let client = BaseClientBuilder::default().retries(0).build()?;

    assert!(
        fetch_ndjson_collect_streaming_cached(&client, &server.url, &cache, |download| {
            if download.key().version().to_string() == "3.14.1" {
                record_read
                    .send(())
                    .expect("server is waiting for the second record");
            }
            true
        })
        .await
        .is_err()
    );
    let expected = format!("{FIRST}{SECOND}");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("extended prefix from interrupted resume")?;
    assert_eq!(content, expected.as_bytes());
    assert_eq!(meta.content_length, expected.len() as u64);
    assert_eq!(meta.representation_length, Some(full.len() as u64));
    assert_eq!(meta.etag.as_deref(), Some("\"v1\""));
    assert!(!meta.complete);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn incomplete_offline_mirror_cache_falls_back_to_complete_origin_cache() -> Result<()> {
    let cache = Cache::temp()?.init().await?;
    let mirror = SafeUrl::parse("https://example.com/mirror.ndjson")?;
    let origin = SafeUrl::parse("https://example.com/origin.ndjson")?;
    seed_prefix(&cache, &mirror).await?;
    let content = concat!(
        r#"{"version":"3.99.0","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/python-3.99.0.tar.gz"}]}"#,
        "\n"
    );
    let shard = versions_cache_shard(&cache, &origin);
    let (content_entry, meta_entry) = versions_cache_entries(&shard);
    write_versions_cache(
        &content_entry,
        &meta_entry,
        content.as_bytes(),
        &VersionsCacheMeta {
            content_length: content.len() as u64,
            representation_length: Some(content.len() as u64),
            complete: true,
            etag: Some("\"origin\"".to_owned()),
            checked_at: SystemTime::now(),
        },
    )
    .await?;
    let source = DownloadListSource {
        location: DownloadListLocation::Http(vec![mirror, origin]),
        format: DownloadListFormat::Ndjson,
        implicit: true,
    };
    let client_builder = BaseClientBuilder::default()
        .connectivity(Connectivity::Offline)
        .retries(0);
    let request = PythonDownloadRequest::from_str("cpython-3-linux-x86_64-gnu")?;

    let download = find_matching_download(&client_builder, &source, &cache, &request)
        .await?
        .context("download from complete origin cache")?;
    assert_eq!(download.key().version().to_string(), "3.99.0");
    Ok(())
}

#[tokio::test]
async fn interrupted_refresh_retains_the_previous_complete_catalog() -> Result<()> {
    let refreshed = format!("{FIRST}{PREPENDED}");
    let partial = &PREPENDED[..PREPENDED.len() / 2];
    let (record_read, received_record) = channel();
    let server = MetadataServer::start_with_interruption(
        vec![
            Exchange {
                method: "HEAD",
                range: None,
                if_match: None,
                response: format!(
                    "HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    refreshed.len()
                ),
            },
            Exchange {
                method: "GET",
                range: None,
                if_match: None,
                response: format!(
                    "HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{FIRST}",
                    refreshed.len()
                ),
            },
        ],
        Some(Interruption {
            exchange: 1,
            record_read: received_record,
            remaining_bytes: partial.to_owned(),
        }),
    )?;
    let cache = Cache::temp()?.init().await?;
    let shard = versions_cache_shard(&cache, &server.url);
    let (content_entry, meta_entry) = versions_cache_entries(&shard);
    let previous_meta = VersionsCacheMeta {
        content_length: SECOND.len() as u64,
        representation_length: Some(SECOND.len() as u64),
        complete: true,
        etag: Some("\"v1\"".to_owned()),
        checked_at: SystemTime::UNIX_EPOCH,
    };
    write_versions_cache(
        &content_entry,
        &meta_entry,
        SECOND.as_bytes(),
        &previous_meta,
    )
    .await?;
    let client = BaseClientBuilder::default().retries(0).build()?;

    let downloads =
        fetch_ndjson_collect_streaming_cached(&client, &server.url, &cache, |download| {
            if download.key().version().to_string() == "3.13.2" {
                record_read
                    .send(())
                    .expect("server is waiting for the refreshed record");
            }
            true
        })
        .await?;
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].key().version().to_string(), "3.14.1");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("previous complete catalog")?;
    assert_eq!(content, SECOND.as_bytes());
    assert_eq!(meta, previous_meta);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn exact_refresh_retains_complete_catalog_coverage() -> Result<()> {
    let refreshed = format!("{FIRST}{PREPENDED}");
    let server = MetadataServer::start(vec![
        Exchange {
            method: "HEAD",
            range: None,
            if_match: None,
            response: format!(
                "HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                refreshed.len()
            ),
        },
        Exchange {
            method: "GET",
            range: None,
            if_match: None,
            response: format!(
                "HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{refreshed}",
                refreshed.len()
            ),
        },
    ])?;
    let cache = Cache::temp()?.init().await?;
    let shard = versions_cache_shard(&cache, &server.url);
    let (content_entry, meta_entry) = versions_cache_entries(&shard);
    write_versions_cache(
        &content_entry,
        &meta_entry,
        SECOND.as_bytes(),
        &VersionsCacheMeta {
            content_length: SECOND.len() as u64,
            representation_length: Some(SECOND.len() as u64),
            complete: true,
            etag: Some("\"v1\"".to_owned()),
            checked_at: SystemTime::UNIX_EPOCH,
        },
    )
    .await?;
    let client = BaseClientBuilder::default().retries(0).build()?;
    let request = PythonDownloadRequest::from_str("cpython-3.13.2-linux-x86_64-gnu")?;

    let download = fetch_ndjson_find_cached(&client, &server.url, &cache, &request)
        .await?
        .context("matching first record")?;
    assert_eq!(download.key().version().to_string(), "3.13.2");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("complete refreshed catalog")?;
    assert_eq!(content, refreshed.as_bytes());
    assert_eq!(meta.content_length, refreshed.len() as u64);
    assert_eq!(meta.representation_length, Some(refreshed.len() as u64));
    assert_eq!(meta.etag.as_deref(), Some("\"v2\""));
    assert!(meta.complete);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn exact_refresh_with_invalid_tail_retains_proven_match_and_complete_cache() -> Result<()> {
    let refreshed = format!("{FIRST}invalid trailing record\n");
    let server = MetadataServer::start(vec![
        Exchange {
            method: "HEAD",
            range: None,
            if_match: None,
            response: format!(
                "HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                refreshed.len()
            ),
        },
        Exchange {
            method: "GET",
            range: None,
            if_match: None,
            response: format!(
                "HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{refreshed}",
                refreshed.len()
            ),
        },
    ])?;
    let cache = Cache::temp()?.init().await?;
    let shard = versions_cache_shard(&cache, &server.url);
    let (content_entry, meta_entry) = versions_cache_entries(&shard);
    let previous_meta = VersionsCacheMeta {
        content_length: SECOND.len() as u64,
        representation_length: Some(SECOND.len() as u64),
        complete: true,
        etag: Some("\"v1\"".to_owned()),
        checked_at: SystemTime::UNIX_EPOCH,
    };
    write_versions_cache(
        &content_entry,
        &meta_entry,
        SECOND.as_bytes(),
        &previous_meta,
    )
    .await?;
    let client = BaseClientBuilder::default().retries(0).build()?;
    let request = PythonDownloadRequest::from_str("cpython-3.13.2-linux-x86_64-gnu")?;

    let download = fetch_ndjson_find_cached(&client, &server.url, &cache, &request)
        .await?
        .context("proven matching first record")?;
    assert_eq!(download.key().version().to_string(), "3.13.2");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("previous complete catalog")?;
    assert_eq!(content, SECOND.as_bytes());
    assert_eq!(meta, previous_meta);
    server.finish()?;
    Ok(())
}

#[tokio::test]
async fn interrupted_exact_refresh_retains_proven_match_and_complete_cache() -> Result<()> {
    let refreshed = format!("{FIRST}{PREPENDED}");
    let partial = &PREPENDED[..PREPENDED.len() / 2];
    let (record_read, received_record) = channel();
    let server = MetadataServer::start_with_interruption(
        vec![
            Exchange {
                method: "HEAD",
                range: None,
                if_match: None,
                response: format!(
                    "HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    refreshed.len()
                ),
            },
            Exchange {
                method: "GET",
                range: None,
                if_match: None,
                response: format!(
                    "HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{FIRST}",
                    refreshed.len()
                ),
            },
        ],
        Some(Interruption {
            exchange: 1,
            record_read: received_record,
            remaining_bytes: partial.to_owned(),
        }),
    )?;
    let cache = Cache::temp()?.init().await?;
    let shard = versions_cache_shard(&cache, &server.url);
    let (content_entry, meta_entry) = versions_cache_entries(&shard);
    let previous_meta = VersionsCacheMeta {
        content_length: SECOND.len() as u64,
        representation_length: Some(SECOND.len() as u64),
        complete: true,
        etag: Some("\"v1\"".to_owned()),
        checked_at: SystemTime::UNIX_EPOCH,
    };
    write_versions_cache(
        &content_entry,
        &meta_entry,
        SECOND.as_bytes(),
        &previous_meta,
    )
    .await?;
    let client = BaseClientBuilder::default().retries(0).build()?;
    let request = PythonDownloadRequest::from_str("cpython-3.13.2-linux-x86_64-gnu")?;

    let download =
        fetch_ndjson_cached_query(&client, &server.url, &cache, false, || ObservedSearch {
            search: DownloadSearch::new(&request),
            record_read: &record_read,
        })
        .await?
        .context("proven matching first record")?;
    assert_eq!(download.key().version().to_string(), "3.13.2");
    let (content, meta) = read_versions_cache_content(&cache, &server.url)
        .await
        .context("previous complete catalog")?;
    assert_eq!(content, SECOND.as_bytes());
    assert_eq!(meta, previous_meta);
    server.finish()?;
    Ok(())
}
