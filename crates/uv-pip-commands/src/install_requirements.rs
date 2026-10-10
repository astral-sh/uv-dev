use std::collections::BTreeMap;
use std::path::PathBuf;

use uv_configuration::{
    DependencyGroups, ExcludeDependency, NoBinary, NoBuild, Override, RequirementsInput,
};
use uv_distribution_types::{
    IndexUrl, NameRequirementSpecification, Requirement, UnresolvedRequirementSpecification,
};
use uv_normalize::PackageName;
use uv_requirements::{RequirementsSpecification, SourceTree};

/// Parsed installation inputs, with shared index and build policy separate from the source mode.
pub(crate) struct InstallRequirements {
    pub(crate) source: InstallSource,
    pub(crate) excludes: Vec<ExcludeDependency>,
    pub(crate) index_url: Option<IndexUrl>,
    pub(crate) extra_index_urls: Vec<IndexUrl>,
    pub(crate) no_index: bool,
    pub(crate) require_hashes: bool,
    pub(crate) find_links: Vec<IndexUrl>,
    pub(crate) no_binary: NoBinary,
    pub(crate) no_build: NoBuild,
}

pub(crate) enum InstallSource {
    Resolve(ResolveRequirements),
    Pylock {
        input: RequirementsInput,
        groups: DependencyGroups,
    },
}

impl InstallSource {
    pub(crate) fn is_locked(&self) -> bool {
        match self {
            Self::Resolve(_) => false,
            Self::Pylock { .. } => true,
        }
    }

    /// A lockfile is an input even when it contains no packages.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::Resolve(requirements) => {
                requirements.requirements.is_empty() && requirements.source_trees.is_empty()
            }
            Self::Pylock { .. } => false,
        }
    }
}

pub(crate) struct ResolveRequirements {
    pub(crate) project: Option<PackageName>,
    pub(crate) requirements: Vec<UnresolvedRequirementSpecification>,
    pub(crate) constraints: Vec<NameRequirementSpecification>,
    pub(crate) overrides: Vec<UnresolvedRequirementSpecification>,
    pub(crate) override_dependencies: Vec<Override<Requirement>>,
    pub(crate) source_trees: Vec<SourceTree>,
    pub(crate) groups: BTreeMap<PathBuf, DependencyGroups>,
}

impl TryFrom<RequirementsSpecification> for InstallRequirements {
    type Error = anyhow::Error;

    fn try_from(specification: RequirementsSpecification) -> Result<Self, Self::Error> {
        let RequirementsSpecification {
            project,
            requirements,
            constraints,
            overrides,
            override_dependencies,
            excludes,
            pylock,
            pylock_groups,
            source_trees,
            groups,
            extras: _,
            index_url,
            extra_index_urls,
            no_index,
            require_hashes,
            find_links,
            no_binary,
            no_build,
        } = specification;

        let source = if let Some(input) = pylock {
            if !requirements.is_empty() || !source_trees.is_empty() {
                anyhow::bail!(
                    "Cannot specify additional requirements alongside a `pylock.toml` file"
                );
            }
            if !constraints.is_empty() {
                anyhow::bail!("Cannot specify constraints with a `pylock.toml` file");
            }
            if !overrides.is_empty() || !override_dependencies.is_empty() {
                anyhow::bail!("Cannot specify overrides with a `pylock.toml` file");
            }
            if !groups.is_empty() {
                anyhow::bail!(
                    "Cannot specify paths for groups with a `pylock.toml` file; all groups must refer to the `pylock.toml` file"
                );
            }
            InstallSource::Pylock {
                input,
                groups: pylock_groups,
            }
        } else {
            InstallSource::Resolve(ResolveRequirements {
                project,
                requirements,
                constraints,
                overrides,
                override_dependencies,
                source_trees,
                groups,
            })
        };
        Ok(Self {
            source,
            excludes,
            index_url,
            extra_index_urls,
            no_index,
            require_hashes,
            find_links,
            no_binary,
            no_build,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use uv_configuration::{DependencyGroups, ExcludeDependency, RequirementsInput};
    use uv_distribution_types::Requirement;
    use uv_normalize::GroupName;
    use uv_requirements::{RequirementsSpecification, SourceTree};

    use super::{InstallRequirements, InstallSource};

    #[test]
    fn locked_input_keeps_groups_and_common_options() -> anyhow::Result<()> {
        let group: GroupName = "dev".parse()?;
        let path = PathBuf::from("pylock.toml");
        let input = InstallRequirements::try_from(RequirementsSpecification {
            pylock: Some(RequirementsInput::Local(path.clone())),
            pylock_groups: DependencyGroups::from_group(group.clone()),
            no_index: true,
            require_hashes: true,
            excludes: vec![ExcludeDependency::Dependency("excluded".parse()?)],
            ..RequirementsSpecification::default()
        })?;
        assert!(input.no_index);
        assert!(input.require_hashes);
        assert_eq!(input.excludes.len(), 1);
        assert!(!input.source.is_empty());
        match input.source {
            InstallSource::Pylock { input, groups } => {
                assert_eq!(input, RequirementsInput::Local(path));
                assert!(groups.contains(&group));
            }
            InstallSource::Resolve(_) => anyhow::bail!("expected a locked input"),
        }
        Ok(())
    }

    #[test]
    fn locked_input_rejects_resolver_inputs() -> anyhow::Result<()> {
        let requirement: Requirement =
            serde_json::from_str(r#"{"name":"demo","specifier":"==1"}"#)?;
        for (mut specification, expected) in [
            (
                RequirementsSpecification {
                    requirements: vec![requirement.clone().into()],
                    ..RequirementsSpecification::default()
                },
                "Cannot specify additional requirements alongside a `pylock.toml` file",
            ),
            (
                RequirementsSpecification {
                    source_trees: vec![SourceTree::SetupPy(PathBuf::from("setup.py"))],
                    ..RequirementsSpecification::default()
                },
                "Cannot specify additional requirements alongside a `pylock.toml` file",
            ),
            (
                RequirementsSpecification {
                    constraints: vec![requirement.clone().into()],
                    ..RequirementsSpecification::default()
                },
                "Cannot specify constraints with a `pylock.toml` file",
            ),
            (
                RequirementsSpecification {
                    overrides: vec![requirement.into()],
                    ..RequirementsSpecification::default()
                },
                "Cannot specify overrides with a `pylock.toml` file",
            ),
            (
                RequirementsSpecification {
                    groups: BTreeMap::from([(
                        PathBuf::from("pyproject.toml"),
                        DependencyGroups::from_group("dev".parse()?),
                    )]),
                    ..RequirementsSpecification::default()
                },
                "Cannot specify paths for groups with a `pylock.toml` file; all groups must refer to the `pylock.toml` file",
            ),
        ] {
            specification.pylock = Some(RequirementsInput::Local(PathBuf::from("pylock.toml")));
            assert_eq!(
                InstallRequirements::try_from(specification)
                    .err()
                    .map(|error| error.to_string())
                    .as_deref(),
                Some(expected)
            );
        }
        Ok(())
    }
}
