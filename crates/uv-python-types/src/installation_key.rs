use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use ref_cast::RefCast;
use thiserror::Error;
use uv_cache_key::{CacheKey, CacheKeyHasher};
use uv_pep440::Prerelease;
use uv_platform::{Arch, Libc, Os, Platform};

use crate::request::parse_python_variant_and_build_name;
use crate::{
    ImplementationName, LenientImplementationName, PythonBuildName, PythonVariant, PythonVersion,
};

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
    pub build_name: Option<PythonBuildName>,
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
            build_name: None,
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
            build_name: None,
        }
    }

    /// Return a new installation key with the given build name.
    #[must_use]
    pub fn with_build_name(mut self, build_name: PythonBuildName) -> Self {
        self.build_name = Some(build_name);
        self
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

    /// Return a registry tag that distinguishes Python variants and build names.
    #[cfg(windows)]
    pub fn registry_tag(&self) -> String {
        // Preserve the Python variant spelling used by older uv versions so registry cleanup recognizes
        // their registrations as belonging to installations that are still present.
        let mut tag = format!(
            "{}{}{}",
            self.implementation().pretty(),
            self.version(),
            self.variant.executable_suffix(),
        );
        if let Some(build_name) = &self.build_name {
            tag.push('+');
            tag.push_str(&build_name.to_string());
        }
        tag
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

    pub fn build_name(&self) -> Option<&PythonBuildName> {
        self.build_name.as_ref()
    }

    fn display_variant_suffix(&self) -> String {
        let mut suffix = match self.variant {
            PythonVariant::Default => String::new(),
            _ => format!("+{}", self.variant),
        };
        if let Some(build_name) = &self.build_name {
            suffix.push('+');
            suffix.push_str(&build_name.to_string());
        }
        suffix
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
        let variant = self.display_variant_suffix();
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

        let (version, variant, build_name) = match version_str.split_once('+') {
            Some((version, variant)) => {
                let (variant, build_name) =
                    parse_python_variant_and_build_name(variant).map_err(|()| {
                        PythonInstallationKeyError::ParseError(
                            key.to_string(),
                            format!("invalid Python variant or build name: {variant}"),
                        )
                    })?;
                (version, variant, build_name)
            }
            None => (*version_str, PythonVariant::Default, None),
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

        let mut key = Self::new_from_version(implementation, &version, platform, variant);
        if let Some(build_name) = build_name {
            key = key.with_build_name(build_name);
        }
        Ok(key)
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
            // Unnamed builds are preferred over named builds
            .then_with(|| self.build_name.cmp(&other.build_name).reverse())
    }
}

/// A view into a [`PythonInstallationKey`] that excludes the patch and prerelease versions.
#[derive(Clone, Eq, Ord, PartialOrd, RefCast)]
#[repr(transparent)]
pub struct PythonInstallationMinorVersionKey(PythonInstallationKey);

impl PythonInstallationMinorVersionKey {
    /// Cast a `&PythonInstallationKey` to a `&PythonInstallationMinorVersionKey` using ref-cast.
    #[inline]
    pub fn ref_cast(key: &PythonInstallationKey) -> &Self {
        RefCast::ref_cast(key)
    }
}

impl fmt::Display for PythonInstallationMinorVersionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display every field on the wrapped key except the patch
        // and prerelease (with special formatting for the variant).
        let variant = self.0.display_variant_suffix();
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
            .field("build_name", &self.0.build_name)
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
            && self.0.build_name == other.0.build_name
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
        self.0.build_name.hash(state);
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

    #[cfg(windows)]
    #[test]
    fn test_python_installation_key_registry_tag() -> Result<(), PythonInstallationKeyError> {
        // Keep the registry names written by older uv versions, including when build names
        // are added alongside existing installations.
        for (variants, expected) in [
            ("", "CPython3.13.7"),
            ("+gil", "CPython3.13.7"),
            ("+debug", "CPython3.13.7d"),
            ("+gil+debug", "CPython3.13.7d"),
            ("+freethreaded", "CPython3.13.7t"),
            ("+freethreaded+debug", "CPython3.13.7td"),
            ("+custom", "CPython3.13.7+custom"),
            ("+gil+custom", "CPython3.13.7+custom"),
            ("+debug+custom", "CPython3.13.7d+custom"),
            ("+gil+debug+custom", "CPython3.13.7d+custom"),
            ("+freethreaded+custom", "CPython3.13.7t+custom"),
            ("+freethreaded+debug+custom", "CPython3.13.7td+custom"),
            ("+freethreaded+custom+debug", "CPython3.13.7td+custom"),
            ("+debug+freethreaded+custom", "CPython3.13.7td+custom"),
            ("+debug+custom+freethreaded", "CPython3.13.7td+custom"),
            ("+custom+freethreaded+debug", "CPython3.13.7td+custom"),
            ("+custom+debug+freethreaded", "CPython3.13.7td+custom"),
            ("+td+custom", "CPython3.13.7td+custom"),
            ("+custom+td", "CPython3.13.7td+custom"),
            ("+custom+gil+debug", "CPython3.13.7d+custom"),
            ("+custom+debug+gil", "CPython3.13.7d+custom"),
            ("+custom+debug", "CPython3.13.7d+custom"),
            ("+custom+freethreaded", "CPython3.13.7t+custom"),
            ("+custom_internal", "CPython3.13.7+custom_internal"),
            (
                "+freethreaded+custom_internal",
                "CPython3.13.7t+custom_internal",
            ),
        ] {
            let key = PythonInstallationKey::from_str(&format!(
                "cpython-3.13.7{variants}-windows-x86_64-none"
            ))?;
            assert_eq!(key.registry_tag(), expected);
        }
        Ok(())
    }

    #[test]
    fn test_python_installation_key_from_str() -> Result<(), PythonInstallationKeyError> {
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

        // Test with separate Python variants and build names
        for (python_variant, expected) in [
            ("freethreaded", PythonVariant::Freethreaded),
            ("freethreaded+debug", PythonVariant::FreethreadedDebug),
            ("gil+debug", PythonVariant::GilDebug),
        ] {
            let key = PythonInstallationKey::from_str(&format!(
                "cpython-3.13.0+{python_variant}+custom_internal-macos-aarch64-none",
            ))?;
            assert_eq!(key.variant, expected);
            assert_eq!(
                key.build_name.as_ref().map(ToString::to_string),
                Some("custom_internal".to_string())
            );
        }

        // Equivalent suffixes have one installation identity and directory name.
        for (canonical, variants) in [
            (
                "freethreaded+debug+custom",
                &[
                    "freethreaded+custom+debug",
                    "debug+freethreaded+custom",
                    "debug+custom+freethreaded",
                    "custom+freethreaded+debug",
                    "custom+debug+freethreaded",
                    "CUSTOM+FREETHREADED+DEBUG",
                    "CUSTOM+DEBUG+FREETHREADED",
                    "td+custom",
                    "custom+td",
                ][..],
            ),
            (
                "gil+debug+custom",
                &[
                    "gil+custom+debug",
                    "debug+gil+custom",
                    "debug+custom+gil",
                    "custom+gil+debug",
                    "custom+debug+gil",
                ][..],
            ),
            ("freethreaded+custom", &["custom+freethreaded"][..]),
            ("debug+custom", &["custom+debug"][..]),
            ("gil+custom", &["custom+gil"][..]),
            ("freethreaded", &["t"][..]),
            ("debug", &["d"][..]),
            ("freethreaded+debug", &["td", "debug+freethreaded"][..]),
            ("gil+debug", &["debug+gil"][..]),
        ] {
            let canonical = format!("cpython-3.13.0+{canonical}-macos-aarch64-none");
            let expected = PythonInstallationKey::from_str(&canonical)?;
            for variant in variants {
                let key = PythonInstallationKey::from_str(&format!(
                    "cpython-3.13.0+{variant}-macos-aarch64-none",
                ))?;
                assert_eq!(key, expected, "variant: {variant}");
                assert_eq!(key.to_string(), canonical, "variant: {variant}");
            }
        }

        // Test error cases
        assert!(PythonInstallationKey::from_str("cpython-3.12.0-linux-x86_64").is_err());
        assert!(PythonInstallationKey::from_str("cpython-3.12.0").is_err());
        assert!(PythonInstallationKey::from_str("cpython").is_err());
        for variants in [
            "+custom",
            "custom+",
            "custom++debug",
            "custom+internal",
            "custom+custom",
            "freethreaded+custom+internal",
            "freethreaded+debug+custom+internal",
            "pgo+lto",
            "freethreaded+",
            "t+d",
            "d+t",
            "t+debug",
            "debug+t",
            "freethreaded+d",
            "d+freethreaded",
            "gil+d",
            "d+gil",
            "custom+t+d",
            "custom+d+t",
            "t+d+custom",
            "t+custom+d",
            "debug+debug",
            "d+debug",
            "td+debug",
            "td+freethreaded",
            "freethreaded+t",
            "gil+freethreaded",
            "freethreaded+gil",
            "debug+custom+gil+freethreaded",
        ] {
            assert!(
                PythonInstallationKey::from_str(&format!(
                    "cpython-3.13.0+{variants}-macos-aarch64-none",
                ))
                .is_err(),
                "variants: {variants}"
            );
        }
        Ok(())
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
            build_name: None,
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
            build_name: None,
        };
        assert_eq!(
            key_with_variant.to_string(),
            "cpython-3.13.0+freethreaded-macos-aarch64-none"
        );

        let key_with_build_name = PythonInstallationKey {
            implementation: LenientImplementationName::from("cpython"),
            major: 3,
            minor: 13,
            patch: 0,
            prerelease: None,
            platform: Platform::from_str("linux-x86_64-gnu").unwrap(),
            variant: PythonVariant::Default,
            build_name: Some(PythonBuildName::from_str("custom_internal").unwrap()),
        };
        assert_eq!(
            key_with_build_name.to_string(),
            "cpython-3.13.0+custom_internal-linux-x86_64-gnu"
        );
        assert_eq!(
            key_with_build_name.executable_name_minor(),
            format!("python3.13{}", std::env::consts::EXE_SUFFIX)
        );
    }
}
