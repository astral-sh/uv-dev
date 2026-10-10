use std::path::Path;

use futures::TryStreamExt;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use url::Url;

use uv_extract::hash::{HashReader, Hasher};
use uv_pypi_types::{HashAlgorithm, HashDigest};
use uv_redacted::DisplaySafeUrl;

use crate::{RegistryClient, WrappedReqwestError};

/// The digest and byte count from the same complete read of a distribution file.
#[derive(Debug)]
pub struct FileHash {
    pub digest: HashDigest,
    pub size: u64,
}

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
    pub async fn hash_file(&self, url: &DisplaySafeUrl) -> Result<FileHash, FileHashError> {
        let mut hashers = [Hasher::from(HashAlgorithm::Sha256)];
        let size = if url.scheme() == "file" {
            let path = url.to_file_path().map_err(|()| FileHashError::UrlToPath)?;
            let file = fs_err::tokio::File::open(&path)
                .await
                .map_err(|err| FileHashError::ReadFile(path.clone().into_boxed_path(), err))?;
            let mut reader = HashReader::new(file, &mut hashers);
            reader
                .finish()
                .await
                .map_err(|err| FileHashError::ReadFile(path.into_boxed_path(), err))?;
            reader.bytes_read()
        } else {
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
                    FileHashError::DownloadFile(
                        Box::new(url.clone()),
                        WrappedReqwestError::from(err),
                    )
                })?;
            let reader = response
                .bytes_stream()
                .map_err(std::io::Error::other)
                .into_async_read();
            let mut reader = HashReader::new(reader.compat(), &mut hashers);
            reader
                .finish()
                .await
                .map_err(|err| FileHashError::StreamFile(Box::new(url.clone()), err))?;
            reader.bytes_read()
        };
        let [hasher] = hashers;
        Ok(FileHash {
            digest: HashDigest::from(hasher),
            size,
        })
    }
}
