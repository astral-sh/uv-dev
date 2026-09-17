use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::str::FromStr;

use glob::{MatchOptions, Pattern};
use serde::{Deserialize, Deserializer, Serialize};

use uv_configuration::NoSources;
use uv_distribution_types::RequiresPython;
use uv_normalize::{GroupName, InvalidNameError, PackageName};
use uv_pep440::VersionSpecifiers;
use uv_pep508::{MarkerTree, Requirement};
use uv_pypi_types::{
    SupportedEnvironments, VerbatimParsedUrl, select_workspace_axis_marker, workspace_axis_marker,
};
use uv_toml::deserialize_unique_map;

use crate::pyproject::{Source, WorkspaceReference};
use crate::workspace_groups::WorkspaceResolution;
use crate::{Workspace, WorkspaceError};

/// The normalized name of a workspace resolution axis.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct WorkspaceAxisName(GroupName);

impl WorkspaceAxisName {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl FromStr for WorkspaceAxisName {
    type Err = InvalidNameError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        GroupName::from_str(name).map(Self)
    }
}

impl Display for WorkspaceAxisName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for WorkspaceAxisName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// The normalized name of one section of a workspace resolution axis.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct WorkspaceSectionName(GroupName);

impl WorkspaceSectionName {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl FromStr for WorkspaceSectionName {
    type Err = InvalidNameError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        GroupName::from_str(name).map(Self)
    }
}

impl Display for WorkspaceSectionName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for WorkspaceSectionName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A single assignment accepted by `--resolution-axis`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct WorkspaceAxisAssignment {
    pub axis: WorkspaceAxisName,
    pub section: WorkspaceSectionName,
}

impl FromStr for WorkspaceAxisAssignment {
    type Err = WorkspaceAxisError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((axis, section)) = value.split_once('=') else {
            return Err(WorkspaceAxisError::InvalidAssignment(value.to_owned()));
        };
        Ok(Self {
            axis: axis
                .parse()
                .map_err(|source| WorkspaceAxisError::InvalidAxisName {
                    name: axis.to_owned(),
                    source,
                })?,
            section: section
                .parse()
                .map_err(|source| WorkspaceAxisError::InvalidSectionName {
                    name: section.to_owned(),
                    source,
                })?,
        })
    }
}

impl Display for WorkspaceAxisAssignment {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}={}", self.axis, self.section)
    }
}

/// A consistent, possibly partial selection of axis sections.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct WorkspaceAxisSelection(BTreeMap<WorkspaceAxisName, WorkspaceSectionName>);

impl<'de> Deserialize<'de> for WorkspaceAxisSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_map(deserializer, |axis: &WorkspaceAxisName| {
            format!("duplicate normalized resolution axis `{axis}`")
        })
        .map(Self)
    }
}

impl WorkspaceAxisSelection {
    pub fn from_assignments(
        assignments: impl IntoIterator<Item = WorkspaceAxisAssignment>,
    ) -> Result<Self, WorkspaceAxisError> {
        let mut selection = Self::default();
        for assignment in assignments {
            selection.insert(assignment)?;
        }
        Ok(selection)
    }

    /// Add an assignment without replacing a different section of the same axis.
    pub fn insert(
        &mut self,
        assignment: WorkspaceAxisAssignment,
    ) -> Result<(), WorkspaceAxisError> {
        if let Some(previous) = self.0.get(&assignment.axis) {
            if previous != &assignment.section {
                return Err(WorkspaceAxisError::ConflictingAssignments {
                    axis: assignment.axis,
                    first: previous.clone(),
                    second: assignment.section,
                });
            }
        } else {
            self.0.insert(assignment.axis, assignment.section);
        }
        Ok(())
    }

    /// Merge another selection, leaving this selection unchanged on conflict.
    pub fn merge(&mut self, other: &Self) -> Result<(), WorkspaceAxisError> {
        for (axis, section) in &other.0 {
            if let Some(previous) = self.0.get(axis)
                && previous != section
            {
                return Err(WorkspaceAxisError::ConflictingAssignments {
                    axis: axis.clone(),
                    first: previous.clone(),
                    second: section.clone(),
                });
            }
        }
        self.0.extend(other.0.clone());
        Ok(())
    }

    pub fn iter(
        &self,
    ) -> impl Iterator<Item = (&WorkspaceAxisName, &WorkspaceSectionName)> + Clone + '_ {
        self.0.iter()
    }

    pub fn get(&self, axis: &WorkspaceAxisName) -> Option<&WorkspaceSectionName> {
        self.0.get(axis)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Return the conjunction of this selection's private axis atoms.
    pub fn to_marker_tree(&self) -> MarkerTree {
        self.iter()
            .fold(MarkerTree::TRUE, |marker, (axis, section)| {
                marker.and(workspace_axis_marker(axis.as_str(), section.as_str()))
            })
    }
}

impl Display for WorkspaceAxisSelection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let mut separator = "";
        for (axis, section) in self.iter() {
            write!(formatter, "{separator}{axis}={section}")?;
            separator = ", ";
        }
        Ok(())
    }
}

/// Members and resolution policy associated with one section of an axis.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct WorkspaceAxisSection {
    /// Normalized `project.name` values, not filesystem paths.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub members: BTreeSet<PackageName>,
    /// Workspace-relative globs matched against already discovered member directories.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub member_paths: BTreeSet<String>,
    #[cfg_attr(feature = "schemars", schemars(with = "Option<String>"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_python: Option<VersionSpecifiers>,
    /// Constraints narrow dependencies that are requested elsewhere; they do not install them.
    #[cfg_attr(feature = "schemars", schemars(with = "Vec<String>"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraint_dependencies: Vec<Requirement<VerbatimParsedUrl>>,
}

/// The named sections of one resolution axis.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct WorkspaceAxis(BTreeMap<WorkspaceSectionName, WorkspaceAxisSection>);

impl<'de> Deserialize<'de> for WorkspaceAxis {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_map(deserializer, |section: &WorkspaceSectionName| {
            format!("duplicate normalized resolution section `{section}`")
        })
        .map(Self)
    }
}

impl WorkspaceAxis {
    pub fn iter(
        &self,
    ) -> impl Iterator<Item = (&WorkspaceSectionName, &WorkspaceAxisSection)> + Clone + '_ {
        self.0.iter()
    }

    pub fn get(&self, section: &WorkspaceSectionName) -> Option<&WorkspaceAxisSection> {
        self.0.get(section)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The workspace's declared, independently selectable resolution axes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct WorkspaceAxes(BTreeMap<WorkspaceAxisName, WorkspaceAxis>);

impl<'de> Deserialize<'de> for WorkspaceAxes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_map(deserializer, |axis: &WorkspaceAxisName| {
            format!("duplicate normalized resolution axis `{axis}`")
        })
        .map(Self)
    }
}

impl WorkspaceAxes {
    pub fn iter(&self) -> impl Iterator<Item = (&WorkspaceAxisName, &WorkspaceAxis)> + Clone + '_ {
        self.0.iter()
    }

    pub fn get(&self, axis: &WorkspaceAxisName) -> Option<&WorkspaceAxis> {
        self.0.get(axis)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A product of allowed sections, represented without enumerating its complete contexts.
///
/// Every complete context chooses exactly one section of each axis. An axis whose set has more
/// than one entry is unresolved, not disabled. Domains used together have the same axis keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct WorkspaceAxisDomain(BTreeMap<WorkspaceAxisName, BTreeSet<WorkspaceSectionName>>);

impl<'de> Deserialize<'de> for WorkspaceAxisDomain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_map(deserializer, |axis: &WorkspaceAxisName| {
            format!("duplicate normalized resolution axis `{axis}`")
        })
        .map(Self)
    }
}

impl WorkspaceAxisDomain {
    pub fn iter(
        &self,
    ) -> impl Iterator<Item = (&WorkspaceAxisName, &BTreeSet<WorkspaceSectionName>)> + Clone + '_
    {
        self.0.iter()
    }

    pub fn get(&self, axis: &WorkspaceAxisName) -> Option<&BTreeSet<WorkspaceSectionName>> {
        self.0.get(axis)
    }

    pub fn is_concrete(&self) -> bool {
        self.0.values().all(|sections| sections.len() == 1)
    }

    pub fn first_splittable_axis(&self) -> Option<&WorkspaceAxisName> {
        self.iter()
            .find_map(|(axis, sections)| (sections.len() > 1).then_some(axis))
    }

    /// Restrict this domain by a possibly partial selection.
    pub fn restrict(&self, selection: &WorkspaceAxisSelection) -> Option<Self> {
        let mut restricted = self.clone();
        for (axis, section) in selection.iter() {
            if !restricted.0.get(axis)?.contains(section) {
                return None;
            }
            restricted
                .0
                .insert(axis.clone(), BTreeSet::from([section.clone()]));
        }
        Some(restricted)
    }

    /// Bisect one axis. The returned nonempty domains are disjoint and cover this domain.
    pub fn split(&self, axis: &WorkspaceAxisName) -> Option<(Self, Self)> {
        let sections = self.0.get(axis)?;
        if sections.len() < 2 {
            return None;
        }
        let midpoint = sections.len() / 2;
        let mut left = self.clone();
        let mut right = self.clone();
        left.0.insert(
            axis.clone(),
            sections.iter().take(midpoint).cloned().collect(),
        );
        right.0.insert(
            axis.clone(),
            sections.iter().skip(midpoint).cloned().collect(),
        );
        Some((left, right))
    }

    pub fn intersection(&self, other: &Self) -> Option<Self> {
        if !self.0.keys().eq(other.0.keys()) {
            return None;
        }
        let mut intersection = BTreeMap::new();
        for (axis, sections) in self.iter() {
            let sections = sections
                .intersection(other.0.get(axis)?)
                .cloned()
                .collect::<BTreeSet<_>>();
            if sections.is_empty() {
                return None;
            }
            intersection.insert(axis.clone(), sections);
        }
        Some(Self(intersection))
    }

    pub fn is_disjoint(&self, other: &Self) -> bool {
        self.intersection(other).is_none()
    }

    /// Subtract another product using at most one residual product per axis.
    pub fn difference(&self, other: &Self) -> Vec<Self> {
        let Some(intersection) = self.intersection(other) else {
            return vec![self.clone()];
        };
        let mut remainder = self.clone();
        let mut residuals = Vec::new();
        for (axis, overlap) in intersection.iter() {
            let Some(sections) = remainder.0.get(axis) else {
                continue;
            };
            let outside = sections
                .difference(overlap)
                .cloned()
                .collect::<BTreeSet<_>>();
            if !outside.is_empty() {
                let mut residual = remainder.clone();
                residual.0.insert(axis.clone(), outside);
                residuals.push(residual);
                remainder.0.insert(axis.clone(), overlap.clone());
            }
        }
        residuals
    }

    pub fn is_covered_by<'a>(&self, domains: impl IntoIterator<Item = &'a Self>) -> bool {
        self.first_uncovered(domains).is_none()
    }

    pub fn first_uncovered<'a>(
        &self,
        domains: impl IntoIterator<Item = &'a Self>,
    ) -> Option<WorkspaceAxisSelection> {
        let mut remaining = vec![self.clone()];
        for domain in domains {
            remaining = remaining
                .into_iter()
                .flat_map(|remaining| remaining.difference(domain))
                .collect();
            if remaining.is_empty() {
                return None;
            }
        }
        remaining.first().and_then(Self::witness)
    }

    /// Return the first complete context in canonical axis and section order.
    pub fn witness(&self) -> Option<WorkspaceAxisSelection> {
        self.iter()
            .map(|(axis, sections)| Some((axis.clone(), sections.first()?.clone())))
            .collect::<Option<BTreeMap<_, _>>>()
            .map(WorkspaceAxisSelection)
    }

    /// Return a complete context satisfying an additional symbolic predicate.
    pub fn witness_for_marker(&self, marker: MarkerTree) -> Option<WorkspaceAxisSelection> {
        let mut remaining = self.to_marker_tree().and(marker);
        if remaining.is_false() {
            return None;
        }
        let mut selection = BTreeMap::new();
        for (axis, sections) in self.iter() {
            let (section, restricted) = sections.iter().find_map(|section| {
                let restricted =
                    select_workspace_axis_marker(remaining, axis.as_str(), section.as_str());
                (!restricted.is_false()).then_some((section, restricted))
            })?;
            selection.insert(axis.clone(), section.clone());
            remaining = restricted;
        }
        Some(WorkspaceAxisSelection(selection))
    }

    /// Express exactly one allowed section per axis.
    ///
    /// Intersect restricted domains with the full model domain to also exclude sections that are
    /// absent from this product; [`ResolvedWorkspaceAxes::marker_for_domain`] does this directly.
    pub fn to_marker_tree(&self) -> MarkerTree {
        self.iter()
            .fold(MarkerTree::TRUE, |marker, (axis, sections)| {
                let mut none = MarkerTree::TRUE;
                let mut one = MarkerTree::FALSE;
                for section in sections {
                    let section = workspace_axis_marker(axis.as_str(), section.as_str());
                    one = one.and(section.negate()).or(none.and(section));
                    none = none.and(section.negate());
                }
                marker.and(one)
            })
    }
}

/// A normalized axis definition, including the members shared by every section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "ResolvedWorkspaceAxesWire",
    into = "ResolvedWorkspaceAxesWire"
)]
pub struct ResolvedWorkspaceAxes {
    definitions: WorkspaceAxes,
    members: BTreeSet<PackageName>,
    assignments: BTreeMap<PackageName, WorkspaceAxisSelection>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ResolvedWorkspaceAxesWire {
    definitions: WorkspaceAxes,
    members: BTreeSet<PackageName>,
}

impl TryFrom<ResolvedWorkspaceAxesWire> for ResolvedWorkspaceAxes {
    type Error = WorkspaceAxisError;

    fn try_from(wire: ResolvedWorkspaceAxesWire) -> Result<Self, Self::Error> {
        Self::from_parts(wire.definitions, wire.members)
    }
}

impl From<ResolvedWorkspaceAxes> for ResolvedWorkspaceAxesWire {
    fn from(axes: ResolvedWorkspaceAxes) -> Self {
        Self {
            definitions: axes.definitions,
            members: axes.members,
        }
    }
}

impl ResolvedWorkspaceAxes {
    /// Construct a validated model from already expanded member selectors.
    ///
    /// Lock readers use this constructor without consulting the filesystem. Path selectors are
    /// retained as configuration provenance; their matched members are already in `members`.
    pub fn from_parts(
        definitions: WorkspaceAxes,
        members: BTreeSet<PackageName>,
    ) -> Result<Self, WorkspaceAxisError> {
        let mut assignments = members
            .iter()
            .cloned()
            .map(|member| (member, WorkspaceAxisSelection::default()))
            .collect::<BTreeMap<_, _>>();
        for (axis, sections) in definitions.iter() {
            if sections.is_empty() {
                return Err(WorkspaceAxisError::EmptyAxis(axis.clone()));
            }
            for (section, definition) in sections.iter() {
                for member in &definition.members {
                    let Some(assignment) = assignments.get_mut(member) else {
                        return Err(WorkspaceAxisError::UnknownSectionMember {
                            axis: axis.clone(),
                            section: section.clone(),
                            member: member.clone(),
                        });
                    };
                    if let Some(previous) = assignment.get(axis)
                        && previous != section
                    {
                        return Err(WorkspaceAxisError::OverlappingMember {
                            axis: axis.clone(),
                            member: member.clone(),
                            first: previous.clone(),
                            second: section.clone(),
                        });
                    }
                    assignment.0.insert(axis.clone(), section.clone());
                }
            }
        }
        Ok(Self {
            definitions,
            members,
            assignments,
        })
    }

    pub fn definitions(&self) -> &WorkspaceAxes {
        &self.definitions
    }

    pub fn members(&self) -> &BTreeSet<PackageName> {
        &self.members
    }

    pub fn assignments(&self) -> &BTreeMap<PackageName, WorkspaceAxisSelection> {
        &self.assignments
    }

    pub fn domain(&self) -> WorkspaceAxisDomain {
        WorkspaceAxisDomain(
            self.definitions
                .iter()
                .map(|(axis, sections)| {
                    (
                        axis.clone(),
                        sections
                            .iter()
                            .map(|(section, _)| section.clone())
                            .collect(),
                    )
                })
                .collect(),
        )
    }

    pub fn validate_selection(
        &self,
        selection: &WorkspaceAxisSelection,
    ) -> Result<(), WorkspaceAxisError> {
        for (axis, section) in selection.iter() {
            let Some(sections) = self.definitions.get(axis) else {
                return Err(WorkspaceAxisError::UnknownAxis(axis.clone()));
            };
            if sections.get(section).is_none() {
                return Err(WorkspaceAxisError::UnknownSection {
                    axis: axis.clone(),
                    section: section.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn validate_domain(&self, domain: &WorkspaceAxisDomain) -> Result<(), WorkspaceAxisError> {
        let full = self.domain();
        if !full.0.keys().eq(domain.0.keys())
            || domain.iter().any(|(axis, sections)| {
                sections.is_empty()
                    || full
                        .get(axis)
                        .is_none_or(|allowed| !sections.is_subset(allowed))
            })
        {
            return Err(WorkspaceAxisError::InvalidDomain);
        }
        Ok(())
    }

    /// Return a marker that also enforces the full axis universe's exclusivity constraints.
    pub fn marker_for_domain(&self, domain: &WorkspaceAxisDomain) -> MarkerTree {
        self.domain().to_marker_tree().and(domain.to_marker_tree())
    }

    pub fn guard_for_member(&self, member: &PackageName) -> Option<MarkerTree> {
        self.assignments
            .get(member)
            .map(WorkspaceAxisSelection::to_marker_tree)
    }

    pub fn selection_for_members<'a>(
        &self,
        members: impl IntoIterator<Item = &'a PackageName>,
    ) -> Result<WorkspaceAxisSelection, WorkspaceAxisError> {
        let mut selection = WorkspaceAxisSelection::default();
        for member in members {
            let Some(assignment) = self.assignments.get(member) else {
                return Err(WorkspaceAxisError::UnknownMember(member.clone()));
            };
            selection.merge(assignment)?;
        }
        Ok(selection)
    }

    /// Members that are roots in at least one context in this product.
    pub fn possible_roots(&self, domain: &WorkspaceAxisDomain) -> BTreeSet<PackageName> {
        self.assignments
            .iter()
            .filter(|(_, assignment)| domain.restrict(assignment).is_some())
            .map(|(member, _)| member.clone())
            .collect()
    }

    /// Members that are roots in every context in this product.
    pub fn guaranteed_roots(&self, domain: &WorkspaceAxisDomain) -> BTreeSet<PackageName> {
        self.assignments
            .iter()
            .filter(|(_, assignment)| {
                assignment.iter().all(|(axis, section)| {
                    domain
                        .get(axis)
                        .is_some_and(|allowed| allowed.len() == 1 && allowed.contains(section))
                })
            })
            .map(|(member, _)| member.clone())
            .collect()
    }
}

/// The exact physical coverage and conservative requirements of a symbolic axis domain.
#[derive(Debug, Clone)]
pub struct WorkspaceAxisEnvironment {
    /// Correlated private axis selectors and ordinary Python/platform markers.
    pub marker: MarkerTree,
    /// The physical projection of `marker`, without private selectors.
    pub environments: MarkerTree,
    pub requires_python: RequiresPython,
    /// The physical environments in which each possible root participates.
    pub roots: BTreeMap<PackageName, MarkerTree>,
    /// A conservative conjunction of the policies that can apply in this domain.
    pub constraint_dependencies: Vec<Requirement<VerbatimParsedUrl>>,
}

/// Whether an ordinary shared solve can cover a domain without losing Python/platform coverage.
#[derive(Debug, Clone)]
pub enum WorkspaceAxisResolutionView {
    Ready {
        workspace: Box<Workspace>,
        environment: WorkspaceAxisEnvironment,
    },
    Refine(WorkspaceAxisName),
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceAxisError {
    #[error("Invalid resolution-axis assignment `{0}`; expected `--resolution-axis AXIS=SECTION`")]
    InvalidAssignment(String),
    #[error("Invalid resolution axis name `{name}`: {source}")]
    InvalidAxisName {
        name: String,
        source: InvalidNameError,
    },
    #[error("Invalid resolution section name `{name}`: {source}")]
    InvalidSectionName {
        name: String,
        source: InvalidNameError,
    },
    #[error("Resolution axis `{axis}` selects both `{first}` and `{second}`")]
    ConflictingAssignments {
        axis: WorkspaceAxisName,
        first: WorkspaceSectionName,
        second: WorkspaceSectionName,
    },
    #[error("Resolution axis `{0}` is not defined")]
    UnknownAxis(WorkspaceAxisName),
    #[error("Resolution axis `{axis}` has no section `{section}`")]
    UnknownSection {
        axis: WorkspaceAxisName,
        section: WorkspaceSectionName,
    },
    #[error("Resolution axis `{0}` has no sections")]
    EmptyAxis(WorkspaceAxisName),
    #[error(
        "Resolution axis `{axis}` section `{section}` contains unknown workspace member `{member}`"
    )]
    UnknownSectionMember {
        axis: WorkspaceAxisName,
        section: WorkspaceSectionName,
        member: PackageName,
    },
    #[error("Workspace member `{0}` is not part of the resolution-axis model")]
    UnknownMember(PackageName),
    #[error(
        "Workspace member `{member}` belongs to both `{first}` and `{second}` on resolution axis `{axis}`"
    )]
    OverlappingMember {
        axis: WorkspaceAxisName,
        member: PackageName,
        first: WorkspaceSectionName,
        second: WorkspaceSectionName,
    },
    #[error(
        "Invalid member path `{path}` in resolution axis `{axis}` section `{section}`: {reason}"
    )]
    InvalidMemberPath {
        axis: WorkspaceAxisName,
        section: WorkspaceSectionName,
        path: String,
        reason: String,
    },
    #[error(
        "Member path `{path}` in resolution axis `{axis}` section `{section}` does not match a workspace member"
    )]
    UnmatchedMemberPath {
        axis: WorkspaceAxisName,
        section: WorkspaceSectionName,
        path: String,
    },
    #[error(
        "`tool.uv.workspace.groups` and `tool.uv.workspace.resolution-axes` cannot be combined"
    )]
    MixedWorkspaceGroups,
    #[error("Resolution-axis domain does not match the workspace's declared axes and sections")]
    InvalidDomain,
    #[error("Resolution context `{selection}` has no compatible Python/platform environment")]
    EmptyEnvironment { selection: WorkspaceAxisSelection },
    #[error(
        "Workspace member `{member}` is required outside its resolution-axis assignments in context `{selection}`"
    )]
    IncompatibleMember {
        member: PackageName,
        selection: WorkspaceAxisSelection,
    },
    #[error(
        "Workspace member `{member}` is missing its current local source from the locked resolution"
    )]
    MissingMemberSource { member: PackageName },
    #[error("The workspace resolution is missing authoritative manifest-root identities")]
    MissingManifestRoots,
    #[error("Invalid dependency in resolution-axis member `{member}`: {reason}")]
    InvalidDependency { member: PackageName, reason: String },
    #[error("Invalid workspace resolution-axis group metadata: {0}")]
    InvalidGroupMetadata(String),
    #[error(transparent)]
    DependencyGroups(#[from] crate::dependency_groups::DependencyGroupError),
    #[error(transparent)]
    DefaultGroups(#[from] crate::DefaultGroupsError),
}

impl Workspace {
    /// Resolve named and path-based section selectors against the discovered workspace members.
    pub fn resolution_axes(&self) -> Result<Option<ResolvedWorkspaceAxes>, WorkspaceError> {
        let Some(workspace) = self
            .pyproject_toml()
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
        else {
            return Ok(None);
        };
        let Some(definitions) = workspace
            .resolution_axes
            .as_ref()
            .filter(|axes| !axes.is_empty())
        else {
            return Ok(None);
        };
        if workspace
            .groups
            .as_ref()
            .is_some_and(|groups| !groups.is_empty())
        {
            return Err(WorkspaceAxisError::MixedWorkspaceGroups.into());
        }
        let paths = self
            .packages()
            .iter()
            .map(|(name, member)| {
                uv_fs::relative_to(member.root(), self.install_path())
                    .map(|path| (name.clone(), path))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let mut definitions = definitions.clone();
        let options = MatchOptions {
            require_literal_separator: true,
            ..MatchOptions::new()
        };
        for (axis, sections) in &mut definitions.0 {
            for (section, definition) in &mut sections.0 {
                for raw in &definition.member_paths {
                    if Path::new(raw).is_absolute() {
                        return Err(WorkspaceAxisError::InvalidMemberPath {
                            axis: axis.clone(),
                            section: section.clone(),
                            path: raw.clone(),
                            reason: "expected a workspace-relative path".to_owned(),
                        }
                        .into());
                    }
                    let normalized = uv_fs::normalize_path(Path::new(raw));
                    let pattern = Pattern::new(&normalized.to_string_lossy()).map_err(|error| {
                        WorkspaceAxisError::InvalidMemberPath {
                            axis: axis.clone(),
                            section: section.clone(),
                            path: raw.clone(),
                            reason: error.to_string(),
                        }
                    })?;
                    let matched = paths
                        .iter()
                        .filter(|(_, path)| {
                            if path.as_os_str().is_empty() {
                                pattern.matches_path_with(Path::new("."), options)
                            } else {
                                pattern.matches_path_with(path, options)
                            }
                        })
                        .map(|(name, _)| name.clone())
                        .collect::<Vec<_>>();
                    if matched.is_empty() {
                        return Err(WorkspaceAxisError::UnmatchedMemberPath {
                            axis: axis.clone(),
                            section: section.clone(),
                            path: raw.clone(),
                        }
                        .into());
                    }
                    definition.members.extend(matched);
                }
            }
        }
        ResolvedWorkspaceAxes::from_parts(definitions, self.packages().keys().cloned().collect())
            .map(Some)
            .map_err(Into::into)
    }

    /// Compute exact conditional Python/platform coverage without enumerating complete contexts.
    pub fn environment_for_domain(
        &self,
        axes: &ResolvedWorkspaceAxes,
        domain: &WorkspaceAxisDomain,
        no_sources: &NoSources,
    ) -> Result<WorkspaceAxisEnvironment, WorkspaceError> {
        axes.validate_domain(domain)?;
        let domain_marker = axes.marker_for_domain(domain);
        let mut marker = domain_marker;
        if let Some(configured) = self.environments().filter(|markers| !markers.is_empty()) {
            marker = marker.and(
                configured
                    .iter()
                    .fold(MarkerTree::FALSE, |marker, environment| {
                        marker.or(*environment)
                    }),
            );
        }
        for (axis, sections) in axes.definitions().iter() {
            for (section, definition) in sections.iter() {
                if let Some(requires_python) = &definition.requires_python {
                    marker = marker.and(
                        workspace_axis_marker(axis.as_str(), section.as_str()).implies(
                            RequiresPython::from_specifiers(requires_python.clone())
                                .to_exact_marker_tree(),
                        ),
                    );
                }
            }
        }
        let reached = self.axis_reachable_workspace_members(axes, domain, no_sources)?;
        for (name, active) in &reached {
            if let Some(requires_python) = self
                .packages()
                .get(name)
                .and_then(|member| member.project().requires_python.as_ref())
            {
                marker = marker.and(active.implies(
                    RequiresPython::from_specifiers(requires_python.clone()).to_exact_marker_tree(),
                ));
            }
        }
        let invalid = domain_marker.and(marker.only_extras().negate());
        if let Some(selection) = domain.witness_for_marker(invalid) {
            return Err(WorkspaceAxisError::EmptyEnvironment { selection }.into());
        }
        for (member, active) in &reached {
            let Some(guard) = axes.guard_for_member(member) else {
                continue;
            };
            let incompatible = marker.and(*active).and(guard.negate());
            if let Some(selection) = domain.witness_for_marker(incompatible) {
                return Err(WorkspaceAxisError::IncompatibleMember {
                    member: member.clone(),
                    selection,
                }
                .into());
            }
        }
        let environments = marker.without_extras();
        let Some(requires_python) = RequiresPython::from_marker_tree(environments) else {
            return Err(WorkspaceAxisError::EmptyEnvironment {
                selection: domain.witness().unwrap_or_default(),
            }
            .into());
        };
        let roots = axes
            .possible_roots(domain)
            .into_iter()
            .filter_map(|member| {
                let guard = axes.guard_for_member(&member)?;
                let environment = marker.and(guard).without_extras();
                (!environment.is_false()).then_some((member, environment))
            })
            .collect();
        let mut constraint_dependencies = Vec::new();
        for (axis, sections) in axes.definitions().iter() {
            for (section, definition) in sections.iter() {
                let environment = marker
                    .and(workspace_axis_marker(axis.as_str(), section.as_str()))
                    .without_extras();
                if environment.is_false() {
                    continue;
                }
                for constraint in &definition.constraint_dependencies {
                    let mut constraint = constraint.clone();
                    constraint.marker = constraint.marker.and(environment);
                    if !constraint.marker.is_false() {
                        constraint_dependencies.push(constraint);
                    }
                }
            }
        }
        Ok(WorkspaceAxisEnvironment {
            marker,
            environments,
            requires_python,
            roots,
            constraint_dependencies,
        })
    }

    /// Create the physical union needed for interpreter discovery before choosing a context.
    pub fn resolution_axis_python_view(
        &self,
        axes: &ResolvedWorkspaceAxes,
        domain: &WorkspaceAxisDomain,
        no_sources: &NoSources,
    ) -> Result<Self, WorkspaceError> {
        let environment = self.environment_for_domain(axes, domain, no_sources)?;
        Ok(self.with_axis_environment(environment))
    }

    /// Prepare a shared solve only when all contexts have identical physical coverage.
    pub fn resolution_axis_view(
        &self,
        axes: &ResolvedWorkspaceAxes,
        domain: &WorkspaceAxisDomain,
        no_sources: &NoSources,
    ) -> Result<WorkspaceAxisResolutionView, WorkspaceError> {
        let environment = self.environment_for_domain(axes, domain, no_sources)?;
        let independent = axes.marker_for_domain(domain).and(environment.environments);
        if !equivalent_markers(environment.marker, independent) {
            let varying = domain.iter().find_map(|(axis, sections)| {
                let mut previous = None;
                for section in sections {
                    let physical = environment
                        .marker
                        .and(workspace_axis_marker(axis.as_str(), section.as_str()))
                        .without_extras();
                    if previous.is_some_and(|previous| !equivalent_markers(previous, physical)) {
                        return Some(axis);
                    }
                    previous = Some(physical);
                }
                None
            });
            if let Some(axis) = varying.or_else(|| domain.first_splittable_axis()) {
                return Ok(WorkspaceAxisResolutionView::Refine(axis.clone()));
            }
        }
        Ok(WorkspaceAxisResolutionView::Ready {
            workspace: Box::new(self.with_axis_environment(environment.clone())),
            environment,
        })
    }

    /// Scope interpreter discovery from an ordinary projected lock, without live member metadata.
    #[must_use]
    pub fn with_resolution_axis_environment(
        &self,
        roots: BTreeSet<PackageName>,
        requires_python: RequiresPython,
        environments: SupportedEnvironments,
    ) -> Self {
        self.with_resolution_axis_environment_inner(roots, requires_python, environments, false)
    }

    /// Scope interpreter discovery after the selected dependency groups' complete Python policy
    /// has already been included in the supplied environment.
    #[must_use]
    pub fn with_resolution_axis_command_environment(
        &self,
        roots: BTreeSet<PackageName>,
        requires_python: RequiresPython,
        environments: SupportedEnvironments,
    ) -> Self {
        self.with_resolution_axis_environment_inner(roots, requires_python, environments, true)
    }

    fn with_resolution_axis_environment_inner(
        &self,
        roots: BTreeSet<PackageName>,
        requires_python: RequiresPython,
        environments: SupportedEnvironments,
        group_python_complete: bool,
    ) -> Self {
        let marker = if environments.is_empty() {
            requires_python.to_exact_marker_tree()
        } else {
            environments
                .iter()
                .fold(MarkerTree::FALSE, |marker, environment| {
                    marker.or(*environment)
                })
        };
        self.with_resolution(WorkspaceResolution {
            roots: roots.into_iter().map(|root| (root, marker)).collect(),
            requires_python,
            environments,
            constraints: Vec::new(),
            unavailable_members: BTreeMap::new(),
            group_python_complete,
        })
    }

    fn with_axis_environment(&self, environment: WorkspaceAxisEnvironment) -> Self {
        let unavailable_members = self
            .packages()
            .iter()
            .filter(|(name, _)| !environment.roots.contains_key(*name))
            .map(|(name, member)| (name.clone(), member.root().clone()))
            .collect();
        self.with_resolution(WorkspaceResolution {
            roots: environment.roots,
            requires_python: environment.requires_python,
            environments: SupportedEnvironments::from_markers(vec![environment.environments]),
            constraints: environment.constraint_dependencies,
            unavailable_members,
            group_python_complete: false,
        })
    }

    /// Follow local production edges with the complete context and physical marker on each path.
    fn axis_reachable_workspace_members(
        &self,
        axes: &ResolvedWorkspaceAxes,
        domain: &WorkspaceAxisDomain,
        no_sources: &NoSources,
    ) -> Result<BTreeMap<PackageName, MarkerTree>, WorkspaceError> {
        let mut reached = BTreeMap::new();
        let mut pending = axes
            .possible_roots(domain)
            .into_iter()
            .filter_map(|name| axes.guard_for_member(&name).map(|marker| (name, marker)))
            .collect::<Vec<_>>();
        while let Some((name, marker)) = pending.pop() {
            let previous = reached.entry(name.clone()).or_insert(MarkerTree::FALSE);
            let active = previous.or(marker);
            if active == *previous {
                continue;
            }
            *previous = active;
            let Some(member) = self.packages().get(&name) else {
                continue;
            };
            let member_sources = member
                .pyproject_toml()
                .tool
                .as_ref()
                .and_then(|tool| tool.uv.as_ref())
                .and_then(|uv| uv.sources.as_ref());
            for dependency in member.project().dependencies.iter().flatten() {
                let requirement =
                    Requirement::<VerbatimParsedUrl>::from_str(dependency).map_err(|error| {
                        WorkspaceAxisError::InvalidDependency {
                            member: name.clone(),
                            reason: error.to_string(),
                        }
                    })?;
                if no_sources.for_package(&requirement.name) {
                    continue;
                }
                let Some(target) = self.packages().get(&requirement.name) else {
                    continue;
                };
                let sources = member_sources
                    .and_then(|sources| sources.inner().get(&requirement.name))
                    .map(|sources| (sources, member.root()))
                    .or_else(|| {
                        self.sources()
                            .get(&requirement.name)
                            .map(|sources| (sources, self.install_path()))
                    });
                let Some((sources, base)) = sources else {
                    continue;
                };
                for source in sources.iter() {
                    if source.extra().is_some() || source.group().is_some() {
                        continue;
                    }
                    let local = match source {
                        Source::Workspace { workspace, .. } => match workspace {
                            WorkspaceReference::Bool(local) => *local,
                            WorkspaceReference::Path(_) => false,
                        },
                        Source::Path { path, .. } => {
                            uv_fs::normalize_path(base.join(path.as_ref())) == *target.root()
                        }
                        Source::Git { .. } | Source::Url { .. } | Source::Registry { .. } => false,
                    };
                    if local {
                        let marker = active
                            .and(requirement.marker.simplify_not_extras_with(|_| true))
                            .and(source.marker().simplify_not_extras_with(|_| true));
                        if !marker.is_false() {
                            pending.push((requirement.name.clone(), marker));
                        }
                    }
                }
            }
        }
        Ok(reached)
    }
}

/// Compare semantic coverage after projection, which can retain redundant decision edges.
fn equivalent_markers(left: MarkerTree, right: MarkerTree) -> bool {
    left == right || (left.and(right.negate()).is_false() && right.and(left.negate()).is_false())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use anyhow::{Context, Result};
    use assert_fs::TempDir;
    use assert_fs::prelude::{FileWriteStr, PathChild};
    use insta::assert_snapshot;

    use uv_cache::Cache;
    use uv_configuration::NoSources;
    use uv_distribution_types::RequiresPython;
    use uv_normalize::PackageName;
    use uv_pep508::MarkerTree;

    use super::{
        ResolvedWorkspaceAxes, WorkspaceAxes, WorkspaceAxis, WorkspaceAxisAssignment,
        WorkspaceAxisResolutionView, WorkspaceAxisSection, WorkspaceAxisSelection,
    };
    use crate::{DiscoveryOptions, Workspace, WorkspaceCache};

    fn model() -> Result<ResolvedWorkspaceAxes> {
        let definitions = toml::from_str::<WorkspaceAxes>(
            r#"
            [python.py312]
            members = ["legacy"]
            [python.py313]
            members = ["api"]
            [sqlalchemy.v1]
            members = ["legacy"]
            [sqlalchemy.v2]
            members = ["api", "worker"]
            [lib.v1]
            [lib.v2]
            [lib.v3]
            members = ["api"]
            "#,
        )?;
        let members = ["api", "legacy", "shared", "worker"]
            .into_iter()
            .map(str::parse::<PackageName>)
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(ResolvedWorkspaceAxes::from_parts(definitions, members)?)
    }

    fn selection(assignments: &[&str]) -> Result<WorkspaceAxisSelection> {
        Ok(WorkspaceAxisSelection::from_assignments(
            assignments
                .iter()
                .map(|assignment| assignment.parse::<WorkspaceAxisAssignment>())
                .collect::<Result<Vec<_>, _>>()?,
        )?)
    }

    async fn discover(root: &str, members: &[(&str, &str)]) -> Result<(TempDir, Arc<Workspace>)> {
        let directory = TempDir::new()?;
        directory.child("pyproject.toml").write_str(root)?;
        for (path, contents) in members {
            directory
                .child(path)
                .child("pyproject.toml")
                .write_str(contents)?;
        }
        let cache = Cache::from_path(directory.path().join(".cache"));
        let workspace = Workspace::discover(
            directory.path(),
            &DiscoveryOptions::default(),
            &cache,
            &WorkspaceCache::default(),
        )
        .await?;
        Ok((directory, workspace))
    }

    #[test]
    fn member_assignments_are_conjunctive() -> Result<()> {
        let axes = model()?;
        let domain = axes
            .domain()
            .restrict(&selection(&["python=py313", "lib=v2"])?)
            .context("valid partial selection")?;
        assert_eq!(
            axes.possible_roots(&domain),
            ["shared", "worker"]
                .into_iter()
                .map(str::parse)
                .collect::<Result<BTreeSet<_>, _>>()?
        );
        assert_eq!(
            axes.guaranteed_roots(&domain),
            BTreeSet::from(["shared".parse()?])
        );
        let domain = domain
            .restrict(&selection(&["sqlalchemy=v2"])?)
            .context("valid SQLAlchemy selection")?;
        assert_eq!(axes.guaranteed_roots(&domain), axes.possible_roots(&domain));
        Ok(())
    }

    #[test]
    fn product_splits_cover_without_enumeration() -> Result<()> {
        let axes = model()?;
        let full = axes.domain();
        let (left, right) = full
            .split(&"lib".parse()?)
            .context("multiway axis can be split")?;
        assert!(left.is_disjoint(&right));
        assert!(full.is_covered_by([&left, &right]));
        assert!(!full.is_covered_by([&left]));
        assert_eq!(full.difference(&left), vec![right.clone()]);
        assert_eq!(full.first_uncovered([&left]), right.witness());
        let selected = full
            .restrict(&selection(&["python=py313", "sqlalchemy=v2", "lib=v3"])?)
            .context("complete selection")?;
        assert!(selected.is_concrete());
        let residual = full.difference(&selected);
        assert_eq!(residual.len(), 3);
        assert!(full.is_covered_by(residual.iter().chain([&selected])));
        for (index, first) in residual.iter().enumerate() {
            assert!(first.is_disjoint(&selected));
            for second in residual.iter().skip(index + 1) {
                assert!(first.is_disjoint(second));
            }
        }
        Ok(())
    }

    #[test]
    fn marker_domain_excludes_other_sections() -> Result<()> {
        let axes = model()?;
        let full = axes.domain();
        let left = full
            .restrict(&selection(&["sqlalchemy=v1"])?)
            .context("first section")?;
        let right = full
            .restrict(&selection(&["sqlalchemy=v2"])?)
            .context("second section")?;
        assert!(
            axes.marker_for_domain(&left)
                .is_disjoint(axes.marker_for_domain(&right))
        );
        assert_eq!(
            axes.marker_for_domain(&left)
                .or(axes.marker_for_domain(&right)),
            axes.marker_for_domain(&full)
        );
        assert_eq!(
            full.witness_for_marker(axes.marker_for_domain(&right)),
            right.witness()
        );
        assert!(
            left.witness_for_marker(selection(&["sqlalchemy=v2"])?.to_marker_tree())
                .is_none()
        );
        assert!(full.witness_for_marker(MarkerTree::FALSE).is_none());
        Ok(())
    }

    #[test]
    fn repeated_selections_cannot_change_sections() -> Result<()> {
        let mut selected = selection(&["python=py312"])?;
        selected.merge(&selection(&["python=py312"])?)?;
        let error = selected
            .merge(&selection(&["python=py313", "sqlalchemy=v2"])?)
            .expect_err("contradictory section selection");
        assert_snapshot!(error.to_string(), @"Resolution axis `python` selects both `py312` and `py313`");
        assert_eq!(selected, selection(&["python=py312"])?);
        Ok(())
    }

    #[test]
    fn overlapping_section_members_are_rejected() -> Result<()> {
        let definitions = toml::from_str::<WorkspaceAxes>(
            r#"
            [python.py312]
            members = ["api"]
            [python.py313]
            members = ["api"]
            "#,
        )?;
        let error =
            ResolvedWorkspaceAxes::from_parts(definitions, BTreeSet::from(["api".parse()?]))
                .expect_err("overlapping sections");
        assert_snapshot!(error.to_string(), @"Workspace member `api` belongs to both `py312` and `py313` on resolution axis `python`");
        Ok(())
    }

    #[test]
    fn normalized_axis_and_section_names_must_be_unique() {
        let error = toml::from_str::<WorkspaceAxes>(
            r"
            [Python.py312]
            [python.py313]
            ",
        )
        .expect_err("duplicate normalized axis");
        assert_snapshot!(error.message(), @"duplicate normalized resolution axis `python`");
        let error = toml::from_str::<WorkspaceAxes>(
            r"
            [python.Py_312]
            [python.py-312]
            ",
        )
        .expect_err("duplicate normalized section");
        assert_snapshot!(error.message(), @"duplicate normalized resolution section `py-312`");
    }

    #[test]
    fn serialized_model_is_validated_on_read() -> Result<()> {
        let axes = model()?;
        let serialized = toml::to_string(&axes)?;
        assert_eq!(toml::from_str::<ResolvedWorkspaceAxes>(&serialized)?, axes);
        Ok(())
    }

    #[test]
    fn large_product_stays_symbolic() -> Result<()> {
        let definitions = WorkspaceAxes(
            (0..48)
                .map(|axis| {
                    Ok((
                        format!("axis-{axis:02}").parse()?,
                        WorkspaceAxis(
                            (0..4)
                                .map(|section| {
                                    Ok((
                                        format!("section-{section}").parse()?,
                                        WorkspaceAxisSection::default(),
                                    ))
                                })
                                .collect::<Result<BTreeMap<_, _>>>()?,
                        ),
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        );
        let axes =
            ResolvedWorkspaceAxes::from_parts(definitions, BTreeSet::from(["shared".parse()?]))?;
        let full = axes.domain();
        assert_eq!(full.iter().count(), 48);
        assert_eq!(
            full.witness().context("nonempty product")?.iter().count(),
            48
        );
        let (left, right) = full
            .split(&"axis-00".parse()?)
            .context("axis can be split")?;
        assert!(full.is_covered_by([&left, &right]));
        assert_eq!(axes.possible_roots(&left), axes.members().clone());
        Ok(())
    }

    #[tokio::test]
    async fn physical_domains_are_projected_before_python_selection() -> Result<()> {
        let (_directory, workspace) = discover(
            r#"
            [tool.uv.workspace]
            members = ["members/*"]
            [tool.uv.workspace.resolution-axes.python]
            py312 = { members = ["legacy"], requires-python = "==3.12.*" }
            py313 = { members = ["api"], requires-python = "==3.13.*" }
            "#,
            &[
                (
                    "members/legacy",
                    r#"
                    [project]
                    name = "legacy"
                    version = "1"
                    requires-python = "==3.12.*"
                    "#,
                ),
                (
                    "members/api",
                    r#"
                    [project]
                    name = "api"
                    version = "1"
                    requires-python = "==3.13.*"
                    "#,
                ),
                (
                    "members/shared",
                    r#"
                    [project]
                    name = "shared"
                    version = "1"
                    requires-python = ">=3.12"
                    "#,
                ),
            ],
        )
        .await?;
        assert!(!workspace.is_workspace_axis_resolution());
        let axes = workspace.resolution_axes()?.context("configured axes")?;
        let full = axes.domain();
        let environment = workspace.environment_for_domain(&axes, &full, &NoSources::None)?;
        let expected = RequiresPython::from_specifiers("==3.12.*".parse()?)
            .to_exact_marker_tree()
            .or(RequiresPython::from_specifiers("==3.13.*".parse()?).to_exact_marker_tree())
            .and(RequiresPython::from_specifiers(">=3.12".parse()?).to_exact_marker_tree());
        assert_eq!(
            environment.environments.and(expected.negate()),
            MarkerTree::FALSE,
            "unexpected environment: {:?}; expected: {:?}",
            environment.environments.kind(),
            expected.kind(),
        );
        assert_eq!(
            expected.and(environment.environments.negate()),
            MarkerTree::FALSE,
            "missing environment: {:?}; expected: {:?}",
            environment.environments.kind(),
            expected.kind(),
        );
        assert_eq!(
            environment.marker.only_extras(),
            axes.marker_for_domain(&full)
        );
        let view = workspace.resolution_axis_view(&axes, &full, &NoSources::None)?;
        let WorkspaceAxisResolutionView::Refine(axis) = view else {
            anyhow::bail!("different Python domains require refinement");
        };
        assert_eq!(axis, "python".parse()?);
        let selected = full
            .restrict(&selection(&["python=py312"])?)
            .context("selected context")?;
        let view = workspace.resolution_axis_view(&axes, &selected, &NoSources::None)?;
        let WorkspaceAxisResolutionView::Ready { workspace, .. } = view else {
            anyhow::bail!("concrete context does not require refinement");
        };
        assert!(workspace.is_workspace_axis_resolution());
        assert_eq!(
            workspace
                .members_requirements()
                .map(|requirement| requirement.name)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["legacy".parse()?, "shared".parse()?])
        );
        Ok(())
    }

    #[tokio::test]
    async fn empty_declared_context_is_not_dropped() -> Result<()> {
        let (_directory, workspace) = discover(
            r#"
            [tool.uv.workspace]
            members = ["members/*"]
            [tool.uv.workspace.resolution-axes.python]
            py312 = { requires-python = "==3.12.*" }
            py313 = { requires-python = "==3.13.*" }
            [tool.uv.workspace.resolution-axes.runtime]
            v1 = { requires-python = "==3.12.*" }
            v2 = { requires-python = "==3.13.*" }
            "#,
            &[(
                "members/shared",
                r#"
                [project]
                name = "shared"
                version = "1"
                requires-python = ">=3.12"
                "#,
            )],
        )
        .await?;
        let axes = workspace.resolution_axes()?.context("configured axes")?;
        let error = workspace
            .environment_for_domain(&axes, &axes.domain(), &NoSources::None)
            .expect_err("cross-axis Python policies exclude a declared context");
        assert_snapshot!(error.to_string(), @"Resolution context `python=py312, runtime=v2` has no compatible Python/platform environment");
        Ok(())
    }

    #[tokio::test]
    async fn path_selectors_expand_to_member_names() -> Result<()> {
        let (_directory, workspace) = discover(
            r#"
            [tool.uv.workspace]
            members = ["services/*"]
            [tool.uv.workspace.resolution-axes.runtime]
            v1 = { member-paths = ["./services/a*"] }
            v2 = { members = ["worker"], constraint-dependencies = ["sqlalchemy>=2"] }
            "#,
            &[
                (
                    "services/api",
                    r#"
                    [project]
                    name = "api"
                    version = "1"
                    requires-python = ">=3.12"
                    "#,
                ),
                (
                    "services/worker",
                    r#"
                    [project]
                    name = "worker"
                    version = "1"
                    requires-python = ">=3.12"
                    "#,
                ),
            ],
        )
        .await?;
        let axes = workspace.resolution_axes()?.context("configured axes")?;
        assert_eq!(
            axes.selection_for_members([&"api".parse()?])?,
            selection(&["runtime=v1"])?
        );
        let selected = axes
            .domain()
            .restrict(&selection(&["runtime=v2"])?)
            .context("selected context")?;
        let WorkspaceAxisResolutionView::Ready { workspace, .. } =
            workspace.resolution_axis_view(&axes, &selected, &NoSources::None)?
        else {
            anyhow::bail!("concrete context does not require refinement");
        };
        assert_eq!(
            workspace
                .constraints()
                .iter()
                .map(|requirement| requirement.name.as_str())
                .collect::<Vec<_>>(),
            vec!["sqlalchemy"]
        );
        assert!(workspace.requirements().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn mandatory_cross_lane_member_is_rejected() -> Result<()> {
        let (_directory, workspace) = discover(
            r#"
            [tool.uv.workspace]
            members = ["members/*"]
            [tool.uv.workspace.resolution-axes.sqlalchemy]
            v1 = { members = ["legacy"] }
            v2 = { members = ["api"] }
            "#,
            &[
                (
                    "members/legacy",
                    r#"
                    [project]
                    name = "legacy"
                    version = "1"
                    requires-python = ">=3.12"
                    "#,
                ),
                (
                    "members/api",
                    r#"
                    [project]
                    name = "api"
                    version = "1"
                    requires-python = ">=3.12"
                    dependencies = ["legacy"]
                    [tool.uv.sources]
                    legacy = { workspace = true }
                    "#,
                ),
            ],
        )
        .await?;
        let axes = workspace.resolution_axes()?.context("configured axes")?;
        let error = workspace
            .environment_for_domain(&axes, &axes.domain(), &NoSources::None)
            .expect_err("required member has an incompatible lane assignment");
        assert_snapshot!(error.to_string(), @"Workspace member `legacy` is required outside its resolution-axis assignments in context `sqlalchemy=v2`");
        Ok(())
    }
}
