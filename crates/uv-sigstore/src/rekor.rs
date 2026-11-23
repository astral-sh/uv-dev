//! Submit a DSSE envelope to Rekor v1 and encode its inclusion evidence as Sigstore protobuf JSON.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use ct_merkle::{InclusionProof as MerkleProof, InclusionVerifError, RootHash};
use hex::FromHex as _;
use itertools::Itertools as _;
use reqwest::{StatusCode, header::LOCATION};
use reqwest_middleware::ClientWithMiddleware;
use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use rustls_pki_types::{CertificateDer, pem::PemObject as _};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use x509_parser::error::X509Error;
use x509_parser::nom::Err as DerError;
use x509_parser::prelude::{FromDer as _, SubjectPublicKeyInfo};

use crate::ServiceError;
use crate::trust::RekorKey;

/// An error submitting a DSSE envelope to Rekor or authenticating its receipt.
#[derive(Debug, Error)]
pub enum RekorError {
    #[error("Failed to submit entry to Rekor")]
    Request(#[source] reqwest_middleware::Error),
    #[error("Failed to retrieve existing Rekor entry {id}")]
    Lookup {
        id: String,
        #[source]
        source: reqwest_middleware::Error,
    },
    #[error(transparent)]
    Status(ServiceError),
    #[error("Invalid Rekor response")]
    Response(#[source] reqwest::Error),
    #[error("Expected one Rekor entry, got {0}")]
    EntryCount(usize),
    #[error("Invalid Rekor DSSE entry")]
    Entry(#[source] serde_json::Error),
    #[error("Invalid hexadecimal {0} in Rekor response")]
    Hex(&'static str, #[source] hex::FromHexError),
    #[error("Invalid base64 {0} in Rekor response")]
    Base64(&'static str, #[source] base64::DecodeError),
    #[error("Invalid trusted Rekor public key")]
    KeyDer(#[source] DerError<X509Error>),
    #[error(
        "No trusted Rekor key for log {id} at Unix time {time} (candidate log IDs: {candidates})"
    )]
    LogId {
        id: String,
        time: i64,
        candidates: String,
    },
    #[error("Invalid Rekor signed entry timestamp for entry {0}")]
    TimestampSignature(u64),
    #[error("Malformed Rekor checkpoint for entry {0}")]
    MalformedCheckpoint(u64),
    #[error("Invalid Rekor checkpoint signature for entry {0}")]
    CheckpointSignature(u64),
    #[error("Rekor tree size {0} exceeds the maximum signed 64-bit integer")]
    TreeTooLarge(u64),
    #[error("Invalid Rekor inclusion proof for leaf {index} in tree of size {size}")]
    InclusionProof {
        index: u64,
        size: u64,
        #[source]
        reason: InclusionVerifError,
    },
    #[error("Missing Rekor checkpoint origin")]
    MissingCheckpointOrigin,
    #[error("Unexpected Rekor {field}: expected {expected:?}, got {actual:?}")]
    Mismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
    #[error("Invalid Rekor verifier certificate")]
    Certificate(#[source] rustls_pki_types::pem::Error),
    #[error("Rekor returned a different envelope signature")]
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

impl LogEntry {
    fn verify_signatures(&self, keys: &[RekorKey<'_>]) -> Result<[u8; 32], RekorError> {
        let key_id =
            <[u8; 32]>::from_hex(&self.log_id).map_err(|err| RekorError::Hex("log ID", err))?;
        let log_key = keys
            .iter()
            .find(|key| key.id == key_id && key.can_verify(self.integrated_time))
            .ok_or_else(|| RekorError::LogId {
                id: self.log_id.clone(),
                time: self.integrated_time,
                candidates: keys.iter().map(|key| hex::encode(key.id)).join(", "),
            })?;
        let (_, spki) =
            SubjectPublicKeyInfo::from_der(log_key.der.as_ref()).map_err(RekorError::KeyDer)?;
        let key = UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &*spki.subject_public_key.data);

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
            .map_err(|_| RekorError::TimestampSignature(self.log_index))?;

        // A signed note includes the final newline of the note text in its signature.
        // Signature lines can also include witness signatures with different key hints.
        let checkpoint = &self.verification.inclusion_proof.checkpoint;
        let (header, signatures) = checkpoint
            .split_once("\n\n")
            .filter(|(_, signatures)| signatures.ends_with('\n') && !signatures.contains("\n\n"))
            .ok_or(RekorError::MalformedCheckpoint(self.log_index))?;
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
            return Err(RekorError::CheckpointSignature(self.log_index));
        }
        Ok(key_id)
    }
}

pub(super) async fn record(
    client: &ClientWithMiddleware,
    url: &str,
    log_keys: &[RekorKey<'_>],
    envelope: &str,
    statement: &str,
    signature: &str,
    pem: &str,
) -> Result<(i64, Value), RekorError> {
    let mut response = client
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
    // A retry can race a successful insertion whose response was lost. Use only the entry ID
    // from Location so the lookup stays on our configured Rekor endpoint.
    if response.status() == StatusCode::CONFLICT
        && let Some(id) = response
            .headers()
            .get(LOCATION)
            .and_then(|location| location.to_str().ok())
            .and_then(|location| location.rsplit('/').next())
            .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        response = client
            .get(format!("{url}/{id}"))
            .send()
            .await
            .map_err(|source| RekorError::Lookup {
                id: id.to_owned(),
                source,
            })?;
    }
    if !response.status().is_success() {
        return Err(RekorError::Status(
            ServiceError::from_response("Rekor", response, None).await,
        ));
    }
    let entries: BTreeMap<String, LogEntry> =
        response.json().await.map_err(RekorError::Response)?;
    let entry = entries
        .into_values()
        .exactly_one()
        .map_err(|entries| RekorError::EntryCount(entries.len()))?;
    let key_id = entry.verify_signatures(log_keys)?;
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
    // Rekor sizes are protobuf int64s. Bound this before ct-merkle doubles the leaf count
    // to index its internal nodes.
    if i64::try_from(proof.tree_size).is_err() {
        return Err(RekorError::TreeTooLarge(proof.tree_size));
    }
    RootHash::<Sha256>::new(root.into(), proof.tree_size)
        .verify_inclusion(
            &body,
            proof.log_index,
            &MerkleProof::from_digests(path.iter().map(Into::into)),
        )
        .map_err(|reason| RekorError::InclusionProof {
            index: proof.log_index,
            size: proof.tree_size,
            reason,
        })?;
    let encoded_root = BASE64_STANDARD.encode(root);
    let mut lines = proof.checkpoint.lines();
    if lines.next().is_none_or(str::is_empty) {
        return Err(RekorError::MissingCheckpointOrigin);
    }
    check_field(
        "checkpoint tree size",
        &proof.tree_size.to_string(),
        lines.next().unwrap_or("<missing>"),
    )?;
    check_field(
        "checkpoint root hash",
        &encoded_root,
        lines.next().unwrap_or("<missing>"),
    )?;
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

/// Only report public receipt fields here, never the DSSE payload or credential.
fn check_field(field: &'static str, expected: &str, actual: &str) -> Result<(), RekorError> {
    if actual != expected {
        return Err(RekorError::Mismatch {
            field,
            expected: expected.to_owned(),
            actual: actual[..actual.floor_char_boundary(128)].to_owned(),
        });
    }
    Ok(())
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
        fn check(&self, field: &'static str, contents: &str) -> Result<(), RekorError> {
            check_field("digest algorithm", "sha256", &self.algorithm)?;
            check_field(field, &hex::encode(Sha256::digest(contents)), &self.value)
        }
    }

    #[derive(Deserialize)]
    struct LoggedSignature {
        signature: String,
        verifier: String,
    }

    let entry: CanonicalEntry = serde_json::from_slice(body).map_err(RekorError::Entry)?;
    check_field("entry kind", "dsse", &entry.kind)?;
    check_field("API version", "0.0.1", &entry.api_version)?;
    entry
        .spec
        .envelope_hash
        .check("envelope digest", envelope)?;
    entry.spec.payload_hash.check("payload digest", statement)?;
    let [logged] = entry.spec.signatures;
    let verifier = BASE64_STANDARD
        .decode(logged.verifier)
        .map_err(|err| RekorError::Base64("verifier", err))?;
    if logged.signature != signature {
        return Err(RekorError::SignatureMismatch);
    }
    let expected =
        CertificateDer::from_pem_slice(pem.as_bytes()).map_err(RekorError::Certificate)?;
    let actual = CertificateDer::from_pem_slice(&verifier).map_err(RekorError::Certificate)?;
    if expected != actual {
        return Err(RekorError::Mismatch {
            field: "certificate fingerprint (SHA-256)",
            expected: hex::encode(Sha256::digest(expected)),
            actual: hex::encode(Sha256::digest(actual)),
        });
    }
    Ok(())
}
