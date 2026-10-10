use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::slice;
use std::str::FromStr;

use uv_configuration::{
    DependencyModifierScope, DependencyModifiers, Excludes, NoSources, Override, Overrides,
    PackageOverride,
};
use uv_distribution_types::{Requirement, RequirementSource, RequiresPython};
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::VersionSpecifiers;
use uv_pep508::{MarkerTree, Requirement as Pep508Requirement, VerbatimUrl};
use uv_pypi_types::{LenientRequirement, SupportedEnvironments, VerbatimParsedUrl};

use crate::pyproject::{Source, ToolUvSources, WorkspaceReference};
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
    definition: WorkspaceGroup,
    requires_python: RequiresPython,
    /// The supported environments, including conditional local-member Python bounds.
    environments: MarkerTree,
    /// The environments in which each local member is reachable from these roots.
    member_environments: BTreeMap<PackageName, MarkerTree>,
}

impl ResolvedWorkspaceGroup {
    pub fn definition(&self) -> &WorkspaceGroup {
        &self.definition
    }

    pub fn requires_python(&self) -> &RequiresPython {
        &self.requires_python
    }

    pub fn environments(&self) -> MarkerTree {
        self.environments
    }

    pub fn member_environments(&self) -> &BTreeMap<PackageName, MarkerTree> {
        &self.member_environments
    }

    /// Consume the validated group for storage or command selection.
    pub fn into_parts(self) -> (WorkspaceGroup, RequiresPython, MarkerTree) {
        (self.definition, self.requires_python, self.environments)
    }

    /// Restrict every representation of the group's effective domain together.
    pub fn narrow_environment(&mut self, marker: MarkerTree) -> Result<(), WorkspaceError> {
        let environments = self.environments.and(marker);
        let requires_python = RequiresPython::from_marker_tree(environments).ok_or_else(|| {
            WorkspaceError::from(WorkspaceErrorKind::DisjointWorkspaceGroupPython(
                self.definition.name.clone(),
            ))
        })?;
        self.environments = environments;
        self.requires_python = requires_python;
        for active in self.member_environments.values_mut() {
            *active = active.and(environments);
        }
        Ok(())
    }
}

/// The roots and Python domain of one shared resolution attempt.
#[derive(Debug, Clone)]
pub struct WorkspaceResolution {
    pub roots: BTreeMap<PackageName, MarkerTree>,
    pub requires_python: RequiresPython,
    pub environments: SupportedEnvironments,
}

impl Workspace {
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
        let overrides = self
            .overrides()
            .into_iter()
            .flat_map(|entry| match entry {
                Override::Requirement(requirement) => self
                    .lower_workspace_sources(
                        requirement.into(),
                        None,
                        self.install_path(),
                        None,
                        no_sources,
                    )
                    .into_iter()
                    .map(Override::Requirement)
                    .collect::<Vec<_>>(),
                Override::Package(package) => vec![Override::Package(PackageOverride {
                    package: package.package,
                    dependencies: package
                        .dependencies
                        .into_vec()
                        .into_iter()
                        .flat_map(|requirement| {
                            self.lower_workspace_sources(
                                requirement.into(),
                                None,
                                self.install_path(),
                                None,
                                no_sources,
                            )
                        })
                        .collect(),
                })],
            })
            .collect();
        let modifiers = DependencyModifiers::new(
            Overrides::from_entries(overrides).map_err(|error| {
                WorkspaceError::from(WorkspaceErrorKind::WorkspaceGroupModifiers(error))
            })?,
            Excludes::from_entries(self.exclude_dependencies()),
        );
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
            let member_environments =
                self.reachable_workspace_members(definition, no_sources, &modifiers)?;
            for (name, active) in &member_environments {
                if let Some(requires_python) = self
                    .packages()
                    .get(name)
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
                member_environments: member_environments
                    .into_iter()
                    .map(|(name, marker)| (name, marker.and(environments)))
                    .collect(),
            });
        }
        Ok(groups)
    }

    /// Follow production dependencies to local members, retaining their activation markers.
    fn reachable_workspace_members(
        &self,
        group: &WorkspaceGroup,
        no_sources: &NoSources,
        modifiers: &DependencyModifiers,
    ) -> Result<BTreeMap<PackageName, MarkerTree>, WorkspaceError> {
        let mut reached = BTreeMap::<PackageName, MarkerTree>::new();
        let mut processed = BTreeMap::new();
        let mut pending = group
            .members
            .iter()
            .cloned()
            .map(|name| (name, None::<ExtraName>, MarkerTree::TRUE))
            .collect::<Vec<_>>();
        while let Some((name, extra, marker)) = pending.pop() {
            let previous = processed
                .entry((name.clone(), extra.clone()))
                .or_insert(MarkerTree::FALSE);
            let active = previous.or(marker);
            if active == *previous {
                continue;
            }
            *previous = active;
            reached
                .entry(name.clone())
                .and_modify(|reached| *reached = reached.or(active))
                .or_insert(active);
            let Some(member) = self.packages().get(&name) else {
                continue;
            };
            let member_sources = member
                .pyproject_toml()
                .tool
                .as_ref()
                .and_then(|tool| tool.uv.as_ref())
                .and_then(|uv| uv.sources.as_ref());
            let dependencies = if let Some(extra) = &extra {
                member
                    .project()
                    .optional_dependencies
                    .as_ref()
                    .and_then(|dependencies| dependencies.get(extra))
            } else {
                member.project().dependencies.as_ref()
            };
            let requirements = dependencies
                .into_iter()
                .flatten()
                .map(|dependency| {
                    LenientRequirement::<VerbatimParsedUrl>::from_str(dependency)
                        .map(Pep508Requirement::from)
                        .map(Requirement::from)
                        .map_err(|error| {
                            WorkspaceError::from(
                                WorkspaceErrorKind::InvalidWorkspaceGroupDependency(
                                    group.name.clone(),
                                    name.clone(),
                                    Box::new(error),
                                ),
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let requirements = requirements
                .into_iter()
                .flat_map(|requirement| {
                    self.lower_workspace_sources(
                        requirement,
                        member_sources,
                        member.root(),
                        extra.as_ref(),
                        no_sources,
                    )
                })
                .collect::<Vec<_>>();
            let scope = member
                .project()
                .version
                .as_ref()
                .map_or(DependencyModifierScope::Global, |version| {
                    DependencyModifierScope::Package(&name, version)
                });
            for requirement in modifiers.apply(scope, &requirements) {
                let marker = match extra.as_ref() {
                    Some(extra) => requirement
                        .marker
                        .simplify_extras(slice::from_ref(extra))
                        .simplify_not_extras_with(|candidate| candidate != extra),
                    None => requirement.marker.simplify_not_extras_with(|_| true),
                };
                if requirement.name == name {
                    let marker = active.and(marker);
                    if !marker.is_false() {
                        pending.extend(
                            requirement
                                .extras
                                .iter()
                                .cloned()
                                .map(|extra| (name.clone(), Some(extra), marker)),
                        );
                    }
                    continue;
                }
                let Some(target) = self.packages().get(&requirement.name) else {
                    continue;
                };
                let RequirementSource::Directory { install_path, .. } = &requirement.source else {
                    continue;
                };
                if uv_fs::normalize_path(install_path.as_ref()) != *target.root() {
                    continue;
                }
                let local = active.and(marker);
                if !local.is_false() {
                    pending.push((requirement.name.clone(), None, local));
                    pending.extend(
                        requirement
                            .extras
                            .iter()
                            .cloned()
                            .map(|extra| (requirement.name.clone(), Some(extra), local)),
                    );
                }
            }
        }
        Ok(reached)
    }

    /// Resolve workspace identities in their declaring source context before applying overrides.
    /// Other sources retain their requirement names so dependency modifiers can still replace them.
    fn lower_workspace_sources(
        &self,
        requirement: Requirement,
        member_sources: Option<&ToolUvSources>,
        member_root: &Path,
        extra: Option<&ExtraName>,
        no_sources: &NoSources,
    ) -> Vec<Requirement> {
        if requirement.marker.is_false() {
            return vec![requirement];
        }
        let Some(target) = self.packages().get(&requirement.name) else {
            return vec![requirement];
        };
        let sources = if no_sources.for_package(&requirement.name) {
            None
        } else {
            member_sources
                .and_then(|sources| sources.inner().get(&requirement.name))
                .map(|sources| (sources, member_root))
                .or_else(|| {
                    self.sources()
                        .get(&requirement.name)
                        .map(|sources| (sources, self.install_path().as_path()))
                })
        };
        let mut remaining = requirement.marker;
        let mut local = MarkerTree::FALSE;
        if let Some((sources, base)) = sources {
            for source in sources.iter() {
                if source.group().is_some()
                    || source.extra().is_some_and(|name| extra != Some(name))
                {
                    continue;
                }
                remaining = remaining.and(source.marker().negate());
                let is_local = match source {
                    Source::Workspace {
                        workspace: WorkspaceReference::Bool(true),
                        ..
                    } => true,
                    Source::Path { path, .. } => {
                        uv_fs::normalize_path(base.join(path.as_ref())) == *target.root()
                    }
                    Source::Git { .. }
                    | Source::Url { .. }
                    | Source::Registry { .. }
                    | Source::Workspace {
                        workspace: WorkspaceReference::Bool(false) | WorkspaceReference::Path(_),
                        ..
                    } => false,
                };
                if is_local {
                    local = local.or(requirement.marker.and(source.marker()));
                }
            }
        }
        let is_local_directory =
            if let RequirementSource::Directory { install_path, .. } = &requirement.source {
                uv_fs::normalize_path(install_path.as_ref()) == *target.root()
            } else {
                false
            };
        if is_local_directory {
            local = local.or(remaining);
        }

        let mut lowered = Vec::new();
        if !local.is_false() {
            lowered.push(Requirement {
                marker: local,
                source: RequirementSource::Directory {
                    install_path: target.root().clone().into_boxed_path(),
                    editable: None,
                    r#virtual: None,
                    url: VerbatimUrl::from_absolute_path(target.root())
                        .expect("workspace path is a valid URL"),
                },
                ..requirement.clone()
            });
        }
        let non_local = requirement.marker.and(local.negate());
        if !non_local.is_false() {
            // A configured external source can replace a direct workspace path. This reachability
            // pass needs its name for overrides, but must not follow the original directory.
            lowered.push(Requirement {
                marker: non_local,
                source: if is_local_directory {
                    RequirementSource::Registry {
                        specifier: VersionSpecifiers::empty(),
                        index: None,
                        conflict: None,
                    }
                } else {
                    requirement.source.clone()
                },
                ..requirement
            });
        }
        lowered
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
        sources: &NoSources,
    ) -> Result<Option<RequiresPython>, WorkspaceError> {
        if let Some(requires_python) = self.resolution_requires_python() {
            return Ok(Some(requires_python.clone()));
        }
        let groups = self.workspace_groups_with_sources(sources)?;
        Ok(RequiresPython::union(
            groups.iter().map(|group| &group.requires_python),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ResolvedWorkspaceGroup, WorkspaceGroup};
    use std::collections::{BTreeMap, BTreeSet};
    use uv_distribution_types::RequiresPython;
    use uv_normalize::PackageName;
    use uv_pep508::MarkerTree;

    #[test]
    fn narrowing_keeps_all_group_domains_consistent() -> Result<(), Box<dyn std::error::Error>> {
        let member: PackageName = "app".parse()?;
        let original: MarkerTree = "python_full_version >= '3.12'".parse()?;
        let active: MarkerTree =
            "python_full_version >= '3.12' and sys_platform == 'linux'".parse()?;
        let mut group = ResolvedWorkspaceGroup {
            definition: WorkspaceGroup {
                name: "main".parse()?,
                members: BTreeSet::from([member.clone()]),
                requires_python: None,
                default: false,
            },
            requires_python: RequiresPython::from_specifiers(">=3.12".parse()?),
            environments: original,
            member_environments: BTreeMap::from([(member.clone(), active)]),
        };
        let narrowed: MarkerTree = "python_full_version >= '3.13'".parse()?;
        group.narrow_environment(narrowed)?;
        assert_eq!(group.environments(), narrowed);
        assert_eq!(group.requires_python().to_exact_marker_tree(), narrowed);
        assert_eq!(group.member_environments()[&member], active.and(narrowed));

        assert!(group.narrow_environment(MarkerTree::FALSE).is_err());
        assert_eq!(group.environments(), narrowed);
        assert_eq!(group.requires_python().to_exact_marker_tree(), narrowed);
        assert_eq!(group.member_environments()[&member], active.and(narrowed));
        Ok(())
    }
}
