//! Submit a DSSE envelope to Rekor v1 and encode its inclusion evidence as Sigstore protobuf JSON.

use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use itertools::Itertools as _;
use reqwest::Client;
use rustls_pki_types::{CertificateDer, pem::PemObject as _};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

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

pub(super) async fn record(
    client: &Client,
    url: &str,
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
    ensure!(
        !BASE64_STANDARD
            .decode(&entry.verification.signed_entry_timestamp)?
            .is_empty(),
        "Missing Rekor signed entry timestamp"
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
