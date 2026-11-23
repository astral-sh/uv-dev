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

use crate::SigningError;

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
    fn has_supported_algorithm(&self) -> bool {
        self.algorithm == "PKIX_ECDSA_P256_SHA_256"
    }

    fn decode(&self) -> Result<SubjectPublicKeyInfoDer<'static>, base64::DecodeError> {
        BASE64_STANDARD.decode(&self.raw_bytes).map(Into::into)
    }
}

#[derive(Debug)]
pub(super) struct RekorKey<'a> {
    pub(super) id: [u8; 32],
    pub(super) der: SubjectPublicKeyInfoDer<'static>,
    valid_for: &'a Validity,
}

impl RekorKey<'_> {
    pub(super) fn valid_at(&self, seconds: i64) -> bool {
        Timestamp::from_second(seconds).is_ok_and(|time| self.valid_for.contains(time))
    }
}

impl TrustedRoot {
    /// URL of the Sigstore trust document. Fetch it without sending the OIDC token.
    ///
    /// The document lists public keys for verifying log signatures. Its URLs identify which log
    /// a key belongs to; they do not determine where uv sends requests.
    pub const URL: &str =
        "https://raw.githubusercontent.com/sigstore/root-signing/main/targets/trusted_root.json";

    pub(super) fn rekor_keys(
        &self,
        origin: &'static str,
    ) -> Result<Vec<RekorKey<'_>>, SigningError> {
        let now = Timestamp::now();
        let mut keys = Vec::new();
        let mut error = SigningError::MissingRekorKey { origin };
        for log in &self.tlogs {
            if log.base_url == origin
                && log.public_key.has_supported_algorithm()
                && log.public_key.valid_for.contains(now)
            {
                // Validity windows can overlap. The receipt identifies the signing key.
                match log.public_key.decode() {
                    Ok(der) => keys.push(RekorKey {
                        id: Sha256::digest(&der).into(),
                        der,
                        valid_for: &log.public_key.valid_for,
                    }),
                    Err(source) => error = SigningError::RekorKeyEncoding { origin, source },
                }
            }
        }
        if keys.is_empty() {
            Err(error)
        } else {
            Ok(keys)
        }
    }

    pub(super) fn ct_key(
        &self,
        id: &[u8; 32],
        signed_at: Timestamp,
    ) -> Option<SubjectPublicKeyInfoDer<'static>> {
        let now = Timestamp::now();
        self.ctlogs
            .iter()
            .filter(|log| log.public_key.has_supported_algorithm())
            // A retired key may verify old releases, but must not vouch for a new signature.
            .filter(|log| log.public_key.valid_for.contains(signed_at))
            .filter(|log| log.public_key.valid_for.contains(now))
            .filter_map(|log| log.public_key.decode().ok())
            .find(|key| Sha256::digest(key.as_ref()).as_slice() == id)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::TrustedRoot;

    #[test]
    fn malformed_key_falls_back_during_rotation() -> Result<(), serde_json::Error> {
        let origin = "https://rekor.example";
        let mut trust: TrustedRoot = serde_json::from_value(json!({
            "mediaType": "application/vnd.dev.sigstore.trustedroot+json;version=0.1",
            "tlogs": [], "ctlogs": [],
        }))?;
        let error = trust.rekor_keys(origin).expect_err("no decodable key");
        insta::assert_snapshot!(error, @"No current Rekor v1 signing key for https://rekor.example in trusted_root.json");

        let mut candidate = json!({
            "baseUrl": origin,
            "publicKey": {"rawBytes": "!", "keyDetails": "PKIX_ECDSA_P256_SHA_256",
                "validFor": {"start": "2000-01-01T00:00:00Z"}},
        });
        trust.tlogs.push(serde_json::from_value(candidate.clone())?);
        let error = trust.rekor_keys(origin).expect_err("no decodable key");
        insta::assert_snapshot!(error, @"Invalid base64 in the Rekor v1 signing key for https://rekor.example in trusted_root.json");

        candidate["publicKey"]["rawBytes"] = json!("AQ==");
        trust.tlogs.push(serde_json::from_value(candidate)?);
        let keys = trust.rekor_keys(origin).expect("second key decodes");
        assert_eq!(keys[0].der.as_ref(), &[1]);
        assert!(!keys[0].valid_at(0));
        Ok(())
    }
}
