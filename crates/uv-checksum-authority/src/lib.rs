//! Signed checksum records and an archive-verifying HTTP client.

mod protocol;

use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use url::{Host, Url};

pub use protocol::{
    ArtifactId, AuthorityPublicKey, ChecksumRecord, Sha256Digest, SignedRecord, VerifiedRecord,
};

const MAX_RESPONSE_SIZE: usize = 16 * 1024;

/// A configured authority and an independently supplied Ed25519 verification key.
#[derive(Debug, Clone)]
pub struct ChecksumAuthority {
    endpoint: Url,
    public_key: AuthorityPublicKey,
    client: reqwest::Client,
}

impl ChecksumAuthority {
    pub fn new(mut endpoint: Url, public_key: AuthorityPublicKey) -> Result<Self, Error> {
        let loopback = match endpoint.host() {
            Some(Host::Domain("localhost")) => true,
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            _ => false,
        };
        if !(endpoint.scheme() == "https" || (endpoint.scheme() == "http" && loopback))
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(Error::InvalidEndpoint);
        }
        if !endpoint.path().ends_with('/') {
            endpoint.set_path(&format!("{}/", endpoint.path()));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            endpoint,
            public_key,
            client,
        })
    }

    pub async fn lookup(&self, artifact: &ArtifactId) -> Result<VerifiedRecord, Error> {
        let endpoint = self
            .endpoint
            .join("v1/checksum")
            .map_err(|_| Error::InvalidEndpoint)?;
        let mut response = self.client.get(endpoint).query(artifact).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::UnknownArtifact(artifact.filename().to_owned()));
        }
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::AuthorityStatus(response.status()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_RESPONSE_SIZE {
                return Err(Error::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let signed: SignedRecord = serde_json::from_slice(&body)?;
        signed.verify(artifact, &self.public_key)
    }

    /// Spool the complete response to disk and authenticate it before exposing any archive bytes.
    /// The original HTTP response parts are retained for downstream cache and download handling.
    pub async fn verify_response(
        &self,
        response: reqwest::Response,
        artifact: &ArtifactId,
        temporary_directory: &Path,
    ) -> Result<reqwest::Response, Error> {
        let verified = self.lookup(artifact).await?;
        let record = verified.record();
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::ArtifactStatus(response.status()));
        }
        let response: http::Response<reqwest::Body> = response.into();
        let (parts, body) = response.into_parts();
        let mut body = reqwest::Response::from(http::Response::new(body));
        let mut file = fs_err::tokio::File::from_std(fs_err::File::from_parts(
            tempfile::tempfile_in(temporary_directory)?,
            temporary_directory,
        ));
        let mut hasher = Sha256::new();
        let mut remaining = record.size();
        while let Some(chunk) = body.chunk().await? {
            let Some(next_remaining) = remaining.checked_sub(chunk.len() as u64) else {
                return Err(Error::SizeMismatch {
                    filename: artifact.filename().to_owned(),
                    expected: record.size(),
                });
            };
            remaining = next_remaining;
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        if remaining != 0 {
            return Err(Error::SizeMismatch {
                filename: artifact.filename().to_owned(),
                expected: record.size(),
            });
        }
        let actual = Sha256Digest::from_bytes(hasher.finalize().into());
        if actual != record.sha256() {
            return Err(Error::Mismatch {
                filename: artifact.filename().to_owned(),
                expected: record.sha256(),
                actual,
            });
        }
        file.rewind().await?;
        Ok(
            http::Response::from_parts(parts, reqwest::Body::wrap_stream(ReaderStream::new(file)))
                .into(),
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid checksum authority artifact identity")]
    InvalidIdentity,
    #[error("Checksum authority records require a lowercase SHA-256 digest")]
    InvalidDigest,
    #[error(
        "Checksum authority must use HTTPS (HTTP is allowed only on loopback), without credentials, query, or fragment"
    )]
    InvalidEndpoint,
    #[error("Checksum authority key must be a 32-byte hexadecimal Ed25519 public key")]
    InvalidPublicKey,
    #[error("Checksum authority signature verification failed")]
    InvalidSignature,
    #[error("Checksum authority returned a record for a different artifact")]
    WrongArtifact,
    #[error("Checksum authority has no trusted record for `{0}`")]
    UnknownArtifact(String),
    #[error("Checksum authority returned HTTP {0}")]
    AuthorityStatus(reqwest::StatusCode),
    #[error("Expected a complete archive response, received HTTP {0}")]
    ArtifactStatus(reqwest::StatusCode),
    #[error("Checksum authority response exceeds the size limit")]
    ResponseTooLarge,
    #[error(
        "Checksum authority mismatch for `{filename}`: expected sha256:{expected}, received sha256:{actual}"
    )]
    Mismatch {
        filename: String,
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
    #[error("Checksum authority size mismatch for `{filename}`: expected {expected} bytes")]
    SizeMismatch { filename: String, expected: u64 },
    #[error("Checksum authority request failed")]
    Http(#[from] reqwest::Error),
    #[error("Invalid checksum authority response")]
    Json(#[from] serde_json::Error),
    #[error("Failed to buffer an archive for checksum verification")]
    Io(#[from] std::io::Error),
}
