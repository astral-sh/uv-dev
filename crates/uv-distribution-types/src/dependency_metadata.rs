use std::fmt;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use uv_normalize::{ExtraName, PackageName};
use uv_pep440::{Version, VersionSpecifiers};
use uv_pep508::Requirement;
use uv_pypi_types::{ResolutionMetadata, VerbatimParsedUrl};

const MAX_LINEAR_METADATA_ENTRIES: usize = 32;

/// Pre-defined [`StaticMetadata`] entries, indexed by [`PackageName`] and [`Version`].
#[derive(Clone, Default)]
pub struct DependencyMetadata {
    entries: FxHashMap<PackageName, Vec<StaticMetadata>>,
    index: FxHashMap<PackageName, DependencyMetadataIndex>,
}

impl fmt::Debug for DependencyMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DependencyMetadata")
            .field(&self.entries)
            .finish()
    }
}

#[derive(Clone)]
struct DependencyMetadataIndex {
    versions: FxHashMap<Version, usize>,
    global: Option<usize>,
}

impl DependencyMetadataIndex {
    fn new(entries: &[StaticMetadata]) -> Self {
        let mut versions = FxHashMap::default();
        versions.reserve(entries.len());
        let mut global = None;
        for (index, entry) in entries.iter().enumerate() {
            if let Some(version) = entry.version.as_ref() {
                versions.entry(version.clone()).or_insert(index);
            } else {
                global.get_or_insert(index);
            }
        }

        Self { versions, global }
    }

    fn exact<'a>(
        &self,
        entries: &'a [StaticMetadata],
        version: &Version,
    ) -> Option<&'a StaticMetadata> {
        entries.get(*self.versions.get(version)?)
    }

    fn global<'a>(&self, entries: &'a [StaticMetadata]) -> Option<&'a StaticMetadata> {
        entries.get(self.global?)
    }
}

impl DependencyMetadata {
    /// Index a set of [`StaticMetadata`] entries by [`PackageName`] and [`Version`].
    pub fn from_entries(entries: impl IntoIterator<Item = StaticMetadata>) -> Self {
        let mut map = Self::default();
        for entry in entries {
            map.entries
                .entry(entry.name.clone())
                .or_default()
                .push(entry);
        }
        for (package, entries) in &map.entries {
            if entries.len() > MAX_LINEAR_METADATA_ENTRIES {
                map.index
                    .insert(package.clone(), DependencyMetadataIndex::new(entries));
            }
        }
        map
    }

    /// Retrieve a [`StaticMetadata`] entry by [`PackageName`] and [`Version`].
    pub fn get(
        &self,
        package: &PackageName,
        version: Option<&Version>,
    ) -> Option<ResolutionMetadata> {
        let entries = self.entries.get(package)?;

        if let Some(version) = version {
            // If a specific version was requested, search for an exact match, then a global match.
            let index = if entries.len() > MAX_LINEAR_METADATA_ENTRIES {
                self.index.get(package)
            } else {
                None
            };
            let exact = index.map_or_else(
                || {
                    entries
                        .iter()
                        .find(|entry| entry.version.as_ref() == Some(version))
                },
                |index| index.exact(entries, version),
            );
            let global = || {
                index.map_or_else(
                    || entries.iter().find(|entry| entry.version.is_none()),
                    |index| index.global(entries),
                )
            };
            let metadata = if let Some(metadata) = exact {
                debug!("Found dependency metadata entry for `{package}=={version}`");
                metadata
            } else if let Some(metadata) = global() {
                debug!("Found global metadata entry for `{package}`");
                metadata
            } else {
                warn!("No dependency metadata entry found for `{package}=={version}`");
                return None;
            };

            Some(ResolutionMetadata {
                name: metadata.name.clone(),
                version: version.clone(),
                requires_dist: metadata.requires_dist.clone(),
                requires_python: metadata.requires_python.clone(),
                provides_extra: metadata.provides_extra.clone(),
                dynamic: false,
            })
        } else {
            // If no version was requested (i.e., it's a direct URL dependency), allow a single
            // versioned match.
            let [metadata] = entries.as_slice() else {
                warn!("Multiple dependency metadata entries found for `{package}`");
                return None;
            };
            let Some(version) = metadata.version.clone() else {
                warn!("No version found in dependency metadata entry for `{package}`");
                return None;
            };
            debug!("Found dependency metadata entry for `{package}` (assuming: `{version}`)");

            Some(ResolutionMetadata {
                name: metadata.name.clone(),
                version,
                requires_dist: metadata.requires_dist.clone(),
                requires_python: metadata.requires_python.clone(),
                provides_extra: metadata.provides_extra.clone(),
                dynamic: false,
            })
        }
    }

    /// Retrieve all [`StaticMetadata`] entries.
    pub fn values(&self) -> impl Iterator<Item = &StaticMetadata> {
        self.entries.values().flatten()
    }
}

/// A subset of the Python Package Metadata 2.3 standard as specified in
/// <https://packaging.python.org/specifications/core-metadata/>.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct StaticMetadata {
    // Mandatory fields
    pub name: PackageName,
    #[cfg_attr(
        feature = "schemars",
        schemars(
            with = "Option<String>",
            description = "PEP 440-style package version, e.g., `1.2.3`"
        )
    )]
    pub version: Option<Version>,
    // Optional fields
    #[serde(default)]
    pub requires_dist: Box<[Requirement<VerbatimParsedUrl>]>,
    #[cfg_attr(
        feature = "schemars",
        schemars(
            with = "Option<String>",
            description = "PEP 508-style Python requirement, e.g., `>=3.10`"
        )
    )]
    pub requires_python: Option<VersionSpecifiers>,
    #[serde(default, alias = "provides-extras")]
    pub provides_extra: Box<[ExtraName]>,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_normalize::{ExtraName, PackageName};
    use uv_pep440::Version;

    use super::{DependencyMetadata, MAX_LINEAR_METADATA_ENTRIES, StaticMetadata};

    fn metadata(name: &str, version: Option<Version>, extra: &str) -> StaticMetadata {
        StaticMetadata {
            name: PackageName::from_str(name).unwrap(),
            version,
            requires_dist: Box::default(),
            requires_python: None,
            provides_extra: Box::new([ExtraName::from_str(extra).unwrap()]),
        }
    }

    #[test]
    fn indexed_lookup_preserves_precedence() {
        let package = PackageName::from_str("demo-package").unwrap();
        let mut entries = vec![
            metadata("demo_package", None, "first-global"),
            metadata("demo-package", Some(Version::new([1, 0])), "first-exact"),
            metadata(
                "demo-package",
                Some(Version::new([1, 0, 0])),
                "duplicate-exact",
            ),
            metadata("demo-package", Some(Version::new([2])), "second-exact"),
            metadata("demo-package", None, "duplicate-global"),
        ];
        entries.extend((0..MAX_LINEAR_METADATA_ENTRIES).map(|version| {
            metadata(
                "demo-package",
                Some(Version::new([10 + version as u64])),
                "unrelated",
            )
        }));
        let metadata = DependencyMetadata::from_entries(entries);

        assert_eq!(
            metadata
                .get(&package, Some(&Version::new([1])))
                .unwrap()
                .provides_extra[0],
            ExtraName::from_str("first-exact").unwrap()
        );
        assert_eq!(
            metadata
                .get(&package, Some(&Version::new([2])))
                .unwrap()
                .provides_extra[0],
            ExtraName::from_str("second-exact").unwrap()
        );
        assert_eq!(
            metadata
                .get(&package, Some(&Version::new([3])))
                .unwrap()
                .provides_extra[0],
            ExtraName::from_str("first-global").unwrap()
        );
    }

    #[test]
    fn versionless_lookup_requires_one_versioned_entry() {
        let package = PackageName::from_str("demo-package").unwrap();

        let single = DependencyMetadata::from_entries([metadata(
            "demo-package",
            Some(Version::new([1])),
            "single",
        )]);
        assert_eq!(
            single.get(&package, None).unwrap().version,
            Version::new([1])
        );

        let global = DependencyMetadata::from_entries([metadata("demo-package", None, "global")]);
        assert!(global.get(&package, None).is_none());

        let duplicate = DependencyMetadata::from_entries([
            metadata("demo-package", Some(Version::new([1])), "first"),
            metadata("demo-package", Some(Version::new([1, 0])), "duplicate"),
        ]);
        assert!(duplicate.get(&package, None).is_none());
    }

    #[test]
    fn values_and_debug_keep_original_entries() {
        let metadata = DependencyMetadata::from_entries([
            metadata("demo-package", Some(Version::new([2])), "second"),
            metadata("demo-package", None, "global"),
            metadata("demo-package", Some(Version::new([1])), "first"),
        ]);
        assert_eq!(
            metadata
                .values()
                .flat_map(|entry| &entry.provides_extra)
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["second", "global", "first"]
        );
        let debug = format!("{metadata:#?}");
        assert!(!debug.contains("index"));
        assert_eq!(
            format!("{:#?}", DependencyMetadata::default()),
            "DependencyMetadata(\n    {},\n)"
        );
    }
}
