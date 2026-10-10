use std::borrow::Cow;
use std::cmp::{Ordering, Reverse};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use ref_cast::RefCast;
use thiserror::Error;
use uv_cache_key::{CacheKey, CacheKeyHasher};
use uv_pep440::Prerelease;
use uv_platform::{Arch, Libc, Os, Platform};

use crate::{ImplementationName, LenientImplementationName, PythonVariant, PythonVersion};

#[derive(Error, Debug)]
pub enum PythonInstallationKeyError {
    #[error("Failed to parse Python installation key `{0}`: {1}")]
    ParseError(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PythonInstallationKey {
    pub implementation: LenientImplementationName,
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
    pub prerelease: Option<Prerelease>,
    pub platform: Platform,
    pub variant: PythonVariant,
}

impl PythonInstallationKey {
    pub fn new(
        implementation: LenientImplementationName,
        major: u8,
        minor: u8,
        patch: u8,
        prerelease: Option<Prerelease>,
        platform: Platform,
        variant: PythonVariant,
    ) -> Self {
        Self {
            implementation,
            major,
            minor,
            patch,
            prerelease,
            platform,
            variant,
        }
    }

    pub fn new_from_version(
        implementation: LenientImplementationName,
        version: &PythonVersion,
        platform: Platform,
        variant: PythonVariant,
    ) -> Self {
        Self {
            implementation,
            major: version.major(),
            minor: version.minor(),
            patch: version.patch().unwrap_or_default(),
            prerelease: version.pre(),
            platform,
            variant,
        }
    }

    pub fn implementation(&self) -> Cow<'_, LenientImplementationName> {
        if self.os().is_emscripten() {
            Cow::Owned(LenientImplementationName::from(ImplementationName::Pyodide))
        } else {
            Cow::Borrowed(&self.implementation)
        }
    }

    pub fn version(&self) -> PythonVersion {
        PythonVersion::from_str(&format!(
            "{}.{}.{}{}",
            self.major,
            self.minor,
            self.patch,
            self.prerelease
                .map(|pre| pre.to_string())
                .unwrap_or_default()
        ))
        .expect("Python installation keys must have valid Python versions")
    }

    /// The version in `x.y.z` format.
    #[cfg(windows)]
    pub fn sys_version(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }

    pub fn major(&self) -> u8 {
        self.major
    }

    pub fn minor(&self) -> u8 {
        self.minor
    }

    pub(crate) fn prerelease(&self) -> Option<Prerelease> {
        self.prerelease
    }

    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    pub fn arch(&self) -> &Arch {
        &self.platform.arch
    }

    pub fn os(&self) -> &Os {
        &self.platform.os
    }

    pub fn libc(&self) -> &Libc {
        &self.platform.libc
    }

    pub fn variant(&self) -> &PythonVariant {
        &self.variant
    }

    /// Return a canonical name for a minor versioned executable.
    pub fn executable_name_minor(&self) -> String {
        format!(
            "{name}{maj}.{min}{var}{exe}",
            name = self.implementation().executable_install_name(),
            maj = self.major,
            min = self.minor,
            var = self.variant.executable_suffix(),
            exe = std::env::consts::EXE_SUFFIX
        )
    }

    /// Return a canonical name for a major versioned executable.
    pub fn executable_name_major(&self) -> String {
        format!(
            "{name}{maj}{var}{exe}",
            name = self.implementation().executable_install_name(),
            maj = self.major,
            var = self.variant.executable_suffix(),
            exe = std::env::consts::EXE_SUFFIX
        )
    }

    /// Return a canonical name for an un-versioned executable.
    pub fn executable_name(&self) -> String {
        format!(
            "{name}{var}{exe}",
            name = self.implementation().executable_install_name(),
            var = self.variant.executable_suffix(),
            exe = std::env::consts::EXE_SUFFIX
        )
    }
}

impl fmt::Display for PythonInstallationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self.variant {
            PythonVariant::Default => String::new(),
            _ => format!("+{}", self.variant),
        };
        write!(
            f,
            "{}-{}.{}.{}{}{}-{}",
            self.implementation(),
            self.major,
            self.minor,
            self.patch,
            self.prerelease
                .map(|pre| pre.to_string())
                .unwrap_or_default(),
            variant,
            self.platform
        )
    }
}

impl CacheKey for PythonInstallationKey {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.hash(state);
    }
}

impl FromStr for PythonInstallationKey {
    type Err = PythonInstallationKeyError;

    fn from_str(key: &str) -> Result<Self, Self::Err> {
        let parts = key.split('-').collect::<Vec<_>>();

        // We need exactly implementation-version-os-arch-libc
        if parts.len() != 5 {
            return Err(PythonInstallationKeyError::ParseError(
                key.to_string(),
                format!(
                    "expected exactly 5 `-`-separated values, got {}",
                    parts.len()
                ),
            ));
        }

        let [implementation_str, version_str, os, arch, libc] = parts.as_slice() else {
            unreachable!()
        };

        let implementation = LenientImplementationName::from(*implementation_str);

        let (version, variant) = match version_str.split_once('+') {
            Some((version, variant)) => {
                let variant = PythonVariant::from_str(variant).map_err(|()| {
                    PythonInstallationKeyError::ParseError(
                        key.to_string(),
                        format!("invalid Python variant: {variant}"),
                    )
                })?;
                (version, variant)
            }
            None => (*version_str, PythonVariant::Default),
        };

        let version = PythonVersion::from_str(version).map_err(|err| {
            PythonInstallationKeyError::ParseError(
                key.to_string(),
                format!("invalid Python version: {err}"),
            )
        })?;

        let platform = Platform::from_parts(os, arch, libc).map_err(|err| {
            PythonInstallationKeyError::ParseError(
                key.to_string(),
                format!("invalid platform: {err}"),
            )
        })?;

        Ok(Self {
            implementation,
            major: version.major(),
            minor: version.minor(),
            patch: version.patch().unwrap_or_default(),
            prerelease: version.pre(),
            platform,
            variant,
        })
    }
}

impl PartialOrd for PythonInstallationKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PythonInstallationKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.implementation
            .cmp(&other.implementation)
            .then_with(|| self.version().cmp(&other.version()))
            // Platforms are sorted in preferred order for the target
            .then_with(|| self.platform.cmp(&other.platform).reverse())
            // Python variants are sorted in preferred order, with `Default` first
            .then_with(|| self.variant.cmp(&other.variant).reverse())
    }
}

/// A view into a [`PythonInstallationKey`] that excludes the patch and prerelease versions.
#[derive(Clone, Eq, RefCast)]
#[repr(transparent)]
pub struct PythonInstallationMinorVersionKey(PythonInstallationKey);

impl PythonInstallationMinorVersionKey {
    /// Cast a `&PythonInstallationKey` to a `&PythonInstallationMinorVersionKey` using ref-cast.
    #[inline]
    pub fn ref_cast(key: &PythonInstallationKey) -> &Self {
        RefCast::ref_cast(key)
    }

    /// Preserve the full key's preferences without its patch and prerelease versions.
    fn ordering_key(
        &self,
    ) -> (
        &LenientImplementationName,
        &u8,
        &u8,
        Reverse<&Platform>,
        Reverse<&PythonVariant>,
    ) {
        // Keep this exhaustive so new identity fields cannot be silently omitted.
        let PythonInstallationKey {
            implementation,
            major,
            minor,
            patch: _,
            prerelease: _,
            platform,
            variant,
        } = &self.0;
        (
            implementation,
            major,
            minor,
            Reverse(platform),
            Reverse(variant),
        )
    }
}

impl fmt::Display for PythonInstallationMinorVersionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display every field on the wrapped key except the patch
        // and prerelease (with special formatting for the variant).
        let variant = match self.0.variant {
            PythonVariant::Default => String::new(),
            _ => format!("+{}", self.0.variant),
        };
        write!(
            f,
            "{}-{}.{}{}-{}",
            self.0.implementation, self.0.major, self.0.minor, variant, self.0.platform,
        )
    }
}

impl fmt::Debug for PythonInstallationMinorVersionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display every field on the wrapped key except the patch
        // and prerelease.
        f.debug_struct("PythonInstallationMinorVersionKey")
            .field("implementation", &self.0.implementation)
            .field("major", &self.0.major)
            .field("minor", &self.0.minor)
            .field("variant", &self.0.variant)
            .field("os", &self.0.platform.os)
            .field("arch", &self.0.platform.arch)
            .field("libc", &self.0.platform.libc)
            .finish()
    }
}

impl PartialEq for PythonInstallationMinorVersionKey {
    fn eq(&self, other: &Self) -> bool {
        // Compare every field on the wrapped key except the patch
        // and prerelease.
        self.0.implementation == other.0.implementation
            && self.0.major == other.0.major
            && self.0.minor == other.0.minor
            && self.0.platform == other.0.platform
            && self.0.variant == other.0.variant
    }
}

impl PartialOrd for PythonInstallationMinorVersionKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PythonInstallationMinorVersionKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ordering_key()
            .cmp(&other.ordering_key())
            // Platform display can omit OS versions and libc identity. Break only those ties.
            .then_with(|| {
                self.0
                    .platform
                    .os
                    .into_str()
                    .cmp(&other.0.platform.os.into_str())
                    .reverse()
            })
            .then_with(|| match (&self.0.platform.libc, &other.0.platform.libc) {
                (Libc::None, Libc::None) | (Libc::Some(_), Libc::Some(_)) => Ordering::Equal,
                (Libc::None, Libc::Some(_)) => Ordering::Greater,
                (Libc::Some(_), Libc::None) => Ordering::Less,
            })
    }
}

impl Hash for PythonInstallationMinorVersionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash every field on the wrapped key except the patch
        // and prerelease.
        self.0.implementation.hash(state);
        self.0.major.hash(state);
        self.0.minor.hash(state);
        self.0.platform.hash(state);
        self.0.variant.hash(state);
    }
}

impl CacheKey for PythonInstallationMinorVersionKey {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.hash(state);
    }
}

impl From<PythonInstallationKey> for PythonInstallationMinorVersionKey {
    fn from(key: PythonInstallationKey) -> Self {
        Self(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uv_platform::ArchVariant;
    #[test]
    fn test_python_installation_key_from_str() {
        // Test basic parsing
        let key = PythonInstallationKey::from_str("cpython-3.12.0-linux-x86_64-gnu").unwrap();
        assert_eq!(
            key.implementation,
            LenientImplementationName::Known(ImplementationName::CPython)
        );
        assert_eq!(key.major, 3);
        assert_eq!(key.minor, 12);
        assert_eq!(key.patch, 0);
        assert_eq!(
            key.platform.os,
            Os::new(target_lexicon::OperatingSystem::Linux)
        );
        assert_eq!(
            key.platform.arch,
            Arch::new(target_lexicon::Architecture::X86_64, None)
        );
        assert_eq!(
            key.platform.libc,
            Libc::Some(target_lexicon::Environment::Gnu)
        );

        // Test with architecture variant
        let key = PythonInstallationKey::from_str("cpython-3.11.2-linux-x86_64_v3-musl").unwrap();
        assert_eq!(
            key.implementation,
            LenientImplementationName::Known(ImplementationName::CPython)
        );
        assert_eq!(key.major, 3);
        assert_eq!(key.minor, 11);
        assert_eq!(key.patch, 2);
        assert_eq!(
            key.platform.os,
            Os::new(target_lexicon::OperatingSystem::Linux)
        );
        assert_eq!(
            key.platform.arch,
            Arch::new(target_lexicon::Architecture::X86_64, Some(ArchVariant::V3))
        );
        assert_eq!(
            key.platform.libc,
            Libc::Some(target_lexicon::Environment::Musl)
        );

        // Test with Python variant (freethreaded)
        let key = PythonInstallationKey::from_str("cpython-3.13.0+freethreaded-macos-aarch64-none")
            .unwrap();
        assert_eq!(
            key.implementation,
            LenientImplementationName::Known(ImplementationName::CPython)
        );
        assert_eq!(key.major, 3);
        assert_eq!(key.minor, 13);
        assert_eq!(key.patch, 0);
        assert_eq!(key.variant, PythonVariant::Freethreaded);
        assert_eq!(
            key.platform.os,
            Os::new(target_lexicon::OperatingSystem::Darwin(None))
        );
        assert_eq!(
            key.platform.arch,
            Arch::new(
                target_lexicon::Architecture::Aarch64(target_lexicon::Aarch64Architecture::Aarch64),
                None
            )
        );
        assert_eq!(key.platform.libc, Libc::None);

        // Test error cases
        assert!(PythonInstallationKey::from_str("cpython-3.12.0-linux-x86_64").is_err());
        assert!(PythonInstallationKey::from_str("cpython-3.12.0").is_err());
        assert!(PythonInstallationKey::from_str("cpython").is_err());
    }

    #[test]
    fn test_python_installation_key_display() {
        let key = PythonInstallationKey {
            implementation: LenientImplementationName::from("cpython"),
            major: 3,
            minor: 12,
            patch: 0,
            prerelease: None,
            platform: Platform::from_str("linux-x86_64-gnu").unwrap(),
            variant: PythonVariant::Default,
        };
        assert_eq!(key.to_string(), "cpython-3.12.0-linux-x86_64-gnu");

        let key_with_variant = PythonInstallationKey {
            implementation: LenientImplementationName::from("cpython"),
            major: 3,
            minor: 13,
            patch: 0,
            prerelease: None,
            platform: Platform::from_str("macos-aarch64-none").unwrap(),
            variant: PythonVariant::Freethreaded,
        };
        assert_eq!(
            key_with_variant.to_string(),
            "cpython-3.13.0+freethreaded-macos-aarch64-none"
        );
    }
}

#[cfg(test)]
mod minor_version_key_ordering_tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

    use target_lexicon::{
        Aarch64Architecture, Architecture, DeploymentTarget, Environment, OperatingSystem,
    };
    use uv_pep440::PrereleaseKind;
    use uv_platform::ArchVariant;

    use super::*;

    fn key(
        patch: u8,
        prerelease: Option<Prerelease>,
        variant: PythonVariant,
    ) -> PythonInstallationKey {
        PythonInstallationKey::new(
            LenientImplementationName::Known(ImplementationName::CPython),
            3,
            12,
            patch,
            prerelease,
            Platform::new(
                Os::new(OperatingSystem::Linux),
                Arch::new(Architecture::X86_64, None),
                Libc::Some(Environment::Gnu),
            ),
            variant,
        )
    }

    #[test]
    fn ordering_ignores_patch_and_prerelease() {
        let keys = [
            key(
                0,
                Some(Prerelease {
                    kind: PrereleaseKind::Alpha,
                    number: 1,
                }),
                PythonVariant::Default,
            ),
            key(
                0,
                Some(Prerelease {
                    kind: PrereleaseKind::Rc,
                    number: 1,
                }),
                PythonVariant::Default,
            ),
            key(0, None, PythonVariant::Default),
            key(1, None, PythonVariant::Default),
            key(9, None, PythonVariant::Default),
        ];

        for pair in keys.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        for left in &keys {
            for right in &keys {
                let left = PythonInstallationMinorVersionKey::ref_cast(left);
                let right = PythonInstallationMinorVersionKey::ref_cast(right);
                assert_eq!(left, right);
                assert_eq!(left.cmp(right), Ordering::Equal);
                assert_eq!(left.partial_cmp(right), Some(Ordering::Equal));
            }
        }
    }

    #[test]
    fn ordering_preserves_identity_and_preference() {
        let base = key(0, None, PythonVariant::Default);
        let mut keys = vec![
            base.clone(),
            PythonInstallationKey {
                implementation: LenientImplementationName::Known(ImplementationName::PyPy),
                ..base.clone()
            },
            PythonInstallationKey {
                implementation: LenientImplementationName::Unknown("custom".to_owned()),
                ..base.clone()
            },
            PythonInstallationKey {
                major: 2,
                ..base.clone()
            },
            PythonInstallationKey {
                minor: 13,
                ..base.clone()
            },
            PythonInstallationKey {
                platform: Platform {
                    os: Os::new(OperatingSystem::Windows),
                    ..base.platform.clone()
                },
                ..base.clone()
            },
            PythonInstallationKey {
                platform: Platform {
                    arch: Arch::new(Architecture::X86_64, Some(ArchVariant::V3)),
                    ..base.platform.clone()
                },
                ..base.clone()
            },
            PythonInstallationKey {
                platform: Platform {
                    arch: Arch::new(Architecture::Aarch64(Aarch64Architecture::Aarch64), None),
                    ..base.platform.clone()
                },
                ..base.clone()
            },
            PythonInstallationKey {
                platform: Platform {
                    libc: Libc::Some(Environment::Musl),
                    ..base.platform.clone()
                },
                ..base
            },
        ];
        keys.extend(
            [
                PythonVariant::Default,
                PythonVariant::Debug,
                PythonVariant::Freethreaded,
                PythonVariant::FreethreadedDebug,
                PythonVariant::Gil,
                PythonVariant::GilDebug,
            ]
            .map(|variant| key(0, None, variant)),
        );

        for left in &keys {
            for right in &keys {
                let expected = left.cmp(right);
                let left = PythonInstallationMinorVersionKey::ref_cast(left);
                let right = PythonInstallationMinorVersionKey::ref_cast(right);
                assert_eq!(left.cmp(right), expected);
                assert_eq!(left.partial_cmp(right), Some(expected));
                assert_eq!(left.cmp(right) == Ordering::Equal, left == right);
            }
        }
    }

    #[test]
    fn ordered_collections_match_hash_identity() {
        let base = key(0, None, PythonVariant::Default);
        let keys = [
            base.clone(),
            key(1, None, PythonVariant::Default),
            key(9, None, PythonVariant::Default),
            PythonInstallationKey {
                minor: 13,
                ..base.clone()
            },
            key(0, None, PythonVariant::Freethreaded),
            PythonInstallationKey {
                implementation: LenientImplementationName::Known(ImplementationName::PyPy),
                ..base
            },
        ]
        .map(PythonInstallationMinorVersionKey::from);

        let hashed = keys.iter().cloned().collect::<HashSet<_>>();
        let ordered = keys.iter().cloned().collect::<BTreeSet<_>>();
        assert_eq!(hashed.len(), 4);
        assert_eq!(ordered.len(), hashed.len());
        assert!(hashed.iter().all(|key| ordered.contains(key)));

        let entries = || {
            keys.iter()
                .cloned()
                .enumerate()
                .map(|(index, key)| (key, index))
        };
        let hashed = entries().collect::<HashMap<_, _>>();
        let ordered = entries().collect::<BTreeMap<_, _>>();
        assert_eq!(ordered.len(), hashed.len());
        for (key, value) in hashed {
            assert_eq!(ordered.get(&key), Some(&value));
        }
    }

    #[test]
    fn platform_display_ties_remain_distinct() {
        let base = key(0, None, PythonVariant::Default);
        let keys = [
            None,
            Some(DeploymentTarget {
                major: 13,
                minor: 0,
                patch: 0,
            }),
            Some(DeploymentTarget {
                major: 14,
                minor: 0,
                patch: 0,
            }),
        ]
        .map(|version| PythonInstallationKey {
            platform: Platform::new(
                Os::new(OperatingSystem::Darwin(version)),
                Arch::new(Architecture::Aarch64(Aarch64Architecture::Aarch64), None),
                Libc::None,
            ),
            ..base.clone()
        });

        for left in &keys {
            for right in &keys {
                assert_eq!(left.platform.cmp(&right.platform), Ordering::Equal);
                assert_eq!(left.cmp(right), Ordering::Equal);
                let expected = left
                    .platform
                    .os
                    .into_str()
                    .cmp(&right.platform.os.into_str())
                    .reverse();
                let left = PythonInstallationMinorVersionKey::ref_cast(left);
                let right = PythonInstallationMinorVersionKey::ref_cast(right);
                assert_eq!(left.cmp(right), expected);
                assert_eq!(left.cmp(right) == Ordering::Equal, left == right);
            }
        }

        let implicit = &keys[0];
        let explicit = PythonInstallationKey {
            platform: Platform {
                libc: Libc::Some(Environment::None),
                ..implicit.platform.clone()
            },
            ..implicit.clone()
        };
        assert_ne!(implicit, &explicit);
        assert_eq!(implicit.cmp(&explicit), Ordering::Equal);
        assert_eq!(
            PythonInstallationMinorVersionKey::ref_cast(implicit)
                .cmp(PythonInstallationMinorVersionKey::ref_cast(&explicit)),
            Ordering::Greater,
        );

        // Existing runtime-variant preference wins before platform identity breaks a tie.
        let mut debug = key(0, None, PythonVariant::Debug);
        debug.platform = keys[1].platform.clone();
        assert_eq!(
            PythonInstallationMinorVersionKey::ref_cast(implicit)
                .cmp(PythonInstallationMinorVersionKey::ref_cast(&debug)),
            implicit.cmp(&debug),
        );

        let keys = keys
            .into_iter()
            .chain([explicit])
            .map(PythonInstallationMinorVersionKey::from);
        let hashed = keys.clone().collect::<HashSet<_>>();
        let ordered = keys.collect::<BTreeSet<_>>();
        assert_eq!(hashed.len(), 4);
        assert_eq!(ordered.len(), hashed.len());
    }
}
