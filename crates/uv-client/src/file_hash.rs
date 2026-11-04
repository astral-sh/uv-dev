use std::path::Path;

use futures::TryStreamExt;
use tokio_util::compat::FuturesAsyncReadCompatExt;

use uv_extract::hash::{HashReader, Hasher};
use uv_pypi_types::{HashAlgorithm, HashDigest};
use uv_redacted::SafeUrl;

use crate::{RegistryClient, WrappedReqwestError};

/// An error while reading or downloading a distribution file to compute its hash.
#[derive(Debug, thiserror::Error)]
pub enum FileHashError {
    #[error("Failed to convert URL to path")]
    UrlToPath,
    // Request and status errors use `WrappedReqwestError`, while errors reading the response body
    // use `io::Error`. Both are download failures.
    #[error("Failed to download `{0}` to compute artifact hashes")]
    DownloadFile(Box<SafeUrl>, #[source] WrappedReqwestError),
    #[error("Failed to download `{0}` to compute artifact hashes")]
    StreamFile(Box<SafeUrl>, #[source] std::io::Error),
    #[error("Failed to read `{0}` to compute artifact hashes")]
    ReadFile(Box<Path>, #[source] std::io::Error),
}

impl RegistryClient {
    /// Read or download a file and compute its SHA-256 digest without extracting its contents.
    pub async fn hash_file(&self, url: &SafeUrl) -> Result<HashDigest, FileHashError> {
        let [digest] = self.hash_file_with(url, [HashAlgorithm::Sha256]).await?;
        Ok(digest)
    }

    /// Compute several digests from the same byte stream.
    pub async fn hash_file_with<const N: usize>(
        &self,
        url: &SafeUrl,
        algorithms: [HashAlgorithm; N],
    ) -> Result<[HashDigest; N], FileHashError> {
        let mut hashers = algorithms.map(Hasher::from);
        if url.scheme() == "file" {
            let path = url.to_file_path().map_err(|()| FileHashError::UrlToPath)?;
            let file = fs_err::tokio::File::open(&path)
                .await
                .map_err(|err| FileHashError::ReadFile(path.clone().into_boxed_path(), err))?;
            HashReader::new(file, &mut hashers)
                .finish()
                .await
                .map_err(|err| FileHashError::ReadFile(path.into_boxed_path(), err))?;
        } else {
            let response = self
                .uncached_client(url)
                .get(url.clone())
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
                    FileHashError::DownloadFile(
                        Box::new(url.clone()),
                        WrappedReqwestError::from(err),
                    )
                })?;
            let reader = response
                .bytes_stream()
                .map_err(std::io::Error::other)
                .into_async_read();
            HashReader::new(reader.compat(), &mut hashers)
                .finish()
                .await
                .map_err(|err| FileHashError::StreamFile(Box::new(url.clone()), err))?;
        }
        Ok(hashers.map(HashDigest::from))
    }
}
