use std::collections::HashSet;
use std::convert::Infallible;
use std::future::ready;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use assert_fs::fixture::{ChildPath, FileWriteStr, PathChild};
use bytes::Bytes;
use http::StatusCode;
use http::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use indoc::formatdoc;
use insta::{allow_duplicates, assert_snapshot};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_stream::wrappers::ReceiverStream;
use wiremock::matchers::{any, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

/// An unchanged strong index hash can identify a previously validated source archive even when
/// its HTTP response is stale. Missing or weak hashes, changed content, and explicit refreshes
/// still require an artifact request.
#[tokio::test]
async fn source_revision_reuses_matching_index_hashes() -> Result<()> {
    for algorithm in [Some("sha256"), Some("md5"), None] {
        let context = uv_test::test_context!("3.12");
        context
            .temp_dir
            .child("requirements.in")
            .write_str("basic-package==0.1.0\n")?;
        let original = fs_err::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../test/links/basic_package-0.1.0.tar.gz"),
        )?;
        let mut changed = original.clone();
        // The gzip timestamp changes the archive digest without changing its source tree.
        changed[4..8].copy_from_slice(&1_u32.to_le_bytes());
        assert_ne!(original, changed);
        let archives = Arc::new([original, changed]);
        let generation = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let server = MockServer::start().await;
        let index_archives = archives.clone();
        let index_generation = generation.clone();
        Mock::given(method("GET"))
            .and(path("/simple/basic-package/"))
            .respond_with(move |_: &Request| {
                let archive = &index_archives[index_generation.load(Ordering::SeqCst)];
                let mut hashes = serde_json::Map::new();
                if let Some(algorithm) = algorithm {
                    let digest = if algorithm == "sha256" {
                        hex::encode(Sha256::digest(archive))
                    } else {
                        let mut hasher =
                            uv_extract::hash::Hasher::from(uv_pypi_types::HashAlgorithm::Md5);
                        hasher.update(archive);
                        uv_pypi_types::HashDigest::from(hasher)
                            .to_string()
                            .strip_prefix("md5:")
                            .unwrap()
                            .to_owned()
                    };
                    hashes.insert(algorithm.to_owned(), json!(digest));
                }
                ResponseTemplate::new(200)
                    .insert_header("Cache-Control", "public, max-age=0")
                    .set_body_raw(
                        json!({
                            "meta": {"api-version": "1.0"},
                            "name": "basic-package",
                            "files": [{
                                "filename": "basic_package-0.1.0.tar.gz",
                                "url": "/files/basic_package-0.1.0.tar.gz",
                                "hashes": hashes,
                                "size": archive.len(),
                                "upload-time": "2023-01-01T00:00:00Z"
                            }]
                        })
                        .to_string(),
                        "application/vnd.pypi.simple.v1+json",
                    )
            })
            .mount(&server)
            .await;
        let artifact_generation = generation.clone();
        let artifact_requests = requests.clone();
        Mock::given(method("GET"))
            .and(path("/files/basic_package-0.1.0.tar.gz"))
            .respond_with(move |_: &Request| {
                artifact_requests.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200)
                    .insert_header("Cache-Control", "public, max-age=0")
                    .set_body_bytes(archives[artifact_generation.load(Ordering::SeqCst)].clone())
            })
            .mount(&server)
            .await;
        let run = async |refresh| -> Result<Vec<u8>> {
            let mut command = context.pip_compile();
            command
                .arg("requirements.in")
                .arg("--python-version")
                .arg("3.13")
                .arg("--default-index")
                .arg(format!("{}/simple", server.uri()))
                .arg("--no-header")
                .arg("--no-annotate");
            if refresh {
                command.arg("--refresh");
            }
            let output = tokio::task::spawn_blocking(move || command.output()).await??;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(output.stdout)
        };
        let first = run(false).await?;
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(run(false).await?, first);
        let reused = usize::from(algorithm == Some("sha256"));
        assert_eq!(requests.load(Ordering::SeqCst), 2 - reused);
        generation.store(1, Ordering::SeqCst);
        assert_eq!(run(false).await?, first);
        assert_eq!(requests.load(Ordering::SeqCst), 3 - reused);
        assert_eq!(run(true).await?, first);
        assert_eq!(requests.load(Ordering::SeqCst), 4 - reused);
    }
    Ok(())
}

/// Complete, hash-verified wheel downloads can supply later resolution metadata. Index hash
/// changes, weak identities, explicit refreshes, and configured file cache policies still fetch
/// metadata from the origin.
#[tokio::test]
async fn resolution_reuses_verified_cached_wheel_metadata() -> Result<()> {
    const FILENAME: &str = "build_tag-1.0.0-1-py2.py3-none-any.whl";
    const METADATA: &str = "Metadata-Version: 2.3\nName: build-tag\nVersion: 1.0.0\n";
    for (algorithm, override_cache) in [
        (Some("sha256"), false),
        (Some("md5"), false),
        (None, false),
        (Some("sha256"), true),
    ] {
        let context = uv_test::test_context!("3.12");
        context
            .temp_dir
            .child("requirements.in")
            .write_str("build-tag==1.0.0\n")?;
        let original = fs_err::read(context.workspace_root.join("test/links").join(FILENAME))?;
        let mut changed = original.clone();
        let end = changed.len();
        assert_eq!(&changed[end - 22..end - 18], b"PK\x05\x06");
        assert_eq!(&changed[end - 2..], &[0, 0]);
        changed[end - 2..].copy_from_slice(&1_u16.to_le_bytes());
        changed.push(b'x');
        let archives = Arc::new([original, changed]);
        let generation = Arc::new(AtomicUsize::new(0));
        let artifact_requests = Arc::new(AtomicUsize::new(0));
        let server = MockServer::start().await;
        let index_archives = archives.clone();
        let index_generation = generation.clone();
        Mock::given(method("GET"))
            .and(path("/simple/build-tag/"))
            .respond_with(move |_: &Request| {
                let archive = &index_archives[index_generation.load(Ordering::SeqCst)];
                let mut hashes = serde_json::Map::new();
                if let Some(algorithm) = algorithm {
                    let digest = if algorithm == "sha256" {
                        hex::encode(Sha256::digest(archive))
                    } else {
                        let mut hasher =
                            uv_extract::hash::Hasher::from(uv_pypi_types::HashAlgorithm::Md5);
                        hasher.update(archive);
                        uv_pypi_types::HashDigest::from(hasher)
                            .to_string()
                            .strip_prefix("md5:")
                            .unwrap()
                            .to_owned()
                    };
                    hashes.insert(algorithm.to_owned(), json!(digest));
                }
                ResponseTemplate::new(200)
                    .insert_header("Cache-Control", "public, max-age=0")
                    .set_body_raw(
                        json!({
                            "meta": {"api-version": "1.0"},
                            "name": "build-tag",
                            "files": [{
                                "filename": FILENAME,
                                "url": format!("/files/{FILENAME}"),
                                "hashes": hashes,
                                "size": archive.len(),
                                "core-metadata": true,
                                "upload-time": "2023-01-01T00:00:00Z"
                            }]
                        })
                        .to_string(),
                        "application/vnd.pypi.simple.v1+json",
                    )
            })
            .mount(&server)
            .await;
        let download_requests = artifact_requests.clone();
        let download_generation = generation.clone();
        Mock::given(method("GET"))
            .and(path(format!("/files/{FILENAME}")))
            .respond_with(move |_: &Request| {
                download_requests.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200)
                    .insert_header("Cache-Control", "public, max-age=0")
                    .set_body_bytes(archives[download_generation.load(Ordering::SeqCst)].clone())
            })
            .mount(&server)
            .await;
        let metadata_requests = artifact_requests.clone();
        Mock::given(method("GET"))
            .and(path(format!("/files/{FILENAME}.metadata")))
            .respond_with(move |_: &Request| {
                metadata_requests.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200)
                    .insert_header("Cache-Control", "public, max-age=0")
                    .set_body_string(METADATA)
            })
            .mount(&server)
            .await;
        let index = format!("{}/simple", server.uri());
        let config = context.temp_dir.child("cache-control.toml");
        config.write_str(&formatdoc! {
            r#"
            [[index]]
            url = "{index}"
            default = true
            cache-control = {{ files = "max-age=0" }}
            "#
        })?;
        let configure = |command: &mut std::process::Command| {
            if override_cache {
                command.arg("--config-file").arg(config.path());
            } else {
                command.arg("--default-index").arg(&index);
            }
        };
        let mut install = context.pip_install();
        install.arg("--no-deps").arg("build-tag==1.0.0");
        configure(&mut install);
        let output = tokio::task::spawn_blocking(move || install.output()).await??;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(artifact_requests.load(Ordering::SeqCst), 1);
        let run = async |refresh| -> Result<Vec<u8>> {
            let mut command = context.pip_compile();
            command
                .arg("requirements.in")
                .arg("--no-header")
                .arg("--no-annotate");
            configure(&mut command);
            if refresh {
                command.arg("--refresh");
            }
            let output = tokio::task::spawn_blocking(move || command.output()).await??;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(output.stdout)
        };
        let expected_reuse = algorithm == Some("sha256") && !override_cache;
        let first = run(false).await?;
        assert_eq!(
            artifact_requests.load(Ordering::SeqCst),
            if expected_reuse { 1 } else { 2 }
        );
        let before = artifact_requests.load(Ordering::SeqCst);
        generation.store(1, Ordering::SeqCst);
        assert_eq!(run(false).await?, first);
        assert!(artifact_requests.load(Ordering::SeqCst) > before);
        let before = artifact_requests.load(Ordering::SeqCst);
        assert_eq!(run(true).await?, first);
        assert!(artifact_requests.load(Ordering::SeqCst) > before);
    }
    Ok(())
}

/// Creates a CONNECT tunnel proxy that forwards connections to the target.
///
/// Returns the proxy address. The proxy runs in a background thread.
fn start_connect_tunnel_proxy() -> std::net::SocketAddr {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn a real OS thread for the proxy server
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut client) = stream else { break };

            // Handle each connection in its own thread
            std::thread::spawn(move || {
                // Read the CONNECT request
                let mut buf = vec![0u8; 4096];
                let mut total_read = 0;
                loop {
                    let n = match client.read(&mut buf[total_read..]) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    total_read += n;
                    if buf[..total_read]
                        .array_windows()
                        .any(|window| window == b"\r\n\r\n")
                    {
                        break;
                    }
                }

                let request = String::from_utf8_lossy(&buf[..total_read]);

                // Parse "CONNECT host:port HTTP/1.1\r\n"
                let Some(target_addr) = request
                    .lines()
                    .next()
                    .and_then(|line| line.strip_prefix("CONNECT "))
                    .and_then(|s| s.split_whitespace().next())
                    .map(ToString::to_string)
                else {
                    return;
                };

                // Connect to the target
                let Ok(mut target) = TcpStream::connect(&target_addr) else {
                    return;
                };

                // Send 200 Connection Established
                if client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .is_err()
                {
                    return;
                }

                // Bidirectionally forward data using two threads
                let mut client_read = client.try_clone().unwrap();
                let mut target_write = target.try_clone().unwrap();

                let c2t =
                    std::thread::spawn(move || std::io::copy(&mut client_read, &mut target_write));

                let _ = std::io::copy(&mut target, &mut client);
                let _ = c2t.join();
            });
        }
    });

    addr
}

/// Creates a mock that serves a Simple API index page for iniconfig.
async fn mock_simple_api(server: &MockServer) {
    // Simple API response for iniconfig pointing to the real PyPI wheel.
    // Uses upload-time before EXCLUDE_NEWER (2024-03-25) so the package is available.
    let body = json!({
        "name": "iniconfig",
        "files": [{
            "filename": "iniconfig-2.0.0-py3-none-any.whl",
            "url": "https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl",
            "hashes": {
                "sha256": "b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374"
            },
            "requires-python": ">=3.8",
            "upload-time": "2024-01-01T00:00:00Z"
        }]
    });

    // Serve the simple index for iniconfig - use any() matcher since HTTP proxy
    // requests may have the full URL in the path
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(body.to_string(), "application/vnd.pypi.simple.v1+json"),
        )
        .mount(server)
        .await;
}

fn connection_reset(_request: &wiremock::Request) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, "Connection reset by peer")
}

/// Returns true if the mock server has received any requests.
async fn has_received_requests(server: &MockServer) -> bool {
    !server.received_requests().await.unwrap().is_empty()
}

/// Answers with a retryable HTTP status 500.
async fn http_error_server() -> (MockServer, String) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(StatusCode::INTERNAL_SERVER_ERROR))
        .mount(&server)
        .await;

    let mock_server_uri = server.uri();
    (server, mock_server_uri)
}

/// Answers with a retryable connection reset IO error.
async fn io_error_server() -> (MockServer, String) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with_err(connection_reset)
        .mount(&server)
        .await;

    let mock_server_uri = server.uri();
    (server, mock_server_uri)
}

/// Answers with a retryable HTTP status 500 for 2 times, then with a retryable connection reset
/// IO error.
///
/// Tests different errors paths inside uv, which retries 3 times by default, for a total for 4
/// requests.
async fn mixed_error_server() -> (MockServer, String) {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with_err(connection_reset)
        .up_to_n_times(2)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(StatusCode::INTERNAL_SERVER_ERROR))
        .up_to_n_times(2)
        .mount(&server)
        .await;

    let mock_server_uri = server.uri();
    (server, mock_server_uri)
}

type StreamingResponse = hyper::Response<BoxBody<Bytes, Infallible>>;

/// Emit some bytes, then wait before ending the response body.
fn delayed_body(bytes: Bytes, delay: Duration) -> BoxBody<Bytes, Infallible> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Frame::data(bytes))).await;
        tokio::time::sleep(delay).await;
    });
    StreamBody::new(ReceiverStream::new(rx)).boxed()
}

fn time_out_response(
    _request: hyper::Request<hyper::body::Incoming>,
) -> Result<StreamingResponse, http::Error> {
    hyper::Response::builder()
        .header("Content-Type", "text/html")
        .body(delayed_body(Bytes::new(), Duration::from_mins(1)))
}

/// Run a streaming HTTP server on its own runtime so test subprocesses cannot starve it.
/// Dropping the guard shuts down the runtime and all connection tasks.
fn streaming_server<F>(handler: F) -> (String, impl Drop)
where
    F: Fn(hyper::Request<hyper::body::Incoming>) -> Result<StreamingResponse, http::Error>
        + Send
        + Sync
        + 'static,
{
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let handler = Arc::new(handler);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            tokio::select! {
                () = async {
                    while let Ok((stream, _)) = listener.accept().await {
                        let handler = handler.clone();
                        tokio::spawn(async move {
                            let _ = hyper_util::server::conn::auto::Builder::new(
                                hyper_util::rt::TokioExecutor::new(),
                            )
                            .serve_connection(TokioIo::new(stream), service_fn(move |request| {
                                ready(handler(request))
                            }))
                            .await;
                        });
                    }
                } => {}
                _ = shutdown_rx => {}
            }
        });
    });
    (server, shutdown_tx)
}

/// Emit a response body only after the gate opens.
fn gated_body(
    bytes: Bytes,
    mut gate: tokio::sync::watch::Receiver<bool>,
) -> BoxBody<Bytes, Infallible> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        while !*gate.borrow_and_update() {
            if gate.changed().await.is_err() {
                return;
            }
        }
        let _ = tx.send(Ok(Frame::data(bytes))).await;
    });
    StreamBody::new(ReceiverStream::new(rx)).boxed()
}

/// A completed resolution does not wait for unused batch-prefetch response bodies.
#[test]
fn resolver_stops_unused_prefetch() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (started_tx, started_rx) = tokio::sync::watch::channel(false);
    let (release_tx, release_rx) = tokio::sync::watch::channel(false);
    let observed_started = started_rx.clone();
    let (server, _guard) = streaming_server(move |request| {
        let path = request.uri().path();
        let (data, content_type, gate) = if let Some(name) = path
            .strip_prefix("/simple/")
            .and_then(|path| path.strip_suffix('/'))
        {
            let maximum = if name == "choice" { 30 } else { 1 };
            let files = (1..=maximum)
                .map(|version| {
                    json!({
                        "filename": format!("{name}-{version}.0-py3-none-any.whl"),
                        "url": format!("/files/{name}-{version}.0-py3-none-any.whl"),
                        "hashes": {},
                        "core-metadata": true,
                        "upload-time": "2024-01-01T00:00:00Z",
                    })
                })
                .collect::<Vec<_>>();
            (
                json!({"name": name, "files": files}).to_string(),
                "application/vnd.pypi.simple.v1+json",
                None,
            )
        } else if let Some(filename) = path
            .strip_prefix("/files/")
            .and_then(|path| path.strip_suffix("-py3-none-any.whl.metadata"))
        {
            let (name, version) = filename.split_once('-').expect("fixture filename");
            let number = version
                .strip_suffix(".0")
                .expect("fixture version")
                .parse::<u32>()
                .expect("numeric fixture version");
            let mut metadata = format!("Metadata-Version: 2.3\nName: {name}\nVersion: {version}\n");
            let gate = if name == "choice" {
                metadata.push_str(if number > 15 {
                    "Requires-Dist: pin==2.0\n"
                } else {
                    "Requires-Dist: pin==1.0\n"
                });
                match number {
                    0..=14 => {
                        started_tx.send_replace(true);
                        Some(release_rx.clone())
                    }
                    15 => {
                        // Make the usable version wait until an unused prefetch is in flight.
                        Some(started_rx.clone())
                    }
                    _ => None,
                }
            } else {
                None
            };
            (metadata, "text/plain", gate)
        } else {
            return hyper::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(http_body_util::Full::new(Bytes::new()).boxed());
        };
        let length = data.len();
        let body = if let Some(gate) = gate {
            gated_body(Bytes::from(data), gate)
        } else {
            http_body_util::Full::new(Bytes::from(data)).boxed()
        };
        hyper::Response::builder()
            .header("Content-Type", content_type)
            .header(CONTENT_LENGTH, length)
            .body(body)
    });
    context
        .temp_dir
        .child("requirements.in")
        .write_str("choice\npin==1.0\n")?;
    let mut child = context
        .pip_compile()
        .arg("--no-header")
        .arg("--no-annotate")
        .arg("--default-index")
        .arg(format!("{server}/simple"))
        .arg("requirements.in")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let completed = loop {
        if child.try_wait()?.is_some() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    release_tx.send_replace(true);
    if !completed {
        child.kill()?;
    }
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(*observed_started.borrow(), "no unused prefetch started");
    assert!(completed, "resolution waited for unused prefetches");
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "choice==15.0\npin==1.0\n"
    );
    Ok(())
}

/// Independent OSV batches share the download limit and refill a completed request slot.
#[tokio::test]
async fn audit_batches_refill_download_slots() -> Result<()> {
    let last_started = Arc::new(tokio::sync::Notify::new());
    let later_release = Arc::new(tokio::sync::Semaphore::new(0));
    let response_started = last_started.clone();
    let response_release = later_release.clone();
    let (started, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let (server, _guard) = streaming_server(move |request| {
        let last_started = response_started.clone();
        let later_release = response_release.clone();
        let started = started.clone();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            let body = request
                .into_body()
                .collect()
                .await
                .expect("request body")
                .to_bytes();
            let body: serde_json::Value = serde_json::from_slice(&body).expect("request JSON");
            let queries = body["queries"].as_array().expect("query array");
            let first = queries[0]["package"]["name"]
                .as_str()
                .expect("package name");
            let _ = started.send(first.to_owned());
            match first {
                "package-0" => last_started.notified().await,
                "package-1000" => later_release
                    .acquire()
                    .await
                    .expect("response gate")
                    .forget(),
                "package-2000" => last_started.notify_one(),
                _ => {}
            }
            let response = json!({"results": vec![json!({"vulns": []}); queries.len()]});
            let _ = sender
                .send(Ok(Frame::data(Bytes::from(response.to_string()))))
                .await;
        });
        hyper::Response::builder()
            .header("Content-Type", "application/json")
            .body(StreamBody::new(ReceiverStream::new(receiver)).boxed())
    });
    let concurrency = uv_configuration::Concurrency::new(2, 1, 1, 1);
    let reserved = concurrency
        .downloads_semaphore
        .clone()
        .acquire_owned()
        .await?;
    let service = uv_audit::osv::Osv::new(
        uv_client::CachedClient::new(uv_client::BaseClientBuilder::default().retries(0).build()?),
        Some(server.parse()?),
        concurrency,
        uv_cache::Cache::temp()?,
    );
    let query = tokio::spawn(async move {
        let dependencies = (0..=2000)
            .map(|index| {
                Ok(uv_audit::Dependency::new(
                    format!("package-{index}").parse()?,
                    "1.0".parse()?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let identifiers = service
            .query_identifiers(&dependencies, uv_audit::osv::Filter::All)
            .await?;
        assert_eq!(
            identifiers.keys().copied().collect::<Vec<_>>(),
            dependencies.iter().collect::<Vec<_>>()
        );
        assert!(identifiers.values().all(HashSet::is_empty));
        Ok::<_, anyhow::Error>(())
    });
    let first = tokio::time::timeout(Duration::from_secs(3), requests.recv())
        .await?
        .expect("first batch");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), requests.recv())
            .await
            .is_err()
    );
    drop(reserved);
    let second = tokio::time::timeout(Duration::from_secs(3), requests.recv())
        .await?
        .expect("second batch");
    let mut initial = [first, second];
    initial.sort();
    assert_eq!(initial, ["package-0", "package-1000"]);
    later_release.add_permits(1);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), requests.recv())
            .await?
            .as_deref(),
        Some("package-2000")
    );
    tokio::time::timeout(Duration::from_secs(3), query).await???;
    Ok(())
}

async fn check_source_prefetch(advertised: bool, pinned: bool) -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let choice_requested = Arc::new(AtomicBool::new(false));
    let selected = if pinned { 2 } else { 1 };
    let gate_metadata = format!(
        "Metadata-Version: 2.3\nName: gate\nVersion: 1.0\nRequires-Dist: choice=={selected}.0\n"
    );
    Mock::given(method("GET"))
        .and(wiremock::matchers::path("/simple/gate/"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                json!({
                    "name": "gate",
                    "files": [{
                        "filename": "gate-1.0-py3-none-any.whl",
                        "url": "/files/gate-1.0-py3-none-any.whl",
                        "hashes": {},
                        "core-metadata": true,
                        "upload-time": "2024-01-01T00:00:00Z",
                    }],
                })
                .to_string(),
                "application/vnd.pypi.simple.v1+json",
            ),
        )
        .mount(&server)
        .await;
    let requested = choice_requested.clone();
    Mock::given(method("GET"))
        .and(wiremock::matchers::path(
            "/files/gate-1.0-py3-none-any.whl.metadata",
        ))
        .respond_with(move |_: &Request| {
            // The exact source must start downloading while the gate is unresolved.
            if pinned && !requested.load(Ordering::Relaxed) {
                ResponseTemplate::new(503)
            } else {
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(300))
                    .set_body_raw(gate_metadata.clone(), "text/plain")
            }
        })
        .mount(&server)
        .await;
    let mut files = Vec::new();
    for version in [1, 2] {
        let filename = format!("choice-{version}.0.tar.gz");
        let metadata = format!("Metadata-Version: 2.3\nName: choice\nVersion: {version}.0\n");
        let pyproject = format!("[project]\nname = \"choice\"\nversion = \"{version}.0\"\n");
        let mut archive = Vec::new();
        uv_test::archive::write_tar_gz(
            &mut archive,
            &[
                (
                    format!("choice-{version}.0/PKG-INFO").as_str(),
                    metadata.as_bytes(),
                ),
                (
                    format!("choice-{version}.0/pyproject.toml").as_str(),
                    pyproject.as_bytes(),
                ),
            ],
        )?;
        files.push(json!({
            "filename": filename,
            "url": format!("/files/{filename}"),
            "hashes": {"sha256": hex::encode(Sha256::digest(&archive))},
            "size": archive.len(),
            "core-metadata": advertised,
            "upload-time": "2024-01-01T00:00:00Z",
        }));
        let requested = choice_requested.clone();
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(format!("/files/{filename}")))
            .respond_with(move |_: &Request| {
                if version == 2 {
                    requested.store(true, Ordering::Relaxed);
                }
                ResponseTemplate::new(200).set_body_bytes(archive.clone())
            })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(format!(
                "/files/{filename}.metadata"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_raw(metadata, "text/plain"))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(wiremock::matchers::path("/simple/choice/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            json!({"name": "choice", "files": files}).to_string(),
            "application/vnd.pypi.simple.v1+json",
        ))
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("requirements.in")
        .write_str(if pinned {
            "gate==1.0\nchoice==2.0\n"
        } else {
            "gate==1.0\nchoice\n"
        })?;
    let output = context
        .pip_compile()
        .arg("--no-header")
        .arg("--no-annotate")
        .arg("--default-index")
        .arg(format!("{}/simple", server.uri()))
        .arg("requirements.in")
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)?,
        format!("choice=={selected}.0\ngate==1.0\n")
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .any(|request| request.url.path() == "/files/choice-2.0.tar.gz"),
        pinned
    );
    assert_eq!(
        requests
            .iter()
            .any(|request| request.url.path() == "/files/choice-2.0.tar.gz.metadata"),
        advertised
    );
    let selected_path = format!(
        "/files/choice-{selected}.0.tar.gz{}",
        if advertised { ".metadata" } else { "" }
    );
    assert!(
        requests
            .iter()
            .any(|request| request.url.path() == selected_path)
    );
    Ok(())
}

/// The resolver only downloads a source archive once it selects that version.
#[tokio::test]
async fn resolver_does_not_prefetch_source_archives() -> Result<()> {
    check_source_prefetch(false, false).await
}

/// Advertised static source metadata can still be prefetched cheaply.
#[tokio::test]
async fn resolver_prefetches_source_sidecars() -> Result<()> {
    check_source_prefetch(true, false).await
}

/// An exact source requirement can start downloading before other metadata resolves.
#[tokio::test]
async fn resolver_prefetches_pinned_source_archives() -> Result<()> {
    check_source_prefetch(false, true).await
}

/// Invalid explicit certificate files disable the default trust roots rather than being ignored.
#[tokio::test]
async fn invalid_ssl_cert_file_warns_default_roots_are_disabled() {
    let context = uv_test::test_context!("3.12");
    let (_server_drop_guard, mock_server_uri) = http_error_server().await;

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--index-url")
        .arg(&mock_server_uri)
        .env(EnvVars::SSL_CERT_FILE, context.temp_dir.join("missing.pem"))
        .env_remove(EnvVars::SSL_CERT_DIR)
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Invalid `SSL_CERT_FILE`. Path does not exist: [TEMP_DIR]/missing.pem. No default certificates will be trusted.
    error: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/tqdm/`
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/tqdm/)
    ");
}

/// Invalid explicit certificate directories disable the default trust roots rather than being ignored.
#[tokio::test]
async fn invalid_ssl_cert_dir_warns_default_roots_are_disabled() {
    let context = uv_test::test_context!("3.12");
    let (_server_drop_guard, mock_server_uri) = http_error_server().await;

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--index-url")
        .arg(&mock_server_uri)
        .env_remove(EnvVars::SSL_CERT_FILE)
        .env(EnvVars::SSL_CERT_DIR, context.temp_dir.join("missing-certs"))
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Invalid `SSL_CERT_DIR`. The directory does not exist: [TEMP_DIR]/missing-certs. No default certificates will be trusted.
    error: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/tqdm/`
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/tqdm/)
    ");
}

/// Check the simple index error message when the server returns HTTP status 500, a retryable error.
#[tokio::test]
async fn simple_http_500() {
    let context = uv_test::test_context!("3.12");

    let (_server_drop_guard, mock_server_uri) = http_error_server().await;

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--index-url")
        .arg(&mock_server_uri)
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/tqdm/`
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/tqdm/)
    ");
}

/// Check the simple index error message when the server returns a retryable IO error.
#[tokio::test]
async fn simple_io_err() {
    let context = uv_test::test_context!("3.12");

    let (_server_drop_guard, mock_server_uri) = io_error_server().await;

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--index-url")
        .arg(&mock_server_uri)
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/tqdm/`
      cause: error sending request for url (http://[LOCALHOST]/tqdm/)
      cause: client error (SendRequest)
      cause: connection closed before message completed
    ");
}

/// Check the find links error message when the server returns HTTP status 500, a retryable error.
#[tokio::test]
async fn find_links_http_500() {
    let context = uv_test::test_context!("3.12");

    let (_server_drop_guard, mock_server_uri) = http_error_server().await;

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&mock_server_uri)
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to read `--find-links` URL: http://[LOCALHOST]/
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/`
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/)
    ");
}

/// Check the find links error message when the server returns a retryable IO error.
#[tokio::test]
async fn find_links_io_error() {
    let context = uv_test::test_context!("3.12");

    let (_server_drop_guard, mock_server_uri) = io_error_server().await;

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&mock_server_uri)
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to read `--find-links` URL: http://[LOCALHOST]/
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/`
      cause: error sending request for url (http://[LOCALHOST]/)
      cause: client error (SendRequest)
      cause: connection closed before message completed
    ");
}

/// Check the error message for a find links index page, a non-streaming request, when the server
/// returns different kinds of retryable errors.
#[tokio::test]
async fn find_links_mixed_error() {
    let context = uv_test::test_context!("3.12");

    let (_server_drop_guard, mock_server_uri) = mixed_error_server().await;

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&mock_server_uri)
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to read `--find-links` URL: http://[LOCALHOST]/
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/`
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/)
    ");
}

/// Check that a missing direct package URL is classified as a user error.
#[tokio::test]
async fn direct_url_http_404() {
    let context = uv_test::test_context!("3.12");

    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(StatusCode::NOT_FOUND))
        .mount(&server)
        .await;

    let tqdm_url = format!("{}/tqdm-4.67.1-py3-none-any.whl", server.uri());
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("tqdm @ {tqdm_url}")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl`
      cause: Failed to fetch: `http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl`
      cause: HTTP status client error (404 Not Found) for url (http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl)
    ");

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("tqdm @ {tqdm_url}"))
        .arg("--quiet"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl`
      cause: Failed to fetch: `http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl`
      cause: HTTP status client error (404 Not Found) for url (http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl)
    ");

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("tqdm @ {tqdm_url}"))
        .arg("--quiet")
        .arg("--quiet"), @"
    exit_code: 1 (failure)
    ");
}

/// Check the direct package URL error message when the server returns HTTP status 500, a retryable
/// error.
#[tokio::test]
async fn direct_url_http_500() {
    let context = uv_test::test_context!("3.12");

    let (_server_drop_guard, mock_server_uri) = http_error_server().await;

    let tqdm_url = format!(
        "{mock_server_uri}/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl"
    );
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("tqdm @ {tqdm_url}"))
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl`
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl`
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl)
    ");
}

/// Check the direct package URL error message when the server returns a retryable IO error.
#[tokio::test]
async fn direct_url_io_error() {
    let context = uv_test::test_context!("3.12");

    let (_server_drop_guard, mock_server_uri) = io_error_server().await;

    let tqdm_url = format!(
        "{mock_server_uri}/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl"
    );
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("tqdm @ {tqdm_url}"))
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl`
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl`
      cause: error sending request for url (http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl)
      cause: client error (SendRequest)
      cause: connection closed before message completed
    ");
}

/// Check the error message for direct package URL, a streaming request, when the server returns
/// different kinds of retryable errors.
#[tokio::test]
async fn direct_url_mixed_error() {
    let context = uv_test::test_context!("3.12");

    let (_server_drop_guard, mock_server_uri) = mixed_error_server().await;

    let tqdm_url = format!(
        "{mock_server_uri}/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl"
    );
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("tqdm @ {tqdm_url}"))
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl`
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl`
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl)
    ");
}

fn write_python_downloads_json(context: &TestContext, mock_server_uri: &String) -> ChildPath {
    let python_downloads_json = context.temp_dir.child("python_downloads.json");
    let interpreter = json!({
        "cpython-3.10.0-darwin-aarch64-none": {
            "arch": {
                "family": "aarch64",
                "variant": null
            },
            "libc": "none",
            "major": 3,
            "minor": 10,
            "name": "cpython",
            "os": "darwin",
            "patch": 0,
            "prerelease": "",
            "sha256": null,
            "url": format!("{mock_server_uri}/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-aarch64-apple-darwin-pgo%2Blto-20211017T1616.tar.zst"),
            "variant": null
        }
    });
    python_downloads_json
        .write_str(&serde_json::to_string(&interpreter).unwrap())
        .unwrap();
    python_downloads_json
}

/// Check the Python install error message when the server returns HTTP status 500, a retryable
/// error.
#[tokio::test]
async fn python_install_http_500() {
    let context = uv_test::test_context!("3.12")
        .without_python_download_cache()
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs();

    let (_server_drop_guard, mock_server_uri) = http_error_server().await;

    let python_downloads_json = write_python_downloads_json(&context, &mock_server_uri);

    uv_snapshot!(context.filters(), context
        .python_install()
        .arg("cpython-3.10.0-darwin-aarch64-none")
        .arg("--python-downloads-json-url")
        .arg(python_downloads_json.path())
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to install cpython-3.10.0-[PLATFORM]
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to download http://[LOCALHOST]/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-[PLATFORM]-pgo%2Blto-20211017T1616.tar.zst
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-[PLATFORM]-pgo%2Blto-20211017T1616.tar.zst)
    ");
}

/// Check the Python install error message when the server returns a retryable IO error.
#[tokio::test]
async fn python_install_io_error() {
    let context = uv_test::test_context!("3.12")
        .without_python_download_cache()
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs();

    let (_server_drop_guard, mock_server_uri) = io_error_server().await;

    let python_downloads_json = write_python_downloads_json(&context, &mock_server_uri);

    uv_snapshot!(context.filters(), context
        .python_install()
        .arg("cpython-3.10.0-darwin-aarch64-none")
        .arg("--python-downloads-json-url")
        .arg(python_downloads_json.path())
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to install cpython-3.10.0-[PLATFORM]
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to download http://[LOCALHOST]/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-[PLATFORM]-pgo%2Blto-20211017T1616.tar.zst
      cause: error sending request for url (http://[LOCALHOST]/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-[PLATFORM]-pgo%2Blto-20211017T1616.tar.zst)
      cause: client error (SendRequest)
      cause: connection closed before message completed
    ");
}

#[tokio::test]
async fn install_http_retries() {
    let context = uv_test::test_context!("3.12");

    let server = MockServer::start().await;

    // Create a server that always fails, so we can see the number of retries used
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .expect(6)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("anyio")
        .arg("--index")
        .arg(server.uri())
        .env(EnvVars::UV_HTTP_RETRIES, "foo"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse environment variable `UV_HTTP_RETRIES` with invalid value `foo`: invalid digit found in string
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("anyio")
        .arg("--index")
        .arg(server.uri())
        .env(EnvVars::UV_HTTP_RETRIES, "-1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse environment variable `UV_HTTP_RETRIES` with invalid value `-1`: invalid digit found in string
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("anyio")
        .arg("--index")
        .arg(server.uri())
        .env(EnvVars::UV_HTTP_RETRIES, "999999999999"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse environment variable `UV_HTTP_RETRIES` with invalid value `999999999999`: number too large to fit in target type
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("anyio")
        .arg("--index")
        .arg(server.uri())
        .env(EnvVars::UV_HTTP_RETRIES, "5")
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Request failed after 5 retries in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/anyio/`
      cause: HTTP status server error (503 Service Unavailable) for url (http://[LOCALHOST]/anyio/)
    "
    );
}

#[tokio::test]
async fn install_http_retry_low_level() {
    let context = uv_test::test_context!("3.12");

    let server = MockServer::start().await;

    // Create a server that fails with a more fundamental error so we trigger
    // earlier error paths
    Mock::given(method("GET"))
        .respond_with_err(|_: &'_ Request| io::Error::new(io::ErrorKind::ConnectionReset, "error"))
        .expect(2)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("anyio")
        .arg("--index")
        .arg(server.uri())
        .env(EnvVars::UV_HTTP_RETRIES, "1")
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Request failed after 1 retry in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/anyio/`
      cause: error sending request for url (http://[LOCALHOST]/anyio/)
      cause: client error (SendRequest)
      cause: connection closed before message completed
    "
    );
}

/// Test problem details with a 403 error containing license compliance information
#[tokio::test]
async fn rfc9457_problem_details_license_violation() {
    let context = uv_test::test_context!("3.12");

    let server = MockServer::start().await;

    let problem_json = r#"{
        "type": "https://example.com/probs/license-violation",
        "title": "License Compliance Issue",
        "status": 403,
        "detail": "This package version has a license that violates organizational policy."
    }"#;

    // Mock HEAD request to return 200 OK
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(StatusCode::OK))
        .mount(&server)
        .await;

    // Mock GET request to return 403 with problem details
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(StatusCode::FORBIDDEN)
                .set_body_raw(problem_json, "application/problem+json"),
        )
        .mount(&server)
        .await;

    let mock_server_uri = server.uri();
    let tqdm_url = format!("{mock_server_uri}/packages/tqdm-4.67.1-py3-none-any.whl");

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("tqdm @ {tqdm_url}")), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ http://[LOCALHOST]/packages/tqdm-4.67.1-py3-none-any.whl`
      cause: Failed to fetch: `http://[LOCALHOST]/packages/tqdm-4.67.1-py3-none-any.whl`
      cause: Server message: License Compliance Issue, This package version has a license that violates organizational policy.
      cause: HTTP status client error (403 Forbidden) for url (http://[LOCALHOST]/packages/tqdm-4.67.1-py3-none-any.whl)
    ");
}

/// Test that invalid proxy URL in uv.toml produces a helpful error message.
#[tokio::test]
async fn proxy_invalid_url_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml
        .write_str(indoc::indoc! {r#"
            http-proxy = "ftp://proxy.example.com:8080"
        "#})
        .unwrap();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("iniconfig")
        .env_remove(EnvVars::HTTP_PROXY)
        .env_remove(EnvVars::HTTPS_PROXY), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: `uv.toml`
      cause: TOML parse error at line 1, column 14
               |
             1 | http-proxy = "ftp://proxy.example.com:8080"
               |              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
             invalid proxy URL scheme `ftp` in `ftp://proxy.example.com:8080/`: expected http, https, socks5, or socks5h
    "#);
}

/// Test that invalid proxy URL (not a URL) in uv.toml produces a helpful error message.
#[tokio::test]
async fn proxy_invalid_url_not_a_url_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml
        .write_str(indoc::indoc! {r#"
            http-proxy = "not a valid url"
        "#})
        .unwrap();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("iniconfig")
        .env_remove(EnvVars::HTTP_PROXY)
        .env_remove(EnvVars::HTTPS_PROXY), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: `uv.toml`
      cause: TOML parse error at line 1, column 14
               |
             1 | http-proxy = "not a valid url"
               |              ^^^^^^^^^^^^^^^^^
             invalid proxy URL: invalid international domain name
    "#);
}

/// Test that a SOCKS proxy URL without a host produces a configuration error.
#[test]
fn proxy_url_without_host() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("proxy.toml")
        .write_str("http-proxy = \"socks5:foo\"\n")?;
    context.temp_dir.child("requirements.in").write_str("")?;

    uv_snapshot!(context.filters(), context
        .pip_compile()
        .arg("requirements.in")
        .arg("--offline")
        .arg("--config-file")
        .arg("proxy.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: `proxy.toml`
      cause: TOML parse error at line 1, column 14
               |
             1 | http-proxy = "socks5:foo"
               |              ^^^^^^^^^^^^
             invalid proxy URL: empty host
    "#);

    Ok(())
}

/// Test that valid proxy URL in uv.toml routes requests through the proxy.
#[cfg(feature = "test-pypi")]
#[tokio::test]
async fn proxy_valid_url_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let target_server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&target_server)
        .await;

    let proxy_server = MockServer::start().await;
    mock_simple_api(&proxy_server).await;

    let target_uri = target_server.uri();
    let proxy_uri = proxy_server.uri();

    let context = context
        .with_filter((target_uri.clone(), "[TARGET]"))
        .with_filter((proxy_uri.clone(), "[PROXY]"));

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml
        .write_str(&format!(r#"http-proxy = "{proxy_uri}""#))
        .unwrap();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("iniconfig")
        .arg("--index-url")
        .arg(&target_uri)
        .arg("--config-file")
        .arg(uv_toml.path())
        .env_remove(EnvVars::HTTP_PROXY)
        .env_remove(EnvVars::HTTPS_PROXY)
        .env_remove(EnvVars::ALL_PROXY)
        .env_remove(EnvVars::NO_PROXY), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    assert!(
        has_received_requests(&proxy_server).await,
        "Proxy should have received the request"
    );
    assert!(
        !has_received_requests(&target_server).await,
        "Target should NOT have been called directly when proxy is configured"
    );
}

/// Test that https-proxy in uv.toml routes HTTPS requests through a CONNECT tunnel proxy.
#[cfg(feature = "test-pypi")]
#[test]
fn proxy_https_proxy_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let proxy_addr = start_connect_tunnel_proxy();
    let proxy_uri = format!("http://{proxy_addr}");

    let context = context.with_filter((proxy_uri.clone(), "[PROXY]"));

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml
        .write_str(&format!(r#"https-proxy = "{proxy_uri}""#))
        .unwrap();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("--config-file")
        .arg(uv_toml.path())
        .arg("iniconfig")
        .env_remove(EnvVars::HTTP_PROXY)
        .env_remove(EnvVars::HTTPS_PROXY)
        .env_remove(EnvVars::ALL_PROXY)
        .env_remove(EnvVars::NO_PROXY), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");
}

/// Test that no-proxy in uv.toml bypasses the proxy for specified hosts.
#[cfg(feature = "test-pypi")]
#[tokio::test]
async fn proxy_no_proxy_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let target_server = MockServer::start().await;
    mock_simple_api(&target_server).await;

    let proxy_server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&proxy_server)
        .await;

    let target_uri = target_server.uri();
    let proxy_uri = proxy_server.uri();

    // Note: reqwest's NoProxy matches on host only, not host:port
    let target_url = url::Url::parse(&target_uri).unwrap();
    let target_host = target_url.host_str().unwrap();

    let context = context
        .with_filter((target_uri.clone(), "[TARGET]"))
        .with_filter((proxy_uri.clone(), "[PROXY]"));

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml
        .write_str(&format!(
            r#"
http-proxy = "{proxy_uri}"
no-proxy = ["{target_host}"]
"#
        ))
        .unwrap();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("iniconfig")
        .arg("--index-url")
        .arg(&target_uri)
        .arg("--config-file")
        .arg(uv_toml.path())
        .env_remove(EnvVars::HTTP_PROXY)
        .env_remove(EnvVars::HTTPS_PROXY)
        .env_remove(EnvVars::ALL_PROXY)
        .env_remove(EnvVars::NO_PROXY), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    assert!(
        has_received_requests(&target_server).await,
        "Target should have received the request directly when in no-proxy list"
    );
    assert!(
        !has_received_requests(&proxy_server).await,
        "Proxy should NOT have received requests when target is in no-proxy list"
    );
}

/// Test that proxy URLs without a scheme in uv.toml default to http://.
#[cfg(feature = "test-pypi")]
#[tokio::test]
async fn proxy_schemeless_url_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let target_server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&target_server)
        .await;

    let proxy_server = MockServer::start().await;
    mock_simple_api(&proxy_server).await;

    let target_uri = target_server.uri();
    let proxy_uri = proxy_server.uri();

    // Strip scheme to test schemeless URL handling
    let proxy_host = proxy_uri
        .strip_prefix("http://")
        .unwrap_or(proxy_uri.as_str());

    let context = context
        .with_filter((target_uri.clone(), "[TARGET]"))
        .with_filter((proxy_uri.clone(), "[PROXY]"))
        .with_filter((proxy_host, "[PROXY_HOST]"));

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml
        .write_str(&format!(r#"http-proxy = "{proxy_host}""#))
        .unwrap();

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("iniconfig")
        .arg("--index-url")
        .arg(&target_uri)
        .arg("--config-file")
        .arg(uv_toml.path())
        .env_remove(EnvVars::HTTP_PROXY)
        .env_remove(EnvVars::HTTPS_PROXY)
        .env_remove(EnvVars::ALL_PROXY)
        .env_remove(EnvVars::NO_PROXY), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    assert!(
        has_received_requests(&proxy_server).await,
        "Proxy should have received the request even with schemeless URL"
    );
    assert!(
        !has_received_requests(&target_server).await,
        "Target should NOT have been called directly when proxy is configured"
    );
}

#[test]
fn connect_timeout_index() {
    let context = uv_test::test_context!("3.12");

    // Create a server that never responds, causing a timeout for our requests.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap().to_string();

    let start = Instant::now();
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--index-url")
        .arg(format!("https://{server}"))
        .env(EnvVars::UV_HTTP_CONNECT_TIMEOUT, "1")
        .env(EnvVars::UV_HTTP_RETRIES, "0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch: `https://[LOCALHOST]/tqdm/`
      cause: error sending request for url (https://[LOCALHOST]/tqdm/)
      cause: client error (Connect)
      cause: operation timed out
    ");

    // Assumption: There's less than 2s overhead for this test and startup.
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "Test with 1s connect timeout took too long"
    );
}

#[test]
fn connect_timeout_stream() {
    let context = uv_test::test_context!("3.12");

    // Create a server that never responds, causing a timeout for our requests.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap().to_string();

    let start = Instant::now();
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("https://{server}/tqdm-0.1-py3-none-any.whl"))
        .env(EnvVars::UV_HTTP_CONNECT_TIMEOUT, "1")
        .env(EnvVars::UV_HTTP_RETRIES, "0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ https://[LOCALHOST]/tqdm-0.1-py3-none-any.whl`
      cause: Failed to fetch: `https://[LOCALHOST]/tqdm-0.1-py3-none-any.whl`
      cause: error sending request for url (https://[LOCALHOST]/tqdm-0.1-py3-none-any.whl)
      cause: client error (Connect)
      cause: operation timed out
    ");

    // Assumption: There's less than 2s overhead for this test and startup.
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "Test with 1s connect timeout took too long"
    );
}

#[tokio::test]
async fn retry_read_timeout_index() {
    let context = uv_test::test_context!("3.12").with_fast_http_retry();

    let (server, _guard) = streaming_server(time_out_response);

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--index-url")
        .arg(server), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Request failed after 1 retry in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/tqdm/`
      cause: error decoding response body for url (http://[LOCALHOST]/tqdm/)
      cause: request or response body error
      cause: operation timed out
    ");
}

#[tokio::test]
async fn retry_read_timeout_python_downloads_json() {
    let context = uv_test::test_context!("3.12").with_fast_http_retry();

    let (server, _guard) = streaming_server(time_out_response);

    uv_snapshot!(context.filters(), context
        .python_list()
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS)
        .arg("--python-downloads-json-url")
        .arg(&server), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Error while fetching remote python downloads json from 'http://[LOCALHOST]/'
      cause: Request failed after 1 retry in [TIME]
      cause: Failed to download http://[LOCALHOST]/
      cause: error decoding response body for url (http://[LOCALHOST]/)
      cause: request or response body error
      cause: operation timed out
    ");
}

#[tokio::test]
async fn retry_read_timeout_stream() {
    let context = uv_test::test_context!("3.12").with_fast_http_retry();

    let (server, _guard) = streaming_server(time_out_response);

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("{server}/tqdm-0.1-py3-none-any.whl")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ http://[LOCALHOST]/tqdm-0.1-py3-none-any.whl`
      cause: Request failed after 1 retry in [TIME]
      cause: Failed to read metadata: `http://[LOCALHOST]/tqdm-0.1-py3-none-any.whl`
      cause: Failed to read from zip file
      cause: an upstream reader returned an error: Failed to download distribution due to network timeout. Try increasing UV_HTTP_TIMEOUT (current value: [TIME]).
      cause: Failed to download distribution due to network timeout. Try increasing UV_HTTP_TIMEOUT (current value: [TIME]).
    ");
}

#[derive(Clone, Copy)]
enum RangeResponse {
    Supported,
    Limited { known_length: bool },
    Interrupted,
    LimitedThenInterrupted,
    Ignored,
    NotAdvertised,
    InvalidContentRange,
    ShortBody,
    Unsatisfiable,
}

#[derive(Default)]
struct DownloadRequests {
    full: AtomicUsize,
    resumed: AtomicUsize,
}

/// Serve metadata normally, then truncate the initial streaming response.
/// Interrupt the first download-to-file response with a timeout.
/// Subsequent requests exercise the configured continuation or full-download fallback.
fn wheel_response(
    request: &hyper::Request<hyper::body::Incoming>,
    wheel: &Bytes,
    range_response: RangeResponse,
    requests: &DownloadRequests,
) -> Result<StreamingResponse, http::Error> {
    let streaming_attempts = 1;
    let resuming = requests.full.load(Ordering::Relaxed) > streaming_attempts;
    let size = wheel.len();
    let mut response = hyper::Response::builder();
    if request.method() == hyper::Method::HEAD {
        return response
            .header(CONTENT_LENGTH, size)
            .header(ACCEPT_RANGES, "bytes")
            .body(http_body_util::Empty::new().boxed());
    }
    if let Some(range) = request.headers().get(RANGE) {
        let (start, end) = range
            .to_str()
            .expect("ASCII range")
            .strip_prefix("bytes=")
            .expect("byte range")
            .split_once('-')
            .expect("range bounds");
        let start: usize = start.parse().expect("range start");
        let mut end = if end.is_empty() {
            size - 1
        } else {
            end.parse().expect("range end")
        };
        let mut content_range_start = start;
        let mut complete_length = size.to_string();
        let mut body_end = None;
        if resuming {
            let resumed_request = requests.resumed.fetch_add(1, Ordering::Relaxed);
            match range_response {
                RangeResponse::Supported | RangeResponse::NotAdvertised => {}
                RangeResponse::Ignored => {
                    return response
                        .header(CONTENT_LENGTH, size)
                        .body(http_body_util::Full::new(wheel.clone()).boxed());
                }
                RangeResponse::Limited { known_length } => {
                    end = end.min(start + size / 4 - 1);
                    if !known_length {
                        complete_length = "*".to_string();
                    }
                }
                RangeResponse::Interrupted | RangeResponse::LimitedThenInterrupted => {
                    if let RangeResponse::LimitedThenInterrupted = range_response
                        && resumed_request == 0
                    {
                        end = start + size / 8 - 1;
                    } else {
                        return response
                            .status(StatusCode::PARTIAL_CONTENT)
                            .header(CONTENT_RANGE, format!("bytes {start}-{end}/{size}"))
                            .header(CONTENT_LENGTH, end - start + 1)
                            .body(delayed_body(
                                wheel.slice(start..start + (end - start).div_ceil(2)),
                                Duration::from_mins(1),
                            ));
                    }
                }
                RangeResponse::InvalidContentRange => content_range_start = 0,
                RangeResponse::ShortBody => body_end = Some(end - 1),
                RangeResponse::Unsatisfiable => {
                    assert_eq!(start, size);
                    return response
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(CONTENT_RANGE, format!("bytes */{size}"))
                        .body(http_body_util::Empty::new().boxed());
                }
            }
        }
        let bytes = wheel.slice(start..=body_end.unwrap_or(end));
        return response
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                CONTENT_RANGE,
                format!("bytes {content_range_start}-{end}/{complete_length}"),
            )
            .header(CONTENT_LENGTH, bytes.len())
            .body(http_body_util::Full::new(bytes).boxed());
    }
    let full_get = requests.full.fetch_add(1, Ordering::Relaxed);
    if full_get < streaming_attempts {
        // Give Hyper time to flush the partial body before closing short of Content-Length.
        return response.header(CONTENT_LENGTH, size).body(delayed_body(
            wheel.slice(..size / 2),
            Duration::from_millis(50),
        ));
    }
    if full_get > streaming_attempts {
        return response
            .header(CONTENT_LENGTH, size)
            .body(http_body_util::Full::new(wheel.clone()).boxed());
    }
    if !matches!(range_response, RangeResponse::NotAdvertised) {
        response = response.header(ACCEPT_RANGES, "bytes");
    }
    if let RangeResponse::Unsatisfiable = range_response {
        // Send all wheel bytes, but stall before terminating the chunked response.
        return response.body(delayed_body(wheel.clone(), Duration::from_mins(1)));
    }
    response.header(CONTENT_LENGTH, size).body(delayed_body(
        wheel.slice(..size / 2),
        Duration::from_mins(1),
    ))
}

fn wheel_server(
    context: &TestContext,
    range_response: RangeResponse,
) -> Result<(String, impl Drop, Arc<DownloadRequests>, String)> {
    let fixtures = context.workspace_root.join("test/links");
    let wheel = Bytes::from(fs_err::read(
        fixtures.join("build_tag-1.0.0-1-py2.py3-none-any.whl"),
    )?);
    let hash = hex::encode(Sha256::digest(&wheel));
    let requests = Arc::new(DownloadRequests::default());
    let server_requests = requests.clone();
    let (server, guard) = streaming_server(move |request| {
        wheel_response(&request, &wheel, range_response, &server_requests)
    });
    Ok((server, guard, requests, hash))
}

#[test]
fn small_registry_wheel_is_reused_after_resolution() -> Result<()> {
    for require_ranges in [false, true] {
        let context = uv_test::test_context!("3.12");
        let wheel = Bytes::from(fs_err::read(
            context
                .workspace_root
                .join("test/links/ok-1.0.0-py3-none-any.whl"),
        )?);
        let index = Bytes::from(
            json!({
                "name": "ok",
                "files": [{
                    "filename": "ok-1.0.0-py3-none-any.whl",
                    "url": "/ok-1.0.0-py3-none-any.whl",
                    "hashes": {"sha256": hex::encode(Sha256::digest(&wheel))},
                    "size": wheel.len(),
                    "upload-time": "2024-01-01T00:00:00Z"
                }]
            })
            .to_string(),
        );
        let full = Arc::new(AtomicUsize::new(0));
        let ranges = Arc::new(AtomicUsize::new(0));
        let heads = Arc::new(AtomicUsize::new(0));
        let (server_full, server_ranges, server_heads) =
            (full.clone(), ranges.clone(), heads.clone());
        let (server, _guard) = streaming_server(move |request| {
            let response =
                hyper::Response::builder().header("cache-control", "public, max-age=3600");
            if request.uri().path() == "/simple/ok/" {
                return response
                    .header("content-type", "application/vnd.pypi.simple.v1+json")
                    .body(http_body_util::Full::new(index.clone()).boxed());
            }
            if request.method() == hyper::Method::HEAD {
                server_heads.fetch_add(1, Ordering::Relaxed);
                return response
                    .header(CONTENT_LENGTH, wheel.len())
                    .header(ACCEPT_RANGES, "bytes")
                    .body(http_body_util::Empty::new().boxed());
            }
            if let Some(range) = request.headers().get(RANGE) {
                server_ranges.fetch_add(1, Ordering::Relaxed);
                let (start, end) = range
                    .to_str()
                    .expect("ASCII range")
                    .strip_prefix("bytes=")
                    .expect("byte range")
                    .split_once('-')
                    .expect("range bounds");
                let start: usize = start.parse().expect("range start");
                let end: usize = end.parse().expect("range end");
                return response
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(
                        CONTENT_RANGE,
                        format!("bytes {start}-{end}/{}", wheel.len()),
                    )
                    .body(http_body_util::Full::new(wheel.slice(start..=end)).boxed());
            }
            server_full.fetch_add(1, Ordering::Relaxed);
            response.body(http_body_util::Full::new(wheel.clone()).boxed())
        });
        let output = context
            .pip_install()
            .arg("ok==1.0.0")
            .arg("--default-index")
            .arg(format!("{server}/simple"))
            .env(
                EnvVars::UV_REQUIRE_METADATA_RANGE_REQUESTS,
                require_ranges.to_string(),
            )
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(full.load(Ordering::Relaxed), 1);
        // The advertised wheel size also avoids HEAD when metadata ranges are required.
        assert_eq!(heads.load(Ordering::Relaxed), 0);
        assert_eq!(ranges.load(Ordering::Relaxed), usize::from(require_ranges));
    }
    Ok(())
}

#[test]
fn small_registry_wheel_retains_cached_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("requirements.in")
        .write_str("ok==1.0.0\n")?;
    let wheel = Bytes::from(fs_err::read(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
    )?);
    let index = Bytes::from(
        json!({
            "name": "ok",
            "files": [{
                "filename": "ok-1.0.0-py3-none-any.whl",
                "url": "/ok-1.0.0-py3-none-any.whl",
                "hashes": {"sha256": hex::encode(Sha256::digest(&wheel))},
                "size": wheel.len(),
                "upload-time": "2024-01-01T00:00:00Z"
            }]
        })
        .to_string(),
    );
    let full = Arc::new(AtomicUsize::new(0));
    let server_full = full.clone();
    let (server, _guard) = streaming_server(move |request| {
        let response = hyper::Response::builder().header("cache-control", "public, max-age=3600");
        if request.uri().path() == "/simple/ok/" {
            return response
                .header("content-type", "application/vnd.pypi.simple.v1+json")
                .body(http_body_util::Full::new(index.clone()).boxed());
        }
        server_full.fetch_add(1, Ordering::Relaxed);
        response.body(http_body_util::Full::new(wheel.clone()).boxed())
    });

    for collect_hashes in [true, false] {
        let mut command = context.pip_compile();
        command
            .arg("requirements.in")
            .arg("--default-index")
            .arg(format!("{server}/simple"));
        if collect_hashes {
            command.arg("--generate-hashes");
        }
        let output = command.output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(full.load(Ordering::Relaxed), 1);
    }

    let output = context
        .pip_install()
        .arg("ok==1.0.0")
        .arg("--default-index")
        .arg(format!("{server}/simple"))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(full.load(Ordering::Relaxed), 2);
    Ok(())
}

fn assert_wheel_download(
    range_response: RangeResponse,
    retries: usize,
    full_requests: usize,
    resumed_requests: usize,
) -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (server, _guard, requests, hash) = wheel_server(&context, range_response)?;
    write_wheel_lockfile(&context, &server, 932, &hash)?;
    allow_duplicates! {
        uv_snapshot!(context.filters(), context
            .pip_sync()
            .arg("--preview")
            .arg("pylock.toml")
            .env(EnvVars::UV_HTTP_RETRIES, retries.to_string())
            .env(EnvVars::UV_HTTP_TIMEOUT, "1")
            .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true")
            .env(EnvVars::RUST_LOG, "warn"), @"
        exit_code: 0 (success)
        ----- stderr -----
        WARN Streaming failed for build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl; downloading wheel to disk (I/O operation failed during extraction)
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + build-tag==1.0.0 (from http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl)
        ");
    }
    assert_eq!(requests.full.load(Ordering::Relaxed), full_requests);
    assert_eq!(requests.resumed.load(Ordering::Relaxed), resumed_requests);

    let site_packages = context.site_packages();
    assert_eq!(
        fs_err::read_to_string(site_packages.join("build_tag/__init__.py"))?,
        "def main():\n    print(\"1\")\n",
    );
    let metadata =
        fs_err::read_to_string(site_packages.join("build_tag-1.0.0.dist-info/METADATA"))?;
    let wheel = fs_err::read_to_string(site_packages.join("build_tag-1.0.0.dist-info/WHEEL"))?;
    allow_duplicates! {
        assert_snapshot!(metadata, @"
        Metadata-Version: 2.3
        Name: build-tag
        Version: 1.0.0
        ");
        assert_snapshot!(wheel, @"
        Wheel-Version: 1.0
        Generator: hatchling 1.26.3
        Root-Is-Purelib: true
        Tag: py2-none-any
        Tag: py3-none-any
        ");
    }
    Ok(())
}

fn assert_wheel_download_timeout(
    range_response: RangeResponse,
    retries: usize,
    full_requests: usize,
    resumed_requests: usize,
) -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (server, _guard, requests, _) = wheel_server(&context, range_response)?;

    let wheel_url = format!("{server}/build_tag-1.0.0-1-py2.py3-none-any.whl");
    allow_duplicates! {
        uv_snapshot!(context.filters(), context
            .pip_install()
            .arg(format!("build-tag @ {wheel_url}"))
            .env(EnvVars::UV_HTTP_RETRIES, retries.to_string())
            .env(EnvVars::UV_HTTP_TIMEOUT, "1")
            .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true")
            .env(EnvVars::RUST_LOG, "warn"), @"
        exit_code: 2 (failure)
        ----- stderr -----
        Resolved 1 package in [TIME]
        WARN Streaming failed for build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl; downloading wheel to disk (I/O operation failed during extraction)
        error: Failed to download `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`
          cause: Failed to write to the distribution cache
          cause: Failed to download distribution due to network timeout. Try increasing UV_HTTP_TIMEOUT (current value: [TIME]).
        ");
    }
    assert_eq!(requests.full.load(Ordering::Relaxed), full_requests);
    assert_eq!(requests.resumed.load(Ordering::Relaxed), resumed_requests);
    Ok(())
}

fn write_wheel_lockfile(context: &TestContext, server: &str, size: u64, hash: &str) -> Result<()> {
    context.temp_dir.child("pylock.toml").write_str(&formatdoc! {
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "build-tag"
        version = "1.0.0"
        archive = {{ url = "{server}/build_tag-1.0.0-1-py2.py3-none-any.whl", size = {size}, hashes = {{ sha256 = "{hash}" }} }}
        "#,
    })?;
    Ok(())
}

#[test]
fn direct_url_content_length_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (server, _guard, requests, hash) = wheel_server(&context, RangeResponse::NotAdvertised)?;
    write_wheel_lockfile(&context, &server, 1, &hash)?;

    uv_snapshot!(context.filters(), context
        .pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .env(EnvVars::UV_HTTP_RETRIES, "1")
        .env(EnvVars::UV_HTTP_TIMEOUT, "1")
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true")
        .env(EnvVars::RUST_LOG, "warn"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    WARN Streaming failed for build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl; downloading wheel to disk (I/O operation failed during extraction)
    error: Failed to download `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`
      cause: Content-Length mismatch for `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`: expected 1 bytes, but the server advertised 932 bytes
    ");
    // The first fallback response fails on its headers without consuming a full-download retry.
    assert_eq!(requests.full.load(Ordering::Relaxed), 2);
    Ok(())
}

#[test]
fn direct_url_range_resume() -> Result<()> {
    assert_wheel_download(RangeResponse::Supported, 1, 2, 1)
}

#[test]
fn direct_url_partial_range_resume() -> Result<()> {
    assert_wheel_download(RangeResponse::Limited { known_length: true }, 1, 2, 2)
}

#[test]
fn direct_url_partial_range_resume_unknown_length() -> Result<()> {
    assert_wheel_download(
        RangeResponse::Limited {
            known_length: false,
        },
        1,
        2,
        2,
    )
}

#[test]
fn direct_url_ignored_range_resume() -> Result<()> {
    assert_wheel_download(RangeResponse::Ignored, 1, 2, 1)
}

#[test]
fn direct_url_no_range_resume() -> Result<()> {
    assert_wheel_download(RangeResponse::NotAdvertised, 1, 3, 0)
}

#[test]
fn direct_url_unsatisfiable_range_retries_in_full() -> Result<()> {
    assert_wheel_download(RangeResponse::Unsatisfiable, 2, 3, 1)
}

#[test]
fn direct_url_unsatisfiable_range_does_not_bypass_retry() -> Result<()> {
    assert_wheel_download_timeout(RangeResponse::Unsatisfiable, 1, 2, 1)
}

/// An invalid continuation response does not bypass regular retry handling.
#[test]
fn direct_url_invalid_range_does_not_bypass_retry() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let (server, _guard, requests, _) = wheel_server(&context, RangeResponse::InvalidContentRange)?;

    let wheel_url = format!("{server}/build_tag-1.0.0-1-py2.py3-none-any.whl");
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("build-tag @ {wheel_url}"))
        .env(EnvVars::UV_HTTP_RETRIES, "1")
        .env(EnvVars::UV_HTTP_TIMEOUT, "1")
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true")
        .env(EnvVars::RUST_LOG, "warn"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    WARN Streaming failed for build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl; downloading wheel to disk (I/O operation failed during extraction)
    WARN Invalid range request response from server that declares HTTP range request support, abandoning resumed download: http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl
    error: Failed to download `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`
      cause: Failed to write to the distribution cache
      cause: Failed to download distribution due to network timeout. Try increasing UV_HTTP_TIMEOUT (current value: [TIME]).
    ");
    assert_eq!(requests.full.load(Ordering::Relaxed), 2);
    assert_eq!(requests.resumed.load(Ordering::Relaxed), 1);
    Ok(())
}

/// A complete HTTP body with the wrong range length fails without retrying the full download.
#[test]
fn direct_url_range_size_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (server, _guard, requests, _) = wheel_server(&context, RangeResponse::ShortBody)?;

    let wheel_url = format!("{server}/build_tag-1.0.0-1-py2.py3-none-any.whl");
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("build-tag @ {wheel_url}"))
        .env(EnvVars::UV_HTTP_RETRIES, "1")
        .env(EnvVars::UV_HTTP_TIMEOUT, "1")
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true")
        .env(EnvVars::RUST_LOG, "warn"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    WARN Streaming failed for build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl; downloading wheel to disk (I/O operation failed during extraction)
    error: Failed to download `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`
      cause: Range response size mismatch for `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`: expected 466 bytes from Content-Range, but received 465 bytes
    ");
    // The streaming attempt precedes the download fallback; the range mismatch ends the attempt.
    assert_eq!(requests.full.load(Ordering::Relaxed), 2);
    Ok(())
}

#[test]
fn direct_url_range_resume_disabled() -> Result<()> {
    assert_wheel_download_timeout(RangeResponse::Supported, 0, 2, 0)
}

#[test]
fn direct_url_range_resume_retry_limit() -> Result<()> {
    assert_wheel_download_timeout(RangeResponse::Interrupted, 2, 2, 2)
}

#[test]
fn direct_url_range_resume_success_does_not_reset_retries() -> Result<()> {
    assert_wheel_download_timeout(RangeResponse::LimitedThenInterrupted, 1, 2, 2)
}
