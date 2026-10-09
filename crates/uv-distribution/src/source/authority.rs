use std::io;
use std::path::{Path, PathBuf};

use fs_err::tokio as fs;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tracing::debug;

use uv_cache::{AUTHORITY_RECEIPT_SUFFIX, CacheEntry, CacheShard};
use uv_checksum_authority::{Sha256Digest, VerificationReceipt};
use uv_client::RegistryClient;
use uv_distribution_filename::WheelFilename;
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
    let name = artifact.path().file_name().unwrap_or_default();
    let key = Sha256Digest::from_bytes(Sha256::digest(name.as_encoded_bytes()).into());
    artifact
        .path()
        .with_file_name(format!("{key}{AUTHORITY_RECEIPT_SUFFIX}"))
}

/// Authorization for a cached artifact, including the digest already computed during verification.
pub(super) enum BuildAuthorization {
    NotRequired,
    Verified(Sha256Digest),
}

impl BuildAuthorization {
    pub(super) fn digest(self) -> Option<Sha256Digest> {
        match self {
            Self::NotRequired => None,
            Self::Verified(digest) => Some(digest),
        }
    }
}

/// Bind the extracted directory to the exact authorized output using a bounded component name.
pub(super) fn authority_wheel_target(
    target: &Path,
    filename: &WheelFilename,
    digest: Sha256Digest,
) -> Box<Path> {
    target
        .with_file_name(format!("{}-{digest}", filename.cache_key()))
        .into_boxed_path()
}

/// Missing, malformed, or stale receipts cannot authorize a cached build.
pub(super) async fn read_authority_receipt(
    client: &RegistryClient,
    artifact: &CacheEntry,
) -> Result<Option<BuildAuthorization>, Error> {
    let Some(authority) = client.checksum_authority() else {
        return Ok(Some(BuildAuthorization::NotRequired));
    };
    let bytes = match fs::read(receipt_path(artifact)).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::CacheRead(error)),
    };
    let receipt: BuildReceipt = match rmp_serde::from_slice(&bytes) {
        Ok(receipt) => receipt,
        Err(error) => {
            debug!("Ignoring invalid checksum authority build receipt: {error}");
            return Ok(None);
        }
    };
    let digest = match sha256_file(artifact.path()).await {
        Ok(digest) => digest,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::CacheRead(error)),
    };
    if digest != receipt.sha256 {
        return Ok(None);
    }
    authority.verify_receipt(&receipt.authorizations).await?;
    Ok(Some(BuildAuthorization::Verified(digest)))
}

pub(super) async fn write_authority_receipt(
    client: &RegistryClient,
    artifact: &CacheEntry,
) -> Result<Option<Sha256Digest>, Error> {
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
        return Ok(Some(receipt.sha256));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_cache_components_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let filename: WheelFilename =
            format!("{}-1.0.0-py3-none-any.whl", "a".repeat(220)).parse()?;
        let artifact = CacheEntry::from_path(directory.path().join(filename.to_string()));
        let receipt = receipt_path(&artifact);
        let target = authority_wheel_target(
            &artifact.path().with_extension(""),
            &filename,
            Sha256Digest::from_bytes([0; 32]),
        );
        assert!(receipt.file_name().unwrap().len() < 255);
        assert!(target.file_name().unwrap().len() < 255);
        let other = CacheEntry::from_path(
            directory
                .path()
                .join(format!("{}-1.0.0-py3-none-any.whl", "b".repeat(220))),
        );
        assert_ne!(receipt, receipt_path(&other));
        fs_err::write(receipt, b"receipt")?;
        fs_err::create_dir_all(target)?;
        Ok(())
    }
}
