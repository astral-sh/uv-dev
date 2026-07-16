use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use either::Either;
use futures::future::join_all;
use rustc_hash::FxHashMap;

use thiserror::Error;
use uv_auth::CredentialsCache;
use uv_cache::Cache;
use uv_distribution_filename::DistExtension;
use uv_distribution_types::{
    Index, IndexCredentialsError, IndexLocations, IndexMetadata, IndexName, Origin, Requirement,
    RequirementSource,
};
use uv_fs::{Simplified, normalize_absolute_path, normalize_path};
use uv_git_types::{GitLfs, GitReference, GitUrl, GitUrlParseError};
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::VersionSpecifiers;
use uv_pep508::{MarkerTree, VerbatimUrl, VersionOrUrl, looks_like_git_repository};
use uv_pypi_types::{
    ConflictItem, ParsedGitDirectoryUrl, ParsedGitPathUrl, ParsedUrl, ParsedUrlError,
    VerbatimParsedUrl,
};
use uv_redacted::{DisplaySafeUrl, DisplaySafeUrlError};
use uv_workspace::pyproject::{PyProjectToml, Source, Sources, WorkspaceReference};
use uv_workspace::{DiscoveryOptions, Workspace, WorkspaceCache, WorkspaceError};

use crate::metadata::GitWorkspaceMember;

#[derive(Debug, Clone)]
pub struct LoweredRequirement(Requirement);

#[derive(Debug, Clone, Copy)]
enum RequirementOrigin {
    /// The `tool.uv.sources` were read from the project.
    Project,
    /// The `tool.uv.sources` were read from the workspace root.
    Workspace,
}

/// A borrowed lookup for indexes referenced by `tool.uv.sources`.
#[derive(Debug)]
pub struct IndexLookup<'data> {
    locations: &'data IndexLocations,
    project_indexes: &'data [Index],
    workspace_indexes: &'data [Index],
    by_name: OnceLock<FxHashMap<&'data IndexName, &'data Index>>,
}

impl<'data> IndexLookup<'data> {
    /// Create an index lookup with CLI, project, and workspace precedence.
    pub fn new(
        locations: &'data IndexLocations,
        project_indexes: &'data [Index],
        workspace_indexes: &'data [Index],
    ) -> Self {
        Self {
            locations,
            project_indexes,
            workspace_indexes,
            by_name: OnceLock::new(),
        }
    }

    /// Return the first eligible index with the given name.
    pub fn get(&self, name: &IndexName) -> Option<&'data Index> {
        if let Some(by_name) = self.by_name.get() {
            return by_name.get(name).copied();
        }

        let mut indexes = self.indexes();
        if let Some(index) = indexes.by_ref().take(16).find(|index| {
            index
                .name
                .as_ref()
                .is_some_and(|candidate| candidate == name)
        }) {
            return Some(index);
        }
        indexes.next()?;

        let by_name = self.by_name.get_or_init(|| {
            let mut by_name = FxHashMap::default();
            for index in self.indexes() {
                if let Some(name) = index.name.as_ref() {
                    by_name.entry(name).or_insert(index);
                }
            }
            by_name
        });
        by_name.get(name).copied()
    }

    fn indexes(&self) -> impl Iterator<Item = &'data Index> + 'data {
        self.locations
            .indexes()
            .filter(|index| matches!(index.origin, Some(Origin::Cli)))
            .chain(self.project_indexes.iter())
            .chain(self.workspace_indexes.iter())
    }
}

impl LoweredRequirement {
    /// Combine `project.dependencies` or `project.optional-dependencies` with `tool.uv.sources`.
    pub(crate) async fn from_requirement<'data>(
        requirement: uv_pep508::Requirement<VerbatimParsedUrl>,
        project_name: Option<&'data PackageName>,
        project_dir: &'data Path,
        project_sources: &'data BTreeMap<PackageName, Sources>,
        indexes: &'data IndexLookup<'data>,
        extra: Option<&ExtraName>,
        group: Option<&GroupName>,
        workspace: &'data Workspace,
        git_member: Option<&'data GitWorkspaceMember<'data>>,
        editable: bool,
        cache: &'data Cache,
        workspace_cache: &'data WorkspaceCache,
        credentials_cache: &'data CredentialsCache,
    ) -> impl Iterator<Item = Result<Self, LoweringError>> + use<'data> + 'data {
        // Identify the source from the `tool.uv.sources` table.
        let (sources, origin) = if let Some(source) = project_sources.get(&requirement.name) {
            (Some(source), RequirementOrigin::Project)
        } else if let Some(source) = workspace.sources().get(&requirement.name) {
            (Some(source), RequirementOrigin::Workspace)
        } else {
            (None, RequirementOrigin::Project)
        };

        // If the source only applies to a given extra or dependency group, filter it out.
        let sources = sources.map(|sources| {
            sources
                .iter()
                .filter(|source| {
                    if let Some(target) = source.extra()
                        && extra != Some(target)
                    {
                        return false;
                    }

                    if let Some(target) = source.group()
                        && group != Some(target)
                    {
                        return false;
                    }

                    true
                })
                .cloned()
                .collect::<Sources>()
        });

        // If you use a package that's part of the workspace...
        if workspace.packages().contains_key(&requirement.name) {
            // And it's not a recursive self-inclusion (extras that activate other extras), e.g.
            // `framework[machine_learning]` depends on `framework[cuda]`.
            if project_name.is_none_or(|project_name| *project_name != requirement.name) {
                // It must be declared as a workspace source.
                let Some(sources) = sources.as_ref() else {
                    // No sources were declared for the workspace package.
                    return Either::Left(std::iter::once(Err(
                        LoweringError::MissingWorkspaceSource(requirement.name.clone()),
                    )));
                };

                for source in sources.iter() {
                    match source {
                        Source::Git { .. } => {
                            return Either::Left(std::iter::once(Err(
                                LoweringError::NonWorkspaceSource(
                                    requirement.name.clone(),
                                    SourceKind::Git,
                                ),
                            )));
                        }
                        Source::Url { .. } => {
                            return Either::Left(std::iter::once(Err(
                                LoweringError::NonWorkspaceSource(
                                    requirement.name.clone(),
                                    SourceKind::Url,
                                ),
                            )));
                        }
                        Source::Path { .. } => {
                            return Either::Left(std::iter::once(Err(
                                LoweringError::NonWorkspaceSource(
                                    requirement.name.clone(),
                                    SourceKind::Path,
                                ),
                            )));
                        }
                        Source::Registry { .. } => {
                            return Either::Left(std::iter::once(Err(
                                LoweringError::NonWorkspaceSource(
                                    requirement.name.clone(),
                                    SourceKind::Registry,
                                ),
                            )));
                        }
                        Source::Workspace {
                            workspace: WorkspaceReference::Bool(true),
                            ..
                        } => {
                            // OK
                        }
                        Source::Workspace { .. } => {
                            return Either::Left(std::iter::once(Err(
                                LoweringError::InvalidWorkspaceSource(requirement.name.clone()),
                            )));
                        }
                    }
                }
            }
        }

        let Some(sources) = sources else {
            return Either::Left(std::iter::once(Self::preserve_git_source(
                requirement,
                git_member,
            )));
        };

        // Determine whether the markers cover the full space for the requirement. If not, fill the
        // remaining space with the negation of the sources.
        let remaining = {
            // Determine the space covered by the sources.
            let mut total = MarkerTree::FALSE;
            for source in sources.iter() {
                total = total.or(source.marker());
            }

            // Determine the space covered by the requirement.
            let mut remaining = total.negate();
            remaining = remaining.and(requirement.marker);

            Self(Requirement {
                marker: remaining,
                ..Requirement::from(requirement.clone())
            })
        };

        Either::Right(
            join_all(sources.into_iter().map(|source| {
                let requirement = &requirement;
                async move {
                    let (source, mut marker) = match source {
                        Source::Git {
                            git,
                            subdirectory,
                            path,
                            rev,
                            tag,
                            branch,
                            lfs,
                            marker,
                            ..
                        } => {
                            let source = git_source(
                                git,
                                subdirectory.map(Box::<Path>::from),
                                path.map(Box::<Path>::from).map(PathBuf::from),
                                rev,
                                tag,
                                branch,
                                lfs,
                            )?;
                            (source, marker)
                        }
                        Source::Url {
                            url,
                            subdirectory,
                            marker,
                            ..
                        } => {
                            let source =
                                url_source(requirement, url, subdirectory.map(Box::<Path>::from))?;
                            (source, marker)
                        }
                        Source::Path {
                            path,
                            editable,
                            package,
                            marker,
                            ..
                        } => {
                            let source = path_source(
                                path,
                                git_member,
                                origin,
                                project_dir,
                                workspace.install_path(),
                                editable,
                                package,
                                true,
                            )?;
                            (source, marker)
                        }
                        Source::Registry {
                            index,
                            marker,
                            extra,
                            group,
                        } => {
                            // Identify the named index with CLI, project, and workspace precedence.
                            let Some(index) = indexes.get(&index) else {
                                let hint = missing_index_hint(indexes.locations, &index);
                                return Err(LoweringError::MissingIndex {
                                    package: requirement.name.clone(),
                                    index,
                                    hint,
                                });
                            };
                            if let Some(credentials) = index.credentials()? {
                                credentials_cache.store_credentials(index.raw_url(), credentials);
                            }
                            let index = IndexMetadata {
                                url: index.url.clone(),
                                format: index.format,
                            };
                            let conflict = project_name.and_then(|project_name| {
                                if let Some(extra) = extra {
                                    Some(ConflictItem::from((project_name.clone(), extra)))
                                } else {
                                    group.map(|group| {
                                        ConflictItem::from((project_name.clone(), group))
                                    })
                                }
                            });
                            let source = registry_source(requirement, index, conflict);
                            (source, marker)
                        }
                        Source::Workspace {
                            workspace: workspace_ref,
                            editable: source_editable,
                            marker,
                            ..
                        } => {
                            let source = workspace_source(
                                requirement,
                                &workspace_ref,
                                source_editable,
                                editable,
                                origin,
                                project_dir,
                                workspace.install_path(),
                                Some(workspace),
                                git_member,
                                cache,
                                workspace_cache,
                            )
                            .await?;
                            (source, marker)
                        }
                    };

                    marker = marker.and(requirement.marker);

                    Ok(Self(Requirement {
                        name: requirement.name.clone(),
                        extras: requirement.extras.clone(),
                        groups: Box::new([]),
                        marker,
                        source,
                        origin: requirement.origin.clone(),
                    }))
                }
            }))
            .await
            .into_iter()
            .chain(std::iter::once(Ok(remaining)))
            .filter(|requirement| match requirement {
                Ok(requirement) => !requirement.0.marker.is_false(),
                Err(_) => true,
            }),
        )
    }

    /// Lower a [`uv_pep508::Requirement`] in a non-workspace setting (for example, in a PEP 723
    /// script, which runs in an isolated context).
    pub async fn from_non_workspace_requirement<'data>(
        requirement: uv_pep508::Requirement<VerbatimParsedUrl>,
        dir: &'data Path,
        sources: &'data BTreeMap<PackageName, Sources>,
        indexes: &'data IndexLookup<'data>,
        cache: &'data Cache,
        workspace_cache: &'data WorkspaceCache,
        credentials_cache: &'data CredentialsCache,
    ) -> impl Iterator<Item = Result<Self, LoweringError>> + 'data {
        let source = sources.get(&requirement.name).cloned();

        let Some(source) = source else {
            return Either::Left(std::iter::once(Ok(Self(Requirement::from(requirement)))));
        };

        // If the source only applies to a given extra, filter it out.
        let source = source
            .iter()
            .filter(|source| {
                source.extra().is_none_or(|target| {
                    requirement
                        .marker
                        .top_level_extra_name()
                        .is_some_and(|extra| &*extra == target)
                })
            })
            .cloned()
            .collect::<Sources>();

        // Determine whether the markers cover the full space for the requirement. If not, fill the
        // remaining space with the negation of the sources.
        let remaining = {
            // Determine the space covered by the sources.
            let mut total = MarkerTree::FALSE;
            for source in source.iter() {
                total = total.or(source.marker());
            }

            // Determine the space covered by the requirement.
            let mut remaining = total.negate();
            remaining = remaining.and(requirement.marker);

            Self(Requirement {
                marker: remaining,
                ..Requirement::from(requirement.clone())
            })
        };

        Either::Right(
            join_all(source.into_iter().map(|source| {
                let requirement = &requirement;
                async move {
                    let (source, mut marker) = match source {
                        Source::Git {
                            git,
                            subdirectory,
                            path,
                            rev,
                            tag,
                            branch,
                            lfs,
                            marker,
                            ..
                        } => {
                            let source = git_source(
                                git,
                                subdirectory.map(Box::<Path>::from),
                                path.map(Box::<Path>::from).map(PathBuf::from),
                                rev,
                                tag,
                                branch,
                                lfs,
                            )?;
                            (source, marker)
                        }
                        Source::Url {
                            url,
                            subdirectory,
                            marker,
                            ..
                        } => {
                            let source =
                                url_source(requirement, url, subdirectory.map(Box::<Path>::from))?;
                            (source, marker)
                        }
                        Source::Path {
                            path,
                            editable,
                            package,
                            marker,
                            ..
                        } => {
                            let source = path_source(
                                path,
                                None,
                                RequirementOrigin::Project,
                                dir,
                                dir,
                                editable,
                                package,
                                true,
                            )?;
                            (source, marker)
                        }
                        Source::Registry { index, marker, .. } => {
                            let Some(index) = indexes.get(&index) else {
                                let hint = missing_index_hint(indexes.locations, &index);
                                return Err(LoweringError::MissingIndex {
                                    package: requirement.name.clone(),
                                    index,
                                    hint,
                                });
                            };
                            if let Some(credentials) = index.credentials()? {
                                credentials_cache.store_credentials(index.raw_url(), credentials);
                            }
                            let index = IndexMetadata {
                                url: index.url.clone(),
                                format: index.format,
                            };
                            let conflict = None;
                            let source = registry_source(requirement, index, conflict);
                            (source, marker)
                        }
                        Source::Workspace {
                            workspace: workspace_ref,
                            editable,
                            marker,
                            ..
                        } => {
                            let source = workspace_source(
                                requirement,
                                &workspace_ref,
                                editable,
                                true,
                                RequirementOrigin::Project,
                                dir,
                                dir,
                                None,
                                None,
                                cache,
                                workspace_cache,
                            )
                            .await?;
                            (source, marker)
                        }
                    };

                    marker = marker.and(requirement.marker);

                    Ok(Self(Requirement {
                        name: requirement.name.clone(),
                        extras: requirement.extras.clone(),
                        groups: Box::new([]),
                        marker,
                        source,
                        origin: requirement.origin.clone(),
                    }))
                }
            }))
            .await
            .into_iter()
            .chain(std::iter::once(Ok(remaining)))
            .filter(|requirement| match requirement {
                Ok(requirement) => !requirement.0.marker.is_false(),
                Err(_) => true,
            }),
        )
    }

    /// Preserve the Git origin for direct path dependencies discovered while lowering metadata from
    /// a checked-out Git repository.
    pub(crate) fn preserve_git_source(
        requirement: uv_pep508::Requirement<VerbatimParsedUrl>,
        git_member: Option<&GitWorkspaceMember>,
    ) -> Result<Self, LoweringError> {
        let Some(git_member) = git_member else {
            return Ok(Self(Requirement::from(requirement)));
        };

        let Some(VersionOrUrl::Url(url)) = &requirement.version_or_url else {
            return Ok(Self(Requirement::from(requirement)));
        };

        let (install_path, is_archive) = match &url.parsed_url {
            ParsedUrl::Directory(directory) => (directory.install_path.as_ref(), false),
            ParsedUrl::Path(path) => (path.install_path.as_ref(), true),
            _ => return Ok(Self(Requirement::from(requirement))),
        };

        let install_path = git_path(install_path)?;
        let fetch_root = git_path(git_member.fetch_root)?;
        if !install_path.starts_with(&fetch_root) {
            return Ok(Self(Requirement::from(requirement)));
        }

        Ok(Self(Requirement {
            name: requirement.name,
            groups: Box::new([]),
            extras: requirement.extras,
            marker: requirement.marker,
            source: if is_archive {
                git_archive_source_from_path(&install_path, git_member)?
            } else {
                git_directory_source_from_path(&install_path, git_member)?
            },
            origin: requirement.origin,
        }))
    }

    /// Convert back into a [`Requirement`].
    pub fn into_inner(self) -> Requirement {
        self.0
    }
}

/// An error parsing and merging `tool.uv.sources` with
/// `project.{dependencies,optional-dependencies}`.
#[derive(Debug, Error)]
pub enum LoweringError {
    #[error(
        "`{0}` is included as a workspace member, but is missing an entry in `tool.uv.sources` (e.g., `{0} = {{ workspace = true }}`)"
    )]
    MissingWorkspaceSource(PackageName),
    #[error(
        "`{0}` is included as a workspace member, but references a {1} in `tool.uv.sources`. Workspace members must be declared as workspace sources (e.g., `{0} = {{ workspace = true }}`)."
    )]
    NonWorkspaceSource(PackageName, SourceKind),
    #[error(
        "`{0}` references a workspace in `tool.uv.sources` (e.g., `{0} = {{ workspace = true }}`), but is not a workspace member"
    )]
    UndeclaredWorkspacePackage(PackageName),
    #[error(
        "`{0}` is included as a workspace member, but does not use `workspace = true` in `tool.uv.sources`"
    )]
    InvalidWorkspaceSource(PackageName),
    #[error("Can only specify one of: `rev`, `tag`, or `branch`")]
    MoreThanOneGitRef,
    #[error(transparent)]
    GitUrlParse(#[from] GitUrlParseError),
    #[error("Package `{package}` references an undeclared index: `{index}`")]
    MissingIndex {
        package: PackageName,
        index: IndexName,
        hint: Option<String>,
    },
    #[error("Workspace members are not allowed in non-workspace contexts")]
    WorkspaceMember,
    #[error(transparent)]
    InvalidUrl(#[from] DisplaySafeUrlError),
    #[error(transparent)]
    IndexCredentials(#[from] IndexCredentialsError),
    #[error(transparent)]
    InvalidVerbatimUrl(#[from] uv_pep508::VerbatimUrlError),
    #[error("Fragments are not allowed in URLs: `{0}`")]
    ForbiddenFragment(DisplaySafeUrl),
    #[error(
        "`{0}` is associated with a URL source, but references a Git repository. Consider using a Git source instead (e.g., `{0} = {{ git = \"{1}\" }}`)"
    )]
    MissingGitSource(PackageName, DisplaySafeUrl),
    #[error("`workspace = false` is not yet supported")]
    WorkspaceFalse,
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(
        "Workspace source path `{}` must point to a workspace root (found workspace at `{}`)", path.simplified_display(), root.simplified_display()
    )]
    WorkspaceSourceNotRoot { path: PathBuf, root: PathBuf },
    #[error("Source with `editable = true` must refer to a local directory, not a file: `{0}`")]
    EditableFile(String),
    #[error("Source with `package = true` must refer to a local directory, not a file: `{0}`")]
    PackagedFile(String),
    #[error(
        "Git repository references local file source, but only directories are supported as transitive Git dependencies: `{0}`"
    )]
    GitFile(String),
    #[error("Git repository references local directory outside the repository: `{0}`")]
    GitDirectory(String),
    #[error(transparent)]
    ParsedUrl(#[from] ParsedUrlError),
    #[error("Path must be UTF-8: `{0}`")]
    NonUtf8Path(PathBuf),
    #[error(transparent)] // Function attaches the context
    RelativeTo(io::Error),
}

impl uv_errors::Hint for LoweringError {
    fn hints(&self) -> uv_errors::Hints<'_> {
        match self {
            Self::MissingIndex {
                hint: Some(hint), ..
            } => uv_errors::Hints::from(hint.clone()),
            _ => uv_errors::Hints::none(),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum SourceKind {
    Path,
    Url,
    Git,
    Registry,
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path => write!(f, "path"),
            Self::Url => write!(f, "URL"),
            Self::Git => write!(f, "Git"),
            Self::Registry => write!(f, "registry"),
        }
    }
}

/// Generate a hint for a missing index if the index name is found in a configuration file
/// (e.g., `uv.toml`) rather than in the project's `pyproject.toml`.
fn missing_index_hint(locations: &IndexLocations, index: &IndexName) -> Option<String> {
    let config_index = locations
        .simple_indexes()
        .filter(|idx| !matches!(idx.origin, Some(Origin::Cli)))
        .find(|idx| idx.name.as_ref().is_some_and(|name| *name == *index));

    config_index.and_then(|idx| {
        let source = match idx.origin {
            Some(Origin::User) => "a user-level `uv.toml`",
            Some(Origin::System) => "a system-level `uv.toml`",
            Some(Origin::Project) => "a project-level `uv.toml`",
            Some(Origin::Cli | Origin::RequirementsTxt) | None => return None,
        };
        Some(format!(
            "Index `{index}` was found in {source}, but indexes \
             referenced via `tool.uv.sources` must be defined in the project's \
             `pyproject.toml`"
        ))
    })
}

/// Convert a Git source into a [`RequirementSource`].
fn git_source(
    git: DisplaySafeUrl,
    subdirectory: Option<Box<Path>>,
    path: Option<PathBuf>,
    rev: Option<String>,
    tag: Option<String>,
    branch: Option<String>,
    lfs: Option<bool>,
) -> Result<RequirementSource, LoweringError> {
    let reference = match (rev, tag, branch) {
        (None, None, None) => GitReference::DefaultBranch,
        (Some(rev), None, None) => GitReference::from_rev(rev),
        (None, Some(tag), None) => GitReference::Tag(tag),
        (None, None, Some(branch)) => GitReference::Branch(branch),
        _ => return Err(LoweringError::MoreThanOneGitRef),
    };

    // Create a PEP 508-compatible URL.
    let mut url = DisplaySafeUrl::parse(&format!("git+{git}"))?;
    if let Some(rev) = reference.as_str() {
        let path = format!("{}@{}", url.path(), rev);
        url.set_path(&path);
    }
    let mut frags: Vec<String> = Vec::new();
    if let Some(subdirectory) = subdirectory.as_ref() {
        let subdirectory = subdirectory
            .to_str()
            .ok_or_else(|| LoweringError::NonUtf8Path(subdirectory.to_path_buf()))?;
        frags.push(format!("subdirectory={subdirectory}"));
    }
    // Loads Git LFS Enablement according to priority.
    // First: lfs = true, lfs = false from pyproject.toml
    // Second: UV_GIT_LFS from environment
    let lfs = GitLfs::from(lfs);
    // Preserve that we're using Git LFS in the Verbatim Url representations
    if lfs.enabled() {
        frags.push("lfs=true".to_string());
    }
    if let Some(path) = path.as_ref() {
        let path = path
            .to_str()
            .ok_or_else(|| LoweringError::NonUtf8Path(path.clone()))?;
        frags.push(format!("path={path}"));
    }
    if !frags.is_empty() {
        url.set_fragment(Some(&frags.join("&")));
    }
    let url = VerbatimUrl::from_url(url);

    let git = GitUrl::from_fields(git, reference, None, lfs)?;

    if let Some(path) = path {
        let ext = match DistExtension::from_path(&path) {
            Ok(ext) => ext,
            Err(err) => {
                return Err(ParsedUrlError::MissingExtensionPath(path, err).into());
            }
        };
        Ok(RequirementSource::GitPath {
            url,
            git,
            install_path: path,
            ext,
        })
    } else {
        Ok(RequirementSource::GitDirectory {
            url,
            git,
            subdirectory,
        })
    }
}

/// Convert a URL source into a [`RequirementSource`].
fn url_source(
    requirement: &uv_pep508::Requirement<VerbatimParsedUrl>,
    url: DisplaySafeUrl,
    subdirectory: Option<Box<Path>>,
) -> Result<RequirementSource, LoweringError> {
    let mut verbatim_url = url.clone();
    if verbatim_url.fragment().is_some() {
        return Err(LoweringError::ForbiddenFragment(url));
    }
    if let Some(subdirectory) = subdirectory.as_ref() {
        let subdirectory = subdirectory
            .to_str()
            .ok_or_else(|| LoweringError::NonUtf8Path(subdirectory.to_path_buf()))?;
        verbatim_url.set_fragment(Some(&format!("subdirectory={subdirectory}")));
    }

    let ext = match DistExtension::from_path(url.path()) {
        Ok(ext) => ext,
        Err(..) if looks_like_git_repository(&url) => {
            return Err(LoweringError::MissingGitSource(
                requirement.name.clone(),
                url.clone(),
            ));
        }
        Err(err) => {
            return Err(ParsedUrlError::MissingExtensionUrl(url.to_string(), err).into());
        }
    };

    let verbatim_url = VerbatimUrl::from_url(verbatim_url);
    Ok(RequirementSource::Url {
        location: url,
        subdirectory,
        ext,
        url: verbatim_url,
    })
}

/// Convert a registry source into a [`RequirementSource`].
fn registry_source(
    requirement: &uv_pep508::Requirement<VerbatimParsedUrl>,
    index: IndexMetadata,
    conflict: Option<ConflictItem>,
) -> RequirementSource {
    match &requirement.version_or_url {
        None => RequirementSource::Registry {
            specifier: VersionSpecifiers::empty(),
            index: Some(index),
            conflict,
        },
        Some(VersionOrUrl::VersionSpecifier(version)) => RequirementSource::Registry {
            specifier: version.clone(),
            index: Some(index),
            conflict,
        },
        Some(VersionOrUrl::Url(_)) => RequirementSource::Registry {
            specifier: VersionSpecifiers::empty(),
            index: Some(index),
            conflict,
        },
    }
}

async fn workspace_source(
    requirement: &uv_pep508::Requirement<VerbatimParsedUrl>,
    workspace_ref: &WorkspaceReference,
    source_editable: Option<bool>,
    default_editable: bool,
    origin: RequirementOrigin,
    project_dir: &Path,
    workspace_root: &Path,
    current_workspace: Option<&Workspace>,
    git_member: Option<&GitWorkspaceMember<'_>>,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
) -> Result<RequirementSource, LoweringError> {
    let base = match origin {
        RequirementOrigin::Project => project_dir,
        RequirementOrigin::Workspace => workspace_root,
    };

    match workspace_ref {
        WorkspaceReference::Bool(false) => Err(LoweringError::WorkspaceFalse),
        WorkspaceReference::Bool(true) => {
            let workspace = current_workspace.ok_or(LoweringError::WorkspaceMember)?;
            let member = workspace.packages().get(&requirement.name).ok_or_else(|| {
                LoweringError::UndeclaredWorkspacePackage(requirement.name.clone())
            })?;

            let value = workspace.required_members().get(&requirement.name);
            let is_required_member = value.is_some();
            let is_package = member.pyproject_toml().is_package(!is_required_member);
            let editable = if is_package {
                Some(value.copied().flatten().unwrap_or(default_editable))
            } else {
                Some(false)
            };
            path_source(
                member.root(),
                git_member,
                origin,
                project_dir,
                workspace_root,
                editable,
                Some(is_package),
                false,
            )
        }
        WorkspaceReference::Path(path) => {
            let workspace_path = VerbatimUrl::from_path(path.as_ref(), base)?
                .to_file_path()
                .map_err(|()| {
                    LoweringError::RelativeTo(io::Error::other("Invalid path in file URL"))
                })?;
            let target_workspace = Workspace::discover(
                &workspace_path,
                &DiscoveryOptions::default(),
                cache,
                workspace_cache,
            )
            .await?;

            if target_workspace.install_path() != &workspace_path {
                return Err(LoweringError::WorkspaceSourceNotRoot {
                    path: workspace_path,
                    root: target_workspace.install_path().clone(),
                });
            }

            let member = target_workspace
                .packages()
                .get(&requirement.name)
                .ok_or_else(|| {
                    LoweringError::UndeclaredWorkspacePackage(requirement.name.clone())
                })?;

            let is_package = member.pyproject_toml().is_package(false);
            let editable = if is_package {
                Some(source_editable.unwrap_or(true))
            } else {
                Some(false)
            };
            let member_path =
                uv_fs::relative_to(member.root(), base).unwrap_or_else(|_| member.root().into());

            path_source(
                member_path,
                git_member,
                origin,
                project_dir,
                workspace_root,
                editable,
                Some(is_package),
                true,
            )
        }
    }
}

/// Convert a path string to a file or directory source.
fn path_source(
    path: impl AsRef<Path>,
    git_member: Option<&GitWorkspaceMember>,
    origin: RequirementOrigin,
    project_dir: &Path,
    workspace_root: &Path,
    editable: Option<bool>,
    package: Option<bool>,
    preserve_given: bool,
) -> Result<RequirementSource, LoweringError> {
    let path = path.as_ref();
    let base = match origin {
        RequirementOrigin::Project => project_dir,
        RequirementOrigin::Workspace => workspace_root,
    };
    let url = VerbatimUrl::from_path(path, base)?;
    let url = if preserve_given {
        url.with_given(path.to_string_lossy())
    } else {
        url
    };
    let install_path = url
        .to_file_path()
        .map_err(|()| LoweringError::RelativeTo(io::Error::other("Invalid path in file URL")))?;

    let is_dir = if let Ok(metadata) = install_path.metadata() {
        metadata.is_dir()
    } else {
        install_path.extension().is_none()
    };
    if is_dir {
        if let Some(git_member) = git_member {
            return git_directory_source_from_path(install_path, git_member);
        }

        if editable == Some(true) {
            Ok(RequirementSource::Directory {
                install_path: install_path.into_boxed_path(),
                url,
                editable,
                r#virtual: Some(false),
            })
        } else {
            // Determine whether the project is a package or virtual.
            // If the `package` option is unset, check if `tool.uv.package` is set
            // on the path source (otherwise, default to `true`).
            let is_package = package.unwrap_or_else(|| {
                let pyproject_path = install_path.join("pyproject.toml");
                fs_err::read_to_string(&pyproject_path)
                    .ok()
                    .and_then(|contents| PyProjectToml::from_string(contents, pyproject_path).ok())
                    // We don't require a build system for path dependencies
                    .is_none_or(|pyproject_toml| pyproject_toml.is_package(false))
            });

            // If the project is not a package, treat it as a virtual dependency.
            let r#virtual = !is_package;

            Ok(RequirementSource::Directory {
                install_path: install_path.into_boxed_path(),
                url,
                editable: Some(false),
                r#virtual: Some(r#virtual),
            })
        }
    } else {
        if let Some(git_member) = git_member {
            return git_archive_source_from_path(install_path, git_member);
        }
        if editable == Some(true) {
            return Err(LoweringError::EditableFile(url.to_string()));
        }
        if package == Some(true) {
            return Err(LoweringError::PackagedFile(url.to_string()));
        }
        Ok(RequirementSource::Path {
            ext: DistExtension::from_path(&install_path)
                .map_err(|err| ParsedUrlError::MissingExtensionPath(path.to_path_buf(), err))?,
            install_path: install_path.into_boxed_path(),
            url,
        })
    }
}

fn git_directory_source_from_path(
    install_path: impl AsRef<Path>,
    git_member: &GitWorkspaceMember,
) -> Result<RequirementSource, LoweringError> {
    let git = git_member.git_source.git.clone();
    let install_path = git_path(install_path.as_ref())?;
    let fetch_root = git_path(git_member.fetch_root)?;
    let subdirectory = uv_fs::relative_to(&install_path, fetch_root)
        .map_err(|_| LoweringError::GitDirectory(install_path.display().to_string()))?;
    let subdirectory = normalize_path(subdirectory);
    let subdirectory = if subdirectory == PathBuf::new() {
        None
    } else {
        Some(subdirectory.into_owned().into_boxed_path())
    };
    let url = DisplaySafeUrl::from(ParsedGitDirectoryUrl {
        url: git.clone(),
        subdirectory: subdirectory.clone(),
    });
    Ok(RequirementSource::GitDirectory {
        git,
        subdirectory,
        url: VerbatimUrl::from_url(url),
    })
}

fn git_archive_source_from_path(
    install_path: impl AsRef<Path>,
    git_member: &GitWorkspaceMember,
) -> Result<RequirementSource, LoweringError> {
    let git = git_member.git_source.git.clone();
    let install_path = git_path(install_path.as_ref())?;
    let fetch_root = git_path(git_member.fetch_root)?;
    let install_path =
        uv_fs::relative_to(install_path, fetch_root).map_err(LoweringError::RelativeTo)?;
    let install_path = normalize_path(install_path).into_owned();
    let ext = DistExtension::from_path(&install_path)
        .map_err(|err| ParsedUrlError::MissingExtensionPath(install_path.clone(), err))?;
    let url = DisplaySafeUrl::from(ParsedGitPathUrl {
        url: git.clone(),
        install_path: install_path.clone(),
        ext,
    });
    Ok(RequirementSource::GitPath {
        git,
        install_path,
        ext,
        url: VerbatimUrl::from_url(url),
    })
}

fn git_path(path: &Path) -> Result<PathBuf, LoweringError> {
    path.simple_canonicalize()
        .or_else(|_| normalize_absolute_path(path))
        .map_err(LoweringError::RelativeTo)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn index(name: &str, host: &str, origin: Option<Origin>) -> Index {
        let index =
            Index::from_str(&format!("{name}=https://{host}/simple")).expect("valid named index");
        if let Some(origin) = origin {
            index.with_origin(origin)
        } else {
            index
        }
    }

    #[test]
    fn index_lookup_preserves_precedence_and_first_duplicate() {
        let cli = vec![
            index("shared", "first-cli.example.com", Some(Origin::Cli)),
            index("shared", "second-cli.example.com", Some(Origin::Cli)),
        ];
        let locations = IndexLocations::new(cli, vec![], false);
        let project = vec![
            index("project", "first-project.example.com", None),
            index("project", "second-project.example.com", None),
            index("shared", "project.example.com", None),
        ];
        let mut workspace = vec![
            index("workspace", "first-workspace.example.com", None),
            index("workspace", "second-workspace.example.com", None),
            index("project", "workspace.example.com", None),
            index("shared", "workspace.example.com", None),
        ];
        workspace.extend((0..32).map(|position| {
            index(
                &format!("padding-{position}"),
                &format!("padding-{position}.example.com"),
                None,
            )
        }));
        let lookup = IndexLookup::new(&locations, &project, &workspace);

        let shared = IndexName::from_str("shared").expect("valid index name");
        let project_name = IndexName::from_str("project").expect("valid index name");
        let workspace_name = IndexName::from_str("workspace").expect("valid index name");

        assert_eq!(
            lookup.get(&shared).expect("CLI index").raw_url().host_str(),
            Some("first-cli.example.com")
        );
        assert_eq!(
            lookup
                .get(&project_name)
                .expect("project index")
                .raw_url()
                .host_str(),
            Some("first-project.example.com")
        );
        assert_eq!(
            lookup
                .get(&workspace_name)
                .expect("workspace index")
                .raw_url()
                .host_str(),
            Some("first-workspace.example.com")
        );
        assert!(lookup.by_name.get().is_none());

        let last = IndexName::from_str("padding-31").expect("valid index name");
        assert_eq!(
            lookup.get(&last).expect("last index").raw_url().host_str(),
            Some("padding-31.example.com")
        );
        assert!(lookup.by_name.get().is_some());
        assert_eq!(
            lookup.get(&shared).expect("CLI index").raw_url().host_str(),
            Some("first-cli.example.com")
        );
        assert_eq!(
            lookup
                .get(&project_name)
                .expect("project index")
                .raw_url()
                .host_str(),
            Some("first-project.example.com")
        );
        assert_eq!(
            lookup
                .get(&workspace_name)
                .expect("workspace index")
                .raw_url()
                .host_str(),
            Some("first-workspace.example.com")
        );
    }

    #[test]
    fn index_lookup_indexes_late_hits_and_misses() {
        let project = (0..64)
            .map(|position| {
                index(
                    &format!("index-{position}"),
                    &format!("index-{position}.example.com"),
                    None,
                )
            })
            .collect::<Vec<_>>();
        let locations = IndexLocations::default();
        let lookup = IndexLookup::new(&locations, &project, &[]);
        let first = IndexName::from_str("index-0").expect("valid index name");
        let last = IndexName::from_str("index-63").expect("valid index name");
        let missing = IndexName::from_str("missing").expect("valid index name");

        assert_eq!(
            lookup
                .get(&first)
                .expect("first index")
                .raw_url()
                .host_str(),
            Some("index-0.example.com")
        );
        assert!(lookup.by_name.get().is_none());
        assert_eq!(
            lookup.get(&last).expect("last index").raw_url().host_str(),
            Some("index-63.example.com")
        );
        assert!(lookup.by_name.get().is_some());
        assert!(lookup.get(&missing).is_none());
    }

    #[tokio::test]
    async fn non_workspace_registry_source_retains_missing_index_hint() {
        let locations = IndexLocations::new(
            vec![index(
                "private",
                "private.example.com",
                Some(Origin::Project),
            )],
            vec![],
            false,
        );
        let lookup = IndexLookup::new(&locations, &[], &[]);
        let requirement = uv_pep508::Requirement::<VerbatimParsedUrl>::from_str("demo")
            .expect("valid requirement");
        let sources = [(
            PackageName::from_str("demo").expect("valid package name"),
            [Source::Registry {
                index: IndexName::from_str("private").expect("valid index name"),
                marker: MarkerTree::TRUE,
                extra: None,
                group: None,
            }]
            .into_iter()
            .collect::<Sources>(),
        )]
        .into();
        let cache = Cache::temp().expect("temporary cache");
        let workspace_cache = WorkspaceCache::default();
        let credentials_cache = CredentialsCache::new();
        let error = LoweredRequirement::from_non_workspace_requirement(
            requirement,
            Path::new("."),
            &sources,
            &lookup,
            &cache,
            &workspace_cache,
            &credentials_cache,
        )
        .await
        .next()
        .expect("lowered requirement")
        .expect_err("missing index");

        assert!(matches!(error, LoweringError::MissingIndex { .. }));
        let hint = uv_errors::Hint::hints(&error).into_iter().next();
        assert_eq!(
            hint.as_deref(),
            Some(
                "Index `private` was found in a project-level `uv.toml`, but indexes referenced via `tool.uv.sources` must be defined in the project's `pyproject.toml`"
            )
        );
    }
}
