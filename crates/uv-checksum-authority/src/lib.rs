//! Signed checksum records and an archive-verifying HTTP client.

mod protocol;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_util::io::ReaderStream;
use url::{Host, Url};

use protocol::insert_record;

pub use protocol::{
    ArtifactId, AuthorityPublicKey, ChecksumRecord, Sha256Digest, SignedRecord,
    VerificationReceipt, VerifiedRecord,
};

const MAX_RESPONSE_SIZE: usize = 16 * 1024;

/// A configured authority and an independently supplied Ed25519 verification key.
#[derive(Debug, Clone)]
pub struct ChecksumAuthority {
    endpoint: Url,
    public_key: AuthorityPublicKey,
    client: reqwest::Client,
    records: Arc<Mutex<BTreeMap<ArtifactId, ChecksumRecord>>>,
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
            records: Arc::default(),
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
        let verified = signed.verify(artifact, &self.public_key)?;
        let mut records = self.records.lock().await;
        insert_record(&mut records, verified.record().clone())?;
        Ok(verified)
    }

    pub fn public_key(&self) -> AuthorityPublicKey {
        self.public_key
    }

    /// Capture all authorizations observed by clients sharing this authority configuration.
    /// A conservative superset also covers dependencies resolved before a build starts.
    pub async fn receipt(&self) -> Result<VerificationReceipt, Error> {
        self.records
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .try_into()
    }

    /// Reauthorize a cached build's inputs against the current authority.
    pub async fn verify_receipt(&self, receipt: &VerificationReceipt) -> Result<(), Error> {
        for record in receipt.records() {
            let current = self.lookup(record.artifact()).await?;
            if current.record().sha256() != record.sha256() {
                return Err(Error::Mismatch {
                    filename: record.artifact().filename().to_owned(),
                    expected: current.record().sha256(),
                    actual: record.sha256(),
                });
            }
            if current.record().size() != record.size() {
                return Err(Error::SizeMismatch {
                    filename: record.artifact().filename().to_owned(),
                    expected: current.record().size(),
                });
            }
        }
        Ok(())
    }

    /// Spool the complete response to disk and authenticate it before exposing any archive bytes.
    /// The original HTTP response parts are retained for downstream cache and download handling.
    pub async fn verify_response(
        &self,
        response: reqwest::Response,
        artifact: &ArtifactId,
        temporary_directory: &Path,
    ) -> Result<reqwest::Response, Error> {
        self.lookup(artifact)
            .await?
            .verify_response(response, temporary_directory)
            .await
    }
}

impl VerifiedRecord {
    /// Authenticate complete archive bytes against this already verified authority record.
    pub async fn verify_response(
        &self,
        response: reqwest::Response,
        temporary_directory: &Path,
    ) -> Result<reqwest::Response, Error> {
        let record = self.record();
        let artifact = record.artifact();
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
    #[error("Checksum authority verification is unavailable in offline mode")]
    Offline,
    #[error("Checksum authority build receipt must contain at least one archive")]
    InvalidReceipt,
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
    #[error("Conflicting checksum authority records for `{0}`")]
    ConflictingRecord(String),
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
