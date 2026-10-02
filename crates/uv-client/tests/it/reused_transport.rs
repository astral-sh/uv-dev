use anyhow::Result;

use uv_client::BaseClientBuilder;
use uv_redacted::DisplaySafeUrl;

use crate::http_util::{generate_self_signed_certs_with_ca, start_https_user_agent_server};

#[tokio::test]
async fn reused_transport_keeps_secure_and_insecure_clients_separate() -> Result<()> {
    let (_, server_cert, _) = generate_self_signed_certs_with_ca()?;
    let builder = BaseClientBuilder::default().retries(0);
    let transport = builder.build()?;

    let (server, address) = start_https_user_agent_server(&server_cert).await?;
    let url = DisplaySafeUrl::parse(&format!("https://{address}"))?;
    let client = builder.clone().reuse_client(&transport).build()?;
    assert!(
        client
            .for_host(&url)
            .get(url.as_str())
            .send()
            .await
            .is_err()
    );
    assert!(server.await?.is_err());

    let (server, address) = start_https_user_agent_server(&server_cert).await?;
    let url = DisplaySafeUrl::parse(&format!("https://{address}"))?;
    let insecure = builder
        .clone()
        .allow_insecure_host(vec!["127.0.0.1".parse()?])
        .reuse_client(&transport)
        .build()?;
    let response = insecure.for_host(&url).get(url.as_str()).send().await?;
    assert!(response.status().is_success());
    response.bytes().await?;
    server.await??;

    let (server, address) = start_https_user_agent_server(&server_cert).await?;
    let url = DisplaySafeUrl::parse(&format!("https://{address}"))?;
    let secure = builder.reuse_client(&insecure).build()?;
    assert!(
        secure
            .for_host(&url)
            .get(url.as_str())
            .send()
            .await
            .is_err()
    );
    assert!(server.await?.is_err());
    Ok(())
}
