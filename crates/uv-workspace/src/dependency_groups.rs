use std::collections::btree_map::Entry;
use std::str::FromStr;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use thiserror::Error;

use uv_distribution_types::RequiresPython;
use uv_fs::Simplified;
use uv_normalize::{DEV_DEPENDENCIES, GroupName, PackageName};
use uv_pep440::VersionSpecifiers;
use uv_pep508::Pep508Error;
use uv_preview::PreviewFeature;
use uv_pypi_types::{DependencyGroupSpecifier, VerbatimParsedUrl};
use uv_warnings::warn_user_once;

use crate::Workspace;
use crate::pyproject::{
    DependencyGroupSettings, PyProjectToml, ToolUvDependencyGroups, WorkspaceGroupInclude,
};

/// PEP 735 dependency groups, with any `include-group` entries resolved.
#[derive(Debug, Default, Clone)]
pub struct FlatDependencyGroups(BTreeMap<GroupName, FlatDependencyGroup>);

#[derive(Debug, Default, Clone)]
pub struct FlatDependencyGroup {
    pub requirements: Vec<uv_pep508::Requirement<VerbatimParsedUrl>>,
    pub requires_python: Option<VersionSpecifiers>,
}

#[derive(Debug, Default)]
struct WorkspaceDependencyGroups {
    root: Option<FlatDependencyGroups>,
    packages: BTreeMap<PackageName, FlatDependencyGroups>,
}

impl FlatDependencyGroups {
    /// Gather and flatten all the dependency-groups defined in the given pyproject.toml
    ///
    /// The path is only used in diagnostics.
    pub(crate) fn from_pyproject_toml(
        path: &Path,
        pyproject_toml: &PyProjectToml,
    ) -> Result<Self, DependencyGroupError> {
        Self::from_pyproject_toml_with_workspace(path, pyproject_toml, None, None)
    }

    /// Gather and flatten dependency groups, including any referenced workspace groups.
    pub fn from_workspace(
        path: &Path,
        pyproject_toml: &PyProjectToml,
        workspace: &Workspace,
    ) -> Result<Self, DependencyGroupError> {
        Self::from_workspace_with_parents(path, pyproject_toml, workspace, None, &mut Vec::new())
    }

    fn from_workspace_with_parents(
        path: &Path,
        pyproject_toml: &PyProjectToml,
        workspace: &Workspace,
        requested_group: Option<&GroupName>,
        parents: &mut Vec<(PackageName, GroupName)>,
    ) -> Result<Self, DependencyGroupError> {
        let selected_groups = requested_group.map(|requested_group| {
            let mut selected_groups = BTreeSet::new();
            let mut pending_groups = vec![requested_group];
            while let Some(group) = pending_groups.pop() {
                if selected_groups.insert(group.clone())
                    && let Some(specifiers) = pyproject_toml
                        .dependency_groups
                        .as_ref()
                        .and_then(|groups| groups.get(group))
                {
                    pending_groups.extend(specifiers.iter().filter_map(|specifier| {
                        if let DependencyGroupSpecifier::IncludeGroup { include_group } = specifier
                        {
                            Some(include_group)
                        } else {
                            None
                        }
                    }));
                }
            }
            selected_groups
        });

        let includes_root_group = pyproject_toml
            .dependency_groups
            .as_ref()
            .is_some_and(|groups| {
                groups.into_iter().any(|(group, _)| {
                    selected_groups
                        .as_ref()
                        .is_none_or(|selected_groups| selected_groups.contains(group))
                        && pyproject_toml
                            .workspace_group_includes(group)
                            .any(|(package, _)| package.is_none())
                })
            });
        let root = (path != workspace.install_path() && includes_root_group)
            .then(|| {
                Self::from_pyproject_toml(workspace.install_path(), workspace.pyproject_toml())
            })
            .transpose()?;

        let mut packages: BTreeMap<PackageName, Self> = BTreeMap::new();
        if let Some(groups) = &pyproject_toml.dependency_groups {
            for (group, _) in groups {
                if selected_groups
                    .as_ref()
                    .is_some_and(|selected_groups| !selected_groups.contains(group))
                {
                    continue;
                }

                let current = pyproject_toml
                    .project
                    .as_ref()
                    .map(|project| (project.name.clone(), group.clone()));
                if let Some(current) = &current {
                    if parents.contains(current) {
                        let cycle = parents
                            .iter()
                            .chain(std::iter::once(current))
                            .map(|(package, group)| format!("{package}:{group}"))
                            .collect::<Vec<_>>()
                            .join(" -> ");
                        return Err(DependencyGroupError {
                            package: current.0.to_string(),
                            path: path.user_display().to_string(),
                            error: DependencyGroupErrorInner::WorkspaceGroupCycle(cycle),
                        });
                    }
                    parents.push(current.clone());
                }

                for (package, included_group) in pyproject_toml.workspace_group_includes(group) {
                    let Some(package) = package else {
                        continue;
                    };
                    if packages
                        .get(package)
                        .is_some_and(|groups| groups.get(included_group).is_some())
                    {
                        continue;
                    }

                    let member =
                        workspace
                            .packages()
                            .get(package)
                            .ok_or_else(|| DependencyGroupError {
                                package: pyproject_toml
                                    .project
                                    .as_ref()
                                    .map(|project| project.name.to_string())
                                    .unwrap_or_default(),
                                path: path.user_display().to_string(),
                                error: DependencyGroupErrorInner::WorkspacePackageNotFound(
                                    package.clone(),
                                    group.clone(),
                                ),
                            })?;
                    let included = Self::from_workspace_with_parents(
                        member.root(),
                        member.pyproject_toml(),
                        workspace,
                        Some(included_group),
                        parents,
                    )?;
                    packages
                        .entry(package.clone())
                        .or_default()
                        .0
                        .extend(included.0);
                }

                if current.is_some() {
                    parents.pop();
                }
            }
        }

        let workspace_groups = WorkspaceDependencyGroups { root, packages };
        Self::from_pyproject_toml_with_workspace(
            path,
            pyproject_toml,
            Some(&workspace_groups),
            selected_groups.as_ref(),
        )
    }

    fn from_pyproject_toml_with_workspace(
        path: &Path,
        pyproject_toml: &PyProjectToml,
        workspace_groups: Option<&WorkspaceDependencyGroups>,
        selected_groups: Option<&BTreeSet<GroupName>>,
    ) -> Result<Self, DependencyGroupError> {
        // First, collect `tool.uv.dev_dependencies`
        let dev_dependencies = pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.dev_dependencies.as_ref());

        // Then, collect `dependency-groups`
        let dependency_groups = pyproject_toml
            .dependency_groups
            .iter()
            .flatten()
            .filter(|(group, _)| {
                selected_groups.is_none_or(|selected_groups| selected_groups.contains(group))
            })
            .collect::<BTreeMap<_, _>>();

        // Get additional settings
        let empty_settings = ToolUvDependencyGroups::default();
        let group_settings = pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.dependency_groups.as_ref())
            .unwrap_or(&empty_settings);
        let selected_settings = selected_groups.map(|selected_groups| {
            group_settings
                .inner()
                .iter()
                .filter(|(group, _)| selected_groups.contains(*group))
                .map(|(group, settings)| (group.clone(), settings.clone()))
                .collect::<BTreeMap<_, _>>()
        });

        // Flatten the dependency groups.
        let mut dependency_groups = Self::from_dependency_groups(
            &dependency_groups,
            selected_settings.as_ref().unwrap_or(group_settings.inner()),
            workspace_groups,
        )
        .map_err(|err| DependencyGroupError {
            package: pyproject_toml
                .project
                .as_ref()
                .map(|project| project.name.to_string())
                .unwrap_or_default(),
            path: path.user_display().to_string(),
            error: err.with_dev_dependencies(dev_dependencies),
        })?;

        // Add the `dev` group, if the legacy `dev-dependencies` is defined.
        //
        // NOTE: the fact that we do this out here means that nothing can inherit from
        // the legacy dev-dependencies group (or define a group requires-python for it).
        // This is intentional, we want groups to be defined in a standard interoperable
        // way, and letting things include-group a group that isn't defined would be a
        // mess for other python tools.
        if let Some(dev_dependencies) = dev_dependencies {
            dependency_groups
                .entry(DEV_DEPENDENCIES.clone())
                .or_insert_with(FlatDependencyGroup::default)
                .requirements
                .extend(dev_dependencies.clone());
        }

        Ok(dependency_groups)
    }

    /// Resolve the dependency groups (which may contain references to other groups) into concrete
    /// lists of requirements.
    fn from_dependency_groups(
        groups: &BTreeMap<&GroupName, &Vec<DependencyGroupSpecifier>>,
        settings: &BTreeMap<GroupName, DependencyGroupSettings>,
        workspace_groups: Option<&WorkspaceDependencyGroups>,
    ) -> Result<Self, DependencyGroupErrorInner> {
        fn resolve_group<'data>(
            resolved: &mut BTreeMap<GroupName, FlatDependencyGroup>,
            groups: &'data BTreeMap<&GroupName, &Vec<DependencyGroupSpecifier>>,
            settings: &BTreeMap<GroupName, DependencyGroupSettings>,
            workspace_groups: Option<&WorkspaceDependencyGroups>,
            name: &'data GroupName,
            parents: &mut Vec<&'data GroupName>,
        ) -> Result<(), DependencyGroupErrorInner> {
            let Some(specifiers) = groups.get(name) else {
                // Missing group
                let parent_name = parents
                    .iter()
                    .last()
                    .copied()
                    .expect("parent when group is missing");
                return Err(DependencyGroupErrorInner::GroupNotFound(
                    name.clone(),
                    parent_name.clone(),
                ));
            };

            // "Dependency Group Includes MUST NOT include cycles, and tools SHOULD report an error if they detect a cycle."
            if parents.contains(&name) {
                return Err(DependencyGroupErrorInner::DependencyGroupCycle(Cycle(
                    parents.iter().copied().cloned().collect(),
                )));
            }

            // If we already resolved this group, short-circuit.
            if resolved.contains_key(name) {
                return Ok(());
            }

            parents.push(name);
            let mut requirements = Vec::with_capacity(specifiers.len());
            let mut requires_python_intersection = VersionSpecifiers::empty();
            for specifier in *specifiers {
                match specifier {
                    DependencyGroupSpecifier::Requirement(requirement) => {
                        match uv_pep508::Requirement::<VerbatimParsedUrl>::from_str(requirement) {
                            Ok(requirement) => requirements.push(requirement),
                            Err(err) => {
                                return Err(DependencyGroupErrorInner::GroupParseError(
                                    name.clone(),
                                    requirement.clone(),
                                    Box::new(err),
                                ));
                            }
                        }
                    }
                    DependencyGroupSpecifier::IncludeGroup { include_group } => {
                        resolve_group(
                            resolved,
                            groups,
                            settings,
                            workspace_groups,
                            include_group,
                            parents,
                        )?;
                        if let Some(included) = resolved.get(include_group) {
                            requirements.extend(included.requirements.iter().cloned());

                            // Intersect the requires-python for this group with the included group's
                            requires_python_intersection = requires_python_intersection
                                .into_iter()
                                .chain(included.requires_python.clone().into_iter().flatten())
                                .collect();
                        }
                    }
                    DependencyGroupSpecifier::Object(map) => {
                        return Err(
                            DependencyGroupErrorInner::DependencyObjectSpecifierNotSupported(
                                name.clone(),
                                map.clone(),
                            ),
                        );
                    }
                }
            }

            let empty_settings = DependencyGroupSettings::default();
            let DependencyGroupSettings {
                requires_python,
                include_workspace_groups,
            } = settings.get(name).unwrap_or(&empty_settings);

            for include in include_workspace_groups {
                let included = match include {
                    WorkspaceGroupInclude::Root(workspace_group) => {
                        let workspace_groups = workspace_groups
                            .and_then(|groups| groups.root.as_ref())
                            .ok_or_else(|| {
                                DependencyGroupErrorInner::WorkspaceGroupOutsideWorkspace(
                                    workspace_group.clone(),
                                    name.clone(),
                                )
                            })?;
                        workspace_groups.get(workspace_group).ok_or_else(|| {
                            DependencyGroupErrorInner::WorkspaceGroupNotFound(
                                workspace_group.clone(),
                                name.clone(),
                            )
                        })?
                    }
                    WorkspaceGroupInclude::Package(include) => {
                        let workspace_groups = workspace_groups
                            .and_then(|groups| groups.packages.get(&include.package))
                            .ok_or_else(|| {
                                DependencyGroupErrorInner::WorkspacePackageNotFound(
                                    include.package.clone(),
                                    name.clone(),
                                )
                            })?;
                        workspace_groups.get(&include.group).ok_or_else(|| {
                            DependencyGroupErrorInner::WorkspacePackageGroupNotFound(
                                include.package.clone(),
                                include.group.clone(),
                                name.clone(),
                            )
                        })?
                    }
                };

                if !uv_preview::is_enabled(PreviewFeature::IncludeGroupWorkspace) {
                    warn_user_once!(
                        "Including workspace dependency groups (`[tool.uv.dependency-groups]` with `include-workspace-groups = [...]`) is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
                        PreviewFeature::IncludeGroupWorkspace
                    );
                }

                requirements.extend(included.requirements.iter().cloned());
                requires_python_intersection = requires_python_intersection
                    .into_iter()
                    .chain(included.requires_python.clone().into_iter().flatten())
                    .collect();
            }

            if let Some(requires_python) = requires_python {
                // Intersect the requires-python for this group to get the final requires-python
                // that will be used by interpreter discovery and checking.
                requires_python_intersection = requires_python_intersection
                    .into_iter()
                    .chain(requires_python.clone())
                    .collect();

                // Add the group requires-python as a marker to each requirement
                // We don't use `requires_python_intersection` because each `include-group`
                // should already have its markers applied to these.
                for requirement in &mut requirements {
                    let extra_markers =
                        RequiresPython::from_specifiers(requires_python.clone()).to_marker_tree();
                    requirement.marker = requirement.marker.and(extra_markers);
                }
            }

            parents.pop();

            resolved.insert(
                name.clone(),
                FlatDependencyGroup {
                    requirements,
                    requires_python: if requires_python_intersection.is_empty() {
                        None
                    } else {
                        Some(requires_python_intersection)
                    },
                },
            );
            Ok(())
        }

        // Validate the settings
        for (group_name, ..) in settings {
            if !groups.contains_key(group_name) {
                return Err(DependencyGroupErrorInner::SettingsGroupNotFound(
                    group_name.clone(),
                ));
            }
        }

        let mut resolved = BTreeMap::new();
        for name in groups.keys() {
            let mut parents = Vec::new();
            resolve_group(
                &mut resolved,
                groups,
                settings,
                workspace_groups,
                name,
                &mut parents,
            )?;
        }
        Ok(Self(resolved))
    }

    /// Return the requirements for a given group, if any.
    pub fn get(&self, group: &GroupName) -> Option<&FlatDependencyGroup> {
        self.0.get(group)
    }

    /// Return the entry for a given group, if any.
    fn entry(&mut self, group: GroupName) -> Entry<'_, GroupName, FlatDependencyGroup> {
        self.0.entry(group)
    }

    /// Consume the [`FlatDependencyGroups`] and return the inner map.
    pub(crate) fn into_inner(self) -> BTreeMap<GroupName, FlatDependencyGroup> {
        self.0
    }
}

impl FromIterator<(GroupName, FlatDependencyGroup)> for FlatDependencyGroups {
    fn from_iter<T: IntoIterator<Item = (GroupName, FlatDependencyGroup)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for FlatDependencyGroups {
    type Item = (GroupName, FlatDependencyGroup);
    type IntoIter = std::collections::btree_map::IntoIter<GroupName, FlatDependencyGroup>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Debug, Error)]
#[error("{} has malformed dependency groups", if path.is_empty() && package.is_empty() {
    "Project".to_string()
} else if path.is_empty() || path == "." {
    format!("Project `{package}`")
} else if package.is_empty() {
    format!("`{path}`")
} else {
    format!("Project `{package} @ {path}`")
})]
pub struct DependencyGroupError {
    package: String,
    path: String,
    #[source]
    error: DependencyGroupErrorInner,
}

#[derive(Debug, Error)]
enum DependencyGroupErrorInner {
    #[error("Failed to parse entry in group `{0}`: `{1}`")]
    GroupParseError(
        GroupName,
        String,
        #[source] Box<Pep508Error<VerbatimParsedUrl>>,
    ),
    #[error("Failed to find group `{0}` included by `{1}`")]
    GroupNotFound(GroupName, GroupName),
    #[error("Failed to find workspace group `{0}` included by `{1}`")]
    WorkspaceGroupNotFound(GroupName, GroupName),
    #[error("Failed to find workspace package `{0}` included by `{1}`")]
    WorkspacePackageNotFound(PackageName, GroupName),
    #[error("Failed to find group `{1}` in workspace package `{0}` included by `{2}`")]
    WorkspacePackageGroupNotFound(PackageName, GroupName, GroupName),
    #[error(
        "Group `{1}` includes workspace group `{0}`, but this project is not a workspace member"
    )]
    WorkspaceGroupOutsideWorkspace(GroupName, GroupName),
    #[error("Detected a cycle in workspace dependency groups: {0}")]
    WorkspaceGroupCycle(String),
    #[error(
        "Group `{0}` includes the `dev` group (`include = \"dev\"`), but only `tool.uv.dev-dependencies` was found. To reference the `dev` group via an `include`, remove the `tool.uv.dev-dependencies` section and add any development dependencies to the `dev` entry in the `[dependency-groups]` table instead."
    )]
    DevGroupInclude(GroupName),
    #[error("Detected a cycle in `dependency-groups`: {0}")]
    DependencyGroupCycle(Cycle),
    #[error("Group `{0}` contains an unknown dependency object specifier: {1:?}")]
    DependencyObjectSpecifierNotSupported(GroupName, BTreeMap<String, String>),
    #[error("Failed to find group `{0}` specified in `[tool.uv.dependency-groups]`")]
    SettingsGroupNotFound(GroupName),
    #[error(
        "`[tool.uv.dependency-groups]` specifies the `dev` group, but only `tool.uv.dev-dependencies` was found. To reference the `dev` group, remove the `tool.uv.dev-dependencies` section and add any development dependencies to the `dev` entry in the `[dependency-groups]` table instead."
    )]
    SettingsDevGroupInclude,
}

impl DependencyGroupErrorInner {
    /// Enrich a [`DependencyGroupError`] with the `tool.uv.dev-dependencies` metadata, if applicable.
    #[must_use]
    fn with_dev_dependencies(
        self,
        dev_dependencies: Option<&Vec<uv_pep508::Requirement<VerbatimParsedUrl>>>,
    ) -> Self {
        match self {
            Self::GroupNotFound(group, parent)
                if dev_dependencies.is_some() && group == *DEV_DEPENDENCIES =>
            {
                Self::DevGroupInclude(parent)
            }
            Self::SettingsGroupNotFound(group)
                if dev_dependencies.is_some() && group == *DEV_DEPENDENCIES =>
            {
                Self::SettingsDevGroupInclude
            }
            _ => self,
        }
    }
}

/// A cycle in the `dependency-groups` table.
#[derive(Debug)]
struct Cycle(Vec<GroupName>);

/// Display a cycle, e.g., `a -> b -> c -> a`.
impl std::fmt::Display for Cycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [first, rest @ ..] = self.0.as_slice() else {
            return Ok(());
        };
        write!(f, "`{first}`")?;
        for group in rest {
            write!(f, " -> `{group}`")?;
        }
        write!(f, " -> `{first}`")?;
        Ok(())
    }
}
