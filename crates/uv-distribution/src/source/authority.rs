use std::io;
use std::path::PathBuf;

use fs_err::tokio as fs;
use serde::{Deserialize, Serialize};
use tracing::debug;

use uv_cache::{CacheEntry, CacheShard};
use uv_checksum_authority::{Sha256Digest, VerificationReceipt};
use uv_client::RegistryClient;
use uv_fs::write_atomic;

use crate::Error;
use crate::hash::sha256_file;

/// Bind a build's authorizations to the exact file produced by that build.
#[derive(Serialize, Deserialize)]
struct BuildReceipt {
    sha256: Sha256Digest,
    authorizations: VerificationReceipt,
}

/// Keep backend-produced artifacts separate from builds made without authority verification.
pub(super) fn authority_build_shard(client: &RegistryClient, shard: CacheShard) -> CacheShard {
    if let Some(authority) = client.checksum_authority() {
        shard.shard(format!("authority-{}", authority.public_key()))
    } else {
        shard
    }
}

fn receipt_path(artifact: &CacheEntry) -> PathBuf {
    let mut path = artifact.path().as_os_str().to_owned();
    path.push(".authority.msgpack");
    PathBuf::from(path)
}

/// Missing, malformed, or stale receipts cannot authorize a cached build.
pub(super) async fn read_authority_receipt(
    client: &RegistryClient,
    artifact: &CacheEntry,
) -> Result<bool, Error> {
    let Some(authority) = client.checksum_authority() else {
        return Ok(true);
    };
    let bytes = match fs::read(receipt_path(artifact)).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(Error::CacheRead(error)),
    };
    let receipt: BuildReceipt = match rmp_serde::from_slice(&bytes) {
        Ok(receipt) => receipt,
        Err(error) => {
            debug!("Ignoring invalid checksum authority build receipt: {error}");
            return Ok(false);
        }
    };
    let digest = match sha256_file(artifact.path()).await {
        Ok(digest) => digest,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(Error::CacheRead(error)),
    };
    if digest != receipt.sha256 {
        return Ok(false);
    }
    authority.verify_receipt(&receipt.authorizations).await?;
    Ok(true)
}

pub(super) async fn write_authority_receipt(
    client: &RegistryClient,
    artifact: &CacheEntry,
) -> Result<(), Error> {
    if let Some(authority) = client.checksum_authority() {
        let receipt = BuildReceipt {
            sha256: sha256_file(artifact.path())
                .await
                .map_err(Error::CacheRead)?,
            authorizations: authority.receipt().await?,
        };
        write_atomic(receipt_path(artifact), rmp_serde::to_vec(&receipt)?)
            .await
            .map_err(Error::CacheWrite)?;
    }
    Ok(())
}
