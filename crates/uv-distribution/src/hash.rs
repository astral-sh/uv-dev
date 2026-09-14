use std::fmt::Display;

use uv_distribution_types::{HashPolicy, Hashed, IndexRoute, RegistryFile};
use uv_pypi_types::{HashAlgorithm, HashDigest};

use crate::Error;

/// Hash requirements for downloading and caching a wheel or source distribution.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArtifactHashPolicy<'a> {
    /// Hashes the caller requires on the returned artifact.
    pub(crate) required: HashPolicy<'a>,
    /// Hashes downloaded or cached bytes must match before entering the cache.
    cache_verification: HashPolicy<'a>,
}

impl<'a> ArtifactHashPolicy<'a> {
    pub(crate) const fn new(required: HashPolicy<'a>, cache_verification: HashPolicy<'a>) -> Self {
        Self {
            required,
            cache_verification,
        }
    }

    /// Verify proxy-served bytes against the canonical artifact's advertised hashes.
    pub(crate) fn for_registry(
        required: HashPolicy<'a>,
        route: &IndexRoute,
        file: &'a RegistryFile,
    ) -> Self {
        let cache_verification = if route.is_proxy() {
            if file.hashes.is_empty() {
                required
            } else {
                HashPolicy::Any(file.hashes.as_slice())
            }
        } else {
            HashPolicy::None
        };
        Self::new(required, cache_verification)
    }

    /// Determine the hash policy for a cached registry artifact.
    ///
    /// Direct indexes can reuse cached artifacts absent from the current registry metadata. A
    /// proxy requires the matching file so its advertised hashes can be checked.
    pub(crate) fn for_cached_registry(
        required: HashPolicy<'a>,
        route: &IndexRoute,
        file: Option<&'a RegistryFile>,
    ) -> Option<Self> {
        if let Some(file) = file {
            Some(Self::for_registry(required, route, file))
        } else if route.is_proxy() {
            None
        } else {
            Some(Self::from(required))
        }
    }

    pub(crate) fn algorithms(self) -> Vec<HashAlgorithm> {
        let mut algorithms = self.required.algorithms();
        algorithms.extend(self.cache_verification.algorithms());
        algorithms.sort_unstable();
        algorithms.dedup();
        algorithms
    }

    pub(crate) fn http_algorithms(self) -> Vec<HashAlgorithm> {
        let mut algorithms = self.algorithms();
        algorithms.push(HashAlgorithm::Sha256);
        algorithms.sort_unstable();
        algorithms.dedup();
        algorithms
    }

    pub(crate) fn admits_cached_artifact(self, artifact: &impl Hashed) -> bool {
        artifact.satisfies(self.cache_verification) && artifact.has_digests(self.required)
    }

    pub(crate) fn validate_download(
        self,
        artifact: &impl Display,
        hashes: &[HashDigest],
    ) -> Result<(), Error> {
        if !self.cache_verification.matches(hashes) {
            return Err(Error::hash_mismatch(
                artifact.to_string(),
                self.cache_verification.digests(),
                hashes,
            ));
        }

        Ok(())
    }

    pub(crate) fn validate_artifact(
        self,
        artifact: &impl Display,
        hashes: &impl Hashed,
    ) -> Result<(), Error> {
        self.validate_download(artifact, hashes.hashes())?;
        if !hashes.satisfies(self.required) {
            return Err(Error::hash_mismatch(
                artifact.to_string(),
                self.required.digests(),
                hashes.hashes(),
            ));
        }

        Ok(())
    }
}

impl<'a> From<HashPolicy<'a>> for ArtifactHashPolicy<'a> {
    fn from(required: HashPolicy<'a>) -> Self {
        Self::new(required, HashPolicy::None)
    }
}

#[cfg(test)]
mod tests {
    use uv_distribution_types::{File, FileLocation, Index, IndexLocations, IndexUrl};
    use uv_pypi_types::HashDigests;

    use super::*;

    #[test]
    fn artifact_hash_policy_preserves_cache_verification_algorithms()
    -> Result<(), Box<dyn std::error::Error>> {
        let cache_verification = HashDigests::from(vec![
            "sha512:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .parse()?,
        ]);
        let hashes = ArtifactHashPolicy::new(
            HashPolicy::None,
            HashPolicy::Any(cache_verification.as_slice()),
        );

        assert_eq!(
            hashes.http_algorithms(),
            vec![HashAlgorithm::Sha256, HashAlgorithm::Sha512]
        );
        Ok(())
    }

    #[test]
    fn artifact_hash_policy_rejects_wrong_cached_digest() -> Result<(), Box<dyn std::error::Error>>
    {
        let cache_verification = HashDigests::from(vec![
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?,
        ]);
        let cached_hashes = vec![
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".parse()?,
        ];
        let hashes = ArtifactHashPolicy::new(
            HashPolicy::None,
            HashPolicy::Any(cache_verification.as_slice()),
        );

        assert!(!hashes.admits_cached_artifact(&cached_hashes));
        Ok(())
    }

    #[test]
    fn cached_registry_policy_checks_provenance_and_hashes()
    -> Result<(), Box<dyn std::error::Error>> {
        let index: IndexUrl = "https://pypi.org/simple/".parse()?;
        let direct = IndexLocations::default().route_for(&index);
        let proxy = Index {
            name: Some("proxy".parse()?),
            proxy_for: Some("pypi".parse()?),
            artifact_base_url: Some("https://proxy.example.com/files/".parse()?),
            ..Index::from_extra_index_url("https://proxy.example.com/simple/".parse()?)
        };
        let proxy = IndexLocations::new(vec![proxy], Vec::new(), false)?.route_for(&index);
        let expected = vec![
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?,
        ];
        let unexpected = vec![
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".parse()?,
        ];
        let file = direct.canonicalize_file(File {
            dist_info_metadata: false,
            filename: "package.whl".into(),
            hashes: expected.clone().into(),
            requires_python: None,
            size: None,
            upload_time_utc_ms: None,
            url: FileLocation::new(
                "https://files.pythonhosted.org/packages/package.whl".into(),
                &"https://pypi.org/simple/package/".into(),
            ),
            yanked: None,
            zstd: None,
        })?;

        let admits = |route, file, required, artifact: &Vec<HashDigest>| {
            ArtifactHashPolicy::for_cached_registry(required, route, file)
                .is_some_and(|hashes| hashes.admits_cached_artifact(artifact))
        };

        assert!(admits(&direct, None, HashPolicy::None, &unexpected));
        assert!(!admits(&proxy, None, HashPolicy::None, &expected));
        assert!(admits(&direct, Some(&file), HashPolicy::None, &unexpected));
        assert!(admits(&proxy, Some(&file), HashPolicy::None, &expected));
        assert!(!admits(&proxy, Some(&file), HashPolicy::None, &unexpected));

        let hashless = RegistryFile {
            hashes: HashDigests::empty(),
            ..file.clone()
        };
        assert!(admits(
            &proxy,
            Some(&hashless),
            HashPolicy::None,
            &unexpected,
        ));
        assert!(admits(
            &proxy,
            Some(&hashless),
            HashPolicy::Any(&expected),
            &expected,
        ));
        assert!(!admits(
            &proxy,
            Some(&hashless),
            HashPolicy::Any(&expected),
            &unexpected,
        ));
        Ok(())
    }
}
