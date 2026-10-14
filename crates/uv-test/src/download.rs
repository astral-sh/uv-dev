//! Fixture downloads for integration tests.

use std::env;
use std::path::Path;
use std::str::FromStr;

use anyhow::Result;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;

use uv_configuration::TrustedHost;
use uv_fs::Simplified;
use uv_static::EnvVars;

/// Download a fixture to disk using the test environment's TLS settings.
#[tokio::main(flavor = "current_thread")]
pub async fn download_to_disk(url: &str, path: &Path) {
    download_to_disk_async(url, path)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "Failed to download `{url}` to `{}`: {error:#}",
                path.user_display()
            )
        });
}

async fn download_to_disk_async(url: &str, path: &Path) -> Result<()> {
    let trusted_hosts = env::var(EnvVars::UV_INSECURE_HOST)
        .unwrap_or_default()
        .split_whitespace()
        .map(TrustedHost::from_str)
        .collect::<Result<Vec<_>, _>>()?;

    let client = uv_client::BaseClientBuilder::default()
        .allow_insecure_host(trusted_hosts)
        .build()?;
    let url = url.parse()?;
    let response = client
        .for_host(&url)
        .get(url)
        .send()
        .await?
        .error_for_status()?;

    let mut file = fs_err::tokio::File::create(path).await?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.sync_all().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;
    use wiremock::MockServer;

    use crate::assert_path_missing;

    use super::download_to_disk_async;

    #[tokio::test]
    async fn rejects_http_error_before_creating_fixture() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("fixture.whl");

        let error = download_to_disk_async(&format!("{}/missing.whl", server.uri()), &destination)
            .await
            .expect_err("an unsuccessful response should not become a fixture");

        assert_eq!(
            error
                .downcast_ref::<reqwest::Error>()
                .and_then(reqwest::Error::status),
            Some(StatusCode::NOT_FOUND)
        );
        assert_path_missing(destination);
        Ok(())
    }
}
