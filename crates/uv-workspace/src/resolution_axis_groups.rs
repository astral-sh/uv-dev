use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use uv_configuration::DependencyGroupsWithDefaults;
use uv_distribution_types::RequiresPython;
use uv_normalize::{DEV_DEPENDENCIES, DefaultGroups, GroupName, PackageName};
use uv_pep440::VersionSpecifiers;
use uv_pep508::MarkerTree;
use uv_toml::deserialize_unique_map;

use crate::dependency_groups::{DependencyGroupError, FlatDependencyGroups};
use crate::pyproject::PyProjectToml;
use crate::{Workspace, WorkspaceAxisError};

/// The effective Python requirement of a declared dependency group.
///
/// An empty value still records that the group exists, which matters when a selected member
/// shadows a group of the same name at the workspace root.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct WorkspaceAxisDependencyGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_python: Option<VersionSpecifiers>,
}

/// Declared dependency groups and their flattened Python policy for an axis-enabled workspace.
///
/// The group declarations are independent of a solved dependency graph. In particular, empty
/// groups and requirements inherited through `include-group` must survive a frozen lockfile.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", try_from = "WorkspaceAxisGroupMetadataWire")]
pub struct WorkspaceAxisGroupMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_root: Option<PackageName>,
    #[serde(default)]
    members: BTreeMap<PackageName, BTreeMap<GroupName, WorkspaceAxisDependencyGroup>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    root_groups: BTreeMap<GroupName, WorkspaceAxisDependencyGroup>,
    member_defaults: BTreeMap<PackageName, DefaultGroups>,
    root_default_groups: DefaultGroups,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct WorkspaceAxisGroupMetadataWire {
    #[serde(default)]
    project_root: Option<PackageName>,
    #[serde(default, deserialize_with = "deserialize_member_groups")]
    members: BTreeMap<PackageName, BTreeMap<GroupName, WorkspaceAxisDependencyGroup>>,
    #[serde(default, deserialize_with = "deserialize_declared_groups")]
    root_groups: BTreeMap<GroupName, WorkspaceAxisDependencyGroup>,
    #[serde(deserialize_with = "deserialize_member_defaults")]
    member_defaults: BTreeMap<PackageName, DefaultGroups>,
    #[serde(deserialize_with = "deserialize_default_groups")]
    root_default_groups: DefaultGroups,
}

#[derive(serde::Deserialize)]
#[serde(transparent)]
struct WorkspaceAxisDeclaredGroupsWire(
    #[serde(deserialize_with = "deserialize_declared_groups")]
    BTreeMap<GroupName, WorkspaceAxisDependencyGroup>,
);

#[derive(serde::Deserialize)]
#[serde(transparent)]
struct WorkspaceAxisDefaultGroupsWire(
    #[serde(deserialize_with = "deserialize_default_groups")] DefaultGroups,
);

fn deserialize_member_groups<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<PackageName, BTreeMap<GroupName, WorkspaceAxisDependencyGroup>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let members: BTreeMap<PackageName, WorkspaceAxisDeclaredGroupsWire> =
        deserialize_unique_map(deserializer, |owner: &PackageName| {
            format!("duplicate normalized dependency-group owner `{owner}`")
        })?;
    Ok(members
        .into_iter()
        .map(|(owner, groups)| (owner, groups.0))
        .collect())
}

fn deserialize_declared_groups<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<GroupName, WorkspaceAxisDependencyGroup>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_unique_map(deserializer, |group: &GroupName| {
        format!(
            "duplicate normalized dependency group `{group}` in workspace resolution-axis metadata"
        )
    })
}

fn deserialize_member_defaults<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<PackageName, DefaultGroups>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let members: BTreeMap<PackageName, WorkspaceAxisDefaultGroupsWire> =
        deserialize_unique_map(deserializer, |owner: &PackageName| {
            format!("duplicate normalized default-group owner `{owner}`")
        })?;
    Ok(members
        .into_iter()
        .map(|(owner, groups)| (owner, groups.0))
        .collect())
}

fn deserialize_default_groups<'de, D>(deserializer: D) -> Result<DefaultGroups, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let defaults = <DefaultGroups as serde::Deserialize>::deserialize(deserializer)?;
    if let DefaultGroups::List(groups) = &defaults {
        let mut seen = BTreeSet::new();
        for group in groups {
            if !seen.insert(group) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate normalized default dependency group `{group}`"
                )));
            }
        }
    }
    Ok(defaults)
}

impl TryFrom<WorkspaceAxisGroupMetadataWire> for WorkspaceAxisGroupMetadata {
    type Error = WorkspaceAxisError;

    fn try_from(value: WorkspaceAxisGroupMetadataWire) -> Result<Self, Self::Error> {
        Self::from_parts_with_defaults(
            value.project_root,
            value.members,
            value.root_groups,
            value.member_defaults,
            value.root_default_groups,
        )
    }
}

impl WorkspaceAxisGroupMetadata {
    /// Construct metadata whose owner identities are internally consistent, with no default
    /// groups enabled. Use [`Self::from_parts_with_defaults`] for declarations read from a project.
    pub fn from_parts(
        project_root: Option<PackageName>,
        mut members: BTreeMap<PackageName, BTreeMap<GroupName, WorkspaceAxisDependencyGroup>>,
        mut root_groups: BTreeMap<GroupName, WorkspaceAxisDependencyGroup>,
    ) -> Result<Self, WorkspaceAxisError> {
        if let Some(root) = &project_root {
            if !members.contains_key(root) {
                return Err(WorkspaceAxisError::InvalidGroupMetadata(format!(
                    "project root `{root}` is not a declared workspace member"
                )));
            }
            if !root_groups.is_empty() {
                return Err(WorkspaceAxisError::InvalidGroupMetadata(
                    "a project workspace cannot contain anonymous root groups".to_owned(),
                ));
            }
        }
        for group in members
            .values_mut()
            .flat_map(BTreeMap::values_mut)
            .chain(root_groups.values_mut())
        {
            if group
                .requires_python
                .as_ref()
                .is_some_and(VersionSpecifiers::is_empty)
            {
                group.requires_python = None;
            }
        }
        let member_defaults = members
            .keys()
            .cloned()
            .map(|member| (member, DefaultGroups::default()))
            .collect();
        Ok(Self {
            project_root,
            members,
            root_groups,
            member_defaults,
            root_default_groups: DefaultGroups::default(),
        })
    }

    /// Construct complete metadata including the exact defaults of every owner.
    pub fn from_parts_with_defaults(
        project_root: Option<PackageName>,
        members: BTreeMap<PackageName, BTreeMap<GroupName, WorkspaceAxisDependencyGroup>>,
        root_groups: BTreeMap<GroupName, WorkspaceAxisDependencyGroup>,
        member_defaults: BTreeMap<PackageName, DefaultGroups>,
        root_default_groups: DefaultGroups,
    ) -> Result<Self, WorkspaceAxisError> {
        Self::from_parts(project_root, members, root_groups)?
            .with_defaults(member_defaults, root_default_groups)
    }

    /// Set defaults after validating their owner keys and declared groups.
    pub fn with_defaults(
        mut self,
        member_defaults: BTreeMap<PackageName, DefaultGroups>,
        root_default_groups: DefaultGroups,
    ) -> Result<Self, WorkspaceAxisError> {
        if self.members.keys().ne(member_defaults.keys()) {
            return Err(WorkspaceAxisError::InvalidGroupMetadata(
                "default-group owners do not match the declared workspace members".to_owned(),
            ));
        }
        let member_defaults = member_defaults
            .into_iter()
            .map(|(member, defaults)| (member, normalize_defaults(defaults)))
            .collect::<BTreeMap<_, _>>();
        let root_default_groups = normalize_defaults(root_default_groups);
        for (member, defaults) in &member_defaults {
            if let Some(declared) = self.members.get(member) {
                validate_defaults(Some(member), declared, defaults)?;
            }
        }
        if let Some(root) = &self.project_root {
            if member_defaults.get(root) != Some(&root_default_groups) {
                return Err(WorkspaceAxisError::InvalidGroupMetadata(format!(
                    "project root `{root}` has inconsistent default groups"
                )));
            }
        } else {
            validate_defaults(None, &self.root_groups, &root_default_groups)?;
        }
        self.member_defaults = member_defaults;
        self.root_default_groups = root_default_groups;
        Ok(self)
    }

    /// Verify that the group owners cover exactly the model's declared local members.
    pub fn validate_members(
        &self,
        members: &BTreeSet<PackageName>,
    ) -> Result<(), WorkspaceAxisError> {
        if let Some(member) = members
            .iter()
            .find(|member| !self.members.contains_key(*member))
        {
            return Err(WorkspaceAxisError::InvalidGroupMetadata(format!(
                "workspace member `{member}` has no group metadata"
            )));
        }
        if let Some(member) = self
            .members
            .keys()
            .find(|member| !members.contains(*member))
        {
            return Err(WorkspaceAxisError::InvalidGroupMetadata(format!(
                "group owner `{member}` is not a declared workspace member"
            )));
        }
        Ok(())
    }

    pub fn project_root(&self) -> Option<&PackageName> {
        self.project_root.as_ref()
    }

    pub fn members(
        &self,
    ) -> &BTreeMap<PackageName, BTreeMap<GroupName, WorkspaceAxisDependencyGroup>> {
        &self.members
    }

    pub fn root_groups(&self) -> &BTreeMap<GroupName, WorkspaceAxisDependencyGroup> {
        &self.root_groups
    }

    pub fn member_defaults(&self) -> &BTreeMap<PackageName, DefaultGroups> {
        &self.member_defaults
    }

    pub fn root_default_groups(&self) -> &DefaultGroups {
        &self.root_default_groups
    }

    /// Return defaults for a named member, or for the current workspace root when no owner is
    /// provided. Unknown member names are not replaced by guessed defaults.
    pub fn default_groups(&self, member: Option<&PackageName>) -> Option<&DefaultGroups> {
        match member {
            Some(member) => self.member_defaults.get(member),
            None => Some(&self.root_default_groups),
        }
    }

    /// Iterate every declared owner/group pair, including groups without a Python bound.
    pub fn iter(
        &self,
    ) -> impl Iterator<
        Item = (
            Option<&PackageName>,
            &GroupName,
            &WorkspaceAxisDependencyGroup,
        ),
    > {
        self.members
            .iter()
            .flat_map(|(owner, groups)| {
                groups
                    .iter()
                    .map(move |(group, definition)| (Some(owner), group, definition))
            })
            .chain(
                self.root_groups
                    .iter()
                    .map(|(group, definition)| (None, group, definition)),
            )
    }

    /// Return the project root when a single selected member explicitly inherits one of its
    /// dependency groups.
    pub fn group_root(
        &self,
        members: &BTreeSet<PackageName>,
        groups: &DependencyGroupsWithDefaults,
    ) -> Option<&PackageName> {
        let selected = single_member(members)?;
        let root = self
            .project_root
            .as_ref()
            .filter(|root| *root != selected)?;
        self.members
            .get(root)?
            .keys()
            .any(|group| self.includes_group(members, Some(root), group, groups))
            .then_some(root)
    }

    /// Apply explicit selection, member defaults, and member-over-root shadowing to one group.
    pub fn includes_group(
        &self,
        members: &BTreeSet<PackageName>,
        owner: Option<&PackageName>,
        group: &GroupName,
        groups: &DependencyGroupsWithDefaults,
    ) -> bool {
        if !groups.contains(group) {
            return false;
        }
        let Some(selected) = single_member(members) else {
            return true;
        };
        if owner == Some(selected) {
            return true;
        }
        !groups.contains_because_default(group)
            && !self
                .members
                .get(selected)
                .is_some_and(|declared| declared.contains_key(group))
    }

    /// Iterate exactly the groups activated by the selected roots and their inherited root.
    pub fn active_groups<'a>(
        &'a self,
        members: &'a BTreeSet<PackageName>,
        groups: &'a DependencyGroupsWithDefaults,
    ) -> impl Iterator<
        Item = (
            Option<&'a PackageName>,
            &'a GroupName,
            &'a WorkspaceAxisDependencyGroup,
        ),
    > + 'a {
        let owners = members
            .iter()
            .chain(self.group_root(members, groups))
            .collect::<BTreeSet<_>>();
        owners
            .into_iter()
            .filter_map(|owner| self.members.get_key_value(owner))
            .flat_map(|(owner, declared)| {
                declared
                    .iter()
                    .map(move |(group, definition)| (Some(owner), group, definition))
            })
            .chain(
                self.root_groups
                    .iter()
                    .map(|(group, definition)| (None, group, definition)),
            )
            .filter(move |(owner, group, _)| self.includes_group(members, *owner, group, groups))
    }

    /// Iterate the effective Python requirements of the activated owner/group pairs.
    pub fn python_requirements<'a>(
        &'a self,
        members: &'a BTreeSet<PackageName>,
        groups: &'a DependencyGroupsWithDefaults,
    ) -> impl Iterator<
        Item = (
            Option<&'a PackageName>,
            &'a GroupName,
            &'a VersionSpecifiers,
        ),
    > + 'a {
        self.active_groups(members, groups)
            .filter_map(|(owner, group, definition)| {
                definition
                    .requires_python
                    .as_ref()
                    .map(|requires_python| (owner, group, requires_python))
            })
    }

    /// Return the exact conjunction of the activated dependency groups' Python requirements.
    pub fn python_marker(
        &self,
        members: &BTreeSet<PackageName>,
        groups: &DependencyGroupsWithDefaults,
    ) -> MarkerTree {
        self.python_requirements(members, groups).fold(
            MarkerTree::TRUE,
            |marker, (_, _, requires_python)| {
                marker.and(
                    RequiresPython::from_specifiers(requires_python.clone()).to_exact_marker_tree(),
                )
            },
        )
    }

    /// Return the Python range for the activated groups, or `None` when their bounds conflict.
    pub fn requires_python(
        &self,
        members: &BTreeSet<PackageName>,
        groups: &DependencyGroupsWithDefaults,
    ) -> Option<RequiresPython> {
        RequiresPython::from_marker_tree(self.python_marker(members, groups))
    }
}

fn single_member(members: &BTreeSet<PackageName>) -> Option<&PackageName> {
    if members.len() == 1 {
        members.first()
    } else {
        None
    }
}

fn normalize_defaults(defaults: DefaultGroups) -> DefaultGroups {
    match defaults {
        DefaultGroups::List(mut groups) => {
            groups.sort_unstable();
            groups.dedup();
            DefaultGroups::List(groups)
        }
        DefaultGroups::All => DefaultGroups::All,
    }
}

fn validate_defaults(
    owner: Option<&PackageName>,
    declared: &BTreeMap<GroupName, WorkspaceAxisDependencyGroup>,
    defaults: &DefaultGroups,
) -> Result<(), WorkspaceAxisError> {
    if let DefaultGroups::List(groups) = defaults {
        for group in groups {
            // The implicit default is `dev`, even for projects without a declared `dev` group.
            if group != &*DEV_DEPENDENCIES && !declared.contains_key(group) {
                let owner = owner.map_or_else(
                    || "the workspace root".to_owned(),
                    |owner| format!("workspace member `{owner}`"),
                );
                return Err(WorkspaceAxisError::InvalidGroupMetadata(format!(
                    "default group `{group}` is not declared by {owner}"
                )));
            }
        }
    }
    Ok(())
}

fn declared_groups(
    path: &Path,
    pyproject: &PyProjectToml,
) -> Result<BTreeMap<GroupName, WorkspaceAxisDependencyGroup>, DependencyGroupError> {
    Ok(FlatDependencyGroups::from_pyproject_toml(path, pyproject)?
        .into_iter()
        .map(|(group, definition)| {
            (
                group,
                WorkspaceAxisDependencyGroup {
                    requires_python: definition.requires_python,
                },
            )
        })
        .collect())
}

impl Workspace {
    /// Collect declared dependency groups, including empty groups and flattened Python policy.
    pub fn resolution_axis_group_metadata(
        &self,
    ) -> Result<WorkspaceAxisGroupMetadata, WorkspaceAxisError> {
        let project_root = self
            .packages()
            .iter()
            .find(|(_, member)| member.root() == self.install_path())
            .map(|(name, _)| name.clone());
        let members = self
            .packages()
            .iter()
            .map(|(name, member)| {
                Ok((
                    name.clone(),
                    declared_groups(member.root(), member.pyproject_toml())?,
                ))
            })
            .collect::<Result<_, DependencyGroupError>>()?;
        let member_defaults = self
            .packages()
            .iter()
            .map(|(name, member)| Ok((name.clone(), member.pyproject_toml().default_groups()?)))
            .collect::<Result<_, crate::DefaultGroupsError>>()?;
        let root_groups = if project_root.is_none() {
            declared_groups(self.install_path(), self.pyproject_toml())?
        } else {
            BTreeMap::new()
        };
        WorkspaceAxisGroupMetadata::from_parts_with_defaults(
            project_root,
            members,
            root_groups,
            member_defaults,
            self.default_groups()?,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use anyhow::{Context, Result};
    use assert_fs::TempDir;
    use assert_fs::prelude::{FileWriteStr, PathChild};

    use uv_cache::Cache;
    use uv_configuration::{DependencyGroups, DependencyGroupsWithDefaults};
    use uv_distribution_types::RequiresPython;
    use uv_normalize::{DEV_DEPENDENCIES, DefaultGroups, GroupName, PackageName};
    use uv_pep508::MarkerTree;
    use uv_pypi_types::SupportedEnvironments;

    use super::{WorkspaceAxisDependencyGroup, WorkspaceAxisGroupMetadata};
    use crate::{DiscoveryOptions, Workspace, WorkspaceCache};

    fn definition(requires_python: Option<&str>) -> Result<WorkspaceAxisDependencyGroup> {
        Ok(WorkspaceAxisDependencyGroup {
            requires_python: requires_python.map(str::parse).transpose()?,
        })
    }

    fn members(names: &[&str]) -> Result<BTreeSet<PackageName>> {
        Ok(names
            .iter()
            .map(|name| name.parse())
            .collect::<Result<_, _>>()?)
    }

    fn groups(names: &[&str]) -> Result<DependencyGroupsWithDefaults> {
        Ok(DependencyGroups::from_args(
            None,
            names
                .iter()
                .map(|name| name.parse())
                .collect::<Result<_, _>>()?,
            Vec::new(),
            true,
            Vec::new(),
            false,
        )
        .with_defaults(DefaultGroups::default()))
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

    fn shadow_metadata(project_root: bool) -> Result<WorkspaceAxisGroupMetadata> {
        let root_groups = BTreeMap::from([
            ("docs".parse()?, definition(Some(">=3.12,<3.14"))?),
            ("lint".parse()?, definition(Some("==3.13.*"))?),
        ]);
        let mut declarations = BTreeMap::from([(
            "member".parse()?,
            BTreeMap::from([("lint".parse()?, definition(Some("==3.12.*"))?)]),
        )]);
        if project_root {
            declarations.insert("root".parse()?, root_groups);
            Ok(WorkspaceAxisGroupMetadata::from_parts(
                Some("root".parse()?),
                declarations,
                BTreeMap::new(),
            )?)
        } else {
            Ok(WorkspaceAxisGroupMetadata::from_parts(
                None,
                declarations,
                root_groups,
            )?)
        }
    }

    #[tokio::test]
    async fn collects_empty_groups_includes_and_defaults() -> Result<()> {
        let (_directory, workspace) = discover(
            r#"
            [project]
            name = "root"
            version = "0.1.0"
            requires-python = ">=3.10"
            [dependency-groups]
            empty = []
            unbounded = []
            docs = [{ include-group = "empty" }]
            [tool.uv]
            default-groups = ["empty", "docs", "docs"]
            [tool.uv.dependency-groups]
            empty = { requires-python = ">=3.12" }
            docs = { requires-python = "<3.14" }
            [tool.uv.workspace]
            members = ["member", "plain"]
            "#,
            &[
                (
                    "member",
                    r#"
                    [project]
                    name = "member"
                    version = "0.1.0"
                    [dependency-groups]
                    lint = []
                    qa = [{ include-group = "lint" }]
                    [tool.uv]
                    default-groups = "all"
                    [tool.uv.dependency-groups]
                    lint = { requires-python = ">=3.11" }
                    qa = { requires-python = "<3.13" }
                    "#,
                ),
                (
                    "plain",
                    r#"
                    [project]
                    name = "plain"
                    version = "0.1.0"
                    "#,
                ),
            ],
        )
        .await?;
        let metadata = workspace.resolution_axis_group_metadata()?;
        let root: PackageName = "root".parse()?;
        let member: PackageName = "member".parse()?;
        let plain: PackageName = "plain".parse()?;
        assert_eq!(metadata.project_root(), Some(&root));
        assert!(metadata.root_groups().is_empty());
        assert_eq!(metadata.members().len(), 3);
        assert!(metadata.members()[&plain].is_empty());
        assert!(
            metadata.members()[&root][&"unbounded".parse::<GroupName>()?]
                .requires_python
                .is_none()
        );
        let docs = metadata.members()[&root][&"docs".parse::<GroupName>()?]
            .requires_python
            .as_ref()
            .context("flattened docs requirement")?;
        assert_eq!(docs.to_string(), ">=3.12, <3.14");
        let qa = metadata.members()[&member][&"qa".parse::<GroupName>()?]
            .requires_python
            .as_ref()
            .context("flattened qa requirement")?;
        assert_eq!(qa.to_string(), ">=3.11, <3.13");
        assert_eq!(
            metadata.default_groups(Some(&member)),
            Some(&DefaultGroups::All)
        );
        assert_eq!(
            metadata.default_groups(Some(&plain)),
            Some(&DefaultGroups::List(vec![DEV_DEPENDENCIES.clone()])),
        );
        assert_eq!(
            metadata.root_default_groups(),
            &DefaultGroups::List(vec!["docs".parse()?, "empty".parse()?]),
        );
        metadata.validate_members(&members(&["member", "plain", "root"])?)?;
        let serialized = toml::to_string(&metadata)?;
        assert_eq!(
            toml::from_str::<WorkspaceAxisGroupMetadata>(&serialized)?,
            metadata
        );
        Ok(())
    }

    #[test]
    fn member_shadowing_is_applied_per_inherited_group() -> Result<()> {
        let metadata = shadow_metadata(true)?;
        let selected = members(&["member"])?;
        let enabled = groups(&["lint", "docs"])?;
        let root: PackageName = "root".parse()?;
        let member: PackageName = "member".parse()?;
        let lint: GroupName = "lint".parse()?;
        let docs: GroupName = "docs".parse()?;
        assert_eq!(metadata.group_root(&selected, &enabled), Some(&root));
        assert!(metadata.includes_group(&selected, Some(&member), &lint, &enabled));
        assert!(!metadata.includes_group(&selected, Some(&root), &lint, &enabled));
        assert!(metadata.includes_group(&selected, Some(&root), &docs, &enabled));
        let active = metadata
            .active_groups(&selected, &enabled)
            .map(|(owner, group, _)| (owner.map(ToString::to_string), group.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(
            active,
            [
                (Some("member".to_owned()), "lint".to_owned()),
                (Some("root".to_owned()), "docs".to_owned()),
            ],
        );
        let requires_python = metadata
            .requires_python(&selected, &enabled)
            .context("compatible selected group bounds")?;
        assert!(requires_python.contains(&"3.12.9".parse()?));
        assert!(!requires_python.contains(&"3.13.0".parse()?));

        let defaults =
            DependencyGroups::default().with_defaults(DefaultGroups::List(vec![docs, lint]));
        assert_eq!(metadata.group_root(&selected, &defaults), None);
        assert_eq!(metadata.active_groups(&selected, &defaults).count(), 1);
        Ok(())
    }

    #[test]
    fn anonymous_root_groups_contribute_python_without_defeating_shadowing() -> Result<()> {
        let metadata = shadow_metadata(false)?;
        let selected = members(&["member"])?;
        let enabled = groups(&["lint", "docs"])?;
        assert_eq!(metadata.group_root(&selected, &enabled), None);
        let active = metadata
            .python_requirements(&selected, &enabled)
            .map(|(owner, group, _)| (owner.map(ToString::to_string), group.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(
            active,
            [
                (Some("member".to_owned()), "lint".to_owned()),
                (None, "docs".to_owned()),
            ],
        );
        let requires_python = metadata
            .requires_python(&selected, &enabled)
            .context("compatible selected group bounds")?;
        assert!(requires_python.contains(&"3.12.9".parse()?));
        assert!(!requires_python.contains(&"3.13.0".parse()?));
        Ok(())
    }

    #[tokio::test]
    async fn exact_command_view_does_not_reapply_live_group_bounds() -> Result<()> {
        let (_directory, workspace) = discover(
            r#"
            [project]
            name = "root"
            version = "0.1.0"
            requires-python = ">=3.10,<3.14"
            [dependency-groups]
            lint = []
            docs = []
            [tool.uv.dependency-groups]
            lint = { requires-python = "==3.13.*" }
            docs = { requires-python = ">=3.12,<3.14" }
            [tool.uv.workspace]
            members = ["member"]
            "#,
            &[(
                "member",
                r#"
                [project]
                name = "member"
                version = "0.1.0"
                requires-python = ">=3.10,<3.14"
                [dependency-groups]
                lint = []
                [tool.uv.dependency-groups]
                lint = { requires-python = "==3.12.*" }
                "#,
            )],
        )
        .await?;
        let selected = members(&["member"])?;
        let enabled = groups(&["lint", "docs"])?;
        let metadata = workspace.resolution_axis_group_metadata()?;
        let marker = metadata.python_marker(&selected, &enabled);
        let requires_python = RequiresPython::from_marker_tree(marker).context("group range")?;
        let roots = members(&["member", "root"])?;
        let environments = SupportedEnvironments::from_markers(vec![marker]);
        let ordinary = workspace.with_resolution_axis_environment(
            roots.clone(),
            requires_python.clone(),
            environments.clone(),
        );
        assert!(!ordinary.is_workspace_axis_resolution());
        assert_eq!(
            ordinary
                .requires_python(&enabled)?
                .keys()
                .filter(|(_, group)| group.is_some())
                .count(),
            3,
        );
        let exact = workspace.with_resolution_axis_command_environment(
            roots,
            requires_python.clone(),
            environments,
        );
        assert!(!exact.is_workspace_axis_resolution());
        assert!(
            exact
                .requires_python(&enabled)?
                .keys()
                .all(|(_, group)| group.is_none())
        );
        assert_eq!(
            exact.workspace_group_requires_python()?,
            Some(requires_python)
        );
        assert_eq!(
            workspace
                .requires_python(&enabled)?
                .keys()
                .filter(|(_, group)| group.is_some())
                .count(),
            3,
        );
        Ok(())
    }

    #[test]
    fn metadata_validates_owners_defaults_and_roundtrips_empty_groups() -> Result<()> {
        let declarations = BTreeMap::from([(
            "member".parse()?,
            BTreeMap::from([("empty".parse()?, definition(None)?)]),
        )]);
        assert!(
            WorkspaceAxisGroupMetadata::from_parts(
                Some("missing".parse()?),
                declarations.clone(),
                BTreeMap::new(),
            )
            .is_err()
        );
        let metadata = WorkspaceAxisGroupMetadata::from_parts(None, declarations, BTreeMap::new())?;
        assert!(
            metadata
                .validate_members(&members(&["member", "missing"])?)
                .is_err()
        );
        assert!(
            metadata
                .clone()
                .with_defaults(BTreeMap::new(), DefaultGroups::default())
                .is_err()
        );
        let metadata = metadata.with_defaults(
            BTreeMap::from([(
                "member".parse()?,
                DefaultGroups::List(vec!["empty".parse()?, "empty".parse()?]),
            )]),
            DefaultGroups::default(),
        )?;
        assert_eq!(
            metadata.default_groups(Some(&"member".parse()?)),
            Some(&DefaultGroups::List(vec!["empty".parse()?])),
        );
        assert_eq!(metadata.iter().count(), 1);
        assert_eq!(
            metadata.python_marker(&members(&["member"])?, &groups(&["empty"])?),
            MarkerTree::TRUE
        );
        let serialized = toml::to_string(&metadata)?;
        assert_eq!(
            toml::from_str::<WorkspaceAxisGroupMetadata>(&serialized)?,
            metadata
        );
        Ok(())
    }

    #[test]
    fn metadata_rejects_normalized_map_key_aliases() {
        for (document, expected) in [
            (
                r"
                root-default-groups = []
                member-defaults = { foo = [] }
                members = { Foo = {}, foo = {} }
                ",
                "duplicate normalized dependency-group owner `foo`",
            ),
            (
                r"
                root-default-groups = []
                member-defaults = { member = [] }
                [members.member]
                Py_312 = {}
                py-312 = {}
                ",
                "duplicate normalized dependency group `py-312` in workspace resolution-axis metadata",
            ),
            (
                r"
                root-default-groups = []
                member-defaults = {}
                [root-groups]
                Py_312 = {}
                py-312 = {}
                ",
                "duplicate normalized dependency group `py-312` in workspace resolution-axis metadata",
            ),
            (
                r"
                root-default-groups = []
                members = { foo = {} }
                member-defaults = { Foo = [], foo = [] }
                ",
                "duplicate normalized default-group owner `foo`",
            ),
        ] {
            let error = toml::from_str::<WorkspaceAxisGroupMetadata>(document)
                .expect_err("normalized metadata map keys must be unique");
            assert_eq!(error.message(), expected);
        }
    }

    #[test]
    fn metadata_rejects_normalized_default_list_aliases() {
        for document in [
            r#"
            root-default-groups = []
            members = { member = { py-312 = {} } }
            member-defaults = { member = ["Py_312", "py-312"] }
            "#,
            r#"
            root-default-groups = ["Py_312", "py-312"]
            member-defaults = {}
            root-groups = { py-312 = {} }
            "#,
        ] {
            let error = toml::from_str::<WorkspaceAxisGroupMetadata>(document)
                .expect_err("normalized metadata defaults must be unique");
            assert_eq!(
                error.message(),
                "duplicate normalized default dependency group `py-312`"
            );
        }
    }
}
