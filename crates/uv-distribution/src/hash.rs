use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use uv_checksum_authority::{Sha256Digest, VerifiedRecord};
use uv_distribution_types::HashPolicy;
use uv_pypi_types::{HashAlgorithm, HashDigest};

/// Return the algorithms to compute for an HTTP distribution.
pub(crate) fn http_hash_algorithms(hashes: HashPolicy<'_>) -> Vec<HashAlgorithm> {
    let mut algorithms = hashes.algorithms();
    algorithms.push(HashAlgorithm::Sha256);
    algorithms.sort();
    algorithms.dedup();
    algorithms
}

/// Match locally computed archive properties against an authenticated authority record.
/// Missing properties require a download; index-provided hashes are not cache evidence.
pub(crate) fn matches_authority(
    authority: Option<&VerifiedRecord>,
    hashes: &[HashDigest],
    size: Option<u64>,
) -> bool {
    let Some(authority) = authority else {
        return true;
    };
    let record = authority.record();
    size == Some(record.size())
        && hashes.iter().any(|hash| {
            hash.algorithm() == HashAlgorithm::Sha256
                && hash
                    .digest
                    .parse::<Sha256Digest>()
                    .is_ok_and(|digest| digest == record.sha256())
        })
}

/// Hash a local build output without loading the complete wheel into memory.
pub(crate) async fn sha256_file(path: &Path) -> std::io::Result<Sha256Digest> {
    let mut file = fs_err::tokio::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let size = file.read(&mut buffer).await?;
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}
