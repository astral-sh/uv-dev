use std::borrow::Cow;

use either::Either;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use serde::de::IntoDeserializer;

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

/// An override, either global or scoped to a specific package version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema), schemars(untagged))]
#[serde(untagged, bound(serialize = "T: serde::Serialize"))]
pub enum Override<T> {
    Package(PackageOverride<T>),
    Requirement(T),
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
            Requirement(T),
        }

        serde_untagged::UntaggedEnumVisitor::new()
            .string(|string| T::deserialize(string.into_deserializer()).map(Self::Requirement))
            .map(|map| {
                map.deserialize::<MapOverride<T>>()
                    .map(|entry| match entry {
                        MapOverride::Package(package) => Self::Package(package),
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
    scoped: FxHashMap<PackageName, ScopedOverrideSet>,
}

const SCOPED_OVERRIDE_INDEX_THRESHOLD: usize = 32;

#[derive(Debug, Default, Clone)]
struct ScopedOverrideSet {
    entries: Vec<ScopedOverrides>,
    exact: Option<FxHashMap<Version, usize>>,
    fallback: Option<usize>,
}

#[derive(Debug, Clone)]
struct ScopedOverrides {
    version: Option<Version>,
    overrides: FxHashMap<PackageName, Vec<Requirement>>,
}

impl ScopedOverrideSet {
    /// Return the index of an exact-version scope, avoiding the hash lookup for the first entry.
    fn exact_position(&self, version: &Version) -> Option<usize> {
        if self
            .entries
            .first()
            .is_some_and(|entry| entry.version.as_ref() == Some(version))
        {
            return Some(0);
        }

        self.exact.as_ref().map_or_else(
            || {
                self.entries
                    .iter()
                    .enumerate()
                    .skip(1)
                    .find_map(|(position, entry)| {
                        (entry.version.as_ref() == Some(version)).then_some(position)
                    })
            },
            |exact| exact.get(version).copied(),
        )
    }

    /// Return an existing scope or append a new one, preserving first-seen scope order.
    fn get_or_insert(&mut self, version: Option<Version>) -> &mut ScopedOverrides {
        let position = match version.as_ref() {
            Some(version) => self.exact_position(version),
            None => self.fallback,
        };
        if let Some(position) = position {
            return &mut self.entries[position];
        }

        let position = self.entries.len();
        self.entries.push(ScopedOverrides {
            version,
            overrides: FxHashMap::default(),
        });

        if let Some(version) = self.entries[position].version.as_ref() {
            if let Some(exact) = self.exact.as_mut() {
                exact.insert(version.clone(), position);
            }
        } else {
            self.fallback = Some(position);
        }

        if self.exact.is_none() && self.entries.len() >= SCOPED_OVERRIDE_INDEX_THRESHOLD {
            self.exact = Some(
                self.entries
                    .iter()
                    .enumerate()
                    .filter_map(|(position, entry)| {
                        Some((entry.version.as_ref()?.clone(), position))
                    })
                    .collect(),
            );
        }

        &mut self.entries[position]
    }

    /// Return the exact-version scope, or the all-versions fallback when no exact scope exists.
    fn get(&self, version: &Version) -> Option<&ScopedOverrides> {
        self.exact_position(version)
            .or(self.fallback)
            .map(|position| &self.entries[position])
    }
}

/// An unsupported source in a scoped dependency override.
#[derive(Debug, thiserror::Error)]
pub enum ScopedOverrideSourceError {
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
        }
    }

    /// Create an indexed set of overrides.
    pub fn from_entries(
        entries: Vec<Override<Requirement>>,
    ) -> Result<Self, ScopedOverrideSourceError> {
        let mut global: FxHashMap<PackageName, Vec<Requirement>> =
            FxHashMap::with_capacity_and_hasher(entries.len(), FxBuildHasher);
        let mut scoped: FxHashMap<PackageName, ScopedOverrideSet> = FxHashMap::default();

        for entry in entries {
            match entry {
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
                                return Err(ScopedOverrideSourceError::Index {
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
                                return Err(ScopedOverrideSourceError::Url {
                                    package: package.package.name.clone(),
                                    dependency: requirement.name.clone(),
                                });
                            }
                        }
                    }
                    let packages = scoped.entry(package.package.name.clone()).or_default();
                    let overrides = &mut packages.get_or_insert(package.package.version).overrides;
                    for requirement in package.dependencies {
                        overrides
                            .entry(requirement.name.clone())
                            .or_default()
                            .push(requirement);
                    }
                }
            }
        }

        Ok(Self { global, scoped })
    }

    /// Return an iterator over all global [`Requirement`]s in the override set.
    pub fn global_requirements(&self) -> impl Iterator<Item = &Requirement> {
        self.global
            .values()
            .flat_map(|requirements| requirements.iter())
    }

    /// Return all scoped [`Requirement`]s with the package and version they apply to.
    pub fn scoped_requirements(
        &self,
    ) -> impl Iterator<Item = (&PackageName, Option<&Version>, &Requirement)> {
        self.scoped.iter().flat_map(|(package, entries)| {
            entries.entries.iter().flat_map(move |entry| {
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

    /// Return whether a package has overrides for an exact version.
    pub(crate) fn has_exact_scope(&self, package: &PackageName, version: &Version) -> bool {
        self.scoped
            .get(package)
            .is_some_and(|entries| entries.exact_position(version).is_some())
    }

    /// Get the overrides for a package.
    fn get(&self, name: &PackageName) -> Option<&Vec<Requirement>> {
        self.global.get(name)
    }

    /// Get the overrides for a specific package version.
    fn scoped_for(&self, package: &PackageName, version: &Version) -> Option<&ScopedOverrides> {
        self.scoped
            .get(package)
            .and_then(|entries| entries.get(version))
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
    use std::path::{Path, PathBuf};
    use std::str::FromStr;

    use anyhow::Result;

    use uv_pep440::VersionSpecifiers;
    use uv_pep508::RequirementOrigin;

    use super::*;

    fn requirement(name: &str, specifier: &str, marker: &str, origin: &str) -> Result<Requirement> {
        Ok(Requirement {
            name: PackageName::from_str(name)?,
            extras: Box::new([]),
            groups: Box::new([]),
            marker: if marker.is_empty() {
                MarkerTree::TRUE
            } else {
                MarkerTree::from_str(marker)?
            },
            source: RequirementSource::Registry {
                specifier: VersionSpecifiers::from_str(specifier)?,
                index: None,
                conflict: None,
            },
            origin: Some(RequirementOrigin::File(PathBuf::from(origin))),
        })
    }

    fn scoped(
        package: &str,
        version: Option<&str>,
        dependencies: Vec<Requirement>,
    ) -> Result<Override<Requirement>> {
        Ok(Override::Package(PackageOverride {
            package: PackageOverrideTarget {
                name: PackageName::from_str(package)?,
                version: version.map(Version::from_str).transpose()?,
            },
            dependencies: dependencies.into_boxed_slice(),
        }))
    }

    fn origins<'a>(requirements: impl IntoIterator<Item = &'a Requirement>) -> Vec<&'a Path> {
        requirements
            .into_iter()
            .filter_map(|requirement| requirement.origin.as_ref().map(RequirementOrigin::path))
            .collect()
    }

    #[test]
    fn scoped_override_index_preserves_normalized_duplicates_and_order() -> Result<()> {
        let parent = PackageName::from_str("parent")?;
        let mut entries = vec![scoped(
            "parent",
            None,
            vec![requirement("target", "==0", "", "fallback.in")?],
        )?];
        for version in 1..=64 {
            entries.push(scoped(
                "parent",
                Some(&format!("{version}.0")),
                vec![requirement(
                    "target",
                    &format!("=={version}"),
                    "",
                    &format!("exact-{version}.in"),
                )?],
            )?);
        }
        entries.push(scoped(
            "parent",
            Some("1.0.0.0"),
            vec![requirement(
                "target",
                "==101",
                "python_version >= '3.12'",
                "duplicate.in",
            )?],
        )?);

        let overrides = Overrides::from_entries(entries)?;
        let scopes = &overrides.scoped[&parent];
        assert_eq!(scopes.entries.len(), 65);
        assert!(scopes.exact.is_some());
        assert_eq!(scopes.fallback, Some(0));

        let normalized = Version::from_str("1.0.0")?;
        assert!(overrides.has_exact_scope(&parent, &normalized));
        assert_eq!(
            origins(overrides.scoped_requirements_for(&parent, &normalized)),
            [Path::new("exact-1.in"), Path::new("duplicate.in")]
        );

        let last = Version::from_str("64.0")?;
        assert!(overrides.has_exact_scope(&parent, &last));
        assert_eq!(
            origins(overrides.scoped_requirements_for(&parent, &last)),
            [Path::new("exact-64.in")]
        );

        let missing = Version::from_str("999.0")?;
        assert!(!overrides.has_exact_scope(&parent, &missing));
        assert_eq!(
            origins(overrides.scoped_requirements_for(&parent, &missing)),
            [Path::new("fallback.in")]
        );

        let order = overrides
            .scoped_requirements()
            .filter_map(|(_, _, requirement)| {
                requirement.origin.as_ref().map(RequirementOrigin::path)
            })
            .collect::<Vec<_>>();
        assert_eq!(order[0], Path::new("fallback.in"));
        assert_eq!(order[1], Path::new("exact-1.in"));
        assert_eq!(order[2], Path::new("duplicate.in"));
        assert_eq!(order[3], Path::new("exact-2.in"));
        assert_eq!(order[65], Path::new("exact-64.in"));

        Ok(())
    }

    #[test]
    fn scoped_override_index_handles_the_threshold_with_a_late_fallback() -> Result<()> {
        let parent = PackageName::from_str("parent")?;
        for exact_count in [30, 31, 32] {
            let mut entries = Vec::new();
            for version in 1..=exact_count {
                entries.push(scoped(
                    "parent",
                    Some(&format!("{version}.0")),
                    vec![requirement(
                        "target",
                        &format!("=={version}"),
                        "",
                        &format!("exact-{version}.in"),
                    )?],
                )?);
            }
            entries.push(scoped(
                "parent",
                None,
                vec![requirement("target", "==0", "", "fallback.in")?],
            )?);

            let overrides = Overrides::from_entries(entries)?;
            let scopes = &overrides.scoped[&parent];
            assert_eq!(
                scopes.exact.is_some(),
                exact_count + 1 >= SCOPED_OVERRIDE_INDEX_THRESHOLD
            );
            assert_eq!(scopes.fallback, Some(exact_count));

            let first = Version::from_str("1.0.0")?;
            assert!(overrides.has_exact_scope(&parent, &first));
            assert_eq!(
                origins(overrides.scoped_requirements_for(&parent, &first)),
                [Path::new("exact-1.in")]
            );
            let last = Version::from_str(&format!("{exact_count}.0"))?;
            assert!(overrides.has_exact_scope(&parent, &last));
            assert_eq!(
                origins(overrides.scoped_requirements_for(&parent, &last)),
                [Path::new(&format!("exact-{exact_count}.in"))]
            );
            let missing = Version::from_str("999.0")?;
            assert!(!overrides.has_exact_scope(&parent, &missing));
            assert_eq!(
                origins(overrides.scoped_requirements_for(&parent, &missing)),
                [Path::new("fallback.in")]
            );
        }

        Ok(())
    }

    #[test]
    fn scoped_override_lookup_preserves_precedence_markers_and_additions() -> Result<()> {
        let parent = PackageName::from_str("parent")?;
        let exact = Version::from_str("1.0")?;
        let fallback = Version::from_str("2.0")?;
        let overrides = Overrides::from_entries(vec![
            Override::Requirement(requirement("target", "==90", "", "global-target.in")?),
            Override::Requirement(requirement("global-only", "==91", "", "global-only.in")?),
            scoped(
                "parent",
                None,
                vec![
                    requirement("target", "==10", "", "fallback-target.in")?,
                    requirement("fallback-added", "==11", "", "fallback-added.in")?,
                ],
            )?,
            scoped(
                "parent",
                Some("1.0.0"),
                vec![
                    requirement(
                        "target",
                        "==20",
                        "python_version >= '3.12'",
                        "exact-target-a.in",
                    )?,
                    requirement("zeta-added", "==22", "", "zeta-added.in")?,
                ],
            )?,
            scoped(
                "parent",
                Some("1.0"),
                vec![
                    requirement(
                        "target",
                        "==21",
                        "sys_platform == 'linux'",
                        "exact-target-b.in",
                    )?,
                    requirement("alpha-added", "==23", "", "alpha-added.in")?,
                ],
            )?,
        ])?;
        let dependencies = vec![
            requirement("target", ">=1", "extra == 'feature'", "original-target.in")?,
            requirement("global-only", ">=1", "", "original-global.in")?,
            requirement("unchanged", ">=1", "", "unchanged.in")?,
        ];

        let exact_requirements = overrides
            .apply_for(&parent, &exact, &dependencies)
            .collect::<Vec<_>>();
        assert_eq!(
            origins(exact_requirements.iter().map(AsRef::as_ref)),
            [
                Path::new("exact-target-a.in"),
                Path::new("exact-target-b.in"),
                Path::new("global-only.in"),
                Path::new("unchanged.in"),
                Path::new("alpha-added.in"),
                Path::new("zeta-added.in"),
            ]
        );
        assert_eq!(
            exact_requirements[0].marker,
            MarkerTree::from_str("extra == 'feature' and python_version >= '3.12'")?
        );
        assert_eq!(
            exact_requirements[1].marker,
            MarkerTree::from_str("extra == 'feature' and sys_platform == 'linux'")?
        );

        let fallback_requirements = overrides
            .apply_for(&parent, &fallback, &dependencies)
            .collect::<Vec<_>>();
        assert_eq!(
            origins(fallback_requirements.iter().map(AsRef::as_ref)),
            [
                Path::new("fallback-target.in"),
                Path::new("global-only.in"),
                Path::new("unchanged.in"),
                Path::new("fallback-added.in"),
            ]
        );
        assert_eq!(
            fallback_requirements[0].marker,
            MarkerTree::from_str("extra == 'feature'")?
        );

        Ok(())
    }

    #[test]
    fn empty_exact_scope_shadows_the_all_versions_fallback() -> Result<()> {
        let parent = PackageName::from_str("parent")?;
        let version = Version::from_str("1.0")?;
        let overrides = Overrides::from_entries(vec![
            Override::Requirement(requirement("global", "==3", "", "global.in")?),
            scoped(
                "parent",
                None,
                vec![requirement("fallback", "==2", "", "fallback.in")?],
            )?,
            scoped("parent", Some("1.0.0"), Vec::new())?,
        ])?;
        let dependencies = vec![requirement("global", ">=1", "", "original.in")?];

        assert!(overrides.has_exact_scope(&parent, &version));
        assert_eq!(
            overrides.scoped_requirements_for(&parent, &version).count(),
            0
        );
        let requirements = overrides
            .apply_for(&parent, &version, &dependencies)
            .collect::<Vec<_>>();
        assert_eq!(
            origins(requirements.iter().map(AsRef::as_ref)),
            [Path::new("global.in")]
        );

        Ok(())
    }

    #[test]
    fn scoped_override_still_rejects_url_sources() -> Result<()> {
        let url = serde_json::from_value::<Requirement>(serde_json::json!({
            "name": "target",
            "url": "https://example.invalid/target-1.0-py3-none-any.whl",
        }))?;
        let result = Overrides::from_entries(vec![scoped("parent", Some("1.0"), vec![url])?]);

        assert!(matches!(
            result,
            Err(ScopedOverrideSourceError::Url {
                package,
                dependency,
            }) if package.as_ref() == "parent" && dependency.as_ref() == "target"
        ));
        Ok(())
    }
}
