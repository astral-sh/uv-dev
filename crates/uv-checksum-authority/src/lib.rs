//! An experimental, signed checksum catalog. This is not a transparency-log protocol.

use std::path::Path;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::signature::{self, Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use url::{Host, Url};

const SIGNATURE_CONTEXT: &[u8] = b"uv-checksum-authority/v1\n";
const MAX_RESPONSE_SIZE: usize = 16 * 1024;

/// The registry namespace and exact archive filename. A direct URL uses the archive URL as its
/// namespace. Credentials and query strings are never sent to the authority.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactId {
    pub source: String,
    pub filename: String,
}

impl ArtifactId {
    pub fn new(source: &Url, filename: &str) -> Result<Self, Error> {
        if !matches!(source.scheme(), "http" | "https")
            || source.host_str().is_none()
            || source.query().is_some()
            || filename.is_empty()
            || filename == "."
            || filename == ".."
            || filename.contains(['/', '\\'])
            || filename.chars().any(char::is_control)
        {
            return Err(Error::InvalidIdentity);
        }
        let mut source = source.clone();
        source
            .set_username("")
            .map_err(|()| Error::InvalidIdentity)?;
        source
            .set_password(None)
            .map_err(|()| Error::InvalidIdentity)?;
        source.set_fragment(None);
        Ok(Self {
            source: source.as_str().trim_end_matches('/').to_owned(),
            filename: filename.to_owned(),
        })
    }

    pub fn validate(&self) -> Result<(), Error> {
        let source = Url::parse(&self.source).map_err(|_| Error::InvalidIdentity)?;
        if Self::new(&source, &self.filename)? != *self {
            return Err(Error::InvalidIdentity);
        }
        Ok(())
    }
}

/// A catalog entry. The SHA-256 digest covers the complete, compressed archive bytes.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecksumRecord {
    pub artifact: ArtifactId,
    pub sha256: String,
}

impl ChecksumRecord {
    pub fn validate(&self) -> Result<(), Error> {
        self.artifact.validate()?;
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::InvalidDigest);
        }
        Ok(())
    }
}

/// The signature covers the decoded payload, including a protocol-specific domain separator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRecord {
    pub payload: String,
    pub signature: String,
}

impl SignedRecord {
    pub fn sign(record: &ChecksumRecord, key: &Ed25519KeyPair) -> Result<Self, Error> {
        record.validate()?;
        let payload = serde_json::to_vec(record)?;
        let message = signature_message(&payload);
        Ok(Self {
            payload: STANDARD.encode(payload),
            signature: STANDARD.encode(key.sign(&message).as_ref()),
        })
    }

    pub fn verify(
        &self,
        artifact: &ArtifactId,
        public_key: &[u8; 32],
    ) -> Result<ChecksumRecord, Error> {
        let payload = STANDARD
            .decode(&self.payload)
            .map_err(|_| Error::InvalidSignature)?;
        let signature = STANDARD
            .decode(&self.signature)
            .map_err(|_| Error::InvalidSignature)?;
        signature::UnparsedPublicKey::new(&signature::ED25519, public_key)
            .verify(&signature_message(&payload), &signature)
            .map_err(|_| Error::InvalidSignature)?;
        let record: ChecksumRecord = serde_json::from_slice(&payload)?;
        record.validate()?;
        if record.artifact != *artifact {
            return Err(Error::WrongArtifact);
        }
        Ok(record)
    }
}

fn signature_message(payload: &[u8]) -> Vec<u8> {
    [SIGNATURE_CONTEXT, payload].concat()
}

/// A configured authority and an independently supplied Ed25519 verification key.
#[derive(Debug, Clone)]
pub struct ChecksumAuthority {
    endpoint: Url,
    public_key: [u8; 32],
    client: reqwest::Client,
}

impl ChecksumAuthority {
    pub fn new(mut endpoint: Url, public_key: &str) -> Result<Self, Error> {
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
        let mut key = [0; 32];
        hex::decode_to_slice(public_key, &mut key).map_err(|_| Error::InvalidPublicKey)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            endpoint,
            public_key: key,
            client,
        })
    }

    pub async fn lookup(&self, artifact: &ArtifactId) -> Result<ChecksumRecord, Error> {
        artifact.validate()?;
        let endpoint = self
            .endpoint
            .join("v1/checksum")
            .map_err(|_| Error::InvalidEndpoint)?;
        let mut response = self.client.get(endpoint).query(artifact).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::UnknownArtifact(artifact.filename.clone()));
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
        let record = self.lookup(artifact).await?;
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
        while let Some(chunk) = body.chunk().await? {
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        let actual = hex::encode(hasher.finalize());
        if actual != record.sha256 {
            return Err(Error::Mismatch {
                filename: artifact.filename.clone(),
                expected: record.sha256,
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

pub fn public_key_hex(key: &Ed25519KeyPair) -> String {
    hex::encode(key.public_key().as_ref())
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
        expected: String,
        actual: String,
    },
    #[error("Checksum authority request failed")]
    Http(#[from] reqwest::Error),
    #[error("Invalid checksum authority response")]
    Json(#[from] serde_json::Error),
    #[error("Failed to buffer an archive for checksum verification")]
    Io(#[from] std::io::Error),
}
