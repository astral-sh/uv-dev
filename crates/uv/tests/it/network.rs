use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use assert_fs::fixture::{FileWriteStr, PathChild};
use bytes::Bytes;
use http::StatusCode;
use http::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use http_body_util::BodyExt;
use indoc::{formatdoc, indoc};
use insta::{allow_duplicates, assert_snapshot};
use serde_json::json;
use sha2::{Digest, Sha256};
use wiremock::matchers::{any, method};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

#[cfg(feature = "test-pypi")]
use uv_redacted::SafeUrl;
use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

use super::network_fixtures::{
    StreamingResponse, StreamingServerGuard, connection_reset, delayed_body, http_error_server,
    io_error_server, streaming_server, time_out_response,
};

const BUILD_TAG_WHEEL: &str = "build_tag-1.0.0-1-py2.py3-none-any.whl";

/// Create a CONNECT proxy on the test runtime, which owns the listener and connection tasks.
#[cfg(feature = "test-pypi")]
async fn start_connect_tunnel_proxy() -> (std::net::SocketAddr, Arc<AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let proxy_connections = Arc::clone(&connections);

    tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            let proxy_connections = Arc::clone(&proxy_connections);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let mut total_read = 0;
                loop {
                    let n = match client.read(&mut buf[total_read..]).await {
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
                let Some(target_addr) = request
                    .lines()
                    .next()
                    .and_then(|line| line.strip_prefix("CONNECT "))
                    .and_then(|s| s.split_whitespace().next())
                else {
                    return;
                };
                let Ok(mut target) = TcpStream::connect(target_addr).await else {
                    return;
                };
                if client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .is_err()
                {
                    return;
                }
                proxy_connections.fetch_add(1, Ordering::Relaxed);
                let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
            });
        }
    });

    (addr, connections)
}

/// Serve a local package through the same host as the requested index.
async fn mock_build_tag_index(context: &TestContext, server: &MockServer) {
    use wiremock::matchers::path;

    let wheel = context.read_bytes(
        context
            .workspace_root
            .join("test/links")
            .join(BUILD_TAG_WHEEL),
    );
    let hash = hex::encode(Sha256::digest(&wheel));
    let body = json!({
        "name": "build-tag",
        "files": [{
            "filename": BUILD_TAG_WHEEL,
            "url": format!("/{BUILD_TAG_WHEEL}"),
            "hashes": { "sha256": hash },
            "upload-time": "2024-01-01T00:00:00Z"
        }]
    });
    Mock::given(method("GET"))
        .and(path("/build-tag/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(body.to_string(), "application/vnd.pypi.simple.v1+json"),
        )
        .expect(1..)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{BUILD_TAG_WHEEL}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .expect(1..)
        .mount(server)
        .await;
}

/// Return two connection resets followed by two HTTP 500 responses across the initial request
/// and three retries.
async fn mixed_error_server() -> (MockServer, String) {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with_err(connection_reset)
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(StatusCode::INTERNAL_SERVER_ERROR))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;

    let mock_server_uri = server.uri();
    (server, mock_server_uri)
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
      cause: Failed to fetch: http://[LOCALHOST]/tqdm/
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
      cause: Failed to fetch: http://[LOCALHOST]/tqdm/
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
      cause: Failed to fetch: http://[LOCALHOST]/tqdm/
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
      cause: Failed to fetch: http://[LOCALHOST]/tqdm/
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
      cause: Failed to fetch: http://[LOCALHOST]/
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
      cause: Failed to fetch: http://[LOCALHOST]/
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
      cause: Failed to fetch: http://[LOCALHOST]/
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/)
    ");
}

/// Check that a missing direct package URL is classified as a user error.
#[tokio::test]
async fn direct_url_http_404() {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

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
      cause: Failed to fetch: http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl
      cause: HTTP status client error (404 Not Found) for url (http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl)
    ");

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("tqdm @ {tqdm_url}"))
        .arg("--quiet"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl`
      cause: Failed to fetch: http://[LOCALHOST]/tqdm-4.67.1-py3-none-any.whl
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
      cause: Failed to fetch: http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl
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
      cause: Failed to fetch: http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl
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
      cause: Failed to fetch: http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/packages/d0/30/dc54f88dd4a2b5dc8a0279bdd7270e735851848b762aeb1c1184ed1f6b14/tqdm-4.67.1-py3-none-any.whl)
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
      cause: Failed to fetch: http://[LOCALHOST]/anyio/
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
      cause: Failed to fetch: http://[LOCALHOST]/anyio/
      cause: error sending request for url (http://[LOCALHOST]/anyio/)
      cause: client error (SendRequest)
      cause: connection closed before message completed
    "
    );
}

/// Test problem details with a 403 error containing license compliance information
#[tokio::test]
async fn rfc9457_problem_details_license_violation() {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

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
      cause: Failed to fetch: http://[LOCALHOST]/packages/tqdm-4.67.1-py3-none-any.whl
      cause: Server message: License Compliance Issue, This package version has a license that violates organizational policy.
      cause: HTTP status client error (403 Forbidden) for url (http://[LOCALHOST]/packages/tqdm-4.67.1-py3-none-any.whl)
    ");
}

/// Test that valid proxy URL in uv.toml routes requests through the proxy.
#[tokio::test]
async fn proxy_valid_url_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let target_server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&target_server)
        .await;

    let proxy_server = MockServer::start().await;
    mock_build_tag_index(&context, &proxy_server).await;

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
        .arg("build-tag")
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
     + build-tag==1.0.0
    ");
}

/// Test that https-proxy in uv.toml routes HTTPS requests through a CONNECT tunnel proxy.
#[cfg(feature = "test-pypi")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proxy_https_proxy_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let (proxy_addr, connections) = start_connect_tunnel_proxy().await;
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
    assert!(
        connections.load(Ordering::Relaxed) > 0,
        "the configured HTTPS proxy should establish a CONNECT tunnel"
    );
}

/// Test that no-proxy in uv.toml bypasses the proxy for specified hosts.
#[tokio::test]
async fn proxy_no_proxy_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let target_server = MockServer::start().await;
    mock_build_tag_index(&context, &target_server).await;

    let proxy_server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&proxy_server)
        .await;

    let target_uri = target_server.uri();
    let proxy_uri = proxy_server.uri();

    // Note: reqwest's NoProxy matches on host only, not host:port
    let target_url = SafeUrl::parse(&target_uri).unwrap();
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
        .arg("build-tag")
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
     + build-tag==1.0.0
    ");
}

/// Test that proxy URLs without a scheme in uv.toml default to http://.
#[tokio::test]
async fn proxy_schemeless_url_in_uv_toml() {
    let context = uv_test::test_context!("3.12");

    let target_server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&target_server)
        .await;

    let proxy_server = MockServer::start().await;
    mock_build_tag_index(&context, &proxy_server).await;

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
        .arg("build-tag")
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
     + build-tag==1.0.0
    ");
}

/// Keep an accepted socket open without completing the TLS handshake.
struct StalledTlsServer {
    address: std::net::SocketAddr,
    connection: tokio::task::JoinHandle<(tokio::net::TcpStream, Instant)>,
}

impl StalledTlsServer {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        // The task result retains the socket until the assertion, keeping the handshake stalled.
        let connection = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            (socket, Instant::now())
        });
        Self {
            address,
            connection,
        }
    }

    async fn assert_connect_timeout(self) {
        let (_socket, connected_at) = tokio::time::timeout(Duration::from_secs(3), self.connection)
            .await
            .expect("uv should connect to the TLS server")
            .expect("TLS server task should complete");
        let elapsed = connected_at.elapsed();
        assert!(
            elapsed < Duration::from_secs(3),
            "one-second connect timeout took {elapsed:?} after accepting the connection"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_timeout_index() {
    let context = uv_test::test_context!("3.12");

    let server = StalledTlsServer::start().await;
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("tqdm")
        .arg("--index-url")
        .arg(format!("https://{}", server.address))
        .env(EnvVars::UV_HTTP_CONNECT_TIMEOUT, "1")
        .env(EnvVars::UV_HTTP_RETRIES, "0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch: https://[LOCALHOST]/tqdm/
      cause: error sending request for url (https://[LOCALHOST]/tqdm/)
      cause: client error (Connect)
      cause: operation timed out
    ");

    server.assert_connect_timeout().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_timeout_stream() {
    let context = uv_test::test_context!("3.12");

    let server = StalledTlsServer::start().await;
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("https://{}/tqdm-0.1-py3-none-any.whl", server.address))
        .env(EnvVars::UV_HTTP_CONNECT_TIMEOUT, "1")
        .env(EnvVars::UV_HTTP_RETRIES, "0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ https://[LOCALHOST]/tqdm-0.1-py3-none-any.whl`
      cause: Failed to fetch: https://[LOCALHOST]/tqdm-0.1-py3-none-any.whl
      cause: error sending request for url (https://[LOCALHOST]/tqdm-0.1-py3-none-any.whl)
      cause: client error (Connect)
      cause: operation timed out
    ");

    server.assert_connect_timeout().await;
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
      cause: Failed to fetch: http://[LOCALHOST]/tqdm/
      cause: error decoding response body for url (http://[LOCALHOST]/tqdm/)
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
      cause: Failed to read metadata: http://[LOCALHOST]/tqdm-0.1-py3-none-any.whl
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

/// Serve metadata normally, then truncate full responses until streaming retries are exhausted.
/// Interrupt the first download-to-file response with a timeout.
/// Subsequent requests exercise the configured continuation or full-download fallback.
fn wheel_response(
    request: &hyper::Request<hyper::body::Incoming>,
    wheel: &Bytes,
    range_response: RangeResponse,
    retries: usize,
    requests: &DownloadRequests,
) -> Result<StreamingResponse, http::Error> {
    let streaming_attempts = 1 + retries;
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

struct WheelServer {
    url: String,
    _guard: StreamingServerGuard,
    requests: Arc<DownloadRequests>,
    hash: String,
    size: usize,
}

impl WheelServer {
    fn write_lockfile(&self, context: &TestContext, declared_size: usize) -> Result<()> {
        context
            .temp_dir
            .child("pylock.toml")
            .write_str(&formatdoc! {
                r#"
            lock-version = "1.0"
            created-by = "uv"

            [[packages]]
            name = "build-tag"
            version = "1.0.0"
            archive = {{ url = "{url}", size = {declared_size}, hashes = {{ sha256 = "{hash}" }} }}
            "#,
                url = self.url,
                hash = self.hash,
            })?;
        Ok(())
    }
}

fn wheel_server(
    context: &TestContext,
    range_response: RangeResponse,
    retries: usize,
) -> WheelServer {
    let fixtures = context.workspace_root.join("test/links");
    let wheel = Bytes::from(context.read_bytes(fixtures.join(BUILD_TAG_WHEEL)));
    let hash = hex::encode(Sha256::digest(&wheel));
    let size = wheel.len();
    let requests = Arc::new(DownloadRequests::default());
    let server_requests = requests.clone();
    let (url, guard) = streaming_server(move |request| {
        wheel_response(&request, &wheel, range_response, retries, &server_requests)
    });
    WheelServer {
        url: format!("{url}/{BUILD_TAG_WHEEL}"),
        _guard: guard,
        requests,
        hash,
        size,
    }
}

fn assert_wheel_download(
    range_response: RangeResponse,
    retries: usize,
    full_requests: usize,
    resumed_requests: usize,
) -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = wheel_server(&context, range_response, retries);
    server.write_lockfile(&context, server.size)?;
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
        WARN Streaming failed for `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`; downloading wheel to disk (I/O operation failed during extraction)
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + build-tag==1.0.0 (from http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl)
        ");
    }
    assert_eq!(server.requests.full.load(Ordering::Relaxed), full_requests);
    assert_eq!(
        server.requests.resumed.load(Ordering::Relaxed),
        resumed_requests
    );

    let site_packages = context.site_packages();
    assert_eq!(
        context.read(site_packages.join("build_tag/__init__.py")),
        indoc! {r#"
            def main():
                print("1")
        "#},
    );
    let metadata = context.read(site_packages.join("build_tag-1.0.0.dist-info/METADATA"));
    let wheel = context.read(site_packages.join("build_tag-1.0.0.dist-info/WHEEL"));
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
) {
    let context = uv_test::test_context!("3.12");
    let server = wheel_server(&context, range_response, retries);

    allow_duplicates! {
        uv_snapshot!(context.filters(), context
            .pip_install()
            .arg(format!("build-tag @ {}", server.url))
            .env(EnvVars::UV_HTTP_RETRIES, retries.to_string())
            .env(EnvVars::UV_HTTP_TIMEOUT, "1")
            .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true")
            .env(EnvVars::RUST_LOG, "warn"), @"
        exit_code: 2 (failure)
        ----- stderr -----
        Resolved 1 package in [TIME]
        WARN Streaming failed for `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`; downloading wheel to disk (I/O operation failed during extraction)
        error: Failed to download `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`
          cause: Failed to write to the distribution cache
          cause: Failed to download distribution due to network timeout. Try increasing UV_HTTP_TIMEOUT (current value: [TIME]).
        ");
    }
    assert_eq!(server.requests.full.load(Ordering::Relaxed), full_requests);
    assert_eq!(
        server.requests.resumed.load(Ordering::Relaxed),
        resumed_requests
    );
}

/// Metadata errors from HTTP range requests must redact URL passwords and `sig` query values.
#[cfg(feature = "test-python")]
#[test]
fn remote_wheel_metadata_error_redacts_url() {
    let context = uv_test::test_context!("3.12");
    let (server, _guard, _, _) = wheel_server(&context, RangeResponse::Supported, 0);
    let server = server.replacen("http://", "http://user:password@", 1);

    // The served wheel contains `build_tag` metadata, which does not match the requested name.
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("--no-index")
        .arg(format!("tqdm @ {server}/tqdm-1.0.0-py3-none-any.whl?sig=signature&keep=value")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `tqdm @ http://user:****@[LOCALHOST]/tqdm-1.0.0-py3-none-any.whl?sig=****&keep=value`
      cause: Failed to read metadata: http://user:****@[LOCALHOST]/tqdm-1.0.0-py3-none-any.whl?sig=****&keep=value
      cause: The .dist-info directory `build_tag-1.0.0` does not start with the normalized package name: tqdm
    ");
}

#[test]
fn direct_url_content_length_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = wheel_server(&context, RangeResponse::NotAdvertised, 1);
    server.write_lockfile(&context, 1)?;

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
    WARN Streaming failed for `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`; downloading wheel to disk (I/O operation failed during extraction)
    error: Failed to download `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`
      cause: Content-Length mismatch for `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`: expected 1 bytes, but the server advertised 932 bytes
    ");
    // The first fallback response fails on its headers without consuming a full-download retry.
    assert_eq!(server.requests.full.load(Ordering::Relaxed), 3);
    Ok(())
}

#[test]
fn direct_url_range_resume() -> Result<()> {
    assert_wheel_download(RangeResponse::Supported, 1, 3, 1)
}

#[test]
fn direct_url_partial_range_resume() -> Result<()> {
    assert_wheel_download(RangeResponse::Limited { known_length: true }, 1, 3, 2)
}

#[test]
fn direct_url_partial_range_resume_unknown_length() -> Result<()> {
    assert_wheel_download(
        RangeResponse::Limited {
            known_length: false,
        },
        1,
        3,
        2,
    )
}

#[test]
fn direct_url_ignored_range_resume() -> Result<()> {
    assert_wheel_download(RangeResponse::Ignored, 1, 3, 1)
}

#[test]
fn direct_url_no_range_resume() -> Result<()> {
    assert_wheel_download(RangeResponse::NotAdvertised, 1, 4, 0)
}

#[test]
fn direct_url_unsatisfiable_range_retries_in_full() -> Result<()> {
    assert_wheel_download(RangeResponse::Unsatisfiable, 2, 5, 1)
}

#[test]
fn direct_url_unsatisfiable_range_does_not_bypass_retry() {
    assert_wheel_download_timeout(RangeResponse::Unsatisfiable, 1, 3, 1);
}

/// An invalid continuation response does not bypass regular retry handling.
#[test]
fn direct_url_invalid_range_does_not_bypass_retry() {
    let context = uv_test::test_context!("3.12");

    let server = wheel_server(&context, RangeResponse::InvalidContentRange, 1);

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("build-tag @ {}", server.url))
        .env(EnvVars::UV_HTTP_RETRIES, "1")
        .env(EnvVars::UV_HTTP_TIMEOUT, "1")
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true")
        .env(EnvVars::RUST_LOG, "warn"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    WARN Streaming failed for `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`; downloading wheel to disk (I/O operation failed during extraction)
    WARN Invalid range request response from server that declares HTTP range request support, abandoning resumed download: http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl
    error: Failed to download `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`
      cause: Failed to write to the distribution cache
      cause: Failed to download distribution due to network timeout. Try increasing UV_HTTP_TIMEOUT (current value: [TIME]).
    ");
    assert_eq!(server.requests.full.load(Ordering::Relaxed), 3);
    assert_eq!(server.requests.resumed.load(Ordering::Relaxed), 1);
}

/// A complete HTTP body with the wrong range length fails without retrying the full download.
#[test]
fn direct_url_range_size_mismatch() {
    let context = uv_test::test_context!("3.12");
    let server = wheel_server(&context, RangeResponse::ShortBody, 1);

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(format!("build-tag @ {}", server.url))
        .env(EnvVars::UV_HTTP_RETRIES, "1")
        .env(EnvVars::UV_HTTP_TIMEOUT, "1")
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true")
        .env(EnvVars::RUST_LOG, "warn"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    WARN Streaming failed for `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`; downloading wheel to disk (I/O operation failed during extraction)
    error: Failed to download `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`
      cause: Range response size mismatch for `build-tag @ http://[LOCALHOST]/build_tag-1.0.0-1-py2.py3-none-any.whl`: expected 466 bytes from Content-Range, but received 465 bytes
    ");
    // Two streaming attempts precede the download fallback; the range mismatch ends the attempt.
    assert_eq!(server.requests.full.load(Ordering::Relaxed), 3);
}

#[test]
fn direct_url_range_resume_disabled() {
    assert_wheel_download_timeout(RangeResponse::Supported, 0, 2, 0);
}

#[test]
fn direct_url_range_resume_retry_limit() {
    assert_wheel_download_timeout(RangeResponse::Interrupted, 2, 4, 2);
}

#[test]
fn direct_url_range_resume_success_does_not_reset_retries() {
    assert_wheel_download_timeout(RangeResponse::LimitedThenInterrupted, 1, 3, 2);
}
