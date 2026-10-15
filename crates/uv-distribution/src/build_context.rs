use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use uv_distribution_types::{BuildableSource, PackageRole, RequirementSource, SourceDist};
use uv_fs::normalize_path;
use uv_normalize::PackageName;
use uv_workspace::Workspace;

/// Workspace source identities and installation exclusions for metadata discovery.
///
/// Excluded and virtual members retain their workspace identity. The build purpose uses this
/// context to evaluate the workspace allowance under `--no-build`.
#[derive(Debug, Default)]
pub struct WorkspaceBuildContext {
    members: BTreeMap<PackageName, WorkspacePackage>,
    exclusions: BTreeSet<PackageName>,
}

#[derive(Debug)]
struct WorkspacePackage {
    install_path: PathBuf,
    r#virtual: Option<bool>,
}

impl WorkspaceBuildContext {
    /// Collect workspace source identities and the packages excluded from installation.
    pub fn from_workspace(workspace: &Workspace, exclusions: &BTreeSet<PackageName>) -> Self {
        let members = workspace
            .members_requirements()
            .filter_map(|requirement| match requirement.source {
                RequirementSource::Directory {
                    install_path,
                    r#virtual,
                    ..
                } => Some((
                    requirement.name,
                    WorkspacePackage {
                        install_path: install_path.into_path_buf(),
                        r#virtual,
                    },
                )),
                RequirementSource::Registry { .. }
                | RequirementSource::Url { .. }
                | RequirementSource::GitDirectory { .. }
                | RequirementSource::GitPath { .. }
                | RequirementSource::Path { .. } => None,
            })
            .collect();
        Self {
            members,
            exclusions: exclusions.clone(),
        }
    }

    /// Classify a source by its name and normalized path, independently of installation exclusions.
    /// This context describes the whole workspace without designating a project.
    pub(crate) fn package_role(&self, name: &PackageName, path: &Path) -> PackageRole {
        if self.member(name, path).is_some() {
            PackageRole::WorkspaceMember
        } else {
            PackageRole::Dependency
        }
    }

    fn member(&self, name: &PackageName, path: &Path) -> Option<&WorkspacePackage> {
        self.members
            .get(name)
            .filter(|member| normalize_path(&member.install_path) == normalize_path(path))
    }

    /// Apply the metadata-build policy to the current workspace and source identities.
    fn allows_metadata_build(&self, source: &BuildableSource<'_>) -> bool {
        let BuildableSource::Dist(SourceDist::Directory(directory)) = source else {
            return false;
        };
        self.member(&directory.name, &directory.install_path)
            .is_some_and(|member| member.r#virtual == Some(false))
            && !directory.r#virtual.unwrap_or(false)
            && !self.exclusions.contains(&directory.name)
    }
}

/// The operation for which a source distribution is being prepared.
#[derive(Debug, Clone, Copy)]
pub enum BuildPurpose<'a> {
    /// Select or prepare sources in an installation operation.
    Installation,
    /// Obtain metadata, including a wheel build if the backend has no metadata hook.
    ///
    /// Supplied workspace context determines the workspace allowance using current source
    /// identities and installation exclusions. Without it, use the source's carried role.
    Metadata(Option<&'a WorkspaceBuildContext>),
}

impl BuildPurpose<'_> {
    /// Return whether workspace membership permits a build despite `--no-build`.
    ///
    /// Editable and virtual-source allowances are applied separately by each operation.
    pub fn allows_workspace_build(self, source: &BuildableSource<'_>) -> bool {
        match self {
            Self::Installation | Self::Metadata(None) => source.is_workspace_member(),
            Self::Metadata(Some(workspace)) => workspace.allows_metadata_build(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anyhow::Result;
    use indoc::indoc;
    use tempfile::TempDir;

    use uv_cache::Cache;
    use uv_distribution_types::DirectorySourceDist;
    use uv_pep508::VerbatimUrl;
    use uv_workspace::{DiscoveryOptions, WorkspaceCache};

    use super::*;

    /// Excluded and virtual members retain their workspace identity for metadata policy.
    #[tokio::test]
    async fn metadata_build_policy_retains_workspace_identity() -> Result<()> {
        let directory = TempDir::new()?;
        fs_err::write(
            directory.path().join("pyproject.toml"),
            indoc! {r#"
            [project]
            name = "project"
            version = "1.0.0"

            [tool.uv]
            package = true

            [tool.uv.workspace]
            members = ["virtual"]
        "#},
        )?;
        let virtual_path = directory.path().join("virtual");
        fs_err::create_dir(&virtual_path)?;
        fs_err::write(
            virtual_path.join("pyproject.toml"),
            indoc! {r#"
            [project]
            name = "virtual"
            version = "1.0.0"

            [tool.uv]
            package = false
        "#},
        )?;
        let workspace = Workspace::discover(
            directory.path(),
            &DiscoveryOptions::default(),
            &Cache::from_path(directory.path().join("cache")),
            &WorkspaceCache::default(),
        )
        .await?;
        let project_name = PackageName::from_str("project")?;
        let context = WorkspaceBuildContext::from_workspace(
            &workspace,
            &BTreeSet::from([project_name.clone()]),
        );
        let project_role = context.package_role(&project_name, directory.path());
        assert_eq!(project_role, PackageRole::WorkspaceMember);
        assert_eq!(
            context.package_role(&PackageName::from_str("virtual")?, &virtual_path),
            PackageRole::WorkspaceMember,
        );
        let source = SourceDist::Directory(DirectorySourceDist {
            name: project_name,
            install_path: directory.path().into(),
            editable: Some(false),
            r#virtual: Some(false),
            role: Some(project_role),
            url: VerbatimUrl::from_absolute_path(directory.path())?,
        });
        let source = BuildableSource::Dist(&source);
        assert!(source.is_workspace_member());
        assert!(!BuildPurpose::Metadata(Some(&context)).allows_workspace_build(&source));

        Ok(())
    }
}
