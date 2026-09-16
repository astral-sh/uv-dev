use std::borrow::Cow;

use either::Either;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use serde::de::IntoDeserializer;
use version_ranges::Ranges;

use uv_distribution_types::{Requirement, RequirementSource};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pep508::MarkerTree;

/// An override that applies to the dependencies of a specific package version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(
    rename_all = "kebab-case",
    deny_unknown_fields,
    bound(
        serialize = "T: serde::Serialize",
        deserialize = "T: serde::Deserialize<'de>"
    )
)]
pub struct PackageOverride<T> {
    pub package: PackageOverrideTarget,
    pub dependencies: Box<[T]>,
}

/// Replace requirements in a version range with a differently named dependency.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct RequirementReplacement<T> {
    pub requirement: T,
    pub replacement: T,
}

/// The package and optional version selected by a [`PackageOverride`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PackageOverrideTarget {
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

/// A same-name override, a package-scoped override, or a requirement replacement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema), schemars(untagged))]
#[serde(untagged, bound(serialize = "T: serde::Serialize"))]
pub enum Override<T> {
    Package(PackageOverride<T>),
    Replacement(RequirementReplacement<T>),
    Requirement(T),
}

impl<T> Override<T> {
    /// Transform every requirement in an override.
    pub fn map<U>(self, mut map: impl FnMut(T) -> U) -> Override<U> {
        match self {
            Self::Package(package) => Override::Package(PackageOverride {
                package: package.package,
                dependencies: package
                    .dependencies
                    .into_vec()
                    .into_iter()
                    .map(map)
                    .collect(),
            }),
            Self::Replacement(replacement) => Override::Replacement(RequirementReplacement {
                requirement: map(replacement.requirement),
                replacement: map(replacement.replacement),
            }),
            Self::Requirement(requirement) => Override::Requirement(map(requirement)),
        }
    }

    /// Fallibly transform every requirement in an override.
    pub fn try_map<U, E>(self, mut map: impl FnMut(T) -> Result<U, E>) -> Result<Override<U>, E> {
        Ok(match self {
            Self::Package(package) => Override::Package(PackageOverride {
                package: package.package,
                dependencies: package
                    .dependencies
                    .into_vec()
                    .into_iter()
                    .map(map)
                    .collect::<Result<_, _>>()?,
            }),
            Self::Replacement(replacement) => Override::Replacement(RequirementReplacement {
                requirement: map(replacement.requirement)?,
                replacement: map(replacement.replacement)?,
            }),
            Self::Requirement(requirement) => Override::Requirement(map(requirement)?),
        })
    }
}

// A derived `#[serde(untagged)]` implementation collapses detailed requirement parse errors into
// "data did not match any variant", so use a type-directed visitor for string requirements.
impl<'de, T> serde::Deserialize<'de> for Override<T>
where
    T: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum MapOverride<T> {
            Package(PackageOverride<T>),
            Replacement(RequirementReplacement<T>),
            Requirement(T),
        }

        serde_untagged::UntaggedEnumVisitor::new()
            .string(|string| T::deserialize(string.into_deserializer()).map(Self::Requirement))
            .map(|map| {
                map.deserialize::<MapOverride<T>>()
                    .map(|entry| match entry {
                        MapOverride::Package(package) => Self::Package(package),
                        MapOverride::Replacement(replacement) => Self::Replacement(replacement),
                        MapOverride::Requirement(requirement) => Self::Requirement(requirement),
                    })
            })
            .deserialize(deserializer)
    }
}

/// A set of overrides for a set of requirements.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    global: FxHashMap<PackageName, Vec<Requirement>>,
    scoped: FxHashMap<PackageName, Vec<ScopedOverrides>>,
    replacements: FxHashMap<PackageName, Vec<RequirementReplacement<Requirement>>>,
}

#[derive(Debug, Clone)]
struct ScopedOverrides {
    version: Option<Version>,
    overrides: FxHashMap<PackageName, Vec<Requirement>>,
}

/// An invalid dependency override.
#[derive(Debug, thiserror::Error)]
pub enum OverrideError {
    #[error(
        "Replacement selector for `{dependency}` must be a plain registry requirement without extras, groups, markers, or an explicit index"
    )]
    ReplacementSelector { dependency: PackageName },
    #[error("Replacement for `{dependency}` must name a different package")]
    ReplacementName { dependency: PackageName },
    #[error("Replacement selectors for `{dependency}` overlap")]
    OverlappingReplacements { dependency: PackageName },
    #[error(
        "Scoped override for `{package}` cannot use a URL or path source for `{dependency}`; scoped overrides currently support version specifiers only"
    )]
    Url {
        package: PackageName,
        dependency: PackageName,
    },
    #[error(
        "Scoped override for `{package}` cannot use an explicit index for `{dependency}`; scoped overrides currently support version specifiers only"
    )]
    Index {
        package: PackageName,
        dependency: PackageName,
    },
}

impl Overrides {
    /// Create a new set of overrides from a set of requirements.
    pub fn from_requirements(requirements: Vec<Requirement>) -> Self {
        let mut global: FxHashMap<PackageName, Vec<Requirement>> =
            FxHashMap::with_capacity_and_hasher(requirements.len(), FxBuildHasher);
        for requirement in requirements {
            global
                .entry(requirement.name.clone())
                .or_default()
                .push(requirement);
        }
        Self {
            global,
            scoped: FxHashMap::default(),
            replacements: FxHashMap::default(),
        }
    }

    /// Create an indexed set of overrides.
    pub fn from_entries(entries: Vec<Override<Requirement>>) -> Result<Self, OverrideError> {
        let mut global: FxHashMap<PackageName, Vec<Requirement>> =
            FxHashMap::with_capacity_and_hasher(entries.len(), FxBuildHasher);
        let mut scoped: FxHashMap<PackageName, Vec<ScopedOverrides>> = FxHashMap::default();
        let mut replacements: FxHashMap<PackageName, Vec<RequirementReplacement<Requirement>>> =
            FxHashMap::default();

        for entry in entries {
            match entry {
                Override::Replacement(replacement) => {
                    let selector = &replacement.requirement;
                    let RequirementSource::Registry {
                        specifier,
                        index: None,
                        conflict: None,
                        ..
                    } = &selector.source
                    else {
                        return Err(OverrideError::ReplacementSelector {
                            dependency: selector.name.clone(),
                        });
                    };
                    if !selector.extras.is_empty()
                        || !selector.groups.is_empty()
                        || !selector.marker.is_true()
                    {
                        return Err(OverrideError::ReplacementSelector {
                            dependency: selector.name.clone(),
                        });
                    }
                    if selector.name == replacement.replacement.name {
                        return Err(OverrideError::ReplacementName {
                            dependency: selector.name.clone(),
                        });
                    }
                    let range = Ranges::from(specifier.clone());
                    let entries = replacements.entry(selector.name.clone()).or_default();
                    for existing in entries.iter() {
                        if let RequirementSource::Registry { specifier, .. } =
                            &existing.requirement.source
                            && !range
                                .intersection(&Ranges::from(specifier.clone()))
                                .is_empty()
                            && !replacement
                                .replacement
                                .marker
                                .is_disjoint(existing.replacement.marker)
                        {
                            return Err(OverrideError::OverlappingReplacements {
                                dependency: selector.name.clone(),
                            });
                        }
                    }
                    entries.push(replacement);
                }
                Override::Requirement(requirement) => {
                    global
                        .entry(requirement.name.clone())
                        .or_default()
                        .push(requirement);
                }
                Override::Package(package) => {
                    for requirement in &package.dependencies {
                        match &requirement.source {
                            RequirementSource::Registry { index: Some(_), .. } => {
                                return Err(OverrideError::Index {
                                    package: package.package.name.clone(),
                                    dependency: requirement.name.clone(),
                                });
                            }
                            RequirementSource::Registry { index: None, .. } => {}
                            RequirementSource::Url { .. }
                            | RequirementSource::GitDirectory { .. }
                            | RequirementSource::GitPath { .. }
                            | RequirementSource::Path { .. }
                            | RequirementSource::Directory { .. } => {
                                return Err(OverrideError::Url {
                                    package: package.package.name.clone(),
                                    dependency: requirement.name.clone(),
                                });
                            }
                        }
                    }
                    let packages = scoped.entry(package.package.name.clone()).or_default();
                    let position = packages
                        .iter()
                        .position(|overrides| overrides.version == package.package.version)
                        .unwrap_or_else(|| {
                            let position = packages.len();
                            packages.push(ScopedOverrides {
                                version: package.package.version,
                                overrides: FxHashMap::default(),
                            });
                            position
                        });
                    let overrides = &mut packages[position].overrides;
                    for requirement in package.dependencies {
                        overrides
                            .entry(requirement.name.clone())
                            .or_default()
                            .push(requirement);
                    }
                }
            }
        }

        Ok(Self {
            global,
            scoped,
            replacements,
        })
    }

    /// Return an iterator over all global [`Requirement`]s in the override set.
    pub fn global_requirements(&self) -> impl Iterator<Item = &Requirement> {
        self.global
            .values()
            .flat_map(|requirements| requirements.iter())
            .chain(
                self.replacements
                    .values()
                    .flatten()
                    .map(|entry| &entry.replacement),
            )
    }

    /// Return all scoped [`Requirement`]s with the package and version they apply to.
    pub fn scoped_requirements(
        &self,
    ) -> impl Iterator<Item = (&PackageName, Option<&Version>, &Requirement)> {
        self.scoped.iter().flat_map(|(package, entries)| {
            entries.iter().flat_map(move |entry| {
                entry
                    .overrides
                    .values()
                    .flatten()
                    .map(move |requirement| (package, entry.version.as_ref(), requirement))
            })
        })
    }

    /// Return the scoped [`Requirement`]s that apply to a specific package version.
    pub fn scoped_requirements_for(
        &self,
        package: &PackageName,
        version: &Version,
    ) -> impl Iterator<Item = &Requirement> {
        self.scoped_for(package, version)
            .into_iter()
            .flat_map(|scoped| scoped.overrides.values().flatten())
    }

    /// Return whether any overrides are scoped to the given package.
    pub fn has_scoped_package(&self, package: &PackageName) -> bool {
        self.scoped.contains_key(package)
    }

    /// Return whether a package has overrides for an exact version.
    pub(crate) fn has_exact_scope(&self, package: &PackageName, version: &Version) -> bool {
        self.scoped.get(package).is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.version.as_ref() == Some(version))
        })
    }

    /// Get the overrides for a package.
    fn get(&self, name: &PackageName) -> Option<&Vec<Requirement>> {
        self.global.get(name)
    }

    /// Get the overrides for a specific package version.
    fn scoped_for(&self, package: &PackageName, version: &Version) -> Option<&ScopedOverrides> {
        self.scoped.get(package).and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.version.as_ref() == Some(version))
                .or_else(|| entries.iter().find(|entry| entry.version.is_none()))
        })
    }

    /// Apply the overrides to a set of requirements.
    ///
    /// NB: Change this method together with [`Constraints::apply`].
    pub fn apply<'a, I>(
        &'a self,
        requirements: I,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + use<'a, I>
    where
        I: IntoIterator<Item = &'a Requirement>,
    {
        self.apply_inner(requirements, None)
    }

    /// Apply the overrides to the dependencies of a specific package version.
    pub fn apply_for<'a, I>(
        &'a self,
        package: &PackageName,
        version: &Version,
        requirements: I,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + use<'a, I>
    where
        I: IntoIterator<Item = &'a Requirement>,
    {
        self.apply_inner(requirements, Some((package, version)))
    }

    /// Apply overrides with optional package-version context.
    pub fn apply_for_package<'a, I>(
        &'a self,
        package: Option<(&PackageName, &Version)>,
        requirements: I,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + use<'a, I>
    where
        I: IntoIterator<Item = &'a Requirement>,
    {
        self.apply_inner(requirements, package)
    }

    fn apply_inner<'a, I>(
        &'a self,
        requirements: I,
        package: Option<(&PackageName, &Version)>,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + use<'a, I>
    where
        I: IntoIterator<Item = &'a Requirement>,
    {
        let package_name = package.map(|(name, _)| name.clone());
        let requirements = self.apply_same_name(requirements, package);
        if self.replacements.is_empty() {
            return Either::Left(requirements);
        }
        Either::Right(requirements.flat_map(move |requirement| {
            self.apply_replacement(requirement, package_name.as_ref())
        }))
    }

    fn apply_replacement<'a>(
        &'a self,
        requirement: Cow<'a, Requirement>,
        package: Option<&PackageName>,
    ) -> Vec<Cow<'a, Requirement>> {
        let Some(replacements) = self.replacements.get(&requirement.name) else {
            return vec![requirement];
        };
        let RequirementSource::Registry {
            specifier,
            index: None,
            conflict: None,
            ..
        } = &requirement.source
        else {
            return vec![requirement];
        };
        if !requirement.extras.is_empty() || !requirement.groups.is_empty() {
            return vec![requirement];
        }
        let requested = Ranges::from(specifier.clone());
        let mut result = Vec::new();
        for entry in replacements {
            let RequirementSource::Registry { specifier, .. } = &entry.requirement.source else {
                continue;
            };
            if requested.is_empty()
                || requested.intersection(&Ranges::from(specifier.clone())) != requested
            {
                continue;
            }
            // A replacement may depend on the original library. Rewriting that edge would
            // replace the library with a self-dependency and remove it from the environment.
            if package == Some(&entry.replacement.name) {
                return vec![requirement];
            }
            let marker = requirement.marker.and(entry.replacement.marker);
            result.push(Cow::Owned(Requirement {
                marker,
                ..entry.replacement.clone()
            }));
        }
        if result.is_empty() {
            vec![requirement]
        } else {
            result
        }
    }

    fn apply_same_name<'a, I>(
        &'a self,
        requirements: I,
        package: Option<(&PackageName, &Version)>,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + use<'a, I>
    where
        I: IntoIterator<Item = &'a Requirement>,
    {
        let scoped = package.and_then(|(package, version)| self.scoped_for(package, version));
        if let Some(scoped) = scoped {
            let requirements = requirements.into_iter().collect::<Vec<_>>();
            let names = requirements
                .iter()
                .map(|requirement| requirement.name.clone())
                .collect::<FxHashSet<_>>();
            let mut additions = scoped
                .overrides
                .iter()
                .filter(|(name, _)| !names.contains(*name))
                .flat_map(|(_, requirements)| requirements)
                .collect::<Vec<_>>();
            additions.sort_unstable();

            return Either::Left(
                requirements
                    .into_iter()
                    .flat_map(move |requirement| self.apply_requirement(requirement, Some(scoped)))
                    .chain(additions.into_iter().map(Cow::Borrowed)),
            );
        }

        if self.global.is_empty() {
            // Fast path: There are no overrides.
            return Either::Right(Either::Left(requirements.into_iter().map(Cow::Borrowed)));
        }

        Either::Right(Either::Right(requirements.into_iter().flat_map(
            move |requirement| self.apply_requirement(requirement, None),
        )))
    }

    fn apply_requirement<'a>(
        &'a self,
        requirement: &'a Requirement,
        scoped: Option<&'a ScopedOverrides>,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> {
        let overrides = scoped
            .and_then(|scoped| scoped.overrides.get(&requirement.name))
            .or_else(|| self.get(&requirement.name));
        let Some(overrides) = overrides else {
            // Case 1: No override(s).
            return Either::Left(std::iter::once(Cow::Borrowed(requirement)));
        };

        // ASSUMPTION: There is one `extra = "..."`, and it's either the only marker or part
        // of the main conjunction.
        let Some(extra_expression) = requirement.marker.top_level_extra() else {
            // Case 2: A non-optional dependency with override(s).
            return Either::Right(Either::Right(overrides.iter().map(Cow::Borrowed)));
        };

        // Case 3: An optional dependency with override(s).
        //
        // When the original requirement is an optional dependency, the override(s) need to
        // be optional for the same extra, otherwise we activate extras that should be inactive.
        Either::Right(Either::Left(overrides.iter().map(
            move |override_requirement| {
                // Add the extra to the override marker.
                let joint_marker = MarkerTree::expression(extra_expression.clone())
                    .and(override_requirement.marker);
                Cow::Owned(Requirement {
                    marker: joint_marker,
                    ..override_requirement.clone()
                })
            },
        )))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde_json::json;

    use uv_pypi_types::VerbatimParsedUrl;

    use super::*;

    fn requirement(value: &str) -> Result<Requirement> {
        let requirement: uv_pep508::Requirement<VerbatimParsedUrl> = value.parse()?;
        Ok(Requirement::from(requirement))
    }

    fn replacements(entries: serde_json::Value) -> Result<Overrides> {
        let entries: Vec<Override<uv_pep508::Requirement<VerbatimParsedUrl>>> =
            serde_json::from_value(entries)?;
        Ok(Overrides::from_entries(
            entries
                .into_iter()
                .map(|entry| entry.map(Requirement::from))
                .collect(),
        )?)
    }

    #[test]
    fn replacement_range_and_recursion() -> Result<()> {
        let overrides = replacements(json!([
            { "requirement": "lib<2", "replacement": "virtual-lib1" },
            { "requirement": "lib>=2", "replacement": "virtual-lib2" }
        ]))?;
        let dependencies = [
            requirement("lib>=1,<2 ; sys_platform == 'win32'")?,
            requirement("lib>=2")?,
            requirement("lib")?,
            requirement("lib[feature]<2")?,
            requirement("lib @ https://example.com/lib-1.0.0.tar.gz")?,
        ];
        let actual = overrides
            .apply(&dependencies)
            .map(Cow::into_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            [
                requirement("virtual-lib1 ; sys_platform == 'win32'")?,
                requirement("virtual-lib2")?,
                requirement("lib")?,
                requirement("lib[feature]<2")?,
                requirement("lib @ https://example.com/lib-1.0.0.tar.gz")?,
            ]
        );

        let package: PackageName = "virtual-lib1".parse()?;
        let version: Version = "0.1.0".parse()?;
        let dependencies = [requirement("lib<2")?];
        let actual = overrides
            .apply_for(&package, &version, &dependencies)
            .map(Cow::into_owned)
            .collect::<Vec<_>>();
        assert_eq!(actual, dependencies);
        Ok(())
    }

    #[test]
    fn replacement_selectors_cannot_overlap() {
        let error = replacements(json!([
            { "requirement": "lib<3", "replacement": "virtual-lib1" },
            { "requirement": "lib>=2", "replacement": "virtual-lib2" }
        ]))
        .expect_err("overlapping replacement selectors must fail");
        insta::assert_snapshot!(error, @"Replacement selectors for `lib` overlap");
    }
}
