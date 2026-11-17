//! Authenticate Sigstore's `trusted_root.json` target using the production TUF repository.
use std::path::Path;

use base64::{Engine as _, prelude::BASE64_STANDARD};
use futures::TryStreamExt as _;
use jiff::Timestamp;
use reqwest::StatusCode;
use reqwest_middleware::ClientWithMiddleware;
use rustls_pki_types::SubjectPublicKeyInfoDer;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tough::{
    IntoVec, RepositoryLoader, TargetName, Transport, TransportError, TransportErrorKind,
    TransportStream,
};
use uv_redacted::{SafeUrl, SafeUrlError};

/// Bootstrap for verifying metadata updates:
/// <https://github.com/sigstore/root-signing/blob/a180c17821f8316a70d834d4d9eec0b4ecb1922c/metadata/root_history/16.root.json>.
const ROOT: &[u8] = include_bytes!("root.json");
const REPOSITORY: &str = "https://tuf-repo-cdn.sigstore.dev/";
const TARGET: &str = "trusted_root.json";

#[derive(Debug, Error)]
pub enum TrustError {
    #[error("Failed to authenticate Sigstore trust metadata")]
    Repository(#[source] Box<tough::error::Error>),
    #[error("Sigstore trust repository is missing trusted_root.json")]
    Missing,
    #[error("Invalid Sigstore trust repository URL")]
    Url(#[from] SafeUrlError),
    #[error("Failed to write cached trusted_root.json")]
    Cache(#[from] std::io::Error),
    #[error("Invalid trusted_root.json")]
    Json(#[from] serde_json::Error),
    #[error("No valid log key for {0} in trusted_root.json")]
    Key(String),
}

impl From<tough::error::Error> for TrustError {
    fn from(error: tough::error::Error) -> Self {
        Self::Repository(Box::new(error))
    }
}

/// Signing keys from an authenticated Sigstore trust root. Reuse across a publish session.
#[derive(Debug)]
pub struct TrustedRoot {
    pub(super) rekor: SubjectPublicKeyInfoDer<'static>,
    ctlogs: Vec<Log>,
}

#[derive(Deserialize)]
struct Document {
    #[serde(rename = "mediaType")]
    _media_type: MediaType,
    tlogs: Vec<Log>,
    ctlogs: Vec<Log>,
}

#[derive(Deserialize)]
enum MediaType {
    #[serde(rename = "application/vnd.dev.sigstore.trustedroot+json;version=0.1")]
    TrustedRoot,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Log {
    base_url: String,
    public_key: Key,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Key {
    raw_bytes: String,
    #[serde(rename = "keyDetails")]
    algorithm: String,
    valid_for: Validity,
}

#[derive(Debug, Deserialize)]
struct Validity {
    start: Timestamp,
    end: Option<Timestamp>,
}

impl Validity {
    fn contains(&self, time: Timestamp) -> bool {
        time >= self.start && self.end.is_none_or(|end| time <= end)
    }
}

impl Log {
    fn key_at(&self, time: Timestamp) -> Option<SubjectPublicKeyInfoDer<'static>> {
        let key = &self.public_key;
        if key.algorithm != "PKIX_ECDSA_P256_SHA_256" || !key.valid_for.contains(time) {
            return None;
        }
        Some(BASE64_STANDARD.decode(&key.raw_bytes).ok()?.into())
    }
}

impl TrustedRoot {
    /// Refresh signed metadata and authenticate the target before parsing it.
    ///
    /// An optional, existing cache directory retains TUF metadata for rollback protection.
    /// Callers sharing that directory must hold an exclusive lock for the duration of the fetch.
    pub async fn fetch(
        client: &ClientWithMiddleware,
        cache: Option<&Path>,
    ) -> Result<Self, TrustError> {
        let base = SafeUrl::parse(REPOSITORY)?;
        let loader = RepositoryLoader::new(
            &ROOT,
            base.clone().into_unsafe_url(),
            base.join("targets/")?.into_unsafe_url(),
        )
        .transport(Http(client.clone()));
        let loader = match cache {
            Some(dir) => loader.datastore(dir),
            None => loader,
        };
        let repository = loader.load().await?;
        let target = TargetName::new(TARGET)?;
        let info = repository
            .targets()
            .signed
            .targets
            .get(&target)
            .ok_or(TrustError::Missing)?;
        let cached = cache.map(|dir| dir.join(TARGET));
        if let Some(path) = &cached
            && let Ok(bytes) = fs_err::tokio::read(path).await
            && bytes.len() as u64 == info.length
            && Sha256::digest(&bytes).as_slice() == &info.hashes.sha256[..]
        {
            return Self::parse(&bytes, Timestamp::now());
        }
        let bytes = repository
            .read_target(&target)
            .await?
            .ok_or(TrustError::Missing)?
            .into_vec()
            .await?;
        let root = Self::parse(&bytes, Timestamp::now())?;
        if let Some(path) = cached {
            // An interrupted write is rejected by the length/hash check on the next load.
            fs_err::tokio::write(path, bytes).await?;
        }
        Ok(root)
    }

    pub(super) fn parse(bytes: &[u8], now: Timestamp) -> Result<Self, TrustError> {
        let document: Document = serde_json::from_slice(bytes)?;
        let rekor = document
            .tlogs
            .iter()
            .filter(|log| log.base_url == super::REKOR_ORIGIN)
            .find_map(|log| log.key_at(now))
            .ok_or_else(|| TrustError::Key(super::REKOR_ORIGIN.to_string()))?;
        Ok(Self {
            rekor,
            // A retired log may verify old attestations, but must not vouch for a new signature.
            ctlogs: document
                .ctlogs
                .into_iter()
                .filter(|log| log.public_key.valid_for.contains(now))
                .collect(),
        })
    }

    pub(super) fn ct_key(
        &self,
        id: &[u8; 32],
        time: Timestamp,
    ) -> Option<SubjectPublicKeyInfoDer<'static>> {
        self.ctlogs
            .iter()
            .filter_map(|log| log.key_at(time))
            .find(|key| Sha256::digest(key.as_ref()).as_slice() == id)
    }
}

/// Send TUF's bounded streaming reads through uv's configured retries, proxy and TLS settings.
#[derive(Debug, Clone)]
struct Http(ClientWithMiddleware);

#[tough::async_trait]
impl Transport for Http {
    #[expect(
        clippy::disallowed_types,
        reason = "tough's Transport trait requires url::Url"
    )]
    async fn fetch(&self, url: url::Url) -> Result<TransportStream, TransportError> {
        let kind = TransportErrorKind::Other;
        let response = self
            .0
            .get(url.clone())
            .send()
            .await
            .map_err(|err| TransportError::new_with_cause(kind, url.as_str(), err))?;
        // A missing next version of root.json ends TUF's root rotation loop.
        if response.status() == StatusCode::NOT_FOUND {
            return Err(TransportError::new(
                TransportErrorKind::FileNotFound,
                url.as_str(),
            ));
        }
        let response = response
            .error_for_status()
            .map_err(|err| TransportError::new_with_cause(kind, url.as_str(), err))?;
        Ok(Box::pin(response.bytes_stream().map_err(move |err| {
            TransportError::new_with_cause(kind, url.as_str(), err)
        })))
    }
}
