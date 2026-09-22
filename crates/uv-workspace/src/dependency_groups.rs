mod diagnostics;

use std::collections::btree_map::Entry;
use std::str::FromStr;
use std::{collections::BTreeMap, path::Path};

use thiserror::Error;

use uv_distribution_types::RequiresPython;
use uv_fs::Simplified;
use uv_normalize::{DEV_DEPENDENCIES, GroupName};
use uv_pep440::VersionSpecifiers;
use uv_pep508::Pep508Error;
use uv_pypi_types::{DependencyGroupSpecifier, VerbatimParsedUrl};

use crate::pyproject::{DependencyGroupSettings, PyProjectToml, ToolUvDependencyGroups};

use self::diagnostics::{DependencyGroupDiagnostic, DependencyGroupProvenance, GroupInclude};

pub use diagnostics::diagnostic_for_error;

/// PEP 735 dependency groups, with any `include-group` entries resolved.
#[derive(Debug, Default, Clone)]
pub struct FlatDependencyGroups(BTreeMap<GroupName, FlatDependencyGroup>);

#[derive(Debug, Default, Clone)]
pub struct FlatDependencyGroup {
    pub requirements: Vec<uv_pep508::Requirement<VerbatimParsedUrl>>,
    pub requires_python: Option<VersionSpecifiers>,
}

impl FlatDependencyGroups {
    /// Gather and flatten all the dependency-groups defined in the given pyproject.toml
    ///
    /// The path is the directory containing `pyproject.toml` and is only used in diagnostics.
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
                |failure| {
                    let DependencyGroupFailure { error, provenance } = failure;
                    let error = error.with_dev_dependencies(dev_dependencies);
                    let diagnostic = provenance
                        .and_then(|provenance| {
                            DependencyGroupDiagnostic::new(path, pyproject_toml, &error, provenance)
                        })
                        .map(Box::new);
                    DependencyGroupError {
                        package: pyproject_toml
                            .project
                            .as_ref()
                            .map(|project| project.name.to_string())
                            .unwrap_or_default(),
                        path: path.user_display().to_string(),
                        error,
                        diagnostic,
                    }
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
    ) -> Result<Self, DependencyGroupFailure> {
        fn resolve_group<'data>(
            resolved: &mut BTreeMap<GroupName, FlatDependencyGroup>,
            groups: &'data BTreeMap<&GroupName, &Vec<DependencyGroupSpecifier>>,
            settings: &BTreeMap<GroupName, DependencyGroupSettings>,
            name: &'data GroupName,
            parents: &mut Vec<&'data GroupName>,
            includes: &mut Vec<GroupInclude>,
        ) -> Result<(), DependencyGroupFailure> {
            let Some(specifiers) = groups.get(name) else {
                // Missing group
                let parent_name = parents
                    .iter()
                    .last()
                    .copied()
                    .expect("parent when group is missing");
                return Err(DependencyGroupFailure {
                    error: DependencyGroupErrorInner::GroupNotFound(
                        name.clone(),
                        parent_name.clone(),
                    ),
                    provenance: Some(DependencyGroupProvenance::Includes(includes.clone())),
                });
            };

            // "Dependency Group Includes MUST NOT include cycles, and tools SHOULD report an error if they detect a cycle."
            if let Some(start) = parents.iter().position(|parent| *parent == name) {
                // Earlier parents lead into the cycle but are not part of it.
                return Err(DependencyGroupFailure {
                    error: DependencyGroupErrorInner::DependencyGroupCycle(Cycle(
                        parents[start..].iter().copied().cloned().collect(),
                    )),
                    provenance: includes
                        .get(start..)
                        .map(|includes| DependencyGroupProvenance::Includes(includes.to_vec())),
                });
            }

            // If we already resolved this group, short-circuit.
            if resolved.contains_key(name) {
                return Ok(());
            }

            parents.push(name);
            let mut requirements = Vec::with_capacity(specifiers.len());
            let mut requires_python_intersection = VersionSpecifiers::empty();
            for (index, specifier) in specifiers.iter().enumerate() {
                match specifier {
                    DependencyGroupSpecifier::Requirement(requirement) => {
                        match uv_pep508::Requirement::<VerbatimParsedUrl>::from_str(requirement) {
                            Ok(requirement) => requirements.push(requirement),
                            Err(err) => {
                                return Err(DependencyGroupErrorInner::GroupParseError(
                                    name.clone(),
                                    requirement.clone(),
                                    Box::new(err),
                                )
                                .into());
                            }
                        }
                    }
                    DependencyGroupSpecifier::IncludeGroup { include_group } => {
                        includes.push(GroupInclude {
                            group: name.clone(),
                            index,
                            included: include_group.clone(),
                        });
                        resolve_group(
                            resolved,
                            groups,
                            settings,
                            include_group,
                            parents,
                            includes,
                        )?;
                        includes.pop();
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
                            )
                            .into(),
                        );
                    }
                }
            }

            let empty_settings = DependencyGroupSettings::default();
            let DependencyGroupSettings { requires_python } =
                settings.get(name).unwrap_or(&empty_settings);
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
                return Err(DependencyGroupFailure {
                    error: DependencyGroupErrorInner::SettingsGroupNotFound(group_name.clone()),
                    provenance: Some(DependencyGroupProvenance::Settings(group_name.clone())),
                });
            }
        }

        let mut resolved = BTreeMap::new();
        for name in groups.keys() {
            let mut parents = Vec::new();
            let mut includes = Vec::new();
            resolve_group(
                &mut resolved,
                groups,
                settings,
                name,
                &mut parents,
                &mut includes,
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
    diagnostic: Option<Box<DependencyGroupDiagnostic>>,
}

/// A semantic error and the traversal entries that produced it.
#[derive(Debug)]
struct DependencyGroupFailure {
    error: DependencyGroupErrorInner,
    provenance: Option<DependencyGroupProvenance>,
}

impl From<DependencyGroupErrorInner> for DependencyGroupFailure {
    fn from(error: DependencyGroupErrorInner) -> Self {
        Self {
            error,
            provenance: None,
        }
    }
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::Path;

    use anyhow::{Context, Result};

    use crate::pyproject::PyProjectToml;

    use super::FlatDependencyGroups;

    #[test]
    fn dependency_group_cycle_excludes_noncycle_parent() -> Result<()> {
        let pyproject = PyProjectToml::from_string(
            r#"
            [dependency-groups]
            first = [{ include-group = "inner-a" }]
            inner-a = [{ include-group = "inner-b" }]
            inner-b = [{ include-group = "inner-a" }]
            "#
            .to_string(),
            Path::new("pyproject.toml"),
        )?;
        let error = FlatDependencyGroups::from_pyproject_toml(Path::new("."), &pyproject)
            .err()
            .context("the dependency groups should contain a cycle")?;

        insta::assert_snapshot!(error.source().context("the cycle error should have a source")?.to_string(), @"Detected a cycle in `dependency-groups`: `inner-a` -> `inner-b` -> `inner-a`");
        Ok(())
    }
}
