//! Submit a DSSE envelope to Rekor v1 and encode its inclusion evidence as Sigstore protobuf JSON.

use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use itertools::Itertools as _;
use reqwest::Client;
use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use rustls_pki_types::{CertificateDer, SubjectPublicKeyInfoDer, pem::PemObject as _};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use x509_parser::prelude::{FromDer as _, SubjectPublicKeyInfo};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    body: String,
    integrated_time: u64,
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

/// The public Rekor v1 instance uses this key for both SETs and checkpoints.
pub(super) fn public_key() -> Result<SubjectPublicKeyInfoDer<'static>> {
    // https://github.com/sigstore/root-signing/blob/main/targets/rekor.pub
    Ok(SubjectPublicKeyInfoDer::from_pem_slice(include_bytes!(
        "rekor-v1.pub"
    ))?)
}

impl LogEntry {
    fn verify_signatures(&self, log_key: &SubjectPublicKeyInfoDer<'_>) -> Result<()> {
        let key_id = Sha256::digest(log_key.as_ref());
        ensure!(
            hash_bytes(&self.log_id)? == key_id.as_slice(),
            "Unexpected Rekor log ID"
        );
        let (_, spki) = SubjectPublicKeyInfo::from_der(log_key.as_ref())
            .map_err(|_| anyhow::anyhow!("Invalid trusted Rekor public key"))?;
        let key = UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_ASN1,
            spki.subject_public_key.data.as_ref(),
        );

        // Rekor signs these four fields as canonical JSON (RFC 8785). The signed index is
        // global, while the inclusion proof's index is relative to its log shard.
        let signed = serde_json::to_vec(&json!({
            "body": self.body,
            "integratedTime": self.integrated_time,
            "logID": self.log_id,
            "logIndex": self.log_index,
        }))?;
        key.verify(
            &signed,
            &BASE64_STANDARD.decode(&self.verification.signed_entry_timestamp)?,
        )
        .map_err(|_| anyhow::anyhow!("Invalid Rekor signed entry timestamp"))?;

        // A signed note includes the final newline of the note text in its signature.
        // Signature lines can also include witness signatures with different key hints.
        let checkpoint = &self.verification.inclusion_proof.checkpoint;
        let (header, signatures) = checkpoint
            .split_once("\n\n")
            .context("Missing Rekor checkpoint signatures")?;
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
        ensure!(valid, "Invalid Rekor checkpoint signature");
        Ok(())
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
) -> Result<Value> {
    let entries: BTreeMap<String, LogEntry> = client
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
        .await?
        .error_for_status()?
        .json()
        .await?;
    let entry = entries
        .into_values()
        .exactly_one()
        .ok()
        .context("Expected one Rekor entry")?;
    entry.verify_signatures(log_key)?;
    let body = BASE64_STANDARD.decode(&entry.body)?;
    check_body(&body, envelope, statement, signature, pem)?;
    let proof = entry.verification.inclusion_proof;
    let root = hash_bytes(&proof.root_hash)?;
    let path = proof
        .hashes
        .iter()
        .map(|sibling| hash_bytes(sibling))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        included(&body, proof.log_index, proof.tree_size, &path, root),
        "Invalid Rekor inclusion proof"
    );
    let encoded_root = BASE64_STANDARD.encode(root);
    let mut lines = proof.checkpoint.lines();
    ensure!(
        lines.next().is_some_and(|origin| !origin.is_empty())
            && lines.next().and_then(|size| size.parse().ok()) == Some(proof.tree_size)
            && lines.next() == Some(encoded_root.as_str()),
        "Rekor checkpoint does not match its inclusion proof"
    );
    let hashes: Vec<_> = path
        .iter()
        .map(|hash| BASE64_STANDARD.encode(hash))
        .collect();

    // The legacy API uses hexadecimal hashes and integer indices. Sigstore's protobuf JSON
    // uses base64 bytes and decimal strings for int64 fields.
    Ok(json!({
        "logIndex": entry.log_index.to_string(),
        "logId": {"keyId": BASE64_STANDARD.encode(hash_bytes(&entry.log_id)?)},
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
    }))
}

/// Match the log's canonical entry to the bytes submitted for signing.
fn check_body(
    body: &[u8],
    envelope: &str,
    statement: &str,
    signature: &str,
    pem: &str,
) -> Result<()> {
    let canonical: Value = serde_json::from_slice(body)?;
    let spec = &canonical["spec"];
    ensure!(
        canonical["kind"] == "dsse"
            && canonical["apiVersion"] == "0.0.1"
            && spec["envelopeHash"]
                == json!({"algorithm":"sha256", "value":hex::encode(Sha256::digest(envelope))})
            && spec["payloadHash"]
                == json!({"algorithm":"sha256", "value":hex::encode(Sha256::digest(statement))}),
        "Rekor returned an entry for a different envelope"
    );
    let signatures = spec["signatures"]
        .as_array()
        .context("Missing Rekor signatures")?;
    let [logged] = signatures.as_slice() else {
        anyhow::bail!("Expected one Rekor signature")
    };
    let verifier = BASE64_STANDARD.decode(
        logged["verifier"]
            .as_str()
            .context("Missing Rekor verifier")?,
    )?;
    ensure!(
        logged["signature"] == signature
            && CertificateDer::from_pem_slice(&verifier)?
                == CertificateDer::from_pem_slice(pem.as_bytes())?,
        "Rekor returned a different signing certificate or signature"
    );
    Ok(())
}

fn hash_bytes(hex: &str) -> Result<[u8; 32]> {
    let mut bytes = [0; 32];
    hex::decode_to_slice(hex, &mut bytes).context("Invalid SHA-256 hash in Rekor response")?;
    Ok(bytes)
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

    use super::{LogEntry, included, public_key};

    /// Rekor receipt from Warehouse's sampleproject-4.0.0.tar.gz.provenance fixture.
    /// Its signed entry index is global, while the inclusion proof uses a shard-local index.
    #[test]
    fn verify_production_log_signatures() -> anyhow::Result<()> {
        let mut receipt: LogEntry = serde_json::from_str(include_str!(
            "../tests/fixtures/sampleproject-4.0.0-rekor.json"
        ))?;
        let key = public_key()?;
        receipt.verify_signatures(&key)?;

        receipt.integrated_time += 1;
        let error = receipt
            .verify_signatures(&key)
            .expect_err("Tampered timestamp");
        assert_eq!(error.to_string(), "Invalid Rekor signed entry timestamp");
        receipt.integrated_time -= 1;

        // Keep the Merkle root and tree size intact while changing the signed note.
        receipt
            .verification
            .inclusion_proof
            .checkpoint
            .insert_str(0, "other-");
        let error = receipt
            .verify_signatures(&key)
            .expect_err("Tampered checkpoint");
        assert_eq!(error.to_string(), "Invalid Rekor checkpoint signature");
        Ok(())
    }

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
