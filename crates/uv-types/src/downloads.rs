use std::sync::Arc;
use uv_distribution_types::{CachedDist, DistributionId, HashPolicy, OwnedHashPolicy};
use uv_once_map::OnceMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InFlightDownload {
    distribution: DistributionId,
    hashes: OwnedHashPolicy,
}

impl InFlightDownload {
    pub fn new(distribution: DistributionId, hashes: HashPolicy<'_>) -> Self {
        Self {
            distribution,
            hashes: hashes.into(),
        }
    }
}

#[derive(Default, Clone)]
pub struct InFlight {
    /// The in-flight distribution downloads.
    pub downloads: Arc<OnceMap<InFlightDownload, Result<CachedDist, String>>>,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_distribution_types::{DistributionId, HashPolicy};
    use uv_pypi_types::HashDigest;

    use super::InFlightDownload;

    #[test]
    fn download_identity_includes_normalized_hash_policy() {
        let distribution = DistributionId::AbsoluteUrl("https://example.com/pkg.whl".to_string());
        let sha256 = HashDigest::from_str(
            "sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
        )
        .unwrap();
        let sha512 = HashDigest::from_str(
            "sha512:f30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2",
        )
        .unwrap();

        assert_eq!(
            InFlightDownload::new(
                distribution.clone(),
                HashPolicy::Any(&[sha256.clone(), sha512.clone()]),
            ),
            InFlightDownload::new(distribution.clone(), HashPolicy::Any(&[sha512, sha256]),),
        );
        assert_ne!(
            InFlightDownload::new(distribution.clone(), HashPolicy::Any(&[])),
            InFlightDownload::new(distribution, HashPolicy::None),
        );
    }
}
