//! Fixture downloads for integration tests.

use std::env;
use std::path::Path;
use std::str::FromStr;

use anyhow::Result;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;

use uv_client::{BaseClient, BaseClientBuilder};
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
    let client = fixture_client()?;
    download_to_disk_with_client(&client, url, path).await
}

async fn download_to_disk_with_client(client: &BaseClient, url: &str, path: &Path) -> Result<()> {
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

/// Create an HTTP client for test fixture downloads.
pub(crate) fn fixture_client() -> Result<BaseClient> {
    let trusted_hosts = env::var(EnvVars::UV_INSECURE_HOST)
        .unwrap_or_default()
        .split_whitespace()
        .map(TrustedHost::from_str)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(BaseClientBuilder::default()
        .allow_insecure_host(trusted_hosts)
        .build()?)
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;
    use uv_client::BaseClientBuilder;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::assert_path_missing;

    use super::download_to_disk_with_client;

    #[tokio::test]
    async fn rejects_http_error_before_creating_fixture() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing.whl"))
            .respond_with(ResponseTemplate::new(StatusCode::NOT_FOUND))
            .expect(1)
            .mount(&server)
            .await;
        let client = BaseClientBuilder::default()
            .custom_client(reqwest::Client::builder().no_proxy().build()?)
            .build()?;
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("fixture.whl");

        let error = download_to_disk_with_client(
            &client,
            &format!("{}/missing.whl", server.uri()),
            &destination,
        )
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
