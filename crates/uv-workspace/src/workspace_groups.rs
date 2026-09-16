use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use uv_configuration::NoSources;
use uv_distribution_types::RequiresPython;
use uv_normalize::{GroupName, PackageName};
use uv_pep440::VersionSpecifiers;
use uv_pep508::{MarkerTree, Requirement};
use uv_pypi_types::{SupportedEnvironments, VerbatimParsedUrl};

use crate::pyproject::{Source, WorkspaceReference};
use crate::{Workspace, WorkspaceError, WorkspaceErrorKind};

/// A named set of workspace resolution roots.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct WorkspaceGroup {
    /// The unique name used by `--workspace-group`.
    pub name: GroupName,
    /// The `project.name` values of the workspace members to use as resolution roots.
    pub members: BTreeSet<PackageName>,
    /// An optional restriction on the Python versions supported by this group.
    #[cfg_attr(feature = "schemars", schemars(with = "Option<String>"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_python: Option<VersionSpecifiers>,
    /// Whether unqualified commands select this group's roots and resolution.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
}

/// A validated workspace group and its effective Python requirement.
#[derive(Debug, Clone)]
pub struct ResolvedWorkspaceGroup {
    pub definition: WorkspaceGroup,
    pub requires_python: RequiresPython,
    /// The supported environments, including conditional local-member Python bounds.
    pub environments: MarkerTree,
}

/// The roots and Python domain of one shared resolution attempt.
#[derive(Debug, Clone)]
pub(crate) struct WorkspaceResolution {
    pub roots: BTreeMap<PackageName, MarkerTree>,
    pub requires_python: RequiresPython,
    pub environments: SupportedEnvironments,
}

impl Workspace {
    /// Validate and resolve the named groups declared by the workspace root.
    pub fn workspace_groups(&self) -> Result<Vec<ResolvedWorkspaceGroup>, WorkspaceError> {
        self.workspace_groups_with_sources(&NoSources::None)
    }

    /// Resolve groups using the source policy of the current operation.
    pub fn workspace_groups_with_sources(
        &self,
        no_sources: &NoSources,
    ) -> Result<Vec<ResolvedWorkspaceGroup>, WorkspaceError> {
        let Some(definitions) = self
            .pyproject_toml()
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
            .and_then(|workspace| workspace.groups.as_ref())
        else {
            return Ok(Vec::new());
        };
        let mut names = BTreeSet::new();
        let mut default = None;
        let mut groups = Vec::with_capacity(definitions.len());
        for definition in definitions {
            if !names.insert(&definition.name) {
                return Err(
                    WorkspaceErrorKind::DuplicateWorkspaceGroup(definition.name.clone()).into(),
                );
            }
            if definition.default
                && let Some(previous) = default.replace(&definition.name)
            {
                return Err(WorkspaceErrorKind::MultipleDefaultWorkspaceGroups(
                    previous.clone(),
                    definition.name.clone(),
                )
                .into());
            }
            if definition.members.is_empty() {
                return Err(
                    WorkspaceErrorKind::EmptyWorkspaceGroup(definition.name.clone()).into(),
                );
            }
            let mut environments =
                definition
                    .requires_python
                    .as_ref()
                    .map_or(MarkerTree::TRUE, |requires_python| {
                        RequiresPython::from_specifiers(requires_python.clone())
                            .to_exact_marker_tree()
                    });
            for name in &definition.members {
                if !self.packages().contains_key(name) {
                    return Err(WorkspaceErrorKind::UnknownWorkspaceGroupMember(
                        definition.name.clone(),
                        name.clone(),
                    )
                    .into());
                }
            }
            for (name, active) in self.reachable_workspace_members(definition, no_sources)? {
                if let Some(requires_python) = self
                    .packages()
                    .get(&name)
                    .and_then(|member| member.project().requires_python.as_ref())
                {
                    let compatible = RequiresPython::from_specifiers(requires_python.clone())
                        .to_exact_marker_tree();
                    environments = environments.and(active.implies(compatible));
                }
            }
            if let Some(configured) = self
                .environments()
                .filter(|configured| !configured.is_empty())
            {
                let supported = configured
                    .iter()
                    .fold(MarkerTree::FALSE, |supported, marker| supported.or(*marker));
                environments = environments.and(supported);
            }
            let requires_python =
                RequiresPython::from_marker_tree(environments).ok_or_else(|| {
                    WorkspaceError::from(WorkspaceErrorKind::DisjointWorkspaceGroupPython(
                        definition.name.clone(),
                    ))
                })?;
            groups.push(ResolvedWorkspaceGroup {
                definition: definition.clone(),
                requires_python,
                environments,
            });
        }
        Ok(groups)
    }

    /// Follow production dependencies to local members, retaining their activation markers.
    fn reachable_workspace_members(
        &self,
        group: &WorkspaceGroup,
        no_sources: &NoSources,
    ) -> Result<BTreeMap<PackageName, MarkerTree>, WorkspaceError> {
        let mut reached = BTreeMap::new();
        let mut pending = group
            .members
            .iter()
            .cloned()
            .map(|name| (name, MarkerTree::TRUE))
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
                        WorkspaceError::from(WorkspaceErrorKind::InvalidWorkspaceGroupDependency(
                            group.name.clone(),
                            name.clone(),
                            error.to_string(),
                        ))
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
                        Source::Workspace {
                            workspace: WorkspaceReference::Bool(true),
                            ..
                        } => true,
                        Source::Path { path, .. } => {
                            uv_fs::normalize_path(base.join(path.as_ref())) == *target.root()
                        }
                        _ => false,
                    };
                    if local {
                        let marker = active.and(requirement.marker).and(source.marker());
                        if !marker.is_false() {
                            pending.push((requirement.name.clone(), marker));
                        }
                    }
                }
            }
        }
        Ok(reached)
    }

    /// Return the named or default workspace group.
    pub fn workspace_group(
        &self,
        name: Option<&GroupName>,
    ) -> Result<Option<ResolvedWorkspaceGroup>, WorkspaceError> {
        let groups = self.workspace_groups()?;
        if let Some(name) = name {
            return groups
                .into_iter()
                .find(|group| group.definition.name == *name)
                .map(Some)
                .ok_or_else(|| WorkspaceErrorKind::UnknownWorkspaceGroup(name.clone()).into());
        }
        Ok(groups.into_iter().find(|group| group.definition.default))
    }

    /// Create a resolution view while retaining all members for source lookup.
    #[must_use]
    pub fn with_workspace_groups(&self, groups: &[ResolvedWorkspaceGroup]) -> Self {
        let Some(requires_python) =
            RequiresPython::union(groups.iter().map(|group| &group.requires_python))
        else {
            return self.clone();
        };
        let mut roots = BTreeMap::<PackageName, MarkerTree>::new();
        let mut environments = MarkerTree::FALSE;
        for group in groups {
            environments = environments.or(group.environments);
            for member in &group.definition.members {
                let marker = roots.entry(member.clone()).or_insert(MarkerTree::FALSE);
                *marker = marker.or(group.environments);
            }
        }
        self.with_resolution(WorkspaceResolution {
            roots,
            requires_python,
            environments: SupportedEnvironments::from_markers(match self.environments() {
                Some(configured) if !configured.is_empty() => configured
                    .iter()
                    .map(|marker| marker.and(environments))
                    .filter(|marker| !marker.is_false())
                    .collect(),
                _ => vec![environments],
            }),
        })
    }

    /// Return the Python domain of a scoped or grouped workspace.
    pub fn workspace_group_requires_python(
        &self,
    ) -> Result<Option<RequiresPython>, WorkspaceError> {
        if let Some(requires_python) = self.resolution_requires_python() {
            return Ok(Some(requires_python.clone()));
        }
        let groups = self.workspace_groups()?;
        Ok(RequiresPython::union(
            groups.iter().map(|group| &group.requires_python),
        ))
    }
}
