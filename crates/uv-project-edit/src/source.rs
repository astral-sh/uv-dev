//! Construct source declarations for project and script metadata.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;
use uv_configuration::GitLfsSetting;
use uv_distribution_types::{IndexName, RequirementSource};
use uv_fs::{PortablePathBuf, try_relative_to_if};
use uv_git_types::GitReference;
use uv_normalize::PackageName;
use uv_pep508::MarkerTree;
use uv_workspace::pyproject::{Source, Sources, WorkspaceReference};

/// An error constructing a source declaration from an added requirement.
#[derive(Error, Debug)]
pub enum SourceError {
    #[error("Failed to resolve Git reference: `{0}`")]
    UnresolvedReference(String),
    #[error("Workspace dependency `{0}` must refer to local directory, not a Git repository")]
    WorkspacePackageGit(String),
    #[error("Workspace dependency `{0}` must refer to local directory, not a URL")]
    WorkspacePackageUrl(String),
    #[error("Workspace dependency `{0}` must refer to local directory, not a file")]
    WorkspacePackageFile(String),
    #[error(
        "`{0}` did not resolve to a Git repository, but a Git reference (`--rev {1}`) was provided."
    )]
    UnusedRev(String, String),
    #[error(
        "`{0}` did not resolve to a Git repository, but a Git reference (`--tag {1}`) was provided."
    )]
    UnusedTag(String, String),
    #[error(
        "`{0}` did not resolve to a Git repository, but a Git reference (`--branch {1}`) was provided."
    )]
    UnusedBranch(String, String),
    #[error(
        "`{0}` did not resolve to a Git repository, but a Git extension (`--lfs`) was provided."
    )]
    UnusedLfs(String),
    #[error(
        "`{0}` did not resolve to a local directory, but the `--editable` flag was provided. Editable installs are only supported for local directories."
    )]
    UnusedEditable(String),
    #[error("Failed to resolve absolute path")]
    Absolute(#[from] std::io::Error),
    #[error("Path contains invalid characters: {}", _0.display())]
    NonUtf8Path(PathBuf),
}

/// Construct a source declaration for an added requirement.
pub fn source_from_requirement(
    name: &PackageName,
    source: RequirementSource,
    workspace: bool,
    editable: Option<bool>,
    index: Option<IndexName>,
    rev: Option<String>,
    tag: Option<String>,
    branch: Option<String>,
    lfs: GitLfsSetting,
    root: &Path,
    existing_sources: Option<&BTreeMap<PackageName, Sources>>,
) -> Result<Option<Source>, SourceError> {
    // If the user specified a Git reference for a non-Git source, try existing Git sources before erroring.
    if !matches!(
        source,
        RequirementSource::GitDirectory { .. } | RequirementSource::GitPath { .. }
    ) && (branch.is_some()
        || tag.is_some()
        || rev.is_some()
        || matches!(lfs, GitLfsSetting::Enabled { .. }))
    {
        if let Some(sources) = existing_sources
            && let Some(package_sources) = sources.get(name)
        {
            for existing_source in package_sources.iter() {
                if let Source::Git {
                    git,
                    subdirectory,
                    path,
                    marker,
                    extra,
                    group,
                    ..
                } = existing_source
                {
                    return Ok(Some(Source::Git {
                        git: git.clone(),
                        subdirectory: subdirectory.clone(),
                        rev,
                        tag,
                        branch,
                        lfs: lfs.into(),
                        marker: *marker,
                        path: path.clone(),
                        extra: extra.clone(),
                        group: group.clone(),
                    }));
                }
            }
        }
        if let Some(rev) = rev {
            return Err(SourceError::UnusedRev(name.to_string(), rev));
        }
        if let Some(tag) = tag {
            return Err(SourceError::UnusedTag(name.to_string(), tag));
        }
        if let Some(branch) = branch {
            return Err(SourceError::UnusedBranch(name.to_string(), branch));
        }
        if matches!(lfs, GitLfsSetting::Enabled { from_env: false }) {
            return Err(SourceError::UnusedLfs(name.to_string()));
        }
    }

    // If we resolved a non-path source, and user specified an `--editable` flag, error.
    if !workspace {
        if !matches!(source, RequirementSource::Directory { .. }) {
            if editable == Some(true) {
                return Err(SourceError::UnusedEditable(name.to_string()));
            }
        }
    }

    // If the source is a workspace package, error if the user tried to specify a source.
    if workspace {
        return match source {
            RequirementSource::Registry { .. } | RequirementSource::Directory { .. } => {
                Ok(Some(Source::Workspace {
                    workspace: WorkspaceReference::Bool(true),
                    editable,
                    marker: MarkerTree::TRUE,
                    extra: None,
                    group: None,
                }))
            }
            RequirementSource::Url { .. } => {
                Err(SourceError::WorkspacePackageUrl(name.to_string()))
            }
            RequirementSource::GitDirectory { .. } => {
                Err(SourceError::WorkspacePackageGit(name.to_string()))
            }
            RequirementSource::GitPath { .. } => {
                Err(SourceError::WorkspacePackageGit(name.to_string()))
            }
            RequirementSource::Path { .. } => {
                Err(SourceError::WorkspacePackageFile(name.to_string()))
            }
        };
    }

    let source = match source {
        RequirementSource::Registry { index: Some(_), .. } => {
            return Ok(None);
        }
        RequirementSource::Registry { index: None, .. } if let Some(index) = index => {
            Source::Registry {
                index,
                marker: MarkerTree::TRUE,
                extra: None,
                group: None,
            }
        }
        RequirementSource::Registry { index: None, .. } => return Ok(None),
        RequirementSource::Path {
            install_path, url, ..
        } => Source::Path {
            editable: None,
            package: None,
            path: PortablePathBuf::from(
                try_relative_to_if(&install_path, root, url.prefers_relative())
                    .map_err(SourceError::Absolute)?
                    .into_boxed_path(),
            ),
            marker: MarkerTree::TRUE,
            extra: None,
            group: None,
        },
        RequirementSource::Directory {
            install_path,
            editable: is_editable,
            url,
            ..
        } => Source::Path {
            editable: editable.or(is_editable),
            package: None,
            path: PortablePathBuf::from(
                try_relative_to_if(&install_path, root, url.prefers_relative())
                    .map_err(SourceError::Absolute)?
                    .into_boxed_path(),
            ),
            marker: MarkerTree::TRUE,
            extra: None,
            group: None,
        },
        RequirementSource::Url {
            location,
            subdirectory,
            ..
        } => Source::Url {
            url: location,
            subdirectory: subdirectory.map(PortablePathBuf::from),
            marker: MarkerTree::TRUE,
            extra: None,
            group: None,
        },
        RequirementSource::GitDirectory {
            git, subdirectory, ..
        } => {
            if rev.is_none() && tag.is_none() && branch.is_none() {
                let rev = match git.reference() {
                    GitReference::Branch(rev) => Some(rev),
                    GitReference::Tag(rev) => Some(rev),
                    GitReference::BranchOrTag(rev) => Some(rev),
                    GitReference::BranchOrTagOrCommit(rev) => Some(rev),
                    GitReference::NamedRef(rev) => Some(rev),
                    GitReference::DefaultBranch => None,
                };
                Source::Git {
                    rev: rev.cloned(),
                    tag,
                    branch,
                    lfs: lfs.into(),
                    git: git.url().clone(),
                    subdirectory: subdirectory.map(PortablePathBuf::from),
                    path: None,
                    marker: MarkerTree::TRUE,
                    extra: None,
                    group: None,
                }
            } else {
                Source::Git {
                    rev,
                    tag,
                    branch,
                    lfs: lfs.into(),
                    git: git.url().clone(),
                    subdirectory: subdirectory.map(PortablePathBuf::from),
                    path: None,
                    marker: MarkerTree::TRUE,
                    extra: None,
                    group: None,
                }
            }
        }
        RequirementSource::GitPath {
            git, install_path, ..
        } => {
            if rev.is_none() && tag.is_none() && branch.is_none() {
                let rev = match git.reference() {
                    GitReference::Branch(rev) => Some(rev),
                    GitReference::Tag(rev) => Some(rev),
                    GitReference::BranchOrTag(rev) => Some(rev),
                    GitReference::BranchOrTagOrCommit(rev) => Some(rev),
                    GitReference::NamedRef(rev) => Some(rev),
                    GitReference::DefaultBranch => None,
                };
                Source::Git {
                    rev: rev.cloned(),
                    tag,
                    branch,
                    lfs: lfs.into(),
                    git: git.url().clone(),
                    subdirectory: None,
                    path: Some(PortablePathBuf::from(install_path.as_path())),
                    marker: MarkerTree::TRUE,
                    extra: None,
                    group: None,
                }
            } else {
                Source::Git {
                    rev,
                    tag,
                    branch,
                    lfs: lfs.into(),
                    git: git.url().clone(),
                    subdirectory: None,
                    path: Some(PortablePathBuf::from(install_path.as_path())),
                    marker: MarkerTree::TRUE,
                    extra: None,
                    group: None,
                }
            }
        }
    };

    Ok(Some(source))
}
