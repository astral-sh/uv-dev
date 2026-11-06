use uv_pypi_types::{HashAlgorithm, HashDigest, HashDigests, HashError, Hashes};
use uv_redacted::SafeUrl;

/// Groups of alternative hashes that must all match the same archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveHashGroups {
    groups: Vec<Vec<HashDigest>>,
}

impl ArchiveHashGroups {
    pub fn new(groups: Vec<Vec<HashDigest>>) -> Self {
        Self { groups }
    }

    pub fn groups(&self) -> &[Vec<HashDigest>] {
        &self.groups
    }
}

/// Hash generation and validation policy for an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveHashPolicy<'a> {
    /// No hash policy is specified.
    None,
    /// Hashes should be generated (specifically, a SHA-256 hash), but not validated.
    Generate,
    /// Hashes should be validated against a pre-defined list of hashes, and any matching digest is
    /// sufficient. If necessary, hashes should be generated so as to ensure that the archive is
    /// valid.
    Any(&'a [HashDigest]),
    /// Hashes should be validated against a pre-defined list of hashes, and every digest must
    /// match. If necessary, hashes should be generated so as to ensure that the archive is valid.
    All(&'a [HashDigest]),
    /// At least one hash in each group and every URL hash must match. A SHA-256 digest binds the
    /// verified archive for subsequent use even when the supplied hashes use weaker algorithms.
    AllOfAny {
        groups: Option<&'a ArchiveHashGroups>,
        url_hashes: &'a [HashDigest],
    },
}

impl<'a> ArchiveHashPolicy<'a> {
    /// Require a matching hash from each group and a SHA-256 digest.
    pub fn all_of_any(groups: &'a ArchiveHashGroups) -> Self {
        Self::AllOfAny {
            groups: Some(groups),
            url_hashes: &[],
        }
    }

    /// Use the hashes advertised for an individual index file when no verification policy is set.
    ///
    /// Explicit validation takes precedence, including empty policies that must reject the
    /// distribution. If the index provides no hashes, preserve the policy.
    #[must_use]
    pub fn with_index_hashes(self, hashes: &'a [HashDigest]) -> Self {
        if self.requires_validation() || hashes.is_empty() {
            self
        } else {
            Self::All(hashes)
        }
    }

    /// Returns `true` if the hash policy is `None`.
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Returns `true` if the hash policy requires verification.
    pub fn requires_validation(&self) -> bool {
        matches!(self, Self::Any(_) | Self::All(_) | Self::AllOfAny { .. })
    }

    /// Return the algorithms used in the hash policy.
    pub fn algorithms(&self) -> Vec<HashAlgorithm> {
        match self {
            Self::None => vec![],
            Self::Generate => vec![HashAlgorithm::Sha256],
            Self::Any(_) | Self::All(_) | Self::AllOfAny { .. } => {
                let mut algorithms = self
                    .digests()
                    .map(HashDigest::algorithm)
                    .collect::<Vec<_>>();
                if matches!(self, Self::AllOfAny { .. }) {
                    // Keep a strong binding to the archive that satisfied the groups, even when
                    // the supplied hashes use only weaker algorithms.
                    algorithms.push(HashAlgorithm::Sha256);
                }
                algorithms.sort();
                algorithms.dedup();
                algorithms
            }
        }
    }

    /// Return the digests used in the hash policy.
    pub fn digests(self) -> impl Iterator<Item = &'a HashDigest> {
        let (groups, digests) = match self {
            Self::None | Self::Generate => (None, &[][..]),
            Self::Any(hashes) | Self::All(hashes) => (None, hashes),
            Self::AllOfAny { groups, url_hashes } => (groups, url_hashes),
        };
        groups
            .into_iter()
            .flat_map(|groups| groups.groups.iter().flatten())
            .chain(digests)
    }

    /// Returns `true` if the given archive hashes satisfy the policy.
    pub fn matches(&self, hashes: &[HashDigest]) -> bool {
        let has_required_sha256 = match self {
            Self::Generate | Self::AllOfAny { .. } => hashes
                .iter()
                .any(|hash| hash.algorithm() == HashAlgorithm::Sha256),
            Self::None | Self::Any(_) | Self::All(_) => true,
        };
        has_required_sha256 && self.matches_index_hashes(hashes)
    }

    /// Check expected digests against the hashes advertised for an index candidate.
    ///
    /// The SHA-256 digest used to bind an archive to its verified bytes is computed when the
    /// archive is fetched.
    pub fn matches_index_hashes(&self, hashes: &[HashDigest]) -> bool {
        match self {
            Self::None | Self::Generate => true,
            Self::Any(required) => {
                !required.is_empty() && hashes.iter().any(|hash| required.contains(hash))
            }
            Self::All(required) => {
                !required.is_empty() && required.iter().all(|hash| hashes.contains(hash))
            }
            Self::AllOfAny { groups, url_hashes } => {
                groups.is_none_or(|groups| {
                    groups
                        .groups
                        .iter()
                        .all(|group| group.iter().any(|hash| hashes.contains(hash)))
                }) && url_hashes.iter().all(|hash| hashes.contains(hash))
            }
        }
    }

    /// Returns `true` if the given hashes include the algorithms required by the policy.
    fn has_required_algorithms(&self, hashes: &[HashDigest]) -> bool {
        let has_algorithm = |algorithm| hashes.iter().any(|hash| hash.algorithm() == algorithm);
        match self {
            Self::None => true,
            Self::Generate => has_algorithm(HashAlgorithm::Sha256),
            Self::Any(required) => required
                .iter()
                .map(HashDigest::algorithm)
                .any(has_algorithm),
            Self::All(required) => {
                !required.is_empty()
                    && required
                        .iter()
                        .map(HashDigest::algorithm)
                        .all(has_algorithm)
            }
            Self::AllOfAny { groups, url_hashes } => {
                has_algorithm(HashAlgorithm::Sha256)
                    && groups.is_none_or(|groups| {
                        groups.groups.iter().all(|group| {
                            !group.is_empty()
                                && (group.iter().any(|hash| hashes.contains(hash))
                                    || group.iter().map(HashDigest::algorithm).all(has_algorithm))
                        })
                    })
                    && url_hashes
                        .iter()
                        .map(HashDigest::algorithm)
                        .all(has_algorithm)
            }
        }
    }
}

/// Archive hashes to collect or validate when fetching distribution metadata.
///
/// For example, `uv pip compile --generate-hashes` requests [`HashCollection::All`] with
/// [`HashValidation::None`]:
///
/// - If the index provides a wheel's hash, metadata lookup uses [`ArchiveHashPolicy::None`].
///   The resolver can record the index-provided hash without hashing the wheel.
/// - If the index provides no hashes, metadata lookup uses [`ArchiveHashPolicy::Generate`]
///   to obtain a SHA-256 hash of the wheel.
///
/// If source metadata requires a build and `validation` contains expected hashes, the fetcher
/// instead uses [`ArchiveHashPolicy::Any`] or [`ArchiveHashPolicy::All`] to check the archive
/// before running the backend. Those expected hashes take precedence over collection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MetadataHashPolicy<'a> {
    /// Which missing hashes to collect for the resolution.
    pub collection: HashCollection,
    /// Expected hashes to validate before source builds. Wheel validation is deferred to installation.
    pub validation: HashValidation<'a>,
}

/// Expected digests to validate against an archive.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HashValidation<'a> {
    /// No expected digests are specified.
    #[default]
    None,
    /// Require at least one expected digest to match.
    Any(&'a [HashDigest]),
    /// Require every expected digest to match. An empty slice rejects all archives.
    All(&'a [HashDigest]),
    /// Require each independent group and the index-provided hashes before building a source.
    Independent {
        groups: Option<&'a ArchiveHashGroups>,
        /// Additional hashes declared by references to this archive in dependency metadata.
        url_hashes: Option<&'a [HashDigest]>,
    },
}

/// Which distributions should have hashes collected during resolution.
///
/// Reuse declared hashes when available; otherwise, compute a SHA-256 hash from the archive.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HashCollection {
    /// Do not collect hashes during resolution.
    #[default]
    None,
    /// Supply hashes for non-registry distributions. Registry hashes come from index metadata.
    Url,
    /// Also compute missing registry hashes when the index metadata does not provide them.
    All,
}

/// Returns whether the digest's algorithm is accepted by `--require-hashes`.
pub fn satisfies_require_hashes(digest: &HashDigest) -> bool {
    match digest.algorithm() {
        HashAlgorithm::Md5 => false,
        HashAlgorithm::Sha256
        | HashAlgorithm::Sha384
        | HashAlgorithm::Sha512
        | HashAlgorithm::Blake2b256 => true,
    }
}

/// Read a URL's declared hash for resolution, excluding MD5, which `--require-hashes` rejects.
pub fn parse_url_hashes(url: &SafeUrl) -> Option<HashDigests> {
    let hashes = url
        .fragment()?
        .split('&')
        .find_map(|fragment| Hashes::parse_fragment(fragment).ok())?;
    let hashes = HashDigests::from(hashes);
    let allowed = hashes.iter().all(satisfies_require_hashes);
    allowed.then_some(hashes)
}

/// Read every supported hash declared in a URL fragment.
pub fn parse_all_url_hashes(url: &SafeUrl) -> Result<Vec<HashDigest>, HashError> {
    let mut digests = Vec::new();
    if let Some(fragment) = url.fragment() {
        for part in fragment.split('&') {
            if let Some(hashes) = Hashes::parse_url_fragment(part)? {
                digests.extend(HashDigests::from(hashes));
            }
        }
    }
    Ok(digests)
}

pub trait Hashed {
    /// Return the [`HashDigest`]s for the archive.
    fn hashes(&self) -> &[HashDigest];

    /// Returns `true` if the archive satisfies the given hash policy.
    fn satisfies(&self, hashes: ArchiveHashPolicy) -> bool {
        hashes.matches(self.hashes())
    }

    /// Returns `true` if the archive includes the algorithms required by the given hash policy.
    fn has_digests(&self, hashes: ArchiveHashPolicy) -> bool {
        hashes.has_required_algorithms(self.hashes())
    }
}

impl Hashed for Vec<HashDigest> {
    fn hashes(&self) -> &[HashDigest] {
        self
    }
}

impl Hashed for &[HashDigest] {
    fn hashes(&self) -> &[HashDigest] {
        self
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_pypi_types::{HashAlgorithm, HashDigest, HashError};

    use super::{ArchiveHashGroups, ArchiveHashPolicy, Hashed};

    /// Current call paths reject missing required hashes before reaching this helper, so test
    /// the defensive empty-policy case directly.
    #[test]
    fn index_hashes_preserve_empty_required_policies() -> Result<(), HashError> {
        let index_hashes = [HashDigest::from_str(
            "sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
        )?];
        assert_eq!(
            ArchiveHashPolicy::Any(&[]).with_index_hashes(&index_hashes),
            ArchiveHashPolicy::Any(&[])
        );
        assert_eq!(
            ArchiveHashPolicy::All(&[]).with_index_hashes(&index_hashes),
            ArchiveHashPolicy::All(&[])
        );

        Ok(())
    }

    #[test]
    fn independent_hashes_require_a_sha256_for_cached_archives() -> Result<(), HashError> {
        let md5 = HashDigest::from_str("md5:0123456789abcdef0123456789abcdef")?;
        let sha256 = HashDigest::from_str(
            "sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
        )?;
        let groups = ArchiveHashGroups::new(vec![vec![md5.clone()]]);
        let policy = ArchiveHashPolicy::all_of_any(&groups);
        assert_eq!(
            policy.algorithms(),
            vec![HashAlgorithm::Md5, HashAlgorithm::Sha256]
        );
        let weak_hashes = vec![md5.clone()];
        assert!(!weak_hashes.has_digests(policy));
        assert!(!weak_hashes.satisfies(policy));
        let secure_hashes = vec![md5, sha256];
        assert!(secure_hashes.has_digests(policy));
        assert!(secure_hashes.satisfies(policy));
        Ok(())
    }

    #[test]
    fn independent_hashes_require_one_digest_per_group() -> Result<(), HashError> {
        let md5 = HashDigest::from_str("md5:0123456789abcdef0123456789abcdef")?;
        let alternate = HashDigest::from_str("md5:fedcba9876543210fedcba9876543210")?;
        let sha256 = HashDigest::from_str(
            "sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
        )?;
        let sha384 = HashDigest::from_str(concat!(
            "sha384:0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ))?;
        let groups = ArchiveHashGroups::new(vec![
            vec![md5.clone(), alternate.clone()],
            vec![sha384.clone()],
        ]);
        let policy = ArchiveHashPolicy::all_of_any(&groups);

        assert!(policy.matches(&[md5.clone(), sha256.clone(), sha384.clone()]));
        assert!(policy.matches(&[alternate, sha256.clone(), sha384.clone()]));
        assert!(!policy.matches(&[md5.clone(), sha256.clone()]));
        assert!(!policy.matches(&[sha256, sha384.clone()]));
        assert!(!policy.matches(&[md5, sha384]));

        Ok(())
    }

    #[test]
    fn independent_hashes_generate_sha256_without_assertions() -> Result<(), HashError> {
        let groups = ArchiveHashGroups::new(vec![]);
        let policy = ArchiveHashPolicy::all_of_any(&groups);
        let sha256 = HashDigest::from_str(
            "sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
        )?;

        assert_eq!(policy.algorithms(), vec![HashAlgorithm::Sha256]);
        assert!(!policy.matches(&[]));
        assert!(policy.matches(&[sha256]));

        Ok(())
    }

    #[test]
    fn validate_all_requires_every_digest() {
        let sha256 = HashDigest::from_str(
            "sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
        )
        .unwrap();
        let sha512 = HashDigest::from_str(
            "sha512:f30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2",
        )
        .unwrap();
        let wrong_sha512 = HashDigest::from_str(
            "sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2",
        )
        .unwrap();

        let policy = ArchiveHashPolicy::All(&[sha256.clone(), sha512.clone()]);
        assert!(policy.matches(&[sha256.clone(), sha512]));
        assert!(!policy.matches(std::slice::from_ref(&sha256)));
        assert!(!policy.matches(&[sha256, wrong_sha512]));
    }

    #[test]
    fn validate_any_requires_one_digest() {
        let sha256 = HashDigest::from_str(
            "sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
        )
        .unwrap();
        let sha512 = HashDigest::from_str(
            "sha512:f30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2",
        )
        .unwrap();
        let wrong_sha512 = HashDigest::from_str(
            "sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2",
        )
        .unwrap();

        let policy = ArchiveHashPolicy::Any(&[sha256.clone(), sha512]);
        assert!(policy.matches(&[sha256]));
        assert!(!policy.matches(&[wrong_sha512]));
    }
}
