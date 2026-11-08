use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::Path;
use std::str::FromStr;

use rustc_hash::FxHashSet;
use thiserror::Error;

use uv_distribution_types::RequiresPython;
use uv_fs::Simplified;
use uv_normalize::{DEV_DEPENDENCIES, GroupName};
use uv_pep440::VersionSpecifiers;
use uv_pep508::{Pep508Error, Requirement};
use uv_pypi_types::{DependencyGroupSpecifier, VerbatimParsedUrl};

use crate::pyproject::{DependencyGroupSettings, PyProjectToml, ToolUvDependencyGroups};

/// PEP 735 dependency groups, with any `include-group` entries resolved.
#[derive(Debug, Default, Clone)]
pub struct FlatDependencyGroups(BTreeMap<GroupName, FlatDependencyGroup>);

#[derive(Debug, Default, Clone)]
pub struct FlatDependencyGroup {
    pub requirements: Vec<Requirement<VerbatimParsedUrl>>,
    pub requires_python: Option<VersionSpecifiers>,
}

impl FlatDependencyGroups {
    /// Gather and flatten all the dependency-groups defined in the given pyproject.toml
    ///
    /// The path is only used in diagnostics.
    pub fn from_pyproject_toml(
        path: &Path,
        pyproject_toml: &PyProjectToml,
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
            .collect::<BTreeMap<_, _>>();

        // Get additional settings
        let empty_settings = ToolUvDependencyGroups::default();
        let group_settings = pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.dependency_groups.as_ref())
            .unwrap_or(&empty_settings);

        // Flatten the dependency groups.
        let mut dependency_groups =
            Self::from_dependency_groups(&dependency_groups, group_settings.inner()).map_err(
                |err| DependencyGroupError {
                    package: pyproject_toml
                        .project
                        .as_ref()
                        .map(|project| project.name.to_string())
                        .unwrap_or_default(),
                    path: path.user_display().to_string(),
                    error: err.with_dev_dependencies(dev_dependencies),
                },
            )?;

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
    ) -> Result<Self, DependencyGroupErrorInner> {
        // Validate the settings
        for (group_name, ..) in settings {
            if !groups.contains_key(group_name) {
                return Err(DependencyGroupErrorInner::SettingsGroupNotFound(
                    group_name.clone(),
                ));
            }
        }

        let mut resolved = BTreeMap::new();
        // Track suspended parents for efficient cycle detection.
        let mut active = FxHashSet::default();
        let mut parents = Vec::new();
        for (name, specifiers) in groups {
            if resolved.contains_key(*name) {
                continue;
            }
            let mut frame = GroupFrame::new(name, specifiers);
            loop {
                match frame.specifiers.next() {
                    Some(DependencyGroupSpecifier::Requirement(requirement)) => {
                        let requirement = Requirement::from_str(requirement).map_err(|err| {
                            DependencyGroupErrorInner::GroupParseError(
                                frame.name.clone(),
                                requirement.clone(),
                                Box::new(err),
                            )
                        })?;
                        frame.requirements.push(requirement);
                    }
                    Some(DependencyGroupSpecifier::IncludeGroup { include_group }) => {
                        if let Some(included) = resolved.get(include_group) {
                            frame.include(included);
                            continue;
                        }
                        let Some(specifiers) = groups.get(include_group) else {
                            return Err(DependencyGroupErrorInner::GroupNotFound(
                                include_group.clone(),
                                frame.name.clone(),
                            ));
                        };
                        active.insert(frame.name);
                        parents.push(frame);
                        if active.contains(include_group) {
                            return Err(DependencyGroupErrorInner::DependencyGroupCycle(Cycle(
                                parents.iter().map(|parent| parent.name.clone()).collect(),
                            )));
                        }
                        frame = GroupFrame::new(include_group, specifiers);
                    }
                    Some(DependencyGroupSpecifier::Object(map)) => {
                        return Err(
                            DependencyGroupErrorInner::DependencyObjectSpecifierNotSupported(
                                frame.name.clone(),
                                map.clone(),
                            ),
                        );
                    }
                    None => {
                        let name = frame.name;
                        let group = frame.finish(settings.get(name));
                        if let Some(parent) = parents.last_mut() {
                            parent.include(&group);
                        }
                        resolved.insert(name.clone(), group);
                        let Some(parent) = parents.pop() else {
                            break;
                        };
                        active.remove(parent.name);
                        frame = parent;
                    }
                }
            }
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

/// An in-progress dependency group in the depth-first traversal.
struct GroupFrame<'a> {
    name: &'a GroupName,
    specifiers: std::slice::Iter<'a, DependencyGroupSpecifier>,
    requirements: Vec<Requirement<VerbatimParsedUrl>>,
    requires_python_intersection: VersionSpecifiers,
}

impl<'a> GroupFrame<'a> {
    fn new(name: &'a GroupName, specifiers: &'a [DependencyGroupSpecifier]) -> Self {
        Self {
            name,
            specifiers: specifiers.iter(),
            requirements: Vec::with_capacity(specifiers.len()),
            requires_python_intersection: VersionSpecifiers::empty(),
        }
    }

    fn include(&mut self, included: &FlatDependencyGroup) {
        self.requirements
            .extend(included.requirements.iter().cloned());
        if let Some(requires_python) = &included.requires_python {
            self.requires_python_intersection =
                std::mem::take(&mut self.requires_python_intersection)
                    .into_iter()
                    .chain(requires_python.iter().cloned())
                    .collect();
        }
    }

    fn finish(mut self, settings: Option<&DependencyGroupSettings>) -> FlatDependencyGroup {
        if let Some(requires_python) =
            settings.and_then(|settings| settings.requires_python.as_ref())
        {
            self.requires_python_intersection = self
                .requires_python_intersection
                .into_iter()
                .chain(requires_python.clone())
                .collect();

            // Each included group already has its own requires-python marker applied.
            if !self.requirements.is_empty() {
                let extra_markers =
                    RequiresPython::from_specifiers(requires_python.clone()).to_marker_tree();
                for requirement in &mut self.requirements {
                    requirement.marker = requirement.marker.and(extra_markers);
                }
            }
        }

        FlatDependencyGroup {
            requirements: self.requirements,
            requires_python: (!self.requires_python_intersection.is_empty())
                .then_some(self.requires_python_intersection),
        }
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
