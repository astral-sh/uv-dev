use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::time::{pause, timeout};
use url::Url;

use uv_client::{BaseClientBuilder, WrappedReqwestError};
use uv_redacted::DisplaySafeUrl;

#[tokio::test]
async fn connect_timeout_during_tls_handshake() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("https://{}/", listener.local_addr()?).parse::<DisplaySafeUrl>()?;
    let client = BaseClientBuilder::default()
        .proxy(reqwest::Proxy::custom(|_| None::<Url>))
        .connect_timeout(Duration::from_secs(1))
        .read_timeout(Duration::from_secs(30))
        .retries(0)
        .build()?;
    let request = tokio::spawn(async move { client.for_host(&url).get(url.as_str()).send().await });

    // Wait for the TLS ClientHello so the connection deadline is already running.
    // Keep the peer open without completing the handshake.
    let (mut connection, _) = timeout(Duration::from_secs(30), listener.accept()).await??;
    let mut hello = [0];
    timeout(Duration::from_secs(30), connection.read_exact(&mut hello)).await??;
    pause();

    let error = timeout(Duration::from_secs(2), request)
        .await
        .context("The connect deadline did not expire before the read deadline")??
        .expect_err("The incomplete TLS handshake must time out");
    let error = WrappedReqwestError::from(error);
    assert!(
        error
            .inner()
            .is_some_and(|error| error.is_connect() && error.is_timeout()),
        "{error:#}"
    );
    Ok(())
}
