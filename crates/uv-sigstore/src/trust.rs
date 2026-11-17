//! Public log keys for checking the evidence that uv is about to upload.
//!
//! We trust HTTPS here rather than verifying TUF metadata. A compromised metadata host can
//! substitute or replay keys: uv might reject a good receipt or accept a bad one. These keys
//! never select a service URL, receive an OIDC token, or decide what uv signs. PyPI verifies
//! the attestation against its own trust roots, so fooling this check cannot authorize a
//! different publisher or make PyPI accept a forged signature.
//!
//! This is a signing-time check. Do not use these keys as the basis for trusting a download.

use base64::{Engine as _, prelude::BASE64_STANDARD};
use jiff::Timestamp;
use rustls_pki_types::SubjectPublicKeyInfoDer;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::REKOR_ORIGIN;

/// The subset of Sigstore's trust document needed when signing.
#[derive(Debug, Deserialize, Serialize)]
pub struct TrustedRoot {
    #[serde(rename = "mediaType")]
    media_type: MediaType,
    tlogs: Vec<Log>,
    ctlogs: Vec<Log>,
}

#[derive(Debug, Deserialize, Serialize)]
enum MediaType {
    #[serde(rename = "application/vnd.dev.sigstore.trustedroot+json;version=0.1")]
    TrustedRoot,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Log {
    base_url: String,
    public_key: Key,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Key {
    raw_bytes: String,
    #[serde(rename = "keyDetails")]
    algorithm: String,
    valid_for: Validity,
}

#[derive(Debug, Deserialize, Serialize)]
struct Validity {
    start: Timestamp,
    end: Option<Timestamp>,
}

impl Validity {
    fn contains(&self, time: Timestamp) -> bool {
        self.start <= time && self.end.is_none_or(|end| time <= end)
    }
}

impl Key {
    fn current(&self, now: Timestamp) -> Option<SubjectPublicKeyInfoDer<'static>> {
        if self.algorithm != "PKIX_ECDSA_P256_SHA_256" || !self.valid_for.contains(now) {
            return None;
        }
        Some(BASE64_STANDARD.decode(&self.raw_bytes).ok()?.into())
    }
}

impl TrustedRoot {
    /// Fetch over HTTPS without credentials. The returned document supplies keys, not endpoints.
    pub const URL: &str =
        "https://raw.githubusercontent.com/sigstore/root-signing/main/targets/trusted_root.json";

    pub(super) fn rekor_key(&self) -> Option<SubjectPublicKeyInfoDer<'static>> {
        let now = Timestamp::now();
        self.tlogs
            .iter()
            .filter(|log| log.base_url == REKOR_ORIGIN)
            .find_map(|log| log.public_key.current(now))
    }

    pub(super) fn ct_key(
        &self,
        id: &[u8; 32],
        signed_at: Timestamp,
    ) -> Option<SubjectPublicKeyInfoDer<'static>> {
        let now = Timestamp::now();
        self.ctlogs
            .iter()
            // A retired key may verify old releases, but must not vouch for a new signature.
            .filter(|log| log.public_key.valid_for.contains(signed_at))
            .filter_map(|log| log.public_key.current(now))
            .find(|key| Sha256::digest(key.as_ref()).as_slice() == id)
    }
}
