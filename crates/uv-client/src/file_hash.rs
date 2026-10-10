use std::io::{self, Read};
use std::path::Path;

use futures::TryStreamExt;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::Span;
use url::Url;

use uv_extract::hash::{HashReader, Hasher};
use uv_pypi_types::{HashAlgorithm, HashDigest};
use uv_redacted::DisplaySafeUrl;

use crate::{RegistryClient, WrappedReqwestError};

/// An error while reading or downloading a distribution file to compute its hash.
#[derive(Debug, thiserror::Error)]
pub enum FileHashError {
    #[error("Failed to convert URL to path")]
    UrlToPath,
    // Request and status errors use `WrappedReqwestError`, while errors reading the response body
    // use `io::Error`. Both are download failures.
    #[error("Failed to download `{0}` to compute missing hashes")]
    DownloadFile(Box<DisplaySafeUrl>, #[source] WrappedReqwestError),
    #[error("Failed to download `{0}` to compute missing hashes")]
    StreamFile(Box<DisplaySafeUrl>, #[source] std::io::Error),
    #[error("Failed to read `{0}` to compute missing hashes")]
    ReadFile(Box<Path>, #[source] std::io::Error),
}

impl RegistryClient {
    /// Read or download a file and compute its SHA-256 digest without extracting its contents.
    pub async fn hash_file(&self, url: &DisplaySafeUrl) -> Result<HashDigest, FileHashError> {
        if url.scheme() == "file" {
            let path = url.to_file_path().map_err(|()| FileHashError::UrlToPath)?;
            let worker_path = path.clone();
            let span = Span::current();
            return tokio::task::spawn_blocking(move || {
                let _entered = span.enter();
                let mut file = fs_err::File::open(worker_path)?;
                let mut hasher = Hasher::from(HashAlgorithm::Sha256);
                let mut buffer = vec![0; 64 * 1024];
                loop {
                    let read = match file.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => read,
                        Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                        Err(err) => return Err(err),
                    };
                    hasher.update(&buffer[..read]);
                }
                Ok(HashDigest::from(hasher))
            })
            .await
            .map_err(|err| {
                FileHashError::ReadFile(path.clone().into_boxed_path(), io::Error::other(err))
            })?
            .map_err(|err| FileHashError::ReadFile(path.into_boxed_path(), err));
        }

        let mut hashers = [Hasher::from(HashAlgorithm::Sha256)];
        let response = self
            .uncached_client(url)
            .get(Url::from(url.clone()))
            .header(
                // `reqwest` defaults to accepting compressed responses.
                // Specify identity encoding to get consistent .whl downloading
                // behavior from servers. ref: https://github.com/pypa/pip/pull/1688
                "accept-encoding",
                reqwest::header::HeaderValue::from_static("identity"),
            )
            .send()
            .await
            .and_then(|response| response.error_for_status().map_err(Into::into))
            .map_err(|err| {
                FileHashError::DownloadFile(Box::new(url.clone()), WrappedReqwestError::from(err))
            })?;
        let reader = response
            .bytes_stream()
            .map_err(std::io::Error::other)
            .into_async_read();
        HashReader::new(reader.compat(), &mut hashers)
            .finish()
            .await
            .map_err(|err| FileHashError::StreamFile(Box::new(url.clone()), err))?;
        let [hasher] = hashers;
        Ok(HashDigest::from(hasher))
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::Duration;

    use uv_cache::Cache;
    use uv_extract::hash::{HashReader, Hasher};
    use uv_pypi_types::{HashAlgorithm, HashDigest};
    use uv_redacted::DisplaySafeUrl;

    use wiremock::matchers::{header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::{BaseClientBuilder, FileHashError, RegistryClient, RegistryClientBuilder};

    type Error = Box<dyn std::error::Error>;

    fn client() -> Result<RegistryClient, Error> {
        Ok(RegistryClientBuilder::new(BaseClientBuilder::default(), Cache::temp()?).build()?)
    }

    #[tokio::test]
    async fn local_hash_matches_streaming_hash() -> Result<(), Error> {
        let client = client()?;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("artifact.whl");
        let url = DisplaySafeUrl::from_file_path(&path).map_err(|()| "invalid fixture URL")?;
        for size in [0, 3, 8193, 65536, 65537, 1024 * 1024] {
            let data: Vec<_> = b"abc".iter().copied().cycle().take(size).collect();
            fs_err::write(&path, &data)?;
            let mut hashers = [Hasher::from(HashAlgorithm::Sha256)];
            HashReader::new(data.as_slice(), &mut hashers)
                .finish()
                .await?;
            let [expected] = hashers;
            assert_eq!(client.hash_file(&url).await?, HashDigest::from(expected));
        }

        fs_err::remove_file(&path)?;
        let error = client.hash_file(&url).await.expect_err("missing file");
        let FileHashError::ReadFile(error_path, source) = error else {
            return Err("expected a path-rich local read error".into());
        };
        assert_eq!(error_path.as_ref(), path);
        assert_eq!(source.kind(), io::ErrorKind::NotFound);
        Ok(())
    }

    #[tokio::test]
    async fn remote_hash_streams_identity_response() -> Result<(), Error> {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("accept-encoding", "identity"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc"))
            .expect(1)
            .mount(&server)
            .await;
        let url = DisplaySafeUrl::parse(&server.uri())?;
        let client = client()?;
        let hash = tokio::time::timeout(Duration::from_secs(10), client.hash_file(&url)).await??;
        assert_eq!(
            hash.to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_hash_follows_symlinks() -> Result<(), Error> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("source.whl");
        let link = directory.path().join("link.whl");
        fs_err::write(&path, b"abc")?;
        fs_err::os::unix::fs::symlink(&path, &link)?;
        let url = DisplaySafeUrl::from_file_path(&link).map_err(|()| "invalid fixture URL")?;
        let hash = client()?.hash_file(&url).await?;
        assert_eq!(
            hash.to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        Ok(())
    }
}
