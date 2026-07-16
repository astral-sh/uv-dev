use std::collections::btree_map::Entry;
use std::str::FromStr;
use std::{collections::BTreeMap, path::Path};

use rustc_hash::FxHashSet;
use thiserror::Error;

use uv_distribution_types::RequiresPython;
use uv_fs::Simplified;
use uv_normalize::{DEV_DEPENDENCIES, GroupName};
use uv_pep440::VersionSpecifiers;
use uv_pep508::Pep508Error;
use uv_pypi_types::{DependencyGroupSpecifier, VerbatimParsedUrl};

use crate::pyproject::{DependencyGroupSettings, PyProjectToml, ToolUvDependencyGroups};

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
        struct Frame<'data> {
            name: &'data GroupName,
            specifiers: std::slice::Iter<'data, DependencyGroupSpecifier>,
            requirements: Vec<uv_pep508::Requirement<VerbatimParsedUrl>>,
            requires_python_intersection: VersionSpecifiers,
        }

        impl<'data> Frame<'data> {
            fn new(name: &'data GroupName, specifiers: &'data [DependencyGroupSpecifier]) -> Self {
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
                self.requires_python_intersection =
                    std::mem::take(&mut self.requires_python_intersection)
                        .into_iter()
                        .chain(included.requires_python.clone().into_iter().flatten())
                        .collect();
            }
        }

        // Validate the settings.
        for (group_name, ..) in settings {
            if !groups.contains_key(group_name) {
                return Err(DependencyGroupErrorInner::SettingsGroupNotFound(
                    group_name.clone(),
                ));
            }
        }

        let mut resolved = BTreeMap::new();
        let mut visiting = FxHashSet::default();
        let mut stack = Vec::new();

        for (&name, specifiers) in groups {
            if resolved.contains_key(name) {
                continue;
            }

            visiting.insert(name);
            stack.push(Frame::new(name, specifiers));

            while !stack.is_empty() {
                let specifier = stack.last_mut().and_then(|frame| frame.specifiers.next());
                match specifier {
                    Some(DependencyGroupSpecifier::Requirement(requirement)) => {
                        match uv_pep508::Requirement::<VerbatimParsedUrl>::from_str(requirement) {
                            Ok(requirement) => {
                                if let Some(frame) = stack.last_mut() {
                                    frame.requirements.push(requirement);
                                }
                            }
                            Err(err) => {
                                let name = stack.last().expect("stack is not empty").name;
                                return Err(DependencyGroupErrorInner::GroupParseError(
                                    name.clone(),
                                    requirement.clone(),
                                    Box::new(err),
                                ));
                            }
                        }
                    }
                    Some(DependencyGroupSpecifier::IncludeGroup { include_group }) => {
                        if let Some(included) = resolved.get(include_group) {
                            if let Some(frame) = stack.last_mut() {
                                frame.include(included);
                            }
                            continue;
                        }

                        // Dependency group includes must not form cycles.
                        if visiting.contains(include_group) {
                            return Err(DependencyGroupErrorInner::DependencyGroupCycle(Cycle(
                                stack.iter().map(|frame| frame.name.clone()).collect(),
                            )));
                        }

                        let Some(specifiers) = groups.get(include_group) else {
                            let parent = stack.last().expect("stack is not empty").name;
                            return Err(DependencyGroupErrorInner::GroupNotFound(
                                include_group.clone(),
                                parent.clone(),
                            ));
                        };

                        visiting.insert(include_group);
                        stack.push(Frame::new(include_group, specifiers));
                    }
                    Some(DependencyGroupSpecifier::Object(map)) => {
                        let name = stack.last().expect("stack is not empty").name;
                        return Err(
                            DependencyGroupErrorInner::DependencyObjectSpecifierNotSupported(
                                name.clone(),
                                map.clone(),
                            ),
                        );
                    }
                    None => {
                        let mut frame = stack.pop().expect("stack is not empty");
                        visiting.remove(frame.name);

                        let empty_settings = DependencyGroupSettings::default();
                        let DependencyGroupSettings { requires_python } =
                            settings.get(frame.name).unwrap_or(&empty_settings);
                        if let Some(requires_python) = requires_python {
                            frame.requires_python_intersection = frame
                                .requires_python_intersection
                                .into_iter()
                                .chain(requires_python.clone())
                                .collect();

                            // Included requirements already have their own group markers applied.
                            for requirement in &mut frame.requirements {
                                let extra_markers =
                                    RequiresPython::from_specifiers(requires_python.clone())
                                        .to_marker_tree();
                                requirement.marker = requirement.marker.and(extra_markers);
                            }
                        }

                        let included = resolved.entry(frame.name.clone()).or_insert_with(|| {
                            FlatDependencyGroup {
                                requirements: frame.requirements,
                                requires_python: if frame.requires_python_intersection.is_empty() {
                                    None
                                } else {
                                    Some(frame.requires_python_intersection)
                                },
                            }
                        });
                        if let Some(parent) = stack.last_mut() {
                            parent.include(included);
                        }
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fmt::Write as _;
    use std::path::Path;

    use uv_normalize::{ExtraName, GroupName};
    use uv_pep508::MarkerTree;

    use super::FlatDependencyGroups;
    use crate::pyproject::PyProjectToml;

    fn flatten(input: &str) -> Result<FlatDependencyGroups, super::DependencyGroupError> {
        let pyproject = PyProjectToml::from_string(input.to_string(), "pyproject.toml")
            .expect("valid pyproject.toml");
        FlatDependencyGroups::from_pyproject_toml(Path::new("pyproject.toml"), &pyproject)
    }

    #[test]
    fn resolves_deep_dependency_group_chain() {
        let depth = 4096;
        let mut input = String::from("[dependency-groups]\n");
        for index in 0..depth {
            writeln!(
                input,
                "g-{index:04} = [{{ include-group = 'g-{:04}' }}]",
                index + 1
            )
            .expect("writing dependency groups into a string should succeed");
        }
        writeln!(input, "g-{depth:04} = ['leaf[extra]>=1']")
            .expect("writing dependency groups into a string should succeed");

        let groups = flatten(&input).expect("acyclic groups should resolve");
        let first = groups
            .get(&"g-0000".parse::<GroupName>().unwrap())
            .expect("first group should resolve");
        assert_eq!(first.requirements.len(), 1);
        assert_eq!(first.requirements[0].name.as_ref(), "leaf");
        assert_eq!(
            first.requirements[0].extras.as_ref(),
            ["extra".parse::<ExtraName>().unwrap()]
        );
    }

    #[test]
    fn preserves_dependency_group_cycle_path() {
        let error = flatten(
            r#"
            [dependency-groups]
            alpha = [{ include-group = "beta" }]
            beta = [{ include-group = "gamma" }]
            gamma = [{ include-group = "alpha" }]
            "#,
        )
        .expect_err("cyclic groups should fail");
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some(
                "Detected a cycle in `dependency-groups`: `alpha` -> `beta` -> `gamma` -> `alpha`"
            )
        );
    }

    #[test]
    fn preserves_missing_dependency_group_parent() {
        let error = flatten(
            r#"
            [dependency-groups]
            alpha = [{ include-group = "beta" }]
            beta = [{ include-group = "missing" }]
            "#,
        )
        .expect_err("missing groups should fail");
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("Failed to find group `missing` included by `beta`")
        );
    }

    #[test]
    fn preserves_shared_dependency_group_order_and_duplicates() {
        let groups = flatten(
            r#"
            [dependency-groups]
            main = [
              "first[extra]>=1; sys_platform == 'linux'",
              { include-group = "left" },
              "middle",
              { include-group = "right" },
              { include-group = "left" },
              "last",
            ]
            left = ["left", { include-group = "shared" }]
            right = [{ include-group = "shared" }, "right"]
            shared = ["shared; platform_machine == 'x86_64'"]

            [tool.uv.dependency-groups]
            left = { requires-python = ">=3.10" }
            shared = { requires-python = "<4" }
            "#,
        )
        .expect("shared groups should resolve");
        let main = groups
            .get(&"main".parse::<GroupName>().unwrap())
            .expect("main group should resolve");
        assert_eq!(
            main.requirements
                .iter()
                .map(|requirement| requirement.name.as_ref())
                .collect::<Vec<_>>(),
            [
                "first", "left", "shared", "middle", "shared", "right", "left", "shared", "last"
            ]
        );
        assert_eq!(
            main.requirements[0].extras.as_ref(),
            ["extra".parse::<ExtraName>().unwrap()]
        );
        assert_eq!(
            main.requirements[0].marker,
            "sys_platform == 'linux'".parse::<MarkerTree>().unwrap()
        );
        assert_eq!(
            main.requirements[1].marker,
            "python_full_version >= '3.10'"
                .parse::<MarkerTree>()
                .unwrap()
        );
        assert_eq!(
            main.requirements[2].marker,
            "platform_machine == 'x86_64' and python_full_version >= '3.10' and python_full_version < '4'"
                .parse::<MarkerTree>()
                .unwrap()
        );
        assert_eq!(
            main.requirements[4].marker,
            "platform_machine == 'x86_64' and python_full_version < '4'"
                .parse::<MarkerTree>()
                .unwrap()
        );
        assert_eq!(
            main.requires_python.as_ref().map(ToString::to_string),
            Some(">=3.10, >=3.10, <4, <4, <4".to_string())
        );
    }
}
