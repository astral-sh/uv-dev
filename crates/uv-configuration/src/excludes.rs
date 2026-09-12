use std::str::FromStr;

use either::Either;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::de::Error;

use uv_normalize::PackageName;
use uv_pep440::Version;

use crate::Overrides;

/// A set of exclusions that applies to the dependencies of a specific package version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PackageExclusion {
    package: PackageExclusionTarget,
    dependencies: Box<[PackageName]>,
}

/// The package and optional version selected by a [`PackageExclusion`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PackageExclusionTarget {
    name: PackageName,
    #[cfg_attr(
        feature = "schemars",
        schemars(
            with = "Option<String>",
            description = "PEP 440-style package version, e.g., `1.2.3`"
        )
    )]
    version: Option<Version>,
}

/// An exclusion, either global or scoped to a specific package version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema), schemars(untagged))]
#[serde(untagged)]
pub enum ExcludeDependency {
    Package(PackageExclusion),
    Dependency(PackageName),
}

impl<'de> serde::Deserialize<'de> for ExcludeDependency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        serde_untagged::UntaggedEnumVisitor::new()
            .string(|string| {
                PackageName::from_str(string)
                    .map(Self::Dependency)
                    .map_err(Error::custom)
            })
            .map(|map| map.deserialize().map(Self::Package))
            .deserialize(deserializer)
    }
}

/// A set of packages to exclude from resolution.
#[derive(Debug, Default, Clone)]
pub struct Excludes {
    global: FxHashSet<PackageName>,
    scoped: FxHashMap<PackageName, ScopedExclusions>,
}

#[derive(Debug, Default, Clone)]
struct ScopedExclusions {
    versionless: Option<FxHashSet<PackageName>>,
    versioned: VersionedExclusions,
}

#[derive(Debug, Clone)]
enum VersionedExclusions {
    Small(Vec<(Version, FxHashSet<PackageName>)>),
    Indexed(FxHashMap<Version, FxHashSet<PackageName>>),
}

impl Default for VersionedExclusions {
    fn default() -> Self {
        Self::Small(Vec::new())
    }
}

impl VersionedExclusions {
    const INDEX_THRESHOLD: usize = 16;

    fn extend(&mut self, version: Version, dependencies: Box<[PackageName]>) {
        match self {
            Self::Small(entries) => {
                if let Some((_, excludes)) = entries
                    .iter_mut()
                    .find(|(entry_version, _)| entry_version == &version)
                {
                    excludes.extend(dependencies);
                    return;
                }
                if entries.len() < Self::INDEX_THRESHOLD {
                    entries.push((version, dependencies.into_iter().collect()));
                    return;
                }

                let mut indexed = std::mem::take(entries)
                    .into_iter()
                    .collect::<FxHashMap<_, _>>();
                indexed.entry(version).or_default().extend(dependencies);
                *self = Self::Indexed(indexed);
            }
            Self::Indexed(entries) => entries.entry(version).or_default().extend(dependencies),
        }
    }

    fn get(&self, version: &Version) -> Option<&FxHashSet<PackageName>> {
        match self {
            Self::Small(entries) => entries
                .iter()
                .find(|(entry_version, _)| entry_version == version)
                .map(|(_, excludes)| excludes),
            Self::Indexed(entries) => entries.get(version),
        }
    }

    fn iter(&self) -> impl Iterator<Item = (&Version, &FxHashSet<PackageName>)> {
        match self {
            Self::Small(entries) => Either::Left(
                entries
                    .iter()
                    .map(|(version, excludes)| (version, excludes)),
            ),
            Self::Indexed(entries) => Either::Right(entries.iter()),
        }
    }
}

impl Excludes {
    /// Create an indexed set of exclusions.
    pub fn from_entries(entries: impl IntoIterator<Item = ExcludeDependency>) -> Self {
        let mut excludes = Self::default();
        for entry in entries {
            match entry {
                ExcludeDependency::Dependency(dependency) => {
                    excludes.global.insert(dependency);
                }
                ExcludeDependency::Package(package) => {
                    let scoped = excludes.scoped.entry(package.package.name).or_default();
                    if let Some(version) = package.package.version {
                        scoped.versioned.extend(version, package.dependencies);
                    } else {
                        scoped
                            .versionless
                            .get_or_insert_default()
                            .extend(package.dependencies);
                    }
                }
            }
        }
        excludes
    }

    /// Check if a package is excluded.
    pub fn contains(&self, name: &PackageName) -> bool {
        self.global.contains(name)
    }

    /// Check if a dependency is excluded from a specific package version.
    pub fn contains_for(
        &self,
        package: &PackageName,
        version: &Version,
        dependency: &PackageName,
    ) -> bool {
        self.contains_for_package(Some((package, version)), dependency)
    }

    /// Check if a dependency is always excluded from a package scope.
    ///
    /// A versionless scope remains eligible if any exact-version exclusion allows the dependency
    /// at a version where the override is not shadowed by an exact override scope.
    pub fn contains_for_scope(
        &self,
        overrides: &Overrides,
        package: &PackageName,
        version: Option<&Version>,
        dependency: &PackageName,
    ) -> bool {
        if let Some(version) = version {
            return self.contains_for(package, version, dependency);
        }
        if self.contains(dependency) {
            return true;
        }

        let Some(entries) = self.scoped.get(package) else {
            return false;
        };
        entries
            .versionless
            .as_ref()
            .is_some_and(|excludes| excludes.contains(dependency))
            && entries
                .versioned
                .iter()
                .filter(|(version, _)| !overrides.has_exact_scope(package, version))
                .all(|(_, excludes)| excludes.contains(dependency))
    }

    /// Check if a dependency is excluded with optional package-version context.
    pub fn contains_for_package(
        &self,
        package: Option<(&PackageName, &Version)>,
        dependency: &PackageName,
    ) -> bool {
        self.contains(dependency)
            || package.is_some_and(|(package, version)| {
                self.scoped.get(package).is_some_and(|entries| {
                    entries
                        .versioned
                        .get(version)
                        .or(entries.versionless.as_ref())
                        .is_some_and(|excludes| excludes.contains(dependency))
                })
            })
    }
}

impl FromIterator<PackageName> for Excludes {
    fn from_iter<I: IntoIterator<Item = PackageName>>(iter: I) -> Self {
        Self::from_entries(iter.into_iter().map(ExcludeDependency::Dependency))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde_json::json;
    use uv_distribution_types::{Requirement, RequirementSource};
    use uv_pep440::VersionSpecifiers;
    use uv_pep508::MarkerTree;

    use super::*;
    use crate::{Override, PackageOverride, PackageOverrideTarget};

    fn package(name: &str) -> Result<PackageName> {
        Ok(PackageName::from_str(name)?)
    }

    fn version(version: &str) -> Result<Version> {
        Ok(Version::from_str(version)?)
    }

    fn scoped(
        package_name: &str,
        package_version: Option<&str>,
        dependencies: &[&str],
    ) -> Result<ExcludeDependency> {
        Ok(ExcludeDependency::Package(PackageExclusion {
            package: PackageExclusionTarget {
                name: package(package_name)?,
                version: package_version.map(version).transpose()?,
            },
            dependencies: dependencies
                .iter()
                .map(|dependency| package(dependency))
                .collect::<Result<_>>()?,
        }))
    }

    #[test]
    fn merges_duplicate_scopes_and_prefers_exact_versions() -> Result<()> {
        let excludes = Excludes::from_entries([
            scoped("Parent_Name", None, &["fallback-one"])?,
            scoped("parent-name", Some("1.0.0.0"), &["exact-one"])?,
            scoped("parent-name", None, &["fallback-two"])?,
            scoped("parent-name", Some("1.0"), &["exact-two"])?,
            scoped("parent-name", Some("2.0"), &[])?,
            ExcludeDependency::Dependency(package("global")?),
        ]);
        let parent = package("parent-name")?;

        for dependency in ["exact-one", "exact-two"] {
            assert!(excludes.contains_for(&parent, &version("1.0")?, &package(dependency)?));
        }
        for dependency in ["fallback-one", "fallback-two"] {
            assert!(!excludes.contains_for(&parent, &version("1.0")?, &package(dependency)?));
            assert!(!excludes.contains_for(&parent, &version("2.0")?, &package(dependency)?));
            assert!(excludes.contains_for(&parent, &version("3.0")?, &package(dependency)?));
        }
        assert!(excludes.contains_for(&parent, &version("1.0")?, &package("global")?));
        assert!(excludes.contains_for_package(None, &package("global")?));
        assert!(!excludes.contains_for_package(None, &package("fallback-one")?));

        Ok(())
    }

    #[test]
    fn versionless_scope_respects_exact_override_shadowing() -> Result<()> {
        let excludes = Excludes::from_entries([
            scoped("parent", None, &["child"])?,
            scoped("parent", Some("1.0"), &["different"])?,
            scoped("parent", Some("2.0"), &["child"])?,
        ]);
        let parent = package("parent")?;
        let child = package("child")?;
        let overrides = Overrides::from_entries(vec![Override::Package(PackageOverride {
            package: serde_json::from_value::<PackageOverrideTarget>(
                json!({ "name": "parent", "version": "1.0" }),
            )?,
            dependencies: Box::new([Requirement {
                name: package("different")?,
                extras: Box::default(),
                groups: Box::default(),
                marker: MarkerTree::TRUE,
                source: RequirementSource::Registry {
                    specifier: VersionSpecifiers::from_str("==1.0")?,
                    index: None,
                    conflict: None,
                },
                origin: None,
            }]),
        })])?;

        assert!(!excludes.contains_for_scope(&Overrides::default(), &parent, None, &child,));
        assert!(excludes.contains_for_scope(&overrides, &parent, None, &child));
        assert!(!excludes.contains_for_scope(&overrides, &parent, Some(&version("1.0")?), &child,));
        assert!(excludes.contains_for_scope(&overrides, &parent, Some(&version("2.0")?), &child,));

        Ok(())
    }

    #[test]
    fn indexes_large_scopes_and_merges_duplicate_versions() -> Result<()> {
        let mut entries = (0..=VersionedExclusions::INDEX_THRESHOLD)
            .map(|minor| scoped("parent", Some(&format!("1.{minor}")), &["child"]))
            .collect::<Result<Vec<_>>>()?;
        entries.push(scoped("parent", Some("1.0.0.0"), &["another"])?);
        entries.push(scoped("parent", Some("2.0"), &[])?);
        entries.push(scoped("parent", None, &["fallback"])?);
        let excludes = Excludes::from_entries(entries);
        let parent = package("parent")?;

        assert!(matches!(
            excludes.scoped.get(&parent).map(|entry| &entry.versioned),
            Some(VersionedExclusions::Indexed(_))
        ));
        assert!(excludes.contains_for(&parent, &version("1.0")?, &package("child")?));
        assert!(excludes.contains_for(&parent, &version("1.0")?, &package("another")?));
        assert!(excludes.contains_for(
            &parent,
            &version(&format!("1.{}", VersionedExclusions::INDEX_THRESHOLD))?,
            &package("child")?,
        ));
        assert!(!excludes.contains_for(&parent, &version("2.0")?, &package("fallback")?));
        assert!(excludes.contains_for(&parent, &version("3.0")?, &package("fallback")?));

        Ok(())
    }
}
