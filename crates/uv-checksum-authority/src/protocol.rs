use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::signature::{self, Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::Error;

const SIGNATURE_CONTEXT: &[u8] = b"uv-checksum-authority/v1\n";
const MAX_SOURCE_LENGTH: usize = 2048;
const MAX_FILENAME_LENGTH: usize = 255;

/// The canonical registry namespace and exact archive filename.
/// Direct requirements use the archive URL as their namespace.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "WireArtifactId")]
pub struct ArtifactId {
    source: String,
    filename: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireArtifactId {
    source: String,
    filename: String,
}

impl ArtifactId {
    pub fn new(source: &Url, filename: &str) -> Result<Self, Error> {
        if !matches!(source.scheme(), "http" | "https")
            || source.host_str().is_none()
            || source.query().is_some()
            || filename.is_empty()
            || filename.len() > MAX_FILENAME_LENGTH
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
        if source.as_str().len() > MAX_SOURCE_LENGTH {
            return Err(Error::InvalidIdentity);
        }
        Ok(Self {
            source: source.as_str().trim_end_matches('/').to_owned(),
            filename: filename.to_owned(),
        })
    }

    /// Parse an identity received over the wire, requiring its canonical spelling.
    pub fn from_canonical(source: &str, filename: &str) -> Result<Self, Error> {
        let url = Url::parse(source).map_err(|_| Error::InvalidIdentity)?;
        let artifact = Self::new(&url, filename)?;
        if artifact.source != source {
            return Err(Error::InvalidIdentity);
        }
        Ok(artifact)
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }
}

impl TryFrom<WireArtifactId> for ArtifactId {
    type Error = Error;

    fn try_from(value: WireArtifactId) -> Result<Self, Self::Error> {
        Self::from_canonical(&value.source, &value.filename)
    }
}

/// A SHA-256 digest, serialized as lowercase hexadecimal.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<[u8; 32]> for Sha256Digest {
    fn from(bytes: [u8; 32]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl FromStr for Sha256Digest {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(Error::InvalidDigest);
        }
        let mut bytes = [0; 32];
        hex::decode_to_slice(value, &mut bytes).map_err(|_| Error::InvalidDigest)?;
        Ok(Self(bytes))
    }
}

impl TryFrom<String> for Sha256Digest {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Sha256Digest> for String {
    fn from(value: Sha256Digest) -> Self {
        value.to_string()
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// An independently distributed Ed25519 verification key.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct AuthorityPublicKey([u8; 32]);

impl AuthorityPublicKey {
    pub fn from_signing_key(key: &Ed25519KeyPair) -> Self {
        let mut bytes = [0; 32];
        bytes.copy_from_slice(key.public_key().as_ref());
        Self(bytes)
    }
}

impl FromStr for AuthorityPublicKey {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut bytes = [0; 32];
        hex::decode_to_slice(value, &mut bytes).map_err(|_| Error::InvalidPublicKey)?;
        Ok(Self(bytes))
    }
}

impl fmt::Display for AuthorityPublicKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

/// The digest and length of the complete, compressed archive bytes.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecksumRecord {
    artifact: ArtifactId,
    sha256: Sha256Digest,
    size: u64,
}

impl ChecksumRecord {
    pub fn new(artifact: ArtifactId, sha256: Sha256Digest, size: u64) -> Self {
        Self {
            artifact,
            sha256,
            size,
        }
    }

    pub fn artifact(&self) -> &ArtifactId {
        &self.artifact
    }

    pub fn sha256(&self) -> Sha256Digest {
        self.sha256
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

/// A record whose signature and requested artifact identity have been checked.
#[derive(Debug, Clone)]
pub struct VerifiedRecord(ChecksumRecord);

impl VerifiedRecord {
    pub fn record(&self) -> &ChecksumRecord {
        &self.0
    }
}

/// The archive authorizations observed while producing a cached build artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Vec<ChecksumRecord>", into = "Vec<ChecksumRecord>")]
pub struct VerificationReceipt(BTreeSet<ChecksumRecord>);

impl VerificationReceipt {
    pub(crate) fn records(&self) -> &BTreeSet<ChecksumRecord> {
        &self.0
    }
}

impl TryFrom<Vec<ChecksumRecord>> for VerificationReceipt {
    type Error = Error;

    fn try_from(records: Vec<ChecksumRecord>) -> Result<Self, Self::Error> {
        if records.is_empty() {
            return Err(Error::InvalidReceipt);
        }
        Ok(Self(records.into_iter().collect()))
    }
}

impl From<VerificationReceipt> for Vec<ChecksumRecord> {
    fn from(receipt: VerificationReceipt) -> Self {
        receipt.0.into_iter().collect()
    }
}

/// A signed payload. Deserialization decodes the envelope; verification authenticates its bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "WireSignedRecord", into = "WireSignedRecord")]
pub struct SignedRecord {
    payload: Vec<u8>,
    signature: [u8; 64],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSignedRecord {
    payload: String,
    signature: String,
}

impl TryFrom<WireSignedRecord> for SignedRecord {
    type Error = Error;

    fn try_from(value: WireSignedRecord) -> Result<Self, Self::Error> {
        let payload = STANDARD
            .decode(value.payload)
            .map_err(|_| Error::InvalidSignature)?;
        let signature = STANDARD
            .decode(value.signature)
            .map_err(|_| Error::InvalidSignature)?;
        Ok(Self {
            payload,
            signature: signature.try_into().map_err(|_| Error::InvalidSignature)?,
        })
    }
}

impl From<SignedRecord> for WireSignedRecord {
    fn from(value: SignedRecord) -> Self {
        Self {
            payload: STANDARD.encode(value.payload),
            signature: STANDARD.encode(value.signature),
        }
    }
}

impl SignedRecord {
    pub fn sign(record: &ChecksumRecord, key: &Ed25519KeyPair) -> Result<Self, Error> {
        let payload = serde_json::to_vec(record)?;
        let mut signature = [0; 64];
        signature.copy_from_slice(key.sign(&signature_message(&payload)).as_ref());
        Ok(Self { payload, signature })
    }

    pub fn verify(
        &self,
        artifact: &ArtifactId,
        public_key: &AuthorityPublicKey,
    ) -> Result<VerifiedRecord, Error> {
        signature::UnparsedPublicKey::new(&signature::ED25519, public_key.0)
            .verify(&signature_message(&self.payload), &self.signature)
            .map_err(|_| Error::InvalidSignature)?;
        let record: ChecksumRecord = serde_json::from_slice(&self.payload)?;
        if record.artifact() != artifact {
            return Err(Error::WrongArtifact);
        }
        Ok(VerifiedRecord(record))
    }
}

fn signature_message(payload: &[u8]) -> Vec<u8> {
    [SIGNATURE_CONTEXT, payload].concat()
}
