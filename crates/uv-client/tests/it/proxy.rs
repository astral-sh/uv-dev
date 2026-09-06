//! An integration test for proxy support in `uv-client`.

use anyhow::Result;
use temp_env::async_with_vars;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use wiremock::matchers::{any, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_client::BaseClientBuilder;
use uv_configuration::ProxyUrl;
use uv_static::EnvVars;

#[tokio::test]
async fn http_proxy() -> Result<()> {
    // Start a mock server to act as the target.
    let target_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&target_server)
        .await;

    // Start a mock server to act as the proxy.
    let proxy_server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&proxy_server)
        .await;

    // Create a client with the proxy.
    let client = BaseClientBuilder::default()
        .http_proxy(Some(proxy_server.uri().parse::<ProxyUrl>()?))
        .build()?;

    // Make a request to the target.
    let response = client
        .for_host(&target_server.uri().parse()?)
        .get(target_server.uri())
        .send()
        .await?;

    assert_eq!(response.status(), 200);

    // Assert that the proxy was called.
    let received_requests = proxy_server.received_requests().await.unwrap();
    assert_eq!(received_requests.len(), 1);

    Ok(())
}

#[tokio::test]
async fn no_proxy() -> Result<()> {
    // Start a mock server to act as the target.
    let target_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&target_server)
        .await;

    // Start a mock server to act as the proxy.
    let proxy_server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&proxy_server)
        .await;

    // The host of the target server should be excluded from proxying.
    let target_host = target_server.address().ip().to_string();

    // Create a client with the proxy.
    let client = BaseClientBuilder::default()
        .http_proxy(Some(proxy_server.uri().parse::<ProxyUrl>()?))
        .no_proxy(Some(vec![target_host]))
        .build()?;

    // Make a request to the target.
    let response = client
        .for_host(&target_server.uri().parse()?)
        .get(target_server.uri())
        .send()
        .await?;

    assert_eq!(response.status(), 200);

    // Assert that the proxy was NOT called.
    let received_requests = proxy_server.received_requests().await.unwrap();
    assert_eq!(received_requests.len(), 0);

    Ok(())
}

#[tokio::test]
async fn configured_http_proxy_preserves_https_proxy_from_environment() -> Result<()> {
    let http_proxy = MockServer::start().await;
    let https_proxy = TcpListener::bind("127.0.0.1:0").await?;
    let https_proxy_url = format!("http://{}", https_proxy.local_addr()?);

    async_with_vars(
        [
            (EnvVars::HTTP_PROXY, None),
            #[cfg(not(windows))]
            ("http_proxy", None),
            (EnvVars::HTTPS_PROXY, Some(https_proxy_url.as_str())),
            #[cfg(not(windows))]
            ("https_proxy", None),
            (EnvVars::ALL_PROXY, None),
            #[cfg(not(windows))]
            ("all_proxy", None),
            (EnvVars::NO_PROXY, None),
            #[cfg(not(windows))]
            ("no_proxy", None),
            ("REQUEST_METHOD", None),
        ],
        async {
            let client = BaseClientBuilder::default()
                .http_proxy(Some(http_proxy.uri().parse::<ProxyUrl>()?))
                .retries(0)
                .build()?;

            let target = "https://example.invalid".parse()?;
            let request = tokio::spawn(async move {
                client
                    .for_host(&target)
                    .get("https://example.invalid")
                    .send()
                    .await
            });

            let (mut stream, _) = timeout(Duration::from_secs(5), https_proxy.accept()).await??;
            let mut buffer = [0; 1024];
            let length = stream.read(&mut buffer).await?;
            assert!(buffer[..length].starts_with(b"CONNECT example.invalid:443 HTTP/1.1"));
            drop(stream);
            assert!(request.await?.is_err());

            Ok(())
        },
    )
    .await
}

#[tokio::test]
async fn configured_https_proxy_preserves_http_proxy_from_environment() -> Result<()> {
    let target = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&target)
        .await;

    let http_proxy = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&http_proxy)
        .await;

    let https_proxy = MockServer::start().await;
    let http_proxy_url = http_proxy.uri();

    async_with_vars(
        [
            (EnvVars::HTTP_PROXY, Some(http_proxy_url.as_str())),
            #[cfg(not(windows))]
            ("http_proxy", None),
            (EnvVars::HTTPS_PROXY, None),
            #[cfg(not(windows))]
            ("https_proxy", None),
            (EnvVars::ALL_PROXY, None),
            #[cfg(not(windows))]
            ("all_proxy", None),
            (EnvVars::NO_PROXY, None),
            #[cfg(not(windows))]
            ("no_proxy", None),
            ("REQUEST_METHOD", None),
        ],
        async {
            let client = BaseClientBuilder::default()
                .https_proxy(Some(https_proxy.uri().parse::<ProxyUrl>()?))
                .build()?;

            let response = client
                .for_host(&target.uri().parse()?)
                .get(target.uri())
                .send()
                .await?;
            assert_eq!(response.status(), 200);
            assert_eq!(http_proxy.received_requests().await.unwrap().len(), 1);
            assert!(target.received_requests().await.unwrap().is_empty());

            Ok(())
        },
    )
    .await
}

#[tokio::test]
async fn configured_no_proxy_bypasses_https_proxy_from_environment() -> Result<()> {
    let target = TcpListener::bind("127.0.0.1:0").await?;
    let target_url = format!("https://{}", target.local_addr()?);
    let proxy = TcpListener::bind("127.0.0.1:0").await?;
    let proxy_url = format!("http://{}", proxy.local_addr()?);

    async_with_vars(
        [
            (EnvVars::HTTP_PROXY, None),
            #[cfg(not(windows))]
            ("http_proxy", None),
            (EnvVars::HTTPS_PROXY, Some(proxy_url.as_str())),
            #[cfg(not(windows))]
            ("https_proxy", None),
            (EnvVars::ALL_PROXY, None),
            #[cfg(not(windows))]
            ("all_proxy", None),
            (EnvVars::NO_PROXY, None),
            #[cfg(not(windows))]
            ("no_proxy", None),
            ("REQUEST_METHOD", None),
        ],
        async {
            let client = BaseClientBuilder::default()
                .no_proxy(Some(vec!["127.0.0.1".to_string()]))
                .retries(0)
                .build()?;

            let target_url_safe = target_url.parse()?;
            let request = tokio::spawn(async move {
                client
                    .for_host(&target_url_safe)
                    .get(target_url)
                    .send()
                    .await
            });

            let (mut stream, _) = timeout(Duration::from_secs(5), target.accept()).await??;
            let mut buffer = [0; 1024];
            assert!(stream.read(&mut buffer).await? > 0);
            drop(stream);
            assert!(request.await?.is_err());
            assert!(
                timeout(Duration::from_millis(100), proxy.accept())
                    .await
                    .is_err()
            );

            Ok(())
        },
    )
    .await
}

#[tokio::test]
async fn environment_no_proxy_bypasses_configured_proxy() -> Result<()> {
    let target = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&target)
        .await;

    let proxy = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&proxy)
        .await;

    let target_host = target.address().ip().to_string();

    async_with_vars(
        [
            (EnvVars::HTTP_PROXY, None),
            #[cfg(not(windows))]
            ("http_proxy", None),
            (EnvVars::HTTPS_PROXY, None),
            #[cfg(not(windows))]
            ("https_proxy", None),
            (EnvVars::ALL_PROXY, None),
            #[cfg(not(windows))]
            ("all_proxy", None),
            (EnvVars::NO_PROXY, Some(target_host.as_str())),
            #[cfg(not(windows))]
            ("no_proxy", None),
            ("REQUEST_METHOD", None),
        ],
        async {
            let client = BaseClientBuilder::default()
                .http_proxy(Some(proxy.uri().parse::<ProxyUrl>()?))
                .build()?;

            let response = client
                .for_host(&target.uri().parse()?)
                .get(target.uri())
                .send()
                .await?;
            assert_eq!(response.status(), 200);
            assert_eq!(target.received_requests().await.unwrap().len(), 1);
            assert!(proxy.received_requests().await.unwrap().is_empty());

            Ok(())
        },
    )
    .await
}

#[tokio::test]
async fn cgi_environment_proxy_settings_do_not_affect_configured_proxy() -> Result<()> {
    let http_target = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&http_target)
        .await;

    let http_proxy = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&http_proxy)
        .await;

    let https_target = TcpListener::bind("127.0.0.1:0").await?;
    let https_target_url = format!("https://{}", https_target.local_addr()?);
    let environment_proxy = TcpListener::bind("127.0.0.1:0").await?;
    let environment_proxy_url = format!("http://{}", environment_proxy.local_addr()?);
    let http_target_host = http_target.address().ip().to_string();

    async_with_vars(
        [
            (EnvVars::HTTP_PROXY, None),
            #[cfg(not(windows))]
            ("http_proxy", None),
            (EnvVars::HTTPS_PROXY, None),
            #[cfg(not(windows))]
            ("https_proxy", None),
            (EnvVars::ALL_PROXY, Some(environment_proxy_url.as_str())),
            #[cfg(not(windows))]
            ("all_proxy", None),
            (EnvVars::NO_PROXY, Some(http_target_host.as_str())),
            #[cfg(not(windows))]
            ("no_proxy", None),
            ("REQUEST_METHOD", Some("GET")),
        ],
        async {
            let client = BaseClientBuilder::default()
                .http_proxy(Some(http_proxy.uri().parse::<ProxyUrl>()?))
                .retries(0)
                .build()?;

            let response = client
                .for_host(&http_target.uri().parse()?)
                .get(http_target.uri())
                .send()
                .await?;
            assert_eq!(response.status(), 200);
            assert_eq!(http_proxy.received_requests().await.unwrap().len(), 1);
            assert!(http_target.received_requests().await.unwrap().is_empty());

            let https_target_url_safe = https_target_url.parse()?;
            let request = tokio::spawn(async move {
                client
                    .for_host(&https_target_url_safe)
                    .get(https_target_url)
                    .send()
                    .await
            });

            let (mut stream, _) = timeout(Duration::from_secs(5), https_target.accept()).await??;
            let mut buffer = [0; 1024];
            assert!(stream.read(&mut buffer).await? > 0);
            drop(stream);
            assert!(request.await?.is_err());
            assert!(
                timeout(Duration::from_millis(100), environment_proxy.accept())
                    .await
                    .is_err()
            );

            Ok(())
        },
    )
    .await
}
