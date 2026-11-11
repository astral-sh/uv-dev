//! Submit a DSSE envelope to Rekor v1 and encode its inclusion evidence as Sigstore protobuf JSON.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use hex::FromHex as _;
use itertools::Itertools as _;
use reqwest::{Client, StatusCode};
use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use rustls_pki_types::{CertificateDer, SubjectPublicKeyInfoDer, pem::PemObject as _};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use x509_parser::error::X509Error;
use x509_parser::nom::Err as DerError;
use x509_parser::prelude::{FromDer as _, SubjectPublicKeyInfo};

/// The trust anchor for the production Rekor v1 log.
///
/// Pinning the public key lets the signer authenticate timestamps and checkpoints
/// independently of the HTTP response. A log key rotation requires updating this key in uv.
///
/// Source: <https://github.com/sigstore/root-signing/blob/fa6dc24dbae1/targets/rekor.pub>
const PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE2G2Y+2tabdTV5BcGiBIx0a9fAFwr
kBbmLSGtks4L3qX6yYY0zufBnhC8Ur/iy55GhWP/9A/bY2LhC30M9+RYtw==
-----END PUBLIC KEY-----
";

/// An error submitting a DSSE envelope to Rekor or authenticating its receipt.
#[derive(Debug, Error)]
pub enum RekorError {
    #[error("Failed to submit entry to Rekor")]
    Request(#[source] reqwest::Error),
    #[error("Rekor returned HTTP {0}")]
    Status(StatusCode),
    #[error("Invalid Rekor response")]
    Response(#[source] reqwest::Error),
    #[error("Expected one Rekor entry")]
    EntryCount,
    #[error("Invalid Rekor DSSE entry")]
    Entry(#[source] serde_json::Error),
    #[error("Invalid hexadecimal {0} in Rekor response")]
    Hex(&'static str, #[source] hex::FromHexError),
    #[error("Invalid base64 {0} in Rekor response")]
    Base64(&'static str, #[source] base64::DecodeError),
    #[error("Invalid trusted Rekor public key")]
    KeyPem(#[source] rustls_pki_types::pem::Error),
    #[error("Invalid trusted Rekor public key")]
    KeyDer(#[source] DerError<X509Error>),
    #[error("Unexpected Rekor log ID")]
    LogId,
    #[error("Invalid Rekor signed entry timestamp")]
    TimestampSignature,
    #[error("Missing Rekor checkpoint signatures")]
    MissingCheckpointSignatures,
    #[error("Invalid Rekor checkpoint signature")]
    CheckpointSignature,
    #[error("Invalid Rekor inclusion proof")]
    InclusionProof,
    #[error("Rekor checkpoint does not match its inclusion proof")]
    CheckpointMismatch,
    #[error("Rekor returned an entry for a different envelope")]
    EnvelopeMismatch,
    #[error("Invalid Rekor verifier certificate")]
    Certificate(#[source] rustls_pki_types::pem::Error),
    #[error("Rekor returned a different signing certificate or signature")]
    SignatureMismatch,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    body: String,
    integrated_time: i64,
    #[serde(rename = "logID")]
    log_id: String,
    log_index: u64,
    verification: Verification,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Verification {
    signed_entry_timestamp: String,
    inclusion_proof: InclusionProof,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InclusionProof {
    log_index: u64,
    tree_size: u64,
    root_hash: String,
    hashes: Vec<String>,
    checkpoint: String,
}

pub(super) fn public_key() -> Result<SubjectPublicKeyInfoDer<'static>, RekorError> {
    SubjectPublicKeyInfoDer::from_pem_slice(PUBLIC_KEY.as_bytes()).map_err(RekorError::KeyPem)
}

impl LogEntry {
    fn verify_signatures(
        &self,
        log_key: &SubjectPublicKeyInfoDer<'_>,
    ) -> Result<[u8; 32], RekorError> {
        let key_id = Sha256::digest(log_key.as_ref());
        if <[u8; 32]>::from_hex(&self.log_id).map_err(|err| RekorError::Hex("log ID", err))?
            != key_id.as_slice()
        {
            return Err(RekorError::LogId);
        }
        let (_, spki) =
            SubjectPublicKeyInfo::from_der(log_key.as_ref()).map_err(RekorError::KeyDer)?;
        let key = UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_ASN1,
            spki.subject_public_key.data.as_ref(),
        );

        // Rekor signs these four fields as canonical JSON (RFC 8785). The signed index is
        // global, while the inclusion proof's index is relative to its log shard.
        let signed = json!({
            "body": self.body,
            "integratedTime": self.integrated_time,
            "logID": self.log_id,
            "logIndex": self.log_index,
        })
        .to_string();
        let timestamp_signature = BASE64_STANDARD
            .decode(&self.verification.signed_entry_timestamp)
            .map_err(|err| RekorError::Base64("signed entry timestamp", err))?;
        key.verify(signed.as_bytes(), &timestamp_signature)
            .map_err(|_| RekorError::TimestampSignature)?;

        // A signed note includes the final newline of the note text in its signature.
        // Signature lines can also include witness signatures with different key hints.
        let checkpoint = &self.verification.inclusion_proof.checkpoint;
        let (header, signatures) = checkpoint
            .split_once("\n\n")
            .ok_or(RekorError::MissingCheckpointSignatures)?;
        let note = &checkpoint.as_bytes()[..=header.len()];
        let valid = signatures
            .lines()
            .filter_map(|line| {
                let (_, encoded) = line.strip_prefix("— ")?.split_once(' ')?;
                BASE64_STANDARD.decode(encoded).ok()
            })
            .any(|signature| {
                signature
                    .strip_prefix(&key_id[..4])
                    .is_some_and(|signature| key.verify(note, signature).is_ok())
            });
        if !valid {
            return Err(RekorError::CheckpointSignature);
        }
        Ok(key_id.into())
    }
}

pub(super) async fn record(
    client: &Client,
    url: &str,
    log_key: &SubjectPublicKeyInfoDer<'_>,
    envelope: &str,
    statement: &str,
    signature: &str,
    pem: &str,
) -> Result<(i64, Value), RekorError> {
    let response = client
        .post(url)
        .json(&json!({
            "apiVersion": "0.0.1",
            "kind": "dsse",
            "spec": {"proposedContent": {
                "envelope": envelope,
                "verifiers": [BASE64_STANDARD.encode(pem)],
            }}
        }))
        .send()
        .await
        .map_err(RekorError::Request)?;
    if !response.status().is_success() {
        return Err(RekorError::Status(response.status()));
    }
    let entries: BTreeMap<String, LogEntry> =
        response.json().await.map_err(RekorError::Response)?;
    let entry = entries
        .into_values()
        .exactly_one()
        .ok()
        .ok_or(RekorError::EntryCount)?;
    let key_id = entry.verify_signatures(log_key)?;
    let body = BASE64_STANDARD
        .decode(&entry.body)
        .map_err(|err| RekorError::Base64("entry body", err))?;
    check_body(&body, envelope, statement, signature, pem)?;
    let proof = entry.verification.inclusion_proof;
    let root =
        <[u8; 32]>::from_hex(&proof.root_hash).map_err(|err| RekorError::Hex("root hash", err))?;
    let path = proof
        .hashes
        .iter()
        .map(<[u8; 32]>::from_hex)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| RekorError::Hex("inclusion proof hash", err))?;
    if !included(&body, proof.log_index, proof.tree_size, &path, root) {
        return Err(RekorError::InclusionProof);
    }
    let encoded_root = BASE64_STANDARD.encode(root);
    let mut lines = proof.checkpoint.lines();
    if lines.next().is_none_or(str::is_empty)
        || lines.next().and_then(|size| size.parse().ok()) != Some(proof.tree_size)
        || lines.next() != Some(encoded_root.as_str())
    {
        return Err(RekorError::CheckpointMismatch);
    }
    let hashes: Vec<_> = path
        .iter()
        .map(|hash| BASE64_STANDARD.encode(hash))
        .collect();

    // The legacy API uses hexadecimal hashes and integer indices. Sigstore's protobuf JSON
    // uses base64 bytes and decimal strings for int64 fields.
    let transparency_entry = json!({
        "logIndex": entry.log_index.to_string(),
        "logId": {"keyId": BASE64_STANDARD.encode(key_id)},
        "kindVersion": {"kind": "dsse", "version": "0.0.1"},
        "integratedTime": entry.integrated_time.to_string(),
        "inclusionPromise": {"signedEntryTimestamp": entry.verification.signed_entry_timestamp},
        "inclusionProof": {
            "logIndex": proof.log_index.to_string(),
            "treeSize": proof.tree_size.to_string(),
            "rootHash": encoded_root,
            "hashes": hashes,
            "checkpoint": {"envelope": proof.checkpoint},
        },
        "canonicalizedBody": entry.body,
    });
    Ok((entry.integrated_time, transparency_entry))
}

/// Match the log's canonical entry to the bytes submitted for signing.
fn check_body(
    body: &[u8],
    envelope: &str,
    statement: &str,
    signature: &str,
    pem: &str,
) -> Result<(), RekorError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CanonicalEntry {
        kind: String,
        api_version: String,
        spec: DsseSpec,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DsseSpec {
        envelope_hash: EntryHash,
        payload_hash: EntryHash,
        signatures: [LoggedSignature; 1],
    }

    #[derive(Deserialize)]
    struct EntryHash {
        algorithm: String,
        value: String,
    }

    impl EntryHash {
        fn matches_sha256(&self, contents: &str) -> bool {
            self.algorithm == "sha256" && self.value == hex::encode(Sha256::digest(contents))
        }
    }

    #[derive(Deserialize)]
    struct LoggedSignature {
        signature: String,
        verifier: String,
    }

    let entry: CanonicalEntry = serde_json::from_slice(body).map_err(RekorError::Entry)?;
    if entry.kind != "dsse"
        || entry.api_version != "0.0.1"
        || !entry.spec.envelope_hash.matches_sha256(envelope)
        || !entry.spec.payload_hash.matches_sha256(statement)
    {
        return Err(RekorError::EnvelopeMismatch);
    }
    let [logged] = entry.spec.signatures;
    let verifier = BASE64_STANDARD
        .decode(logged.verifier)
        .map_err(|err| RekorError::Base64("verifier", err))?;
    if logged.signature != signature
        || CertificateDer::from_pem_slice(&verifier).map_err(RekorError::Certificate)?
            != CertificateDer::from_pem_slice(pem.as_bytes()).map_err(RekorError::Certificate)?
    {
        return Err(RekorError::SignatureMismatch);
    }
    Ok(())
}

/// Verify a CT Merkle inclusion path (RFC 9162 section 2.1.3.2).
fn included(body: &[u8], mut index: u64, size: u64, path: &[[u8; 32]], root: [u8; 32]) -> bool {
    if index >= size {
        return false;
    }
    let mut last = size - 1;
    let mut hash = Sha256::new()
        .chain_update([0])
        .chain_update(body)
        .finalize();
    for sibling in path {
        if last == 0 {
            return false;
        }
        let node = Sha256::new().chain_update([1]);
        hash = if !index.is_multiple_of(2) || index == last {
            // The incomplete right edge skips levels with no sibling in the proof.
            // Ascend to the first ancestor whose left sibling supplied this hash.
            while index != 0 && index.is_multiple_of(2) {
                index >>= 1;
                last >>= 1;
            }
            node.chain_update(sibling).chain_update(hash).finalize()
        } else {
            node.chain_update(hash).chain_update(sibling).finalize()
        };
        index >>= 1;
        last >>= 1;
    }
    last == 0 && hash.as_slice() == root
}

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};

    use super::included;

    #[test]
    fn verify_unbalanced_inclusion_proof() {
        // In a three-leaf tree the rightmost leaf is a direct child of the root.
        let first = Sha256::digest(b"\x00a");
        let second = Sha256::digest(b"\x00b");
        let third = Sha256::digest(b"\x00c");
        let left = Sha256::new()
            .chain_update([1])
            .chain_update(first)
            .chain_update(second)
            .finalize();
        let root = Sha256::new()
            .chain_update([1])
            .chain_update(left)
            .chain_update(third)
            .finalize()
            .into();
        assert!(included(b"c", 2, 3, &[left.into()], root));
        assert!(included(b"b", 1, 3, &[first.into(), third.into()], root));
        assert!(!included(b"a", 2, 3, &[left.into()], root));
        assert!(!included(b"b", 1, 3, &[first.into()], root));
    }
}
