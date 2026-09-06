use std::borrow::Cow;
use std::iter;

use either::Either;

use uv_distribution_types::{IndexMetadata, Requirement, RequirementSource};
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::{Version, VersionSpecifiers};
use uv_pep508::RequirementOrigin;
use uv_pypi_types::{ConflictItem, ConflictItemRef, VerbatimParsedUrl};

use crate::FxHashbrownSet;
use crate::pubgrub::{PubGrubPackage, PubGrubPackageInner, Range};
use crate::resolver::UnsatisfiableRequirement;

/// The source constraint carried by a single dependency edge.
///
/// Most dependency edges are source-agnostic and use [`DependencySource::Unspecified`]. Direct
/// URLs and group-scoped explicit indexes use a concrete source so fork construction can keep
/// that source information attached to the edge that introduced it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum DependencySource {
    /// The dependency does not carry an edge-local source constraint.
    #[default]
    Unspecified,
    /// The dependency was introduced by a direct URL-like requirement.
    Url(Box<VerbatimParsedUrl>),
    /// The dependency was introduced by a requirement pinned to an explicit index.
    ExplicitIndex(IndexMetadata),
}

impl DependencySource {
    /// Derive the edge-local source constraint from a requirement.
    ///
    /// Registry requirements only carry a source here when they are tied to a group-scoped
    /// explicit index. Direct URL-like requirements always preserve their verbatim URL.
    fn from_requirement(requirement: &Requirement) -> Self {
        match &requirement.source {
            RequirementSource::Registry { index, .. }
                if matches!(
                    requirement.origin.as_ref(),
                    Some(RequirementOrigin::Group(_, Some(_), _))
                ) =>
            {
                index
                    .clone()
                    .map(Self::ExplicitIndex)
                    .unwrap_or(Self::Unspecified)
            }
            RequirementSource::Registry { .. } => Self::Unspecified,
            RequirementSource::Url { .. }
            | RequirementSource::GitDirectory { .. }
            | RequirementSource::GitPath { .. }
            | RequirementSource::Path { .. }
            | RequirementSource::Directory { .. } => requirement
                .source
                .to_verbatim_parsed_url()
                .map(Box::new)
                .map(Self::Url)
                .unwrap_or(Self::Unspecified),
        }
    }

    /// Return the direct URL attached to this source, if any.
    pub(crate) fn verbatim_url(&self) -> Option<&VerbatimParsedUrl> {
        match self {
            Self::Url(url) => Some(url.as_ref()),
            Self::Unspecified | Self::ExplicitIndex(_) => None,
        }
    }

    /// Return the explicit index attached to this source, if any.
    pub(crate) fn explicit_index(&self) -> Option<&IndexMetadata> {
        match self {
            Self::ExplicitIndex(index) => Some(index),
            Self::Unspecified | Self::Url(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PubGrubDependency {
    pub(crate) package: PubGrubPackage,
    pub(crate) version: Range<Version>,

    /// When the parent that created this dependency is a "normal" package
    /// (non-extra non-group), this corresponds to its name.
    ///
    /// This is used to create project-level `ConflictItemRef` for a specific
    /// package. In effect, this lets us "delay" filtering of project
    /// dependencies when a conflict is declared between the project and a
    /// group.
    ///
    /// The main problem with dealing with project level conflicts is that if you
    /// declare a conflict between a package and a group, we represent that
    /// group as a dependency of that package. So if you filter out the package
    /// in a fork due to a conflict, you also filter out the group. Therefore,
    /// we introduce this parent field to enable "delayed" filtering.
    pub(crate) parent: Option<PackageName>,

    /// The direct source constraint attached to this dependency edge.
    ///
    /// This is only populated when the edge itself needs source identity, e.g. for direct URLs
    /// or group-scoped explicit indexes. Manifest-wide URL and index constraints are still applied
    /// separately via `Urls` and `Indexes`.
    pub(crate) source: DependencySource,
}

impl PubGrubDependency {
    /// Convert flattened requirements into PubGrub dependency edges.
    ///
    /// An empty range cannot retain the specifiers that produced it, so return the source
    /// requirement details instead. The resolver attaches them to the parent package as the
    /// reason that package cannot be selected.
    pub(crate) fn from_requirements<'a>(
        conflict_items: &FxHashbrownSet<ConflictItem>,
        requirements: impl IntoIterator<Item = Cow<'a, Requirement>>,
        group_name: Option<&'a GroupName>,
        parent_package: Option<&'a PubGrubPackage>,
    ) -> Result<Vec<Self>, UnsatisfiableRequirement> {
        let mut dependencies = Vec::new();
        for requirement in requirements {
            dependencies.extend(Self::from_requirement(
                conflict_items,
                requirement,
                group_name,
                parent_package,
            )?);
        }
        Ok(dependencies)
    }

    fn from_requirement<'a>(
        conflict_items: &FxHashbrownSet<ConflictItem>,
        requirement: Cow<'a, Requirement>,
        group_name: Option<&'a GroupName>,
        parent_package: Option<&'a PubGrubPackage>,
    ) -> Result<impl Iterator<Item = Self> + 'a, UnsatisfiableRequirement> {
        if let Some(requirement) = UnsatisfiableRequirement::from_requirement(&requirement) {
            return Err(requirement);
        }

        let parent_name = parent_package.and_then(|package| package.name_no_root());
        let is_normal_parent = parent_package
            .is_some_and(|parent| parent.extra().is_none() && parent.group().is_none());
        let iter = if !requirement.extras.is_empty() {
            // This is crazy subtle, but if any of the extras in the
            // requirement are part of a declared conflict, then we
            // specifically need (at time of writing) to include the
            // base package as a dependency. This results in both
            // the base package and the extra package being sibling
            // dependencies at the point in which forks are created
            // base on conflicting extras. If the base package isn't
            // present at that point, then it's impossible for the
            // fork that excludes all conflicting extras to reach
            // the non-extra dependency, which may be necessary for
            // correctness.
            //
            // But why do we not include the base package in the first
            // place? Well, that's part of an optimization[1].
            //
            // [1]: https://github.com/astral-sh/uv/pull/9540
            let base = if requirement.extras.iter().any(|extra| {
                conflict_items.contains(&ConflictItemRef::from((&requirement.name, extra)))
            }) {
                Either::Left(iter::once((None, None)))
            } else {
                Either::Right(iter::empty())
            };
            Either::Left(Either::Left(base.chain(
                Box::into_iter(requirement.extras.clone()).map(|extra| (Some(extra), None)),
            )))
        } else if !requirement.groups.is_empty() {
            let base = if requirement.groups.iter().any(|group| {
                conflict_items.contains(&ConflictItemRef::from((&requirement.name, group)))
            }) {
                Either::Left(iter::once((None, None)))
            } else {
                Either::Right(iter::empty())
            };
            Either::Left(Either::Right(base.chain(
                Box::into_iter(requirement.groups.clone()).map(|group| (None, Some(group))),
            )))
        } else {
            Either::Right(iter::once((None, None)))
        };

        // Add the package, plus any extra variants.
        Ok(iter.map(move |(extra, group)| {
            let pubgrub_requirement =
                PubGrubRequirement::from_requirement(&requirement, extra, group);
            let PubGrubRequirement {
                package,
                version,
                source,
            } = pubgrub_requirement;
            match &*package {
                PubGrubPackageInner::Package { .. } => Self {
                    package,
                    version,
                    parent: if is_normal_parent {
                        parent_name.cloned()
                    } else {
                        None
                    },
                    source,
                },
                PubGrubPackageInner::Marker { .. } => Self {
                    package,
                    version,
                    parent: if is_normal_parent {
                        parent_name.cloned()
                    } else {
                        None
                    },
                    source,
                },
                PubGrubPackageInner::Extra { name, .. } => {
                    if group_name.is_none() {
                        debug_assert!(
                            parent_name.is_none_or(|parent_name| parent_name != name),
                            "extras not flattened for {name}"
                        );
                    }
                    Self {
                        package,
                        version,
                        parent: None,
                        source,
                    }
                }
                PubGrubPackageInner::Group { name, .. } => {
                    if group_name.is_none() {
                        debug_assert!(
                            parent_name.is_none_or(|parent_name| parent_name != name),
                            "group not flattened for {name}"
                        );
                    }
                    Self {
                        package,
                        version,
                        parent: None,
                        source,
                    }
                }
                PubGrubPackageInner::Root(_) => unreachable!("Root package in dependencies"),
                PubGrubPackageInner::Python(_) => {
                    unreachable!("Python package in dependencies")
                }
                PubGrubPackageInner::System(_) => unreachable!("System package in dependencies"),
            }
        }))
    }

    /// Extracts a possible conflicting item from this dependency.
    ///
    /// If this package can't possibly be classified as conflicting, then this
    /// returns `None`.
    pub(crate) fn conflicting_item(&self) -> Option<ConflictItemRef<'_>> {
        self.package.conflicting_item()
    }
}

/// A PubGrub-compatible package and version range.
#[derive(Debug, Clone)]
struct PubGrubRequirement {
    package: PubGrubPackage,
    version: Range<Version>,
    source: DependencySource,
}

impl PubGrubRequirement {
    fn package_for_requirement(
        requirement: &Requirement,
        extra: Option<ExtraName>,
        group: Option<GroupName>,
    ) -> PubGrubPackage {
        PubGrubPackage::from_package(requirement.name.clone(), extra, group, requirement.marker)
    }

    /// Convert a [`Requirement`] to a PubGrub-compatible package and range, while returning the URL
    /// on the [`Requirement`], if any.
    fn from_requirement(
        requirement: &Requirement,
        extra: Option<ExtraName>,
        group: Option<GroupName>,
    ) -> Self {
        if let RequirementSource::Registry { specifier, .. } = &requirement.source {
            return Self::from_registry_requirement(specifier, extra, group, requirement);
        }

        Self {
            package: Self::package_for_requirement(requirement, extra, group),
            version: Range::full(),
            source: DependencySource::from_requirement(requirement),
        }
    }

    fn from_registry_requirement(
        specifier: &VersionSpecifiers,
        extra: Option<ExtraName>,
        group: Option<GroupName>,
        requirement: &Requirement,
    ) -> Self {
        Self {
            package: Self::package_for_requirement(requirement, extra, group),
            source: DependencySource::from_requirement(requirement),
            version: Range::from(specifier.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::str::FromStr;

    use uv_distribution_types::IndexUrl;
    use uv_pep508::MarkerTree;
    use uv_pypi_types::{ConflictSet, Conflicts};

    use super::*;

    fn requirement(extras: &[&str], groups: &[&str]) -> Requirement {
        let project = PackageName::from_str("workspace-root").unwrap();
        let group = GroupName::from_str("dev").unwrap();
        Requirement {
            name: PackageName::from_str("demo-pkg").unwrap(),
            extras: extras
                .iter()
                .map(|extra| ExtraName::from_str(extra).unwrap())
                .collect(),
            groups: groups
                .iter()
                .map(|group| GroupName::from_str(group).unwrap())
                .collect(),
            marker: MarkerTree::from_str("python_version >= '3.10'").unwrap(),
            source: RequirementSource::Registry {
                specifier: VersionSpecifiers::from_str(">=1.2,<2").unwrap(),
                index: Some(IndexMetadata::from(
                    IndexUrl::from_str("https://example.invalid/simple").unwrap(),
                )),
                conflict: None,
            },
            origin: Some(RequirementOrigin::Group(
                PathBuf::from("pyproject.toml"),
                Some(project),
                group,
            )),
        }
    }

    fn build_conflict_items(sets: Vec<Vec<ConflictItem>>) -> FxHashbrownSet<ConflictItem> {
        let mut conflicts = Conflicts::empty();
        for set in sets {
            conflicts.push(ConflictSet::try_from(set).unwrap());
        }
        conflicts
            .iter()
            .flat_map(|set| set.iter().cloned())
            .collect()
    }

    fn variants(dependencies: &[PubGrubDependency]) -> Vec<(Option<String>, Option<String>)> {
        dependencies
            .iter()
            .map(|dependency| {
                (
                    dependency.package.extra().map(ToString::to_string),
                    dependency.package.group().map(ToString::to_string),
                )
            })
            .collect()
    }

    #[test]
    fn indexed_conflicts_preserve_extra_edges_and_source() {
        let package = PackageName::from_str("demo-pkg").unwrap();
        let other = PackageName::from_str("other-pkg").unwrap();
        let first = ExtraName::from_str("first-extra").unwrap();
        let second = ExtraName::from_str("second-extra").unwrap();
        let set = vec![
            ConflictItem::from((package.clone(), first)),
            ConflictItem::from((package.clone(), second)),
            ConflictItem::from((other, ExtraName::from_str("first-extra").unwrap())),
        ];
        let conflict_items = build_conflict_items(vec![set.clone(), set]);
        assert_eq!(conflict_items.len(), 3);

        let requirement = requirement(
            &[
                "missing-extra",
                "first_extra",
                "second-extra",
                "second-extra",
            ],
            &["ignored-group"],
        );
        let dependencies = PubGrubDependency::from_requirements(
            &conflict_items,
            [Cow::Borrowed(&requirement)],
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            variants(&dependencies),
            vec![
                (None, None),
                (Some("missing-extra".to_string()), None),
                (Some("first-extra".to_string()), None),
                (Some("second-extra".to_string()), None),
                (Some("second-extra".to_string()), None),
            ]
        );
        let expected_range = Range::from(VersionSpecifiers::from_str(">=1.2,<2").unwrap());
        let expected_source = DependencySource::ExplicitIndex(IndexMetadata::from(
            IndexUrl::from_str("https://example.invalid/simple").unwrap(),
        ));
        assert!(
            dependencies
                .iter()
                .all(|dependency| dependency.version == expected_range)
        );
        assert!(
            dependencies
                .iter()
                .all(|dependency| dependency.source == expected_source)
        );
        assert!(
            dependencies
                .iter()
                .all(|dependency| dependency.package.marker() == requirement.marker)
        );
    }

    #[test]
    fn indexed_conflicts_distinguish_groups_projects_and_packages() {
        let package = PackageName::from_str("demo-pkg").unwrap();
        let other = PackageName::from_str("other-pkg").unwrap();
        let shared_group = GroupName::from_str("shared-group").unwrap();
        let conflict_items = build_conflict_items(vec![vec![
            ConflictItem::from(package.clone()),
            ConflictItem::from((package.clone(), shared_group)),
            ConflictItem::from((package.clone(), ExtraName::from_str("group-only").unwrap())),
            ConflictItem::from((other, ExtraName::from_str("shared-extra").unwrap())),
        ]]);

        let group_requirement =
            requirement(&[], &["missing-group", "shared_group", "shared-group"]);
        let group_dependencies = PubGrubDependency::from_requirements(
            &conflict_items,
            [Cow::Borrowed(&group_requirement)],
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            variants(&group_dependencies),
            vec![
                (None, None),
                (None, Some("missing-group".to_string())),
                (None, Some("shared-group".to_string())),
                (None, Some("shared-group".to_string())),
            ]
        );

        let extra_requirement = requirement(&["shared-extra", "group-only"], &[]);
        let extra_dependencies = PubGrubDependency::from_requirements(
            &conflict_items,
            [Cow::Borrowed(&extra_requirement)],
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            variants(&extra_dependencies),
            vec![
                (None, None),
                (Some("shared-extra".to_string()), None),
                (Some("group-only".to_string()), None),
            ]
        );

        let project_only = build_conflict_items(vec![vec![
            ConflictItem::from(package),
            ConflictItem::from((
                PackageName::from_str("other-pkg").unwrap(),
                ExtraName::from_str("shared-extra").unwrap(),
            )),
        ]]);
        let project_dependencies = PubGrubDependency::from_requirements(
            &project_only,
            [Cow::Borrowed(&extra_requirement)],
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            variants(&project_dependencies),
            vec![
                (Some("shared-extra".to_string()), None),
                (Some("group-only".to_string()), None),
            ]
        );

        let cross_kind = build_conflict_items(vec![vec![
            ConflictItem::from((
                PackageName::from_str("demo-pkg").unwrap(),
                GroupName::from_str("shared-group").unwrap(),
            )),
            ConflictItem::from((
                PackageName::from_str("demo-pkg").unwrap(),
                ExtraName::from_str("group-only").unwrap(),
            )),
        ]]);
        let extra_miss = requirement(&["shared-group"], &[]);
        let extra_miss_dependencies = PubGrubDependency::from_requirements(
            &cross_kind,
            [Cow::Borrowed(&extra_miss)],
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            variants(&extra_miss_dependencies),
            vec![(Some("shared-group".to_string()), None)]
        );
        let group_miss = requirement(&[], &["group-only"]);
        let group_miss_dependencies = PubGrubDependency::from_requirements(
            &cross_kind,
            [Cow::Borrowed(&group_miss)],
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            variants(&group_miss_dependencies),
            vec![(None, Some("group-only".to_string()))]
        );
    }
}
