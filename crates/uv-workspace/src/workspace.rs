//! Resolve the current [`ProjectWorkspace`] or [`Workspace`].

use std::assert_matches;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::hash::BuildHasherDefault;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use glob::{GlobError, MatchOptions, Pattern, PatternError, glob};
use itertools::Itertools;
use rustc_hash::{FxHashSet, FxHasher};
use tracing::{debug, trace, warn};

use uv_cache::Cache;
use uv_configuration::{ActiveEnvironment, DependencyGroupsWithDefaults, ExcludeDependency};
use uv_distribution_types::{Index, Requirement, RequirementSource};
use uv_fs::{CWD, Simplified, normalize_path};
use uv_normalize::{DEV_DEPENDENCIES, DefaultGroups, GroupName, PackageName};
use uv_once_map::OnceMap;
use uv_pep440::VersionSpecifiers;
use uv_pep508::{MarkerTree, VerbatimUrl};
use uv_pypi_types::{ConflictError, Conflicts, SupportedEnvironments, VerbatimParsedUrl};
use uv_static::EnvVars;
use uv_warnings::warn_user_once;

use crate::dependency_groups::{DependencyGroupError, FlatDependencyGroup, FlatDependencyGroups};
use crate::pyproject::{
    BuildConstraintDependency, OverrideDependency, Project, PyProjectToml, PyprojectTomlError,
    Source, Sources, ToolUvSources, ToolUvWorkspace, WorkspaceReference,
};

/// The workspace project environment selected by configuration and command-line options.
#[derive(Debug)]
pub enum ProjectEnvironmentSelection {
    /// Use the workspace's default project environment.
    Default,
    /// A path selected by `UV_PROJECT_ENVIRONMENT`.
    Override(PathBuf),
    /// The active virtual environment selected by `VIRTUAL_ENV` and `--active`.
    Active(PathBuf),
}

impl ProjectEnvironmentSelection {
    /// Returns `true` if the workspace's default project environment was selected.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }

    /// Returns the explicitly selected environment path, if any.
    pub fn explicit_path(&self) -> Option<&Path> {
        match self {
            Self::Default => None,
            Self::Override(path) | Self::Active(path) => Some(path),
        }
    }
}

type WorkspaceMembers = Arc<BTreeMap<PackageName, WorkspaceMember>>;
type FxOnceMap<K, V> = OnceMap<K, V, BuildHasherDefault<FxHasher>>;
type CachedWorkspaceResult = Result<Arc<Workspace>, WorkspaceError>;

/// Cache for workspace discovery.
///
/// Avoid re-reading the `pyproject.toml` files in a workspace for each member by caching the
/// workspace members by their workspace root.
///
/// The cache is indexed both by the workspace root and by the path of each workspace member.
///
/// The cache makes assumptions about [`DiscoveryOptions`]:
/// * `stop_discovery_at` is only used for isolation workspaces in the cache. Otherwise, we avoid
///   traversing into an external cache if `cache` is accidentally included in the workspace member
///   glob.
/// * Only [`MemberDiscovery::All`] results are stored. Successful results can be reused for
///   [`MemberDiscovery::Existing`], which discovers the same members when none are missing.
#[derive(Debug, Default, Clone)]
pub struct WorkspaceCache {
    workspaces: Arc<FxOnceMap<PathBuf, CachedWorkspaceResult>>,
}

impl WorkspaceCache {
    /// Insert a workspace discovery into the cache that may have succeeded or failed.
    ///
    /// Once an error is inserted, it will be returned to all future callers that query the failed
    /// query path.
    fn insert(&self, result: CachedWorkspaceResult, install_path: &Path) {
        match result {
            Ok(workspace) => {
                for package in workspace.packages.values() {
                    // Historically, upward workspace discovery stopped at an intermediate
                    // `pyproject.toml`, so don't map this member to the outer workspace in that
                    // case.
                    // See: <https://github.com/astral-sh/uv/issues/19916>
                    if has_intermediate_pyproject(&workspace.install_path, &package.root) {
                        continue;
                    }
                    self.workspaces
                        .done(package.root.clone(), Ok(workspace.clone()));
                }
                self.workspaces
                    .done(workspace.install_path.clone(), Ok(workspace));
            }
            Err(err) => {
                self.workspaces.done(install_path.to_path_buf(), Err(err));
            }
        }
    }

    /// Register workspace discovery for a root, or wait for an in-flight discovery.
    ///
    /// Calling this function ensures that - given a workspace root - the discovery is only done by
    /// one thread.
    async fn register_or_wait(&self, workspace_root: &PathBuf) -> Option<CachedWorkspaceResult> {
        self.workspaces.register_or_wait(workspace_root).await
    }

    /// Get the cached workspace, if any, from the path to the workspace root or to a member root.
    ///
    /// A successful complete discovery can satisfy [`MemberDiscovery::Existing`]. Cached errors
    /// cannot, since `Existing` intentionally tolerates missing workspace members.
    fn get(
        &self,
        path: &Path,
        member_discovery: &MemberDiscovery,
    ) -> Option<CachedWorkspaceResult> {
        match member_discovery {
            MemberDiscovery::All => self.workspaces.get(path),
            MemberDiscovery::Existing => match self.workspaces.get(path) {
                Some(Ok(workspace)) => Some(Ok(workspace)),
                Some(Err(_)) | None => None,
            },
            MemberDiscovery::None | MemberDiscovery::Ignore(_) => None,
        }
    }

    /// Remove all cached workspace entries for the given workspace root. Used before modifying the
    /// workspace.
    ///
    /// Contract: There are no parallel workspace operations, this is the only thread operating on
    /// workspaces.
    fn invalidate_workspace(&self, workspace: &Workspace) {
        if let Some(Ok(workspace)) = self.workspaces.remove(workspace.install_path()) {
            for member in workspace.packages.values() {
                self.workspaces.remove(&member.root);
            }
        }
    }
}

/// Returns `true` when a `pyproject.toml` sits between the member project directory and the
/// workspace root.
fn has_intermediate_pyproject(workspace_root: &Path, project_dir: &Path) -> bool {
    if project_dir == workspace_root {
        return false;
    }

    let Ok(_) = project_dir.strip_prefix(workspace_root) else {
        return false;
    };

    project_dir
        .ancestors()
        .skip(1)
        .take_while(|ancestor| *ancestor != workspace_root)
        .any(|ancestor| ancestor.join("pyproject.toml").is_file())
}

#[derive(Debug, Clone)]
pub struct WorkspaceError(Arc<WorkspaceErrorKind>);

impl AsRef<WorkspaceErrorKind> for WorkspaceError {
    fn as_ref(&self) -> &WorkspaceErrorKind {
        &self.0
    }
}

impl<T> From<T> for WorkspaceError
where
    T: Into<WorkspaceErrorKind>,
{
    fn from(error: T) -> Self {
        Self(Arc::new(error.into()))
    }
}

impl Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum WorkspaceErrorKind {
    // Workspace structure errors.
    #[error("No `pyproject.toml` found in current directory or any parent directory")]
    MissingPyprojectToml,
    #[error("Workspace member `{}` is missing a `pyproject.toml` (matches: `{}`)", _0.simplified_display(), _1)]
    MissingPyprojectTomlMember(PathBuf, String),
    #[error("No `project` table found in: {}", _0.simplified_display())]
    MissingProject(PathBuf),
    #[error("No workspace found for: {}", _0.simplified_display())]
    MissingWorkspace(PathBuf),
    #[error("The project is marked as unmanaged: {}", _0.simplified_display())]
    NonWorkspace(PathBuf),
    #[error("Workspace member `{}` is a workspace root; register it in `tool.uv.workspace.workspaces` instead", _0.simplified_display())]
    NestedWorkspace(PathBuf),
    #[error("Child workspace `{}` is missing a `pyproject.toml` (matches: `{}`)", _0.simplified_display(), _1)]
    MissingChildWorkspace(PathBuf, String),
    #[error("Child workspace `{}` must contain a `tool.uv.workspace` table", _0.simplified_display())]
    MissingChildWorkspaceDefinition(PathBuf),
    #[error("Child workspace path `{}` is not a directory", _0.simplified_display())]
    ChildWorkspaceNotDirectory(PathBuf),
    #[error("Child workspace pattern `{pattern}` must be a relative path below `{}`", workspace.simplified_display())]
    InvalidChildWorkspacePattern { workspace: PathBuf, pattern: String },
    #[error("Child workspace `{}` must be a strict descendant of `{}`", child.simplified_display(), workspace.simplified_display())]
    ChildWorkspaceOutside { workspace: PathBuf, child: PathBuf },
    #[error("Workspace member `{}` overlaps independently locked child workspace `{}`", member.simplified_display(), child.simplified_display())]
    ChildWorkspaceMemberOverlap { member: PathBuf, child: PathBuf },
    #[error("The workspace does not have a member {}: {}", _0, _1.simplified_display())]
    NoSuchMember(PackageName, PathBuf),
    #[error("Two workspace members are both named `{name}`: `{}` and `{}`", first.simplified_display(), second.simplified_display())]
    DuplicatePackage {
        name: PackageName,
        first: PathBuf,
        second: PathBuf,
    },
    #[error("pyproject.toml section is declared as dynamic, but must be static: `{0}`")]
    DynamicNotAllowed(&'static str),
    #[error(
        "Workspace member `{}` was requested as both `editable = true` and `editable = false`",
        _0
    )]
    EditableConflict(PackageName),
    #[error("Failed to find directories for glob: `{0}`")]
    Pattern(String, #[source] PatternError),
    // Syntax and other errors.
    #[error("Directory walking failed for `tool.uv.workspace.members` glob: `{0}`")]
    GlobWalk(String, #[source] GlobError),
    #[error("Directory walking failed for `tool.uv.workspace.workspaces` glob: `{0}`")]
    ChildWorkspaceGlobWalk(String, #[source] GlobError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Failed to parse: `{}`", _0.user_display())]
    Toml(PathBuf, #[source] Box<PyprojectTomlError>),
    #[error(transparent)]
    Conflicts(#[from] ConflictError),
    // On Windows and Unix, this is not a regular IO failure, but requires e.g. `current_dir` to
    // fail.
    #[error("Failed to normalize workspace member path")]
    Normalize(#[source] std::io::Error),
}

/// An error selecting the default dependency groups for a project.
#[derive(Debug, thiserror::Error)]
pub enum DefaultGroupsError {
    #[error("Package `{0}` not found in workspace")]
    MissingPackage(PackageName),
    #[error(
        "Default group `{0}` (from `tool.uv.default-groups`) is not defined in the project's `dependency-groups` table"
    )]
    MissingGroup(GroupName),
}

#[derive(Debug, Default, Clone, Hash, PartialEq, Eq)]
pub enum MemberDiscovery {
    /// Discover all workspace members.
    #[default]
    All,
    /// Discover workspace members that are present, but ignore missing members.
    Existing,
    /// Don't discover any workspace members.
    None,
    /// Discover workspace members, but ignore the given paths.
    Ignore(BTreeSet<PathBuf>),
}

#[derive(Debug, Default, Clone, Hash, PartialEq, Eq)]
pub struct DiscoveryOptions {
    /// The path to stop discovery at.
    ///
    /// Assumption: This is only used for directories in the cache to avoid them escaping the cache.
    /// If you want to use it for other cases too, you need to also update the cache handling in
    /// the workspace discovery glob walking.
    pub stop_discovery_at: Option<PathBuf>,
    /// The strategy to use when discovering workspace members.
    pub members: MemberDiscovery,
}

pub type RequiresPythonSources = BTreeMap<(PackageName, Option<GroupName>), VersionSpecifiers>;

pub type Editability = Option<bool>;

/// A workspace, consisting of a root directory and members. See [`ProjectWorkspace`].
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct Workspace {
    /// The path to the workspace root.
    ///
    /// The workspace root is the directory containing the top level `pyproject.toml` with
    /// the `uv.tool.workspace`, or the `pyproject.toml` in an implicit single workspace project.
    install_path: PathBuf,
    /// The members of the workspace.
    packages: WorkspaceMembers,
    /// The workspace members that are required by other members, and whether they were requested
    /// as editable.
    required_members: BTreeMap<PackageName, Editability>,
    /// The sources table from the workspace `pyproject.toml`.
    ///
    /// This table is overridden by the project sources.
    sources: BTreeMap<PackageName, Sources>,
    /// The index table from the workspace `pyproject.toml`.
    ///
    /// This table is overridden by the project indexes.
    indexes: Vec<Index>,
    /// The `pyproject.toml` of the workspace root.
    pyproject_toml: PyProjectToml,
}

impl Workspace {
    /// Find the workspace containing the given path.
    ///
    /// Unlike the [`ProjectWorkspace`] discovery, this does not require a current project. It also
    /// always uses absolute path, i.e., this method only supports discovering the main workspace.
    ///
    /// Steps of workspace discovery: Start by looking at the closest `pyproject.toml`:
    /// * If it's an explicit workspace root: Collect workspace from this root, we're done.
    /// * If it's also not a project: Error, must be either a workspace root or a project.
    /// * Otherwise, try to find an explicit workspace root above:
    ///   * If an explicit workspace root exists: Collect workspace from this root, we're done.
    ///   * If there is no explicit workspace: We have a single project workspace, we're done.
    ///
    /// Note that there are two kinds of workspace roots: projects, and non-project roots.
    /// The non-project roots lack a `[project]` table, and so are not themselves projects, as in:
    /// ```toml
    /// [tool.uv.workspace]
    /// members = ["packages/*"]
    ///
    /// [tool.uv]
    /// dev-dependencies = ["ruff"]
    /// ```
    pub async fn discover(
        path: &Path,
        options: &DiscoveryOptions,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
    ) -> Result<Arc<Self>, WorkspaceError> {
        let path = std::path::absolute(path)
            .map_err(WorkspaceErrorKind::Normalize)?
            .clone();
        let path = normalize_path(&path);

        let project_path = path
            .ancestors()
            .find(|path| path.join("pyproject.toml").is_file())
            .ok_or(WorkspaceErrorKind::MissingPyprojectToml)?
            .to_path_buf();

        // Fast path: The workspace was already fully discovered.
        // It's possible that there are two separate discoveries for the same workspace going on
        // at the same time from different roots, both failing this check. These cases are fine, we
        // synchronize them after finding the workspace root and allow only one of them to perform
        // the full discovery.
        if let Some(workspace) = workspace_cache.get(&project_path, &options.members) {
            return workspace;
        }

        let pyproject_path = project_path.join("pyproject.toml");
        let contents = fs_err::tokio::read_to_string(&pyproject_path).await?;
        let pyproject_toml = PyProjectToml::from_string(contents, &pyproject_path)
            .map_err(|err| WorkspaceErrorKind::Toml(pyproject_path.clone(), Box::new(err)))?;

        // Check if the project is explicitly marked as unmanaged.
        if pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.managed)
            == Some(false)
        {
            debug!(
                "Project `{}` is marked as unmanaged",
                project_path.simplified_display()
            );
            return Err(WorkspaceError::from(WorkspaceErrorKind::NonWorkspace(
                project_path,
            )));
        }

        // Check if the current project is also an explicit workspace root.
        let explicit_root = pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
            .map(|workspace| {
                (
                    project_path.clone(),
                    workspace.clone(),
                    pyproject_toml.clone(),
                )
            });

        let (workspace_root, workspace_definition, workspace_pyproject_toml) =
            if let Some(workspace) = explicit_root {
                // We have found the explicit root immediately.
                workspace
            } else if pyproject_toml.project.is_none() {
                // Without a project, it can't be an implicit root
                return Err(WorkspaceError::from(WorkspaceErrorKind::MissingProject(
                    pyproject_path,
                )));
            } else if let Some(workspace) = find_workspace(&project_path, options, cache).await? {
                // We have found an explicit root above.
                workspace
            } else {
                // Support implicit single project workspaces.
                (
                    project_path.clone(),
                    ToolUvWorkspace::default(),
                    pyproject_toml.clone(),
                )
            };

        if options.members == MemberDiscovery::All {
            // Ensure that workspace discovery runs only once for any given workspace root.
            // If two threads start at different packages at the same time, they only read their
            // package `pyproject.toml` and the workspace root `pyproject.toml` before arriving
            // here. At this point, only one thread can continue and the other waits, then uses the
            // cached workspace.
            if let Some(workspace) = workspace_cache.register_or_wait(&workspace_root).await {
                return workspace;
            }
        }

        debug!(
            "Found workspace root: `{}`",
            workspace_root.simplified_display()
        );

        // Unlike in `ProjectWorkspace` discovery, we might be in a non-project root without
        // being in any specific project.
        let current_project = pyproject_toml
            .project
            .clone()
            .map(|project| WorkspaceMember {
                root: project_path,
                project,
                pyproject_toml,
            });

        let result = Self::build(
            workspace_root.clone(),
            workspace_definition,
            workspace_pyproject_toml,
            current_project,
            options,
            cache,
        )
        .await;
        if options.members == MemberDiscovery::All {
            workspace_cache.insert(result.clone(), &workspace_root);
        }
        result
    }

    /// Set the current project to the given workspace member.
    ///
    /// Returns `None` if the package is not part of the workspace.
    fn with_current_project(
        self: Arc<Self>,
        package_name: PackageName,
    ) -> Option<ProjectWorkspace> {
        let member = self.packages.get(&package_name)?;
        Some(ProjectWorkspace {
            project_root: member.root().clone(),
            project_name: package_name,
            workspace: self,
        })
    }

    /// Set the [`ProjectWorkspace`] for a given workspace member.
    ///
    /// Assumes that the project name is unchanged in the updated [`PyProjectToml`], and that the
    /// caller holds the only reference to this workspace (to avoid a situation where another part
    /// of uv still holds a reference to the old workspace structure).
    fn update_member(
        self: Arc<Self>,
        package_name: &PackageName,
        pyproject_toml: PyProjectToml,
    ) -> Result<Option<Arc<Self>>, WorkspaceError> {
        debug_assert_eq!(
            Arc::strong_count(&self),
            1,
            "cannot modify workspace still in use",
        );

        let slf = Arc::unwrap_or_clone(self);
        let mut packages = slf.packages;

        let Some(member) = Arc::make_mut(&mut packages).get_mut(package_name) else {
            return Ok(None);
        };

        if member.root == slf.install_path {
            // If the member is also the workspace root, update _both_ the member entry and the
            // root `pyproject.toml`.
            let workspace_pyproject_toml = pyproject_toml.clone();

            // Refresh the workspace sources.
            let workspace_sources = workspace_pyproject_toml
                .tool
                .clone()
                .and_then(|tool| tool.uv)
                .and_then(|uv| uv.sources)
                .map(ToolUvSources::into_inner)
                .unwrap_or_default();

            // Set the `pyproject.toml` for the member.
            member.pyproject_toml = pyproject_toml;

            // Recompute required_members with the updated data
            let required_members = Self::collect_required_members(
                &packages,
                &workspace_sources,
                &workspace_pyproject_toml,
            )?;

            let workspace = Self {
                pyproject_toml: workspace_pyproject_toml,
                sources: workspace_sources,
                packages,
                required_members,
                ..slf
            };
            Ok(Some(Arc::new(workspace)))
        } else {
            // Set the `pyproject.toml` for the member.
            member.pyproject_toml = pyproject_toml;

            // Recompute required_members with the updated member data
            let required_members =
                Self::collect_required_members(&packages, &slf.sources, &slf.pyproject_toml)?;

            let workspace = Self {
                packages,
                required_members,
                ..slf
            };
            Ok(Some(Arc::new(workspace)))
        }
    }

    /// Returns `true` if the workspace has a non-project root.
    pub fn is_non_project(&self) -> bool {
        !self
            .packages
            .values()
            .any(|member| *member.root() == self.install_path)
    }

    /// Returns the set of all workspace members.
    pub fn members_requirements(&self) -> impl Iterator<Item = Requirement> + '_ {
        self.packages.iter().filter_map(|(name, member)| {
            let url = VerbatimUrl::from_absolute_path(&member.root).expect("path is valid URL");
            Some(Requirement {
                name: member.pyproject_toml.project.as_ref()?.name.clone(),
                extras: Box::new([]),
                groups: Box::new([]),
                marker: MarkerTree::TRUE,
                source: if member
                    .pyproject_toml()
                    .is_package(!self.is_required_member(name))
                {
                    RequirementSource::Directory {
                        install_path: member.root.clone().into_boxed_path(),
                        editable: Some(
                            self.required_members
                                .get(name)
                                .copied()
                                .flatten()
                                .unwrap_or(true),
                        ),
                        r#virtual: Some(false),
                        url,
                    }
                } else {
                    RequirementSource::Directory {
                        install_path: member.root.clone().into_boxed_path(),
                        editable: Some(false),
                        r#virtual: Some(true),
                        url,
                    }
                },
                origin: None,
            })
        })
    }

    /// The workspace members that are required my another member of the workspace.
    pub fn required_members(&self) -> &BTreeMap<PackageName, Editability> {
        &self.required_members
    }

    /// Compute the workspace members that are required by another member of the workspace, and
    /// determine whether they should be installed as editable or non-editable.
    ///
    /// N.B. this checks if a workspace member is required by inspecting `tool.uv.source` entries,
    /// but does not actually check if the source is _used_, which could result in false positives
    /// but is easier to compute.
    fn collect_required_members(
        packages: &BTreeMap<PackageName, WorkspaceMember>,
        sources: &BTreeMap<PackageName, Sources>,
        pyproject_toml: &PyProjectToml,
    ) -> Result<BTreeMap<PackageName, Editability>, WorkspaceError> {
        let mut required_members = BTreeMap::new();

        for (package, sources) in sources
            .iter()
            .filter(|(name, _)| {
                pyproject_toml
                    .project
                    .as_ref()
                    .is_none_or(|project| project.name != **name)
            })
            .chain(
                packages
                    .iter()
                    .filter_map(|(name, member)| {
                        member
                            .pyproject_toml
                            .tool
                            .as_ref()
                            .and_then(|tool| tool.uv.as_ref())
                            .and_then(|uv| uv.sources.as_ref())
                            .map(ToolUvSources::inner)
                            .map(move |sources| {
                                sources
                                    .iter()
                                    .filter(move |(source_name, _)| name != *source_name)
                            })
                    })
                    .flatten(),
            )
        {
            for source in sources.iter() {
                let Source::Workspace {
                    workspace: WorkspaceReference::Bool(true),
                    editable,
                    ..
                } = &source
                else {
                    continue;
                };
                let existing = required_members.insert(package.clone(), *editable);
                if let Some(Some(existing)) = existing {
                    if let Some(editable) = editable {
                        // If there are conflicting `editable` values, raise an error.
                        if existing != *editable {
                            return Err(WorkspaceError::from(
                                WorkspaceErrorKind::EditableConflict(package.clone()),
                            ));
                        }
                    }
                }
            }
        }

        Ok(required_members)
    }

    /// Whether a given workspace member is required by another member.
    fn is_required_member(&self, name: &PackageName) -> bool {
        self.required_members().contains_key(name)
    }

    /// Returns the set of all workspace member dependency groups.
    pub fn group_requirements(&self) -> impl Iterator<Item = Requirement> + '_ {
        self.packages.iter().filter_map(|(name, member)| {
            let url = VerbatimUrl::from_absolute_path(&member.root).expect("path is valid URL");

            let groups = {
                let mut groups = member
                    .pyproject_toml
                    .dependency_groups
                    .as_ref()
                    .map(|groups| groups.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                if member
                    .pyproject_toml
                    .tool
                    .as_ref()
                    .and_then(|tool| tool.uv.as_ref())
                    .and_then(|uv| uv.dev_dependencies.as_ref())
                    .is_some()
                {
                    groups.push(DEV_DEPENDENCIES.clone());
                    groups.sort_unstable();
                }
                groups
            };
            if groups.is_empty() {
                return None;
            }

            let value = self.required_members.get(name);
            let is_required_member = value.is_some();
            let editability = value.copied().flatten();

            Some(Requirement {
                name: member.pyproject_toml.project.as_ref()?.name.clone(),
                extras: Box::new([]),
                groups: groups.into_boxed_slice(),
                marker: MarkerTree::TRUE,
                source: if member.pyproject_toml().is_package(!is_required_member) {
                    RequirementSource::Directory {
                        install_path: member.root.clone().into_boxed_path(),
                        editable: Some(editability.unwrap_or(true)),
                        r#virtual: Some(false),
                        url,
                    }
                } else {
                    RequirementSource::Directory {
                        install_path: member.root.clone().into_boxed_path(),
                        editable: Some(false),
                        r#virtual: Some(true),
                        url,
                    }
                },
                origin: None,
            })
        })
    }

    /// Returns the set of supported environments for the workspace.
    pub fn environments(&self) -> Option<&SupportedEnvironments> {
        self.pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.environments.as_ref())
    }

    /// Returns the set of required platforms for the workspace.
    pub fn required_environments(&self) -> Option<&SupportedEnvironments> {
        self.pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.required_environments.as_ref())
    }

    /// Returns the set of conflicts for the workspace.
    pub fn conflicts(&self) -> Result<Conflicts, WorkspaceError> {
        let mut conflicting = Conflicts::empty();
        if self.is_non_project()
            && let Some(root_conflicts) = self
                .pyproject_toml
                .tool
                .as_ref()
                .and_then(|tool| tool.uv.as_ref())
                .and_then(|uv| uv.conflicts.as_ref())
        {
            let mut root_conflicts = root_conflicts.to_conflicts()?;
            conflicting.append(&mut root_conflicts);
        }
        for member in self.packages.values() {
            conflicting.append(&mut member.pyproject_toml.conflicts()?);
        }
        Ok(conflicting)
    }

    /// Returns an iterator over the `requires-python` values for each member of the workspace.
    pub fn requires_python(
        &self,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<RequiresPythonSources, DependencyGroupError> {
        let mut requires = RequiresPythonSources::new();
        for (name, member) in self.packages() {
            // Get the top-level requires-python for this package, which is always active
            //
            // Arguably we could check groups.prod() to disable this, since, the requires-python
            // of the project is *technically* not relevant if you're doing `--only-group`, but,
            // that would be a big surprising change, so let's *not* do that until someone asks!
            let top_requires = member
                .pyproject_toml()
                .project
                .as_ref()
                .and_then(|project| project.requires_python.as_ref())
                .map(|requires_python| ((name.to_owned(), None), requires_python.clone()));
            requires.extend(top_requires);

            // Get the requires-python for each enabled group on this package
            // We need to do full flattening here because include-group can transfer requires-python
            let dependency_groups =
                FlatDependencyGroups::from_pyproject_toml(member.root(), &member.pyproject_toml)?;
            let group_requires =
                dependency_groups
                    .into_iter()
                    .filter_map(move |(group_name, flat_group)| {
                        if groups.contains(&group_name) {
                            flat_group.requires_python.map(|requires_python| {
                                ((name.to_owned(), Some(group_name)), requires_python)
                            })
                        } else {
                            None
                        }
                    });
            requires.extend(group_requires);
        }
        Ok(requires)
    }

    /// Returns any requirements that are exclusive to the workspace root, i.e., not included in
    /// any of the workspace members.
    ///
    /// For now, there are no such requirements.
    pub fn requirements(&self) -> Vec<uv_pep508::Requirement<VerbatimParsedUrl>> {
        Vec::new()
    }

    /// Returns any dependency groups that are exclusive to the workspace root, i.e., not included
    /// in any of the workspace members.
    ///
    /// For workspaces with non-`[project]` roots, returns the dependency groups defined in the
    /// corresponding `pyproject.toml`.
    ///
    /// Otherwise, returns an empty list.
    pub fn workspace_dependency_groups(
        &self,
    ) -> Result<BTreeMap<GroupName, FlatDependencyGroup>, DependencyGroupError> {
        if self
            .packages
            .values()
            .any(|member| *member.root() == self.install_path)
        {
            // If the workspace has an explicit root, the root is a member, so we don't need to
            // include any root-only requirements.
            Ok(BTreeMap::default())
        } else {
            // Otherwise, return the dependency groups in the non-project workspace root.
            let dependency_groups = FlatDependencyGroups::from_pyproject_toml(
                &self.install_path,
                &self.pyproject_toml,
            )?;
            Ok(dependency_groups.into_inner())
        }
    }

    /// Returns the set of overrides for the workspace.
    pub fn overrides(&self) -> Vec<OverrideDependency> {
        let Some(overrides) = self
            .pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.override_dependencies.as_ref())
        else {
            return vec![];
        };
        overrides.clone()
    }

    /// Returns the set of dependency exclusions for the workspace.
    pub fn exclude_dependencies(&self) -> Vec<ExcludeDependency> {
        let Some(excludes) = self
            .pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.exclude_dependencies.as_ref())
        else {
            return vec![];
        };
        excludes.clone()
    }

    /// Returns the set of constraints for the workspace.
    pub fn constraints(&self) -> Vec<uv_pep508::Requirement<VerbatimParsedUrl>> {
        let Some(constraints) = self
            .pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.constraint_dependencies.as_ref())
        else {
            return vec![];
        };
        constraints.clone()
    }

    /// Returns the set of build constraints for the workspace.
    pub fn build_constraints(&self) -> Vec<BuildConstraintDependency> {
        let Some(build_constraints) = self
            .pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.build_constraint_dependencies.as_ref())
        else {
            return vec![];
        };
        build_constraints.clone()
    }

    /// The path to the workspace root, the directory containing the top level `pyproject.toml` with
    /// the `uv.tool.workspace`, or the `pyproject.toml` in an implicit single workspace project.
    pub fn install_path(&self) -> &PathBuf {
        &self.install_path
    }

    /// The workspace project environment selection.
    ///
    /// If `UV_PROJECT_ENVIRONMENT` is set, it will take precedence. If a relative path is provided,
    /// it is resolved relative to the install path.
    ///
    /// If `active` is [`ActiveEnvironment::Prefer`], the `VIRTUAL_ENV` variable will be preferred.
    /// If it is [`ActiveEnvironment::Ignore`], warnings about mismatches between the active
    /// environment and the project environment will be silenced.
    pub fn environment_selection(&self, active: ActiveEnvironment) -> ProjectEnvironmentSelection {
        /// Resolve the `UV_PROJECT_ENVIRONMENT` value, if any.
        fn from_project_environment_variable(workspace: &Workspace) -> Option<PathBuf> {
            let value = std::env::var_os(EnvVars::UV_PROJECT_ENVIRONMENT)?;

            if value.is_empty() {
                return None;
            }

            let path = PathBuf::from(value);
            if path.is_absolute() {
                return Some(path);
            }

            // Resolve the path relative to the install path.
            Some(workspace.install_path.join(path))
        }

        /// Resolve the `VIRTUAL_ENV` variable, if any.
        fn from_virtual_env_variable() -> Option<PathBuf> {
            let value = std::env::var_os(EnvVars::VIRTUAL_ENV)?;

            if value.is_empty() {
                return None;
            }

            let path = PathBuf::from(value);
            if path.is_absolute() {
                return Some(path);
            }

            // Resolve the path relative to current directory.
            // Note this differs from `UV_PROJECT_ENVIRONMENT`
            Some(CWD.join(path))
        }

        let selection = from_project_environment_variable(self)
            .map(ProjectEnvironmentSelection::Override)
            .unwrap_or(ProjectEnvironmentSelection::Default);
        let project_environment_path = selection
            .explicit_path()
            .map_or_else(|| self.install_path.join(".venv"), Path::to_path_buf);

        // Warn if it conflicts with `VIRTUAL_ENV`
        if let Some(from_virtual_env) = from_virtual_env_variable() {
            let matches_project =
                uv_fs::is_same_file_allow_missing(&from_virtual_env, &project_environment_path)
                    .unwrap_or(false);
            match active {
                ActiveEnvironment::Prefer => {
                    if !matches_project {
                        debug!(
                            "Using active virtual environment `{}` instead of project environment `{}`",
                            from_virtual_env.user_display(),
                            project_environment_path.user_display()
                        );
                    }
                    return ProjectEnvironmentSelection::Active(from_virtual_env);
                }
                ActiveEnvironment::Ignore => {}
                ActiveEnvironment::Warn if !matches_project => {
                    warn_user_once!(
                        "`VIRTUAL_ENV={}` does not match the project environment path `{}` and will be ignored; use `--active` to target the active environment instead",
                        from_virtual_env.user_display(),
                        project_environment_path.user_display()
                    );
                }
                ActiveEnvironment::Warn => {}
            }
        } else {
            if active == ActiveEnvironment::Prefer {
                debug!(
                    "Use of the active virtual environment was requested, but `VIRTUAL_ENV` is not set"
                );
            }
        }

        selection
    }

    /// The members of the workspace.
    pub fn packages(&self) -> &BTreeMap<PackageName, WorkspaceMember> {
        &self.packages
    }

    /// The sources table from the workspace `pyproject.toml`.
    pub fn sources(&self) -> &BTreeMap<PackageName, Sources> {
        &self.sources
    }

    /// The index table from the workspace `pyproject.toml`.
    pub fn indexes(&self) -> &[Index] {
        &self.indexes
    }

    /// The `pyproject.toml` of the workspace.
    pub fn pyproject_toml(&self) -> &PyProjectToml {
        &self.pyproject_toml
    }

    /// Return the default dependency groups for the workspace root.
    pub fn default_groups(&self) -> Result<DefaultGroups, DefaultGroupsError> {
        self.pyproject_toml.default_groups()
    }

    /// Returns `true` if the path is excluded by the workspace.
    pub fn excludes(&self, project_path: &Path) -> Result<bool, WorkspaceError> {
        if let Some(workspace) = self
            .pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
        {
            is_excluded_from_workspace(project_path, &self.install_path, workspace)
        } else {
            Ok(false)
        }
    }

    /// Returns `true` if the path is included by the workspace.
    pub fn includes(&self, project_path: &Path) -> Result<bool, WorkspaceError> {
        if let Some(workspace) = self
            .pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
        {
            is_included_in_workspace(project_path, &self.install_path, workspace)
        } else {
            Ok(false)
        }
    }

    /// Return the nearest ancestor that explicitly registers this workspace.
    ///
    /// This lookup does not read a lockfile or discover the parent's other members. Callers can
    /// defer it until a new resolution is needed, so an existing child lockfile remains usable
    /// without its parent.
    pub async fn parent_workspace_root(
        &self,
        options: &DiscoveryOptions,
        cache: &Cache,
    ) -> Result<Option<PathBuf>, WorkspaceError> {
        find_parent_workspace_root(
            &self.install_path,
            &self.pyproject_toml,
            Some(self.packages()),
            options,
            cache,
        )
        .await
    }

    /// Return the independently locked workspaces registered directly by this workspace.
    ///
    /// The returned paths are canonical, sorted, and deduplicated. A workspace matched by a more
    /// distant ancestor's glob is omitted when a nearer ancestor also registers it. Reading a
    /// child root does not discover its members or read its lockfile.
    pub async fn child_workspace_roots(
        &self,
        options: &DiscoveryOptions,
        cache: &Cache,
    ) -> Result<Vec<PathBuf>, WorkspaceError> {
        let Some(workspace) = workspace_definition(&self.pyproject_toml) else {
            return Ok(Vec::new());
        };
        let children = WorkspaceChildren::new(&self.install_path, workspace)?;
        if children.is_empty() {
            return Ok(Vec::new());
        }

        let workspace_root = fs_err::tokio::canonicalize(&self.install_path).await?;
        let mut child_roots = Vec::new();
        for (child_root, pyproject_toml) in children.validated_roots(options, cache).await? {
            let parent =
                find_parent_workspace_root(&child_root, &pyproject_toml, None, options, cache)
                    .await?;
            if parent.as_ref() == Some(&workspace_root) {
                child_roots.push(child_root);
            }
        }
        Ok(child_roots)
    }

    /// Collect the workspace member projects and build the workspace object.
    async fn build(
        workspace_root: PathBuf,
        workspace_definition: ToolUvWorkspace,
        workspace_pyproject_toml: PyProjectToml,
        current_project: Option<WorkspaceMember>,
        options: &DiscoveryOptions,
        cache: &Cache,
    ) -> Result<Arc<Self>, WorkspaceError> {
        trace!(
            "Discovering workspace members for: `{}`",
            &workspace_root.simplified_display()
        );
        let workspace_members = Self::collect_members_only(
            &workspace_root,
            &workspace_definition,
            &workspace_pyproject_toml,
            options,
            cache,
        )
        .await?;
        let mut workspace_members = Arc::new(workspace_members);

        // For the cases such as `MemberDiscovery::None`, add the current project if missing.
        if let Some(root_member) = current_project
            && !workspace_members.contains_key(&root_member.project.name)
        {
            assert_matches!(
                options.members,
                MemberDiscovery::None | MemberDiscovery::Ignore(_)
            );
            debug!(
                "Adding current workspace member: `{}`",
                root_member.root.simplified_display()
            );

            Arc::make_mut(&mut workspace_members)
                .insert(root_member.project.name.clone(), root_member);
        }

        let workspace_sources = workspace_pyproject_toml
            .tool
            .clone()
            .and_then(|tool| tool.uv)
            .and_then(|uv| uv.sources)
            .map(ToolUvSources::into_inner)
            .unwrap_or_default();

        let workspace_indexes = workspace_pyproject_toml
            .tool
            .clone()
            .and_then(|tool| tool.uv)
            .and_then(|uv| uv.index)
            .unwrap_or_default();

        let required_members = Self::collect_required_members(
            &workspace_members,
            &workspace_sources,
            &workspace_pyproject_toml,
        )?;

        let dev_dependencies_members = workspace_members
            .values()
            .filter_map(|member| {
                member
                    .pyproject_toml
                    .tool
                    .as_ref()
                    .and_then(|tool| tool.uv.as_ref())
                    .and_then(|uv| uv.dev_dependencies.as_ref())
                    .map(|_| format!("`{}`", member.root().join("pyproject.toml").user_display()))
            })
            .join(", ");
        if !dev_dependencies_members.is_empty() {
            warn_user_once!(
                "The `tool.uv.dev-dependencies` field (used in {}) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead",
                dev_dependencies_members
            );
        }

        let workspace = Self {
            install_path: workspace_root,
            packages: workspace_members,
            required_members,
            sources: workspace_sources,
            indexes: workspace_indexes,
            pyproject_toml: workspace_pyproject_toml,
        };
        Ok(Arc::new(workspace))
    }

    async fn collect_members_only(
        workspace_root: &PathBuf,
        workspace_definition: &ToolUvWorkspace,
        workspace_pyproject_toml: &PyProjectToml,
        options: &DiscoveryOptions,
        cache: &Cache,
    ) -> Result<BTreeMap<PackageName, WorkspaceMember>, WorkspaceError> {
        let mut workspace_members = BTreeMap::new();
        let child_workspaces = WorkspaceChildren::new(workspace_root, workspace_definition)?;
        // A parent validates its child roots without collecting their independently resolved
        // members. Partial discovery can omit roots that are not present in an isolated checkout.
        let child_roots: BTreeSet<PathBuf> = match &options.members {
            MemberDiscovery::None => BTreeSet::new(),
            MemberDiscovery::All | MemberDiscovery::Existing | MemberDiscovery::Ignore(_) => {
                child_workspaces
                    .validated_roots(options, cache)
                    .await?
                    .into_keys()
                    .collect()
            }
        };
        // Avoid reading a `pyproject.toml` more than once.
        let mut seen = FxHashSet::default();

        let external_cache_root = options
            .stop_discovery_at
            .is_none()
            .then(|| {
                // We may receive an uninitialized cache with a relative cache root.
                let cache_root = if cache.root().is_absolute() {
                    cache.root().to_path_buf()
                } else {
                    CWD.join(cache.root())
                };
                normalize_path(&cache_root).into_owned()
            })
            .filter(|cache_root| !workspace_root.starts_with(cache_root));

        // Add the project at the workspace root, if it exists and if it's distinct from the current
        // project. If it is the current project, it is added as such in the next step.
        if let Some(project) = &workspace_pyproject_toml.project {
            debug!(
                "Adding root workspace member: `{}`",
                workspace_root.simplified_display()
            );

            seen.insert(workspace_root.clone());
            workspace_members.insert(
                project.name.clone(),
                WorkspaceMember {
                    root: workspace_root.clone(),
                    project: project.clone(),
                    pyproject_toml: workspace_pyproject_toml.clone(),
                },
            );
        }

        // Prepare exclusions only after finding a member that is not explicitly ignored.
        let mut exclusions = None;

        // Add all other workspace members.
        for member_glob in workspace_definition.members.as_deref().unwrap_or_default() {
            // Normalize the member glob to remove leading `./` and other relative path components
            let normalized_glob = normalize_path(Path::new(member_glob.as_str()));
            let absolute_glob = PathBuf::from(glob::Pattern::escape(
                workspace_root.simplified().to_string_lossy().as_ref(),
            ))
            .join(normalized_glob.as_ref())
            .to_string_lossy()
            .to_string();
            for member_root in glob(&absolute_glob)
                .map_err(|err| WorkspaceErrorKind::Pattern(absolute_glob.clone(), err))?
            {
                let member_root = member_root
                    .map_err(|err| WorkspaceErrorKind::GlobWalk(absolute_glob.clone(), err))?;
                if external_cache_root
                    .as_ref()
                    .is_some_and(|cache_root| member_root.starts_with(cache_root))
                {
                    debug!(
                        "Ignoring cache directory while discovering workspace members: `{}`",
                        member_root.simplified_display()
                    );
                    continue;
                }
                if !seen.insert(member_root.clone()) {
                    continue;
                }
                let member_root =
                    std::path::absolute(&member_root).map_err(WorkspaceErrorKind::Normalize)?;

                // If the directory is explicitly ignored, skip it.
                let skip = match &options.members {
                    MemberDiscovery::All | MemberDiscovery::Existing => false,
                    MemberDiscovery::None => true,
                    MemberDiscovery::Ignore(ignore) => ignore.contains(member_root.as_path()),
                };
                if skip {
                    debug!(
                        "Ignoring workspace member: `{}`",
                        member_root.simplified_display()
                    );
                    continue;
                }

                // If the member is excluded, ignore it.
                if exclusions
                    .get_or_insert_with(|| {
                        WorkspaceExclusions::new(workspace_root, workspace_definition)
                    })
                    .as_ref()
                    .map_err(WorkspaceError::clone)?
                    .matches(&member_root)
                {
                    debug!(
                        "Ignoring workspace member: `{}`",
                        member_root.simplified_display()
                    );
                    continue;
                }

                trace!(
                    "Processing workspace member: `{}`",
                    member_root.user_display()
                );

                // Read the member `pyproject.toml`.
                let pyproject_path = member_root.join("pyproject.toml");
                let contents = match fs_err::tokio::read_to_string(&pyproject_path).await {
                    Ok(contents) => contents,
                    Err(err) => {
                        let metadata = match fs_err::metadata(&member_root) {
                            Ok(metadata) => metadata,
                            Err(err)
                                if matches!(options.members, MemberDiscovery::Existing)
                                    && err.kind() == std::io::ErrorKind::NotFound =>
                            {
                                debug!(
                                    "Ignoring missing workspace member: `{}`",
                                    member_root.simplified_display()
                                );
                                continue;
                            }
                            Err(err) => return Err(err.into()),
                        };
                        if !metadata.is_dir() {
                            warn!(
                                "Ignoring non-directory workspace member: `{}`",
                                member_root.simplified_display()
                            );
                            continue;
                        }

                        // A directory exists, but it doesn't contain a `pyproject.toml`.
                        if err.kind() == std::io::ErrorKind::NotFound {
                            // If the directory is hidden, skip it.
                            if member_root
                                .file_name()
                                .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."))
                            {
                                debug!(
                                    "Ignoring hidden workspace member: `{}`",
                                    member_root.simplified_display()
                                );
                                continue;
                            }

                            // If the directory only contains gitignored files
                            // (e.g., `__pycache__`), skip it.
                            if has_only_gitignored_files(&member_root) {
                                debug!(
                                    "Ignoring workspace member with only gitignored files: `{}`",
                                    member_root.simplified_display()
                                );
                                continue;
                            }

                            if matches!(options.members, MemberDiscovery::Existing) {
                                debug!(
                                    "Ignoring missing workspace member: `{}`",
                                    member_root.simplified_display()
                                );
                                continue;
                            }

                            return Err(WorkspaceError::from(
                                WorkspaceErrorKind::MissingPyprojectTomlMember(
                                    member_root,
                                    member_glob.to_string(),
                                ),
                            ));
                        }

                        return Err(err.into());
                    }
                };
                let pyproject_toml = PyProjectToml::from_string(contents, &pyproject_path)
                    .map_err(|err| {
                        WorkspaceErrorKind::Toml(pyproject_path.clone(), Box::new(err))
                    })?;

                // Check if the current project is explicitly marked as unmanaged.
                if pyproject_toml
                    .tool
                    .as_ref()
                    .and_then(|tool| tool.uv.as_ref())
                    .and_then(|uv| uv.managed)
                    == Some(false)
                {
                    if let Some(project) = pyproject_toml.project.as_ref() {
                        debug!(
                            "Project `{}` is marked as unmanaged; omitting from workspace members",
                            project.name
                        );
                    } else {
                        debug!(
                            "Workspace member at `{}` is marked as unmanaged; omitting from workspace members",
                            member_root.simplified_display()
                        );
                    }
                    continue;
                }

                // Extract the package name.
                let Some(project) = pyproject_toml.project.clone() else {
                    return Err(WorkspaceError::from(WorkspaceErrorKind::MissingProject(
                        pyproject_path,
                    )));
                };

                debug!(
                    "Adding discovered workspace member: `{}`",
                    member_root.simplified_display()
                );

                if let Some(existing) = workspace_members.insert(
                    project.name.clone(),
                    WorkspaceMember {
                        root: member_root.clone(),
                        project,
                        pyproject_toml,
                    },
                ) {
                    return Err(WorkspaceError::from(WorkspaceErrorKind::DuplicatePackage {
                        name: existing.project.name,
                        first: existing.root.clone(),
                        second: member_root,
                    }));
                }
            }
        }

        // An independently locked child cannot also contribute packages to this workspace.
        if !child_roots.is_empty() {
            for member in workspace_members.values() {
                if member.root() == workspace_root {
                    continue;
                }
                let member_root = fs_err::tokio::canonicalize(member.root()).await?;
                if let Some(child_root) = child_roots
                    .iter()
                    .find(|child_root| member_root.starts_with(child_root))
                {
                    return Err(WorkspaceErrorKind::ChildWorkspaceMemberOverlap {
                        member: member.root().clone(),
                        child: child_root.clone(),
                    }
                    .into());
                }
            }
        }

        // A nested workspace selected as an ordinary member still cannot be resolved as part of
        // this workspace.
        for member in workspace_members.values() {
            if member.root() != workspace_root
                && member
                    .pyproject_toml
                    .tool
                    .as_ref()
                    .and_then(|tool| tool.uv.as_ref())
                    .and_then(|uv| uv.workspace.as_ref())
                    .is_some()
            {
                return Err(WorkspaceError::from(WorkspaceErrorKind::NestedWorkspace(
                    member.root.clone(),
                )));
            }
        }
        Ok(workspace_members)
    }
}

/// A project in a workspace.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct WorkspaceMember {
    /// The path to the project root.
    root: PathBuf,
    /// The `[project]` table, from the `pyproject.toml` of the project found at
    /// `<root>/pyproject.toml`.
    project: Project,
    /// The `pyproject.toml` of the project, found at `<root>/pyproject.toml`.
    pyproject_toml: PyProjectToml,
}

impl WorkspaceMember {
    /// The path to the project root.
    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// The `[project]` table, from the `pyproject.toml` of the project found at
    /// `<root>/pyproject.toml`.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The `pyproject.toml` of the project, found at `<root>/pyproject.toml`.
    pub fn pyproject_toml(&self) -> &PyProjectToml {
        &self.pyproject_toml
    }

    /// Return the default dependency groups for this workspace member.
    fn default_groups(&self) -> Result<DefaultGroups, DefaultGroupsError> {
        self.pyproject_toml.default_groups()
    }
}

/// The current project and the workspace it is part of, with all of the workspace members.
///
/// # Structure
///
/// The workspace root is a directory with a `pyproject.toml`, all members need to be below that
/// directory. The workspace root defines members and exclusions. All packages below it must either
/// be a member or excluded. The workspace root can be a package itself or a virtual manifest.
///
/// For a simple single package project, the workspace root is implicitly the current project root
/// and the workspace has only this single member. Otherwise, a workspace root is declared through
/// a `tool.uv.workspace` section.
///
/// A workspace itself does not declare dependencies, instead one member is the current project used
/// as main requirement.
///
/// Each member is a directory with a `pyproject.toml` that contains a `[project]` section. Each
/// member is a Python package, with a name, a version and dependencies. Workspace members can
/// depend on other workspace members (`foo = { workspace = true }`). You can consider the
/// workspace another package source or index, similar to `--find-links`.
///
/// # Usage
///
/// There a two main usage patterns: A root package and helpers, and the flat workspace.
///
/// Root package and helpers:
///
/// ```text
/// albatross
/// ├── packages
/// │   ├── provider_a
/// │   │   ├── pyproject.toml
/// │   │   └── src
/// │   │       └── provider_a
/// │   │           ├── __init__.py
/// │   │           └── foo.py
/// │   └── provider_b
/// │       ├── pyproject.toml
/// │       └── src
/// │           └── provider_b
/// │               ├── __init__.py
/// │               └── bar.py
/// ├── pyproject.toml
/// ├── Readme.md
/// ├── uv.lock
/// └── src
///     └── albatross
///         ├── __init__.py
///         └── main.py
/// ```
///
/// Flat workspace:
///
/// ```text
/// albatross
/// ├── packages
/// │   ├── albatross
/// │   │   ├── pyproject.toml
/// │   │   └── src
/// │   │       └── albatross
/// │   │           ├── __init__.py
/// │   │           └── main.py
/// │   ├── provider_a
/// │   │   ├── pyproject.toml
/// │   │   └── src
/// │   │       └── provider_a
/// │   │           ├── __init__.py
/// │   │           └── foo.py
/// │   └── provider_b
/// │       ├── pyproject.toml
/// │       └── src
/// │           └── provider_b
/// │               ├── __init__.py
/// │               └── bar.py
/// ├── pyproject.toml
/// ├── Readme.md
/// └── uv.lock
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(serde::Serialize))]
pub struct ProjectWorkspace {
    /// The path to the project root.
    project_root: PathBuf,
    /// The name of the package.
    project_name: PackageName,
    /// The workspace the project is part of.
    workspace: Arc<Workspace>,
}

impl ProjectWorkspace {
    fn from_cache(
        project_root: &Path,
        options: &DiscoveryOptions,
        cache: &WorkspaceCache,
    ) -> Result<Option<Self>, WorkspaceError> {
        let workspace = match cache.get(project_root, &options.members) {
            Some(Ok(workspace)) => workspace,
            Some(Err(error)) => return Err(error),
            None => return Ok(None),
        };
        let Some((project_name, _member)) = workspace
            .packages
            .iter()
            .find(|(_project_name, member)| member.root() == project_root)
        else {
            return Ok(None);
        };

        Ok(Some(Self {
            project_root: project_root.to_path_buf(),
            project_name: project_name.clone(),
            workspace,
        }))
    }

    /// Find the current project and workspace, given the current directory.
    ///
    /// `stop_discovery_at` must be either `None` or an ancestor of the current directory. If set,
    /// only directories between the current path and `stop_discovery_at` are considered.
    pub async fn discover(
        path: &Path,
        options: &DiscoveryOptions,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
    ) -> Result<Self, WorkspaceError> {
        assert!(
            path.is_absolute(),
            "project workspace discovery with relative path"
        );
        let project_root = path
            .ancestors()
            .take_while(|path| {
                // Only walk up the given directory, if any.
                options
                    .stop_discovery_at
                    .as_deref()
                    .and_then(Path::parent)
                    .is_none_or(|stop_discovery_at| stop_discovery_at != *path)
            })
            .find(|path| path.join("pyproject.toml").is_file())
            .ok_or_else(|| WorkspaceErrorKind::MissingPyprojectToml)?;

        debug!(
            "Found project root: `{}`",
            project_root.simplified_display()
        );

        Self::from_project_root(project_root, options, cache, workspace_cache).await
    }

    /// Discover the workspace starting from the directory containing the `pyproject.toml`.
    async fn from_project_root(
        project_root: &Path,
        options: &DiscoveryOptions,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
    ) -> Result<Self, WorkspaceError> {
        if let Some(project) = Self::from_cache(project_root, options, workspace_cache)? {
            return Ok(project);
        }

        // Read the current `pyproject.toml`.
        let pyproject_path = project_root.join("pyproject.toml");

        let contents = fs_err::tokio::read_to_string(&pyproject_path).await?;
        let pyproject_toml = PyProjectToml::from_string(contents, &pyproject_path)
            .map_err(|err| WorkspaceErrorKind::Toml(pyproject_path.clone(), Box::new(err)))?;

        // It must have a `[project]` table.
        let project = pyproject_toml
            .project
            .clone()
            .ok_or_else(|| WorkspaceErrorKind::MissingProject(pyproject_path))?;

        Self::from_project(
            project_root,
            &project,
            &pyproject_toml,
            options,
            cache,
            workspace_cache,
        )
        .await
    }

    /// If the current directory contains a `pyproject.toml` with a `project` table, discover the
    /// workspace and return it, otherwise it is a dynamic path dependency and we return `Ok(None)`.
    pub async fn from_maybe_project_root(
        project_root: &Path,
        options: &DiscoveryOptions,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
    ) -> Result<Option<Self>, WorkspaceError> {
        if let Some(project) = Self::from_cache(project_root, options, workspace_cache)? {
            return Ok(Some(project));
        }

        // Read the `pyproject.toml`.
        let pyproject_path = project_root.join("pyproject.toml");
        let Ok(contents) = fs_err::tokio::read_to_string(&pyproject_path).await else {
            // No `pyproject.toml`, but there may still be a `setup.py` or `setup.cfg`.
            return Ok(None);
        };
        let pyproject_toml = PyProjectToml::from_string(contents, &pyproject_path)
            .map_err(|err| WorkspaceErrorKind::Toml(pyproject_path.clone(), Box::new(err)))?;

        // Extract the `[project]` metadata.
        let Some(project) = pyproject_toml.project.clone() else {
            // We have to build to get the metadata.
            return Ok(None);
        };

        match Self::from_project(
            project_root,
            &project,
            &pyproject_toml,
            options,
            cache,
            workspace_cache,
        )
        .await
        {
            Ok(workspace) => Ok(Some(workspace)),
            Err(error) if matches!(error.as_ref(), WorkspaceErrorKind::NonWorkspace(_)) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Returns the directory containing the closest `pyproject.toml` that defines the current
    /// project.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Returns the [`PackageName`] of the current project.
    pub fn project_name(&self) -> &PackageName {
        &self.project_name
    }

    /// Returns the [`Workspace`] containing the current project.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Returns the current project as a [`WorkspaceMember`].
    pub fn current_project(&self) -> &WorkspaceMember {
        &self.workspace().packages[&self.project_name]
    }

    /// Return the default dependency groups for the current project.
    fn default_groups(&self) -> Result<DefaultGroups, DefaultGroupsError> {
        self.current_project().default_groups()
    }

    /// Set the `pyproject.toml` for the current project.
    ///
    /// Assumes that the project name is unchanged in the updated [`PyProjectToml`].
    fn update_member(self, pyproject_toml: PyProjectToml) -> Result<Option<Self>, WorkspaceError> {
        let Some(workspace) =
            Workspace::update_member(self.workspace, &self.project_name, pyproject_toml)?
        else {
            return Ok(None);
        };
        Ok(Some(Self { workspace, ..self }))
    }

    /// Find the workspace for a project.
    async fn from_project(
        install_path: &Path,
        project: &Project,
        project_pyproject_toml: &PyProjectToml,
        options: &DiscoveryOptions,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
    ) -> Result<Self, WorkspaceError> {
        let project_path = std::path::absolute(install_path)
            .map_err(WorkspaceErrorKind::Normalize)?
            .clone();
        let project_path = normalize_path(&project_path);

        // Check if workspaces are explicitly disabled for the project.
        if project_pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.managed)
            == Some(false)
        {
            debug!("Project `{}` is marked as unmanaged", project.name);
            return Err(WorkspaceError::from(WorkspaceErrorKind::NonWorkspace(
                project_path.to_path_buf(),
            )));
        }

        if let Some(project) = Self::from_cache(&project_path, options, workspace_cache)? {
            return Ok(project);
        }

        // Check if the current project is also an explicit workspace root.
        let mut workspace = project_pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
            .map(|workspace| {
                (
                    project_path.to_path_buf(),
                    workspace.clone(),
                    project_pyproject_toml.clone(),
                )
            });

        if workspace.is_none() {
            // The project isn't an explicit workspace root, check if we're a regular workspace
            // member by looking for an explicit workspace root above.
            workspace = find_workspace(&project_path, options, cache).await?;
        }

        let current_project = WorkspaceMember {
            root: project_path.to_path_buf(),
            project: project.clone(),
            pyproject_toml: project_pyproject_toml.clone(),
        };

        let Some((workspace_root, workspace_definition, workspace_pyproject_toml)) = workspace
        else {
            // The project isn't an explicit workspace root, but there's also no workspace root
            // above it, so the project is an implicit workspace root identical to the project root.
            debug!("No workspace root found, using project root");

            let current_project_as_members = Arc::new(BTreeMap::from_iter([(
                project.name.clone(),
                current_project,
            )]));
            let workspace_sources = BTreeMap::default();
            let required_members = Workspace::collect_required_members(
                &current_project_as_members,
                &workspace_sources,
                project_pyproject_toml,
            )?;

            let workspace = Workspace {
                install_path: project_path.to_path_buf(),
                packages: current_project_as_members,
                required_members,
                // There may be package sources, but we don't need to duplicate them into the
                // workspace sources.
                sources: workspace_sources,
                indexes: Vec::default(),
                pyproject_toml: project_pyproject_toml.clone(),
            };
            let workspace = Arc::new(workspace);
            if options.members == MemberDiscovery::All {
                workspace_cache.insert(Ok(workspace.clone()), &project_path);
            }
            return Ok(Self {
                project_root: project_path.to_path_buf(),
                project_name: project.name.clone(),
                workspace,
            });
        };

        if options.members == MemberDiscovery::All {
            // Ensure that workspace discovery runs only once for any given workspace root.
            if let Some(workspace) = workspace_cache.register_or_wait(&workspace_root).await {
                return workspace.map(|workspace| Self {
                    project_root: project_path.to_path_buf(),
                    project_name: project.name.clone(),
                    workspace,
                });
            }
        }

        debug!(
            "Found workspace root: `{}`",
            workspace_root.simplified_display()
        );

        let result = Workspace::build(
            workspace_root.clone(),
            workspace_definition,
            workspace_pyproject_toml,
            Some(current_project),
            options,
            cache,
        )
        .await;
        if options.members == MemberDiscovery::All {
            workspace_cache.insert(result.clone(), &workspace_root);
        }

        Ok(Self {
            project_root: project_path.to_path_buf(),
            project_name: project.name.clone(),
            workspace: result?,
        })
    }
}

/// The independently locked workspaces declared by one workspace root.
struct WorkspaceChildren<'workspace> {
    workspace_root: &'workspace Path,
    patterns: Vec<ChildWorkspacePattern>,
}

struct ChildWorkspacePattern {
    relative: PathBuf,
    absolute: Pattern,
    original: String,
    has_glob: bool,
}

impl ChildWorkspacePattern {
    fn new(workspace_root: &Path, child_glob: &str) -> Result<Self, WorkspaceError> {
        let relative = normalize_path(Path::new(child_glob));
        if relative.as_os_str().is_empty()
            || relative.components().any(|component| match component {
                Component::Prefix(_) | Component::RootDir | Component::ParentDir => true,
                Component::CurDir | Component::Normal(_) => false,
            })
        {
            return Err(WorkspaceErrorKind::InvalidChildWorkspacePattern {
                workspace: workspace_root.to_path_buf(),
                pattern: child_glob.to_string(),
            }
            .into());
        }

        Ok(Self {
            absolute: Self::absolute_pattern(workspace_root, relative.as_ref())?,
            relative: relative.into_owned(),
            original: child_glob.to_string(),
            has_glob: child_glob.contains(['*', '?', '[']),
        })
    }

    fn absolute_pattern(
        workspace_root: &Path,
        child_glob: &Path,
    ) -> Result<Pattern, WorkspaceError> {
        let absolute = PathBuf::from(Pattern::escape(
            workspace_root.simplified().to_string_lossy().as_ref(),
        ))
        .join(child_glob);
        let absolute = normalize_path(&absolute);
        let absolute = absolute.to_string_lossy();
        Pattern::new(&absolute)
            .map_err(|err| WorkspaceErrorKind::Pattern(absolute.to_string(), err).into())
    }
}

impl<'workspace> WorkspaceChildren<'workspace> {
    fn new(
        workspace_root: &'workspace Path,
        workspace: &ToolUvWorkspace,
    ) -> Result<Self, WorkspaceError> {
        let patterns = workspace
            .workspaces
            .iter()
            .flatten()
            .map(|child_glob| ChildWorkspacePattern::new(workspace_root, child_glob.as_str()))
            .collect::<Result<_, WorkspaceError>>()?;

        Ok(Self {
            workspace_root,
            patterns,
        })
    }

    /// Inspect only the registration table before committing to an ancestor as a parent.
    fn from_registration(
        workspace_root: &'workspace Path,
        pyproject_toml: &toml::Value,
        child_root: &Path,
        canonical_child_root: &Path,
    ) -> Result<Option<Self>, WorkspaceError> {
        let Some(patterns) = pyproject_toml
            .get("tool")
            .and_then(|tool| tool.get("uv"))
            .and_then(|uv| uv.get("workspace"))
            .and_then(|workspace| workspace.get("workspaces"))
            .and_then(toml::Value::as_array)
        else {
            return Ok(None);
        };
        let options = MatchOptions {
            require_literal_separator: true,
            ..MatchOptions::new()
        };
        let mut children = Vec::new();
        for pattern in patterns.iter().filter_map(toml::Value::as_str) {
            match ChildWorkspacePattern::new(workspace_root, pattern) {
                Ok(pattern) => children.push(pattern),
                Err(error) => {
                    // Invalid declarations in unrelated ancestors do not change standalone
                    // resolution. A declaration that directly identifies this child is still an
                    // error, even when its path is absolute or escapes the parent.
                    if let Ok(pattern) =
                        ChildWorkspacePattern::absolute_pattern(workspace_root, Path::new(pattern))
                        && (pattern.matches_path_with(child_root, options)
                            || pattern.matches_path_with(canonical_child_root, options))
                    {
                        return Err(error);
                    }
                }
            }
        }
        Ok(Some(Self {
            workspace_root,
            patterns: children,
        }))
    }

    fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Validate child workspace roots without discovering their members.
    async fn validated_roots(
        &self,
        options: &DiscoveryOptions,
        cache: &Cache,
    ) -> Result<BTreeMap<PathBuf, PyProjectToml>, WorkspaceError> {
        if self.is_empty() {
            return Ok(BTreeMap::new());
        }
        let allow_missing = match &options.members {
            MemberDiscovery::Existing => true,
            MemberDiscovery::All | MemberDiscovery::None | MemberDiscovery::Ignore(_) => false,
        };
        let mut roots = BTreeMap::new();
        for (child_root, pattern) in self.roots(options, cache, !allow_missing).await? {
            let pyproject_toml = match read_child_workspace(&child_root, &pattern).await {
                Ok(pyproject_toml) => pyproject_toml,
                Err(error) => {
                    if allow_missing
                        && let WorkspaceErrorKind::MissingChildWorkspace(..) = error.as_ref()
                    {
                        continue;
                    }
                    return Err(error);
                }
            };
            validate_explicit_child_workspace(&child_root, &pyproject_toml)?;
            roots.insert(child_root, pyproject_toml);
        }
        Ok(roots)
    }

    /// Expand child-root paths without reading the children's metadata.
    async fn roots(
        &self,
        options: &DiscoveryOptions,
        cache: &Cache,
        require_present: bool,
    ) -> Result<BTreeMap<PathBuf, String>, WorkspaceError> {
        let workspace_root = fs_err::tokio::canonicalize(self.workspace_root).await?;
        let external_cache = discovery_cache_boundary(options, cache).filter(|boundary| {
            !boundary.contains(self.workspace_root) && !boundary.contains(&workspace_root)
        });
        let mut roots = BTreeMap::new();

        for pattern in &self.patterns {
            let mut found = false;
            for child_root in glob(pattern.absolute.as_str())
                .map_err(|err| WorkspaceErrorKind::Pattern(pattern.absolute.to_string(), err))?
            {
                let child_root = child_root.map_err(|err| {
                    WorkspaceErrorKind::ChildWorkspaceGlobWalk(pattern.absolute.to_string(), err)
                })?;
                found = true;
                if external_cache
                    .as_ref()
                    .is_some_and(|boundary| boundary.contains(&child_root))
                {
                    continue;
                }
                let metadata = match fs_err::tokio::metadata(&child_root).await {
                    Ok(metadata) => metadata,
                    Err(err) if !require_present && err.kind() == std::io::ErrorKind::NotFound => {
                        continue;
                    }
                    Err(err) => return Err(err.into()),
                };
                if !metadata.is_dir() {
                    if !pattern.has_glob {
                        return Err(
                            WorkspaceErrorKind::ChildWorkspaceNotDirectory(child_root).into()
                        );
                    }
                    continue;
                }
                let child_root = fs_err::tokio::canonicalize(child_root).await?;
                if external_cache
                    .as_ref()
                    .is_some_and(|boundary| boundary.contains(&child_root))
                {
                    debug!(
                        "Ignoring cache directory while discovering child workspaces: `{}`",
                        child_root.simplified_display()
                    );
                    continue;
                }
                validate_child_workspace_containment(&workspace_root, &child_root)?;
                roots
                    .entry(child_root)
                    .or_insert_with(|| pattern.original.clone());
            }
            if require_present && !found && !pattern.has_glob {
                return Err(WorkspaceErrorKind::MissingChildWorkspace(
                    self.workspace_root.join(&pattern.relative),
                    pattern.original.clone(),
                )
                .into());
            }
        }
        Ok(roots)
    }

    /// Match one child without reading or validating unrelated child metadata.
    async fn contains(
        &self,
        child_root: &Path,
        canonical_child_root: &Path,
        discovery_options: &DiscoveryOptions,
        cache: &Cache,
    ) -> Result<bool, WorkspaceError> {
        let options = MatchOptions {
            require_literal_separator: true,
            ..MatchOptions::new()
        };
        if self
            .patterns
            .iter()
            .any(|pattern| pattern.absolute.matches_path_with(child_root, options))
        {
            return Ok(true);
        }

        // A registered path can be an in-tree symlink to the child's canonical root. Inspecting
        // only the paths makes lookup consistent when the child is opened through either spelling.
        let external_cache = discovery_cache_boundary(discovery_options, cache);
        for pattern in &self.patterns {
            for candidate in glob(pattern.absolute.as_str())
                .map_err(|err| WorkspaceErrorKind::Pattern(pattern.absolute.to_string(), err))?
            {
                let candidate = candidate.map_err(|err| {
                    WorkspaceErrorKind::ChildWorkspaceGlobWalk(pattern.absolute.to_string(), err)
                })?;
                if external_cache
                    .as_ref()
                    .is_some_and(|boundary| boundary.contains(&candidate))
                {
                    continue;
                }
                match fs_err::tokio::canonicalize(candidate).await {
                    Ok(candidate)
                        if candidate == canonical_child_root
                            && !external_cache
                                .as_ref()
                                .is_some_and(|boundary| boundary.contains(&candidate)) =>
                    {
                        return Ok(true);
                    }
                    Ok(_) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err.into()),
                }
            }
        }
        Ok(false)
    }
}

fn workspace_definition(pyproject_toml: &PyProjectToml) -> Option<&ToolUvWorkspace> {
    pyproject_toml
        .tool
        .as_ref()
        .and_then(|tool| tool.uv.as_ref())
        .and_then(|uv| uv.workspace.as_ref())
}

fn validate_explicit_child_workspace(
    child_root: &Path,
    pyproject_toml: &PyProjectToml,
) -> Result<(), WorkspaceError> {
    if pyproject_toml
        .tool
        .as_ref()
        .and_then(|tool| tool.uv.as_ref())
        .and_then(|uv| uv.managed)
        == Some(false)
    {
        return Err(WorkspaceErrorKind::NonWorkspace(child_root.to_path_buf()).into());
    }
    if workspace_definition(pyproject_toml).is_none() {
        return Err(
            WorkspaceErrorKind::MissingChildWorkspaceDefinition(child_root.to_path_buf()).into(),
        );
    }
    Ok(())
}

fn validate_child_workspace_containment(
    workspace_root: &Path,
    child_root: &Path,
) -> Result<(), WorkspaceError> {
    if child_root == workspace_root || !child_root.starts_with(workspace_root) {
        return Err(WorkspaceErrorKind::ChildWorkspaceOutside {
            workspace: workspace_root.to_path_buf(),
            child: child_root.to_path_buf(),
        }
        .into());
    }
    Ok(())
}

async fn read_child_workspace(
    child_root: &Path,
    pattern: &str,
) -> Result<PyProjectToml, WorkspaceError> {
    let pyproject_path = child_root.join("pyproject.toml");
    let contents = match fs_err::tokio::read_to_string(&pyproject_path).await {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(WorkspaceErrorKind::MissingChildWorkspace(
                child_root.to_path_buf(),
                pattern.to_string(),
            )
            .into());
        }
        Err(err) => return Err(err.into()),
    };
    PyProjectToml::from_string(contents, &pyproject_path)
        .map_err(|err| WorkspaceErrorKind::Toml(pyproject_path, Box::new(err)).into())
}

struct DiscoveryCacheBoundary {
    root: PathBuf,
    canonical_root: PathBuf,
}

impl DiscoveryCacheBoundary {
    fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.root) || path.starts_with(&self.canonical_root)
    }
}

/// The configured cache is an isolation boundary unless the caller supplied a tighter boundary.
fn discovery_cache_boundary(
    options: &DiscoveryOptions,
    cache: &Cache,
) -> Option<DiscoveryCacheBoundary> {
    if options.stop_discovery_at.is_some() {
        return None;
    }
    let cache_root = if cache.root().is_absolute() {
        cache.root().to_path_buf()
    } else {
        CWD.join(cache.root())
    };
    let cache_root = normalize_path(&cache_root).into_owned();
    Some(DiscoveryCacheBoundary {
        canonical_root: fs_err::canonicalize(&cache_root).unwrap_or_else(|_| cache_root.clone()),
        root: cache_root,
    })
}

async fn find_parent_workspace_root(
    child_root: &Path,
    child_pyproject_toml: &PyProjectToml,
    child_members: Option<&BTreeMap<PackageName, WorkspaceMember>>,
    options: &DiscoveryOptions,
    cache: &Cache,
) -> Result<Option<PathBuf>, WorkspaceError> {
    let child_root = std::path::absolute(child_root).map_err(WorkspaceErrorKind::Normalize)?;
    let child_root = normalize_path(&child_root);
    let canonical_child_root = fs_err::tokio::canonicalize(child_root.as_ref()).await?;
    let canonical_stop = match options.stop_discovery_at.as_deref() {
        Some(stop) => Some(fs_err::tokio::canonicalize(stop).await?),
        None => None,
    };
    if canonical_stop
        .as_ref()
        .is_some_and(|stop| !canonical_child_root.starts_with(stop))
    {
        return Ok(None);
    }
    if discovery_cache_boundary(options, cache).is_some_and(|boundary| {
        boundary.contains(child_root.as_ref()) || boundary.contains(&canonical_child_root)
    }) {
        return Ok(None);
    }

    for parent_root in child_root
        .ancestors()
        .take_while(|path| {
            options
                .stop_discovery_at
                .as_deref()
                .and_then(Path::parent)
                .is_none_or(|stop_discovery_at| stop_discovery_at != *path)
        })
        .skip(1)
    {
        let canonical_parent_root = if let Some(stop) = canonical_stop.as_ref() {
            let parent = fs_err::tokio::canonicalize(parent_root).await?;
            if !parent.starts_with(stop) {
                break;
            }
            Some(parent)
        } else {
            None
        };
        let pyproject_path = parent_root.join("pyproject.toml");
        if !pyproject_path.is_file() {
            continue;
        }
        let contents = match fs_err::tokio::read_to_string(&pyproject_path).await {
            Ok(contents) => contents,
            Err(_) => {
                debug!(
                    "Ignoring unreadable ancestor `pyproject.toml` during parent workspace lookup: `{}`",
                    pyproject_path.simplified_display()
                );
                continue;
            }
        };
        let Ok(registration) = toml::from_str::<toml::Value>(&contents) else {
            debug!(
                "Ignoring malformed ancestor `pyproject.toml` during parent workspace lookup: `{}`",
                pyproject_path.simplified_display()
            );
            continue;
        };
        let Some(children) = WorkspaceChildren::from_registration(
            parent_root,
            &registration,
            child_root.as_ref(),
            &canonical_child_root,
        )?
        else {
            continue;
        };
        if children.is_empty()
            || !children
                .contains(child_root.as_ref(), &canonical_child_root, options, cache)
                .await?
        {
            continue;
        }

        // Once the registration matches, the owning workspace is authoritative and all of its
        // configuration must be valid.
        let pyproject_toml = PyProjectToml::from_string(contents, &pyproject_path)
            .map_err(|err| WorkspaceErrorKind::Toml(pyproject_path, Box::new(err)))?;
        let Some(workspace) = workspace_definition(&pyproject_toml) else {
            continue;
        };
        WorkspaceChildren::new(parent_root, workspace)?;

        let canonical_parent_root = match canonical_parent_root {
            Some(parent) => parent,
            None => fs_err::tokio::canonicalize(parent_root).await?,
        };
        validate_child_workspace_containment(&canonical_parent_root, &canonical_child_root)?;
        validate_explicit_child_workspace(&canonical_child_root, child_pyproject_toml)?;
        validate_explicit_child_workspace(&canonical_parent_root, &pyproject_toml)?;
        validate_parent_member_overlap(
            parent_root,
            workspace,
            &canonical_child_root,
            child_members,
            options,
            cache,
        )
        .await?;
        return Ok(Some(canonical_parent_root));
    }
    Ok(None)
}

/// Check the parent's member paths without discovering unrelated member metadata.
async fn validate_parent_member_overlap(
    parent_root: &Path,
    parent_definition: &ToolUvWorkspace,
    child_root: &Path,
    child_members: Option<&BTreeMap<PackageName, WorkspaceMember>>,
    options: &DiscoveryOptions,
    cache: &Cache,
) -> Result<(), WorkspaceError> {
    let canonical_parent_root = fs_err::tokio::canonicalize(parent_root).await?;
    let external_cache = discovery_cache_boundary(options, cache).filter(|boundary| {
        !boundary.contains(parent_root) && !boundary.contains(&canonical_parent_root)
    });
    let exclusions = WorkspaceExclusions::new(parent_root, parent_definition)?;
    let mut canonical_child_members = FxHashSet::default();
    if let Some(child_members) = child_members {
        for member in child_members.values() {
            canonical_child_members.insert(fs_err::tokio::canonicalize(member.root()).await?);
        }
    }

    for member_glob in parent_definition.members.iter().flatten() {
        let normalized_glob = normalize_path(Path::new(member_glob.as_str()));
        let absolute_glob = PathBuf::from(Pattern::escape(
            parent_root.simplified().to_string_lossy().as_ref(),
        ))
        .join(normalized_glob.as_ref())
        .to_string_lossy()
        .to_string();
        for member_root in glob(&absolute_glob)
            .map_err(|err| WorkspaceErrorKind::Pattern(absolute_glob.clone(), err))?
        {
            let member_root = member_root
                .map_err(|err| WorkspaceErrorKind::GlobWalk(absolute_glob.clone(), err))?;
            if !member_root.is_dir()
                || exclusions.matches(&member_root)
                || external_cache
                    .as_ref()
                    .is_some_and(|boundary| boundary.contains(&member_root))
            {
                continue;
            }
            let canonical_member_root = fs_err::tokio::canonicalize(&member_root).await?;
            if external_cache
                .as_ref()
                .is_some_and(|boundary| boundary.contains(&canonical_member_root))
            {
                continue;
            }
            if canonical_member_root.starts_with(child_root)
                || canonical_child_members.contains(&canonical_member_root)
            {
                return Err(WorkspaceErrorKind::ChildWorkspaceMemberOverlap {
                    member: member_root,
                    child: child_root.to_path_buf(),
                }
                .into());
            }
        }
    }
    Ok(())
}

/// Find the workspace root above the current project, if any.
async fn find_workspace(
    project_root: &Path,
    options: &DiscoveryOptions,
    cache: &Cache,
) -> Result<Option<(PathBuf, ToolUvWorkspace, PyProjectToml)>, WorkspaceError> {
    let external_cache_root = if options.stop_discovery_at.is_none() {
        // We may receive an uninitialized cache with a relative cache root.
        let cache_root = if cache.root().is_absolute() {
            cache.root().to_path_buf()
        } else {
            CWD.join(cache.root())
        };
        Some(normalize_path(&cache_root).into_owned())
    } else {
        None
    };
    // Avoid panicking in the odd (unsupported) case that uv is running inside the cache dir.
    if let Some(cache_root) = external_cache_root
        && project_root.starts_with(cache_root)
    {
        debug!(
            "Project is contained in cache directory: `{}`",
            project_root.simplified_display()
        );
        return Ok(None);
    }

    // Skip 1 to ignore the current project itself.
    for workspace_root in project_root
        .ancestors()
        .take_while(|path| {
            // Only walk up the given directory, if any.
            options
                .stop_discovery_at
                .as_deref()
                .and_then(Path::parent)
                .is_none_or(|stop_discovery_at| stop_discovery_at != *path)
        })
        .skip(1)
    {
        let pyproject_path = workspace_root.join("pyproject.toml");
        if !pyproject_path.is_file() {
            continue;
        }
        trace!(
            "Found `pyproject.toml` at: `{}`",
            pyproject_path.simplified_display()
        );

        // Read the `pyproject.toml`.
        let contents = fs_err::tokio::read_to_string(&pyproject_path).await?;
        let pyproject_toml = PyProjectToml::from_string(contents, &pyproject_path)
            .map_err(|err| WorkspaceErrorKind::Toml(pyproject_path.clone(), Box::new(err)))?;

        return if let Some(workspace) = pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
        {
            if !is_included_in_workspace(project_root, workspace_root, workspace)? {
                debug!(
                    "Found workspace root `{}`, but project is not included",
                    workspace_root.simplified_display()
                );
                return Ok(None);
            }

            if is_excluded_from_workspace(project_root, workspace_root, workspace)? {
                debug!(
                    "Found workspace root `{}`, but project is excluded",
                    workspace_root.simplified_display()
                );
                return Ok(None);
            }

            // We found a workspace root.
            Ok(Some((
                workspace_root.to_path_buf(),
                workspace.clone(),
                pyproject_toml,
            )))
        } else if pyproject_toml.project.is_some() {
            // We're in a directory of another project, e.g. tests or examples.
            // Example:
            // ```
            // albatross
            // ├── examples
            // │   └── bird-feeder [CURRENT DIRECTORY]
            // │       ├── pyproject.toml
            // │       └── src
            // │           └── bird_feeder
            // │               └── __init__.py
            // ├── pyproject.toml
            // └── src
            //     └── albatross
            //         └── __init__.py
            // ```
            // The current project is the example (non-workspace) `bird-feeder` in `albatross`,
            // we ignore all `albatross` is doing and any potential workspace it might be
            // contained in.
            debug!(
                "Project is contained in non-workspace project: `{}`",
                workspace_root.simplified_display()
            );
            Ok(None)
        } else {
            // We require that a `project.toml` file either declares a workspace or a project.
            warn!(
                "`pyproject.toml` does not contain a `project` table: `{}`",
                pyproject_path.simplified_display()
            );
            Ok(None)
        };
    }

    Ok(None)
}

/// Check if a directory only contains files that are ignored.
///
/// Returns `true` if walking the directory while respecting `.gitignore` and `.ignore` rules
/// yields no files, indicating that any files present (e.g., `__pycache__`) are all ignored.
fn has_only_gitignored_files(path: &Path) -> bool {
    let walker = ignore::WalkBuilder::new(path)
        .hidden(false)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker {
        let Ok(entry) = entry else {
            // If we can't read an entry, assume non-ignored content exists.
            return false;
        };

        // Skip directories.
        if entry.path().is_dir() {
            continue;
        }

        // A non-ignored entry exists.
        return false;
    }

    true
}

/// Check if we're in the `tool.uv.workspace.excluded` of a workspace.
fn is_excluded_from_workspace(
    project_path: &Path,
    workspace_root: &Path,
    workspace: &ToolUvWorkspace,
) -> Result<bool, WorkspaceError> {
    Ok(WorkspaceExclusions::new(workspace_root, workspace)?.matches(project_path))
}

/// Compiled workspace exclusion patterns.
#[derive(Debug)]
struct WorkspaceExclusions<'workspace> {
    workspace_root: &'workspace Path,
    patterns: Vec<WorkspaceExclusion<'workspace>>,
}

/// A workspace exclusion that can reuse its parsed pattern or requires normalization.
#[derive(Debug)]
enum WorkspaceExclusion<'workspace> {
    Relative(&'workspace Pattern),
    Absolute(Pattern),
}

impl<'workspace> WorkspaceExclusions<'workspace> {
    /// Compile the normalized workspace exclusion patterns.
    fn new(
        workspace_root: &'workspace Path,
        workspace: &'workspace ToolUvWorkspace,
    ) -> Result<Self, WorkspaceError> {
        let patterns = workspace
            .exclude
            .iter()
            .flatten()
            .map(|exclude_glob| Self::compile_pattern(workspace_root, exclude_glob))
            .collect::<Result<_, _>>()?;

        Ok(Self {
            workspace_root,
            patterns,
        })
    }

    /// Return whether any workspace exclusion matches the project path.
    fn matches(&self, project_path: &Path) -> bool {
        let relative_path = project_path
            .simplified()
            .strip_prefix(self.workspace_root.simplified())
            .ok();

        self.patterns.iter().any(|pattern| match pattern {
            WorkspaceExclusion::Relative(pattern) => {
                relative_path.is_some_and(|relative_path| pattern.matches_path(relative_path))
            }
            WorkspaceExclusion::Absolute(pattern) => pattern.matches_path(project_path),
        })
    }

    /// Reuse an already compiled relative pattern or compile its normalized absolute equivalent.
    fn compile_pattern(
        workspace_root: &Path,
        exclude_glob: &'workspace Pattern,
    ) -> Result<WorkspaceExclusion<'workspace>, WorkspaceError> {
        // Normalize the exclude glob to remove leading `./` and other relative path components.
        let normalized_glob = normalize_path(Path::new(exclude_glob.as_str()));
        if matches!(&normalized_glob, Cow::Borrowed(_)) && normalized_glob.is_relative() {
            return Ok(WorkspaceExclusion::Relative(exclude_glob));
        }

        let absolute_glob = PathBuf::from(Pattern::escape(
            workspace_root.simplified().to_string_lossy().as_ref(),
        ))
        .join(normalized_glob.as_ref());
        let absolute_glob = absolute_glob.to_string_lossy();
        Pattern::new(&absolute_glob)
            .map(WorkspaceExclusion::Absolute)
            .map_err(|err| WorkspaceErrorKind::Pattern(absolute_glob.to_string(), err).into())
    }
}

/// Check if we're in the `tool.uv.workspace.members` of a workspace.
fn is_included_in_workspace(
    project_path: &Path,
    workspace_root: &Path,
    workspace: &ToolUvWorkspace,
) -> Result<bool, WorkspaceError> {
    let options = MatchOptions {
        require_literal_separator: true,
        ..MatchOptions::new()
    };
    for member_glob in workspace.members.iter().flatten() {
        // Normalize the member glob to remove leading `./` and other relative path components
        let normalized_glob = normalize_path(Path::new(member_glob.as_str()));
        let absolute_glob = PathBuf::from(glob::Pattern::escape(
            workspace_root.simplified().to_string_lossy().as_ref(),
        ))
        .join(normalized_glob);
        let absolute_glob = absolute_glob.to_string_lossy();
        let include_pattern = glob::Pattern::new(&absolute_glob)
            .map_err(|err| WorkspaceErrorKind::Pattern(absolute_glob.to_string(), err))?;
        if include_pattern.matches_path_with(project_path, options) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A project that can be discovered.
///
/// The project could be a package within a workspace, a real workspace root, or a non-project
/// workspace root, which can define its own dev dependencies.
#[derive(Debug, Clone)]
pub enum VirtualProject {
    /// A project (which could be a workspace root or member).
    Project(ProjectWorkspace),
    /// A non-project workspace root.
    NonProject(Arc<Workspace>),
}

impl VirtualProject {
    /// Find the current project or virtual workspace root, given the current directory.
    ///
    /// Similar to calling [`ProjectWorkspace::discover`] with a fallback to [`Workspace::discover`],
    /// but avoids rereading the `pyproject.toml` (and relying on error-handling as control flow).
    ///
    /// This method requires an absolute path and panics otherwise, i.e. this method only supports
    /// discovering the main workspace.
    pub async fn discover(
        path: &Path,
        options: &DiscoveryOptions,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
    ) -> Result<Self, WorkspaceError> {
        assert!(
            path.is_absolute(),
            "virtual project discovery with relative path"
        );
        let project_root = path
            .ancestors()
            .take_while(|path| {
                // Only walk up the given directory, if any.
                options
                    .stop_discovery_at
                    .as_deref()
                    .and_then(Path::parent)
                    .is_none_or(|stop_discovery_at| stop_discovery_at != *path)
            })
            .find(|path| path.join("pyproject.toml").is_file())
            .ok_or(WorkspaceErrorKind::MissingPyprojectToml)?;

        debug!(
            "Found project root: `{}`",
            project_root.simplified_display()
        );

        // Fast path: The workspace is already cached.
        if let Some(workspace) = workspace_cache.get(project_root, &options.members) {
            let workspace = workspace?;
            let virtual_project = if let Some((project_name, _member)) = workspace
                .packages
                .iter()
                .find(|(_package_name, member)| member.root == project_root)
            {
                Self::Project(ProjectWorkspace {
                    project_root: project_root.to_path_buf(),
                    project_name: project_name.clone(),
                    workspace,
                })
            } else {
                Self::NonProject(workspace.clone())
            };
            return Ok(virtual_project);
        }

        // Read the current `pyproject.toml`.
        let pyproject_path = project_root.join("pyproject.toml");
        let contents = fs_err::tokio::read_to_string(&pyproject_path).await?;
        let pyproject_toml = PyProjectToml::from_string(contents, &pyproject_path)
            .map_err(|err| WorkspaceErrorKind::Toml(pyproject_path.clone(), Box::new(err)))?;

        if let Some(project) = pyproject_toml.project.as_ref() {
            // If the `pyproject.toml` contains a `[project]` table, it's a project.
            let project = ProjectWorkspace::from_project(
                project_root,
                project,
                &pyproject_toml,
                options,
                cache,
                workspace_cache,
            )
            .await?;
            Ok(Self::Project(project))
        } else if let Some(workspace) = pyproject_toml
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
        {
            // Otherwise, if it contains a `tool.uv.workspace` table, it's a non-project workspace
            // root.
            let project_path = std::path::absolute(project_root)
                .map_err(WorkspaceErrorKind::Normalize)?
                .clone();

            let result = Workspace::build(
                project_path.clone(),
                workspace.clone(),
                pyproject_toml,
                None,
                options,
                cache,
            )
            .await;
            if options.members == MemberDiscovery::All {
                workspace_cache.insert(result.clone(), &project_path);
            }
            Ok(Self::NonProject(result?))
        } else {
            // Otherwise it's a pyproject.toml that maybe contains dependency-groups
            // that we want to treat like a project/workspace to handle those uniformly
            let project_path = std::path::absolute(project_root)
                .map_err(WorkspaceErrorKind::Normalize)?
                .clone();

            let result = Workspace::build(
                project_path.clone(),
                ToolUvWorkspace::default(),
                pyproject_toml,
                None,
                options,
                cache,
            )
            .await;
            if options.members == MemberDiscovery::All {
                workspace_cache.insert(result.clone(), &project_path);
            }
            Ok(Self::NonProject(result?))
        }
    }

    /// Discover a project workspace with the member package.
    pub async fn discover_with_package(
        path: &Path,
        options: &DiscoveryOptions,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
        package: PackageName,
    ) -> Result<Self, WorkspaceError> {
        let workspace = Workspace::discover(path, options, cache, workspace_cache).await?;
        let Some(project_workspace) =
            Workspace::with_current_project(workspace.clone(), package.clone())
        else {
            return Err(WorkspaceError::from(WorkspaceErrorKind::NoSuchMember(
                package,
                workspace.install_path.clone(),
            )));
        };
        Ok(Self::Project(project_workspace))
    }

    /// Update the `pyproject.toml` for the current project.
    ///
    /// Assumes that the project name is unchanged in the updated [`PyProjectToml`].
    ///
    /// Contract: There are no parallel workspace operations, this is the only thread operating on
    /// workspaces.
    ///
    /// The [`WorkspaceCache`] is passed to ensure the caller doesn't forget to clear it.
    pub fn update_member(
        self,
        pyproject_toml: PyProjectToml,
        workspace_cache: &WorkspaceCache,
    ) -> Result<Option<Self>, WorkspaceError> {
        // Our modifying operations run on a single workspace, clear that workspace.
        workspace_cache.invalidate_workspace(self.workspace());
        Ok(match self {
            Self::Project(project) => {
                let Some(project) = project.update_member(pyproject_toml)? else {
                    return Ok(None);
                };
                Some(Self::Project(project))
            }
            Self::NonProject(workspace) => {
                debug_assert_eq!(
                    Arc::strong_count(&workspace),
                    1,
                    "cannot modify workspace still in use",
                );

                let workspace = Arc::unwrap_or_clone(workspace);
                // If this is a non-project workspace root, then by definition the root isn't a
                // member, so we can just update the top-level `pyproject.toml`.
                let workspace = Workspace {
                    pyproject_toml,
                    ..workspace
                };
                Some(Self::NonProject(Arc::new(workspace)))
            }
        })
    }

    /// Clone while detaching from the original workspace `Arc`, freeing the original state for
    /// modification.
    ///
    /// This is intended for rollbacks only.
    #[must_use]
    pub fn clone_detach(&self) -> Self {
        match self {
            Self::Project(project) => Self::Project(ProjectWorkspace {
                project_root: project.project_root.clone(),
                project_name: project.project_name.clone(),
                workspace: Arc::new((*project.workspace).clone()),
            }),
            Self::NonProject(workspace) => Self::NonProject(Arc::new((**workspace).clone())),
        }
    }

    /// Return the root of the project.
    pub fn root(&self) -> &Path {
        match self {
            Self::Project(project) => project.project_root(),
            Self::NonProject(workspace) => workspace.install_path(),
        }
    }

    /// Return the [`PyProjectToml`] of the project.
    pub fn pyproject_toml(&self) -> &PyProjectToml {
        match self {
            Self::Project(project) => project.current_project().pyproject_toml(),
            Self::NonProject(workspace) => &workspace.pyproject_toml,
        }
    }

    /// Return the default dependency groups for the current project.
    pub fn default_groups(&self) -> Result<DefaultGroups, DefaultGroupsError> {
        match self {
            Self::Project(project) => project.default_groups(),
            Self::NonProject(workspace) => workspace.default_groups(),
        }
    }

    /// Return the default dependency groups for a package selection.
    ///
    /// A single selected package uses that member's defaults. With zero or multiple packages,
    /// use the current project's defaults. Every selected package must belong to the workspace.
    pub fn default_groups_for_packages(
        &self,
        packages: &[PackageName],
    ) -> Result<DefaultGroups, DefaultGroupsError> {
        if let [name] = packages {
            self.workspace()
                .packages()
                .get(name)
                .ok_or_else(|| DefaultGroupsError::MissingPackage(name.clone()))?
                .default_groups()
        } else {
            for name in packages {
                if !self.workspace().packages().contains_key(name) {
                    return Err(DefaultGroupsError::MissingPackage(name.clone()));
                }
            }
            self.default_groups()
        }
    }

    /// Return the [`Workspace`] of the project.
    pub fn workspace(&self) -> &Workspace {
        match self {
            Self::Project(project) => project.workspace(),
            Self::NonProject(workspace) => workspace,
        }
    }

    /// Return the [`PackageName`] of the project, if available.
    pub fn project_name(&self) -> Option<&PackageName> {
        match self {
            Self::Project(project) => Some(project.project_name()),
            Self::NonProject(_) => None,
        }
    }

    /// Returns `true` if the project is a virtual workspace root.
    pub fn is_non_project(&self) -> bool {
        matches!(self, Self::NonProject(_))
    }
}

#[cfg(test)]
#[cfg(unix)] // Avoid path escaping for the unit tests
mod tests {
    use std::collections::BTreeMap;
    use std::env;
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::str::FromStr;
    use std::sync::Arc;

    use anyhow::Result;
    use assert_fs::fixture::ChildPath;
    use assert_fs::prelude::*;
    use insta::{assert_json_snapshot, assert_snapshot};

    use uv_cache::Cache;
    use uv_normalize::{GroupName, PackageName};
    use uv_pypi_types::DependencyGroupSpecifier;

    use crate::pyproject::PyProjectToml;
    use crate::workspace::{DiscoveryOptions, MemberDiscovery, ProjectWorkspace, Workspace};
    use crate::{WorkspaceCache, WorkspaceError};

    async fn workspace_test(folder: &str) -> (ProjectWorkspace, String) {
        let root_dir = env::current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("test")
            .join("workspaces");
        let cache = Cache::from_path(root_dir.join(".uv_cache"));
        let project = ProjectWorkspace::discover(
            &root_dir.join(folder),
            &DiscoveryOptions::default(),
            &cache,
            &WorkspaceCache::default(),
        )
        .await
        .unwrap();
        let root_escaped = regex::escape(root_dir.to_string_lossy().as_ref());
        (project, root_escaped)
    }

    async fn temporary_test(
        folder: &Path,
    ) -> Result<(ProjectWorkspace, String), (WorkspaceError, String)> {
        let root_escaped = regex::escape(folder.to_string_lossy().as_ref());
        let cache = Cache::from_path(env::temp_dir().join("uv-workspace-cache"));
        let project = ProjectWorkspace::discover(
            folder,
            &DiscoveryOptions::default(),
            &cache,
            &WorkspaceCache::default(),
        )
        .await
        .map_err(|error| (error, root_escaped.clone()))?;

        Ok((project, root_escaped))
    }

    #[tokio::test]
    async fn albatross_in_example() {
        let (project, root_escaped) =
            workspace_test("albatross-in-example/examples/bird-feeder").await;
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
        assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r#"
        {
          "project_root": "[ROOT]/albatross-in-example/examples/bird-feeder",
          "project_name": "bird-feeder",
          "workspace": {
            "install_path": "[ROOT]/albatross-in-example/examples/bird-feeder",
            "packages": {
              "bird-feeder": {
                "root": "[ROOT]/albatross-in-example/examples/bird-feeder",
                "project": {
                  "name": "bird-feeder",
                  "version": "1.0.0",
                  "requires-python": ">=3.12",
                  "dependencies": [
                    "iniconfig>=2,<3"
                  ],
                  "optional-dependencies": null
                },
                "pyproject_toml": "[PYPROJECT_TOML]"
              }
            },
            "required_members": {},
            "sources": {},
            "indexes": [],
            "pyproject_toml": {
              "project": {
                "name": "bird-feeder",
                "version": "1.0.0",
                "requires-python": ">=3.12",
                "dependencies": [
                  "iniconfig>=2,<3"
                ],
                "optional-dependencies": null
              },
              "tool": null,
              "dependency-groups": null
            }
          }
        }
        "#);
        });
    }

    #[tokio::test]
    async fn albatross_project_in_excluded() {
        let (project, root_escaped) =
            workspace_test("albatross-project-in-excluded/excluded/bird-feeder").await;
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r#"
            {
              "project_root": "[ROOT]/albatross-project-in-excluded/excluded/bird-feeder",
              "project_name": "bird-feeder",
              "workspace": {
                "install_path": "[ROOT]/albatross-project-in-excluded/excluded/bird-feeder",
                "packages": {
                  "bird-feeder": {
                    "root": "[ROOT]/albatross-project-in-excluded/excluded/bird-feeder",
                    "project": {
                      "name": "bird-feeder",
                      "version": "1.0.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "iniconfig>=2,<3"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "required_members": {},
                "sources": {},
                "indexes": [],
                "pyproject_toml": {
                  "project": {
                    "name": "bird-feeder",
                    "version": "1.0.0",
                    "requires-python": ">=3.12",
                    "dependencies": [
                      "iniconfig>=2,<3"
                    ],
                    "optional-dependencies": null
                  },
                  "tool": null,
                  "dependency-groups": null
                }
              }
            }
            "#);
        });
    }

    #[tokio::test]
    async fn albatross_root_workspace() {
        let (project, root_escaped) = workspace_test("albatross-root-workspace").await;
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r#"
            {
              "project_root": "[ROOT]/albatross-root-workspace",
              "project_name": "albatross",
              "workspace": {
                "install_path": "[ROOT]/albatross-root-workspace",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]/albatross-root-workspace",
                    "project": {
                      "name": "albatross",
                      "version": "0.1.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "bird-feeder",
                        "iniconfig>=2,<3"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "bird-feeder": {
                    "root": "[ROOT]/albatross-root-workspace/packages/bird-feeder",
                    "project": {
                      "name": "bird-feeder",
                      "version": "1.0.0",
                      "requires-python": ">=3.8",
                      "dependencies": [
                        "iniconfig>=2,<3",
                        "seeds"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "seeds": {
                    "root": "[ROOT]/albatross-root-workspace/packages/seeds",
                    "project": {
                      "name": "seeds",
                      "version": "1.0.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "idna==3.6"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "required_members": {
                  "bird-feeder": null,
                  "seeds": null
                },
                "sources": {
                  "bird-feeder": [
                    {
                      "workspace": true,
                      "editable": null,
                      "extra": null,
                      "group": null
                    }
                  ]
                },
                "indexes": [],
                "pyproject_toml": {
                  "project": {
                    "name": "albatross",
                    "version": "0.1.0",
                    "requires-python": ">=3.12",
                    "dependencies": [
                      "bird-feeder",
                      "iniconfig>=2,<3"
                    ],
                    "optional-dependencies": null
                  },
                  "tool": {
                    "uv": {
                      "sources": {
                        "bird-feeder": [
                          {
                            "workspace": true,
                            "editable": null,
                            "extra": null,
                            "group": null
                          }
                        ]
                      },
                      "index": null,
                      "workspace": {
                        "members": [
                          "packages/*"
                        ],
                        "exclude": null
                      },
                      "managed": null,
                      "package": null,
                      "default-groups": null,
                      "dependency-groups": null,
                      "dev-dependencies": null,
                      "override-dependencies": null,
                      "exclude-dependencies": null,
                      "constraint-dependencies": null,
                      "build-constraint-dependencies": null,
                      "environments": null,
                      "required-environments": null,
                      "conflicts": null,
                      "build-backend": null
                    }
                  },
                  "dependency-groups": null
                }
              }
            }
            "#);
        });
    }

    #[tokio::test]
    async fn albatross_virtual_workspace() {
        let (project, root_escaped) =
            workspace_test("albatross-virtual-workspace/packages/albatross").await;
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r#"
            {
              "project_root": "[ROOT]/albatross-virtual-workspace/packages/albatross",
              "project_name": "albatross",
              "workspace": {
                "install_path": "[ROOT]/albatross-virtual-workspace",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]/albatross-virtual-workspace/packages/albatross",
                    "project": {
                      "name": "albatross",
                      "version": "0.1.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "bird-feeder",
                        "iniconfig>=2,<3"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "bird-feeder": {
                    "root": "[ROOT]/albatross-virtual-workspace/packages/bird-feeder",
                    "project": {
                      "name": "bird-feeder",
                      "version": "1.0.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "anyio>=4.3.0,<5",
                        "seeds"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "seeds": {
                    "root": "[ROOT]/albatross-virtual-workspace/packages/seeds",
                    "project": {
                      "name": "seeds",
                      "version": "1.0.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "idna==3.6"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "required_members": {
                  "bird-feeder": null,
                  "seeds": null
                },
                "sources": {},
                "indexes": [],
                "pyproject_toml": {
                  "project": null,
                  "tool": {
                    "uv": {
                      "sources": null,
                      "index": null,
                      "workspace": {
                        "members": [
                          "packages/*"
                        ],
                        "exclude": null
                      },
                      "managed": null,
                      "package": null,
                      "default-groups": null,
                      "dependency-groups": null,
                      "dev-dependencies": null,
                      "override-dependencies": null,
                      "exclude-dependencies": null,
                      "constraint-dependencies": null,
                      "build-constraint-dependencies": null,
                      "environments": null,
                      "required-environments": null,
                      "conflicts": null,
                      "build-backend": null
                    }
                  },
                  "dependency-groups": null
                }
              }
            }
            "#);
        });
    }

    #[tokio::test]
    async fn workspace_cache_reuses_workspace_for_member() -> Result<()> {
        let root = tempfile::TempDir::new()?;
        let root = ChildPath::new(root.path());

        root.child("pyproject.toml").write_str(
            r#"
            [project]
            name = "albatross"
            version = "0.1.0"
            requires-python = ">=3.12"

            [tool.uv.workspace]
            members = ["packages/*"]
            "#,
        )?;

        root.child("packages")
            .child("seeds")
            .child("pyproject.toml")
            .write_str(
                r#"
            [project]
            name = "seeds"
            version = "1.0.0"
            requires-python = ">=3.12"
            "#,
            )?;

        let cache = Cache::from_path(env::temp_dir().join("uv-workspace-cache"));
        let workspace_cache = WorkspaceCache::default();
        let root_workspace = Workspace::discover(
            root.as_ref(),
            &DiscoveryOptions::default(),
            &cache,
            &workspace_cache,
        )
        .await?;
        let member_workspace = Workspace::discover(
            root.child("packages").child("seeds").as_ref(),
            &DiscoveryOptions::default(),
            &cache,
            &workspace_cache,
        )
        .await?;

        assert!(Arc::ptr_eq(&root_workspace, &member_workspace));

        root.child("pyproject.toml")
            .write_str("not valid toml >.<")?;
        let member_project = ProjectWorkspace::from_maybe_project_root(
            root.child("packages").child("seeds").as_ref(),
            &DiscoveryOptions::default(),
            &cache,
            &workspace_cache,
        )
        .await?
        .expect("cached workspace member ignores invalid change in the meantime");

        assert!(Arc::ptr_eq(&root_workspace, &member_project.workspace));

        Ok(())
    }

    #[tokio::test]
    async fn workspace_cache_does_not_store_partial_discovery() -> Result<()> {
        let root = tempfile::TempDir::new()?;
        let root = ChildPath::new(root.path());

        root.child("pyproject.toml").write_str(
            r#"
            [project]
            name = "albatross"
            version = "0.1.0"
            requires-python = ">=3.12"

            [tool.uv.workspace]
            members = ["packages/*"]
            "#,
        )?;

        root.child("packages")
            .child("seeds")
            .child("pyproject.toml")
            .write_str(
                r#"
            [project]
            name = "seeds"
            version = "1.0.0"
            requires-python = ">=3.12"
            "#,
            )?;

        let cache = Cache::from_path(env::temp_dir().join("uv-workspace-cache"));
        let workspace_cache = WorkspaceCache::default();
        let partial_options = DiscoveryOptions {
            members: MemberDiscovery::None,
            ..DiscoveryOptions::default()
        };
        let partial_project =
            ProjectWorkspace::discover(root.as_ref(), &partial_options, &cache, &workspace_cache)
                .await?;

        assert_eq!(partial_project.workspace().packages().len(), 1);

        let member_project = ProjectWorkspace::discover(
            root.child("packages").child("seeds").as_ref(),
            &DiscoveryOptions::default(),
            &cache,
            &workspace_cache,
        )
        .await?;
        let seeds = PackageName::from_str("seeds")?;

        assert_eq!(member_project.project_name(), &seeds);
        assert_eq!(member_project.workspace().packages().len(), 2);
        assert!(member_project.workspace().packages().contains_key(&seeds));

        Ok(())
    }

    #[tokio::test]
    async fn albatross_just_project() {
        let (project, root_escaped) = workspace_test("albatross-just-project").await;
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r#"
            {
              "project_root": "[ROOT]/albatross-just-project",
              "project_name": "albatross",
              "workspace": {
                "install_path": "[ROOT]/albatross-just-project",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]/albatross-just-project",
                    "project": {
                      "name": "albatross",
                      "version": "0.1.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "iniconfig>=2,<3"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "required_members": {},
                "sources": {},
                "indexes": [],
                "pyproject_toml": {
                  "project": {
                    "name": "albatross",
                    "version": "0.1.0",
                    "requires-python": ">=3.12",
                    "dependencies": [
                      "iniconfig>=2,<3"
                    ],
                    "optional-dependencies": null
                  },
                  "tool": null,
                  "dependency-groups": null
                }
              }
            }
            "#);
        });
    }

    #[tokio::test]
    async fn exclude_package() -> Result<()> {
        let root = tempfile::TempDir::new()?;
        let root = ChildPath::new(root.path());

        // Create the root.
        root.child("pyproject.toml").write_str(
            r#"
            [project]
            name = "albatross"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["tqdm>=4,<5"]

            [tool.uv.workspace]
            members = ["packages/*"]
            exclude = ["packages/bird-feeder"]

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
            "#,
        )?;
        root.child("albatross").child("__init__.py").touch()?;

        // Create an included package (`seeds`).
        root.child("packages")
            .child("seeds")
            .child("pyproject.toml")
            .write_str(
                r#"
            [project]
            name = "seeds"
            version = "1.0.0"
            requires-python = ">=3.12"
            dependencies = ["idna==3.6"]

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
            "#,
            )?;
        root.child("packages")
            .child("seeds")
            .child("seeds")
            .child("__init__.py")
            .touch()?;

        // Create an excluded package (`bird-feeder`).
        root.child("packages")
            .child("bird-feeder")
            .child("pyproject.toml")
            .write_str(
                r#"
            [project]
            name = "bird-feeder"
            version = "1.0.0"
            requires-python = ">=3.12"
            dependencies = ["anyio>=4.3.0,<5"]

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
            "#,
            )?;
        root.child("packages")
            .child("bird-feeder")
            .child("bird_feeder")
            .child("__init__.py")
            .touch()?;

        let (project, root_escaped) = temporary_test(root.as_ref()).await.unwrap();
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r#"
            {
              "project_root": "[ROOT]",
              "project_name": "albatross",
              "workspace": {
                "install_path": "[ROOT]",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]",
                    "project": {
                      "name": "albatross",
                      "version": "0.1.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "tqdm>=4,<5"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "seeds": {
                    "root": "[ROOT]/packages/seeds",
                    "project": {
                      "name": "seeds",
                      "version": "1.0.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "idna==3.6"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "required_members": {},
                "sources": {},
                "indexes": [],
                "pyproject_toml": {
                  "project": {
                    "name": "albatross",
                    "version": "0.1.0",
                    "requires-python": ">=3.12",
                    "dependencies": [
                      "tqdm>=4,<5"
                    ],
                    "optional-dependencies": null
                  },
                  "tool": {
                    "uv": {
                      "sources": null,
                      "index": null,
                      "workspace": {
                        "members": [
                          "packages/*"
                        ],
                        "exclude": [
                          "packages/bird-feeder"
                        ]
                      },
                      "managed": null,
                      "package": null,
                      "default-groups": null,
                      "dependency-groups": null,
                      "dev-dependencies": null,
                      "override-dependencies": null,
                      "exclude-dependencies": null,
                      "constraint-dependencies": null,
                      "build-constraint-dependencies": null,
                      "environments": null,
                      "required-environments": null,
                      "conflicts": null,
                      "build-backend": null
                    }
                  },
                  "dependency-groups": null
                }
              }
            }
            "#);
        });

        // Rewrite the members to both include and exclude `bird-feeder` by name.
        root.child("pyproject.toml").write_str(
            r#"
            [project]
            name = "albatross"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["tqdm>=4,<5"]

            [tool.uv.workspace]
            members = ["packages/seeds", "packages/bird-feeder"]
            exclude = ["packages/bird-feeder"]

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
            "#,
        )?;

        // `bird-feeder` should still be excluded.
        let (project, root_escaped) = temporary_test(root.as_ref()).await.unwrap();
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r#"
            {
              "project_root": "[ROOT]",
              "project_name": "albatross",
              "workspace": {
                "install_path": "[ROOT]",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]",
                    "project": {
                      "name": "albatross",
                      "version": "0.1.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "tqdm>=4,<5"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "seeds": {
                    "root": "[ROOT]/packages/seeds",
                    "project": {
                      "name": "seeds",
                      "version": "1.0.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "idna==3.6"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "required_members": {},
                "sources": {},
                "indexes": [],
                "pyproject_toml": {
                  "project": {
                    "name": "albatross",
                    "version": "0.1.0",
                    "requires-python": ">=3.12",
                    "dependencies": [
                      "tqdm>=4,<5"
                    ],
                    "optional-dependencies": null
                  },
                  "tool": {
                    "uv": {
                      "sources": null,
                      "index": null,
                      "workspace": {
                        "members": [
                          "packages/seeds",
                          "packages/bird-feeder"
                        ],
                        "exclude": [
                          "packages/bird-feeder"
                        ]
                      },
                      "managed": null,
                      "package": null,
                      "default-groups": null,
                      "dependency-groups": null,
                      "dev-dependencies": null,
                      "override-dependencies": null,
                      "exclude-dependencies": null,
                      "constraint-dependencies": null,
                      "build-constraint-dependencies": null,
                      "environments": null,
                      "required-environments": null,
                      "conflicts": null,
                      "build-backend": null
                    }
                  },
                  "dependency-groups": null
                }
              }
            }
            "#);
        });

        // Rewrite the exclusion to use the top-level directory (`packages`).
        root.child("pyproject.toml").write_str(
            r#"
            [project]
            name = "albatross"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["tqdm>=4,<5"]

            [tool.uv.workspace]
            members = ["packages/seeds", "packages/bird-feeder"]
            exclude = ["packages"]

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
            "#,
        )?;

        // `bird-feeder` should now be included.
        let (project, root_escaped) = temporary_test(root.as_ref()).await.unwrap();
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r#"
            {
              "project_root": "[ROOT]",
              "project_name": "albatross",
              "workspace": {
                "install_path": "[ROOT]",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]",
                    "project": {
                      "name": "albatross",
                      "version": "0.1.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "tqdm>=4,<5"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "bird-feeder": {
                    "root": "[ROOT]/packages/bird-feeder",
                    "project": {
                      "name": "bird-feeder",
                      "version": "1.0.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "anyio>=4.3.0,<5"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  },
                  "seeds": {
                    "root": "[ROOT]/packages/seeds",
                    "project": {
                      "name": "seeds",
                      "version": "1.0.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "idna==3.6"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "required_members": {},
                "sources": {},
                "indexes": [],
                "pyproject_toml": {
                  "project": {
                    "name": "albatross",
                    "version": "0.1.0",
                    "requires-python": ">=3.12",
                    "dependencies": [
                      "tqdm>=4,<5"
                    ],
                    "optional-dependencies": null
                  },
                  "tool": {
                    "uv": {
                      "sources": null,
                      "index": null,
                      "workspace": {
                        "members": [
                          "packages/seeds",
                          "packages/bird-feeder"
                        ],
                        "exclude": [
                          "packages"
                        ]
                      },
                      "managed": null,
                      "package": null,
                      "default-groups": null,
                      "dependency-groups": null,
                      "dev-dependencies": null,
                      "override-dependencies": null,
                      "exclude-dependencies": null,
                      "constraint-dependencies": null,
                      "build-constraint-dependencies": null,
                      "environments": null,
                      "required-environments": null,
                      "conflicts": null,
                      "build-backend": null
                    }
                  },
                  "dependency-groups": null
                }
              }
            }
            "#);
        });

        // Rewrite the exclusion to use the top-level directory with a glob (`packages/*`).
        root.child("pyproject.toml").write_str(
            r#"
            [project]
            name = "albatross"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["tqdm>=4,<5"]

            [tool.uv.workspace]
            members = ["packages/seeds", "packages/bird-feeder"]
            exclude = ["packages/*"]

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
            "#,
        )?;

        // `bird-feeder` and `seeds` should now be excluded.
        let (project, root_escaped) = temporary_test(root.as_ref()).await.unwrap();
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_json_snapshot!(
            project,
            {
                ".workspace.packages.*.pyproject_toml" => "[PYPROJECT_TOML]"
            },
            @r#"
            {
              "project_root": "[ROOT]",
              "project_name": "albatross",
              "workspace": {
                "install_path": "[ROOT]",
                "packages": {
                  "albatross": {
                    "root": "[ROOT]",
                    "project": {
                      "name": "albatross",
                      "version": "0.1.0",
                      "requires-python": ">=3.12",
                      "dependencies": [
                        "tqdm>=4,<5"
                      ],
                      "optional-dependencies": null
                    },
                    "pyproject_toml": "[PYPROJECT_TOML]"
                  }
                },
                "required_members": {},
                "sources": {},
                "indexes": [],
                "pyproject_toml": {
                  "project": {
                    "name": "albatross",
                    "version": "0.1.0",
                    "requires-python": ">=3.12",
                    "dependencies": [
                      "tqdm>=4,<5"
                    ],
                    "optional-dependencies": null
                  },
                  "tool": {
                    "uv": {
                      "sources": null,
                      "index": null,
                      "workspace": {
                        "members": [
                          "packages/seeds",
                          "packages/bird-feeder"
                        ],
                        "exclude": [
                          "packages/*"
                        ]
                      },
                      "managed": null,
                      "package": null,
                      "default-groups": null,
                      "dependency-groups": null,
                      "dev-dependencies": null,
                      "override-dependencies": null,
                      "exclude-dependencies": null,
                      "constraint-dependencies": null,
                      "build-constraint-dependencies": null,
                      "environments": null,
                      "required-environments": null,
                      "conflicts": null,
                      "build-backend": null
                    }
                  },
                  "dependency-groups": null
                }
              }
            }
            "#);
        });

        Ok(())
    }

    #[tokio::test]
    async fn exclude_package_with_normalized_glob_and_escaped_root() -> Result<()> {
        let temp_dir = tempfile::TempDir::new()?;
        let temp_dir_root = ChildPath::new(temp_dir.path());
        let root = temp_dir_root.child("workspace[glob]?");

        root.child("pyproject.toml").write_str(
            r#"
            [project]
            name = "albatross"
            version = "0.1.0"
            requires-python = ">=3.12"

            [tool.uv.workspace]
            members = ["./packages/*", "../external-*"]
            exclude = [
                "packages/excluded-borrowed-*",
                "./ignored/../packages/excluded",
                "./packages/./excluded-glob-*",
                "../external-excluded",
            ]
            "#,
        )?;

        for member in [
            "included",
            "excluded",
            "excluded-glob-one",
            "excluded-borrowed-one",
        ] {
            root.child("packages")
                .child(member)
                .child("pyproject.toml")
                .write_str(&format!(
                    r#"
                    [project]
                    name = "{member}"
                    version = "0.1.0"
                    requires-python = ">=3.12"
                    "#,
                ))?;
        }

        for member in ["external-included", "external-excluded"] {
            temp_dir_root
                .child(member)
                .child("pyproject.toml")
                .write_str(&format!(
                    r#"
                    [project]
                    name = "{member}"
                    version = "0.1.0"
                    requires-python = ">=3.12"
                    "#,
                ))?;
        }

        let (project, _) = temporary_test(root.as_ref())
            .await
            .map_err(|(error, _)| error)?;
        assert_json_snapshot!(
            project.workspace().packages().keys().collect::<Vec<_>>(),
            @r#"
        [
          "albatross",
          "external-included",
          "included"
        ]
        "#
        );

        Ok(())
    }

    #[test]
    fn read_dependency_groups() {
        let toml = r#"
[dependency-groups]
foo = ["a", {include-group = "bar"}]
bar = ["b"]
future = [{include-group = "bar", unknown = "value"}]
"#;

        let result = PyProjectToml::from_string(toml.to_string(), "pyproject.toml")
            .expect("Deserialization should succeed");

        let groups = result
            .dependency_groups
            .expect("`dependency-groups` should be present");
        let foo = groups
            .get(&GroupName::from_str("foo").unwrap())
            .expect("Group `foo` should be present");
        assert_eq!(
            foo,
            &[
                DependencyGroupSpecifier::Requirement("a".to_string()),
                DependencyGroupSpecifier::IncludeGroup {
                    include_group: GroupName::from_str("bar").unwrap(),
                }
            ]
        );

        let bar = groups
            .get(&GroupName::from_str("bar").unwrap())
            .expect("Group `bar` should be present");
        assert_eq!(
            bar,
            &[DependencyGroupSpecifier::Requirement("b".to_string())]
        );

        let future = groups
            .get(&GroupName::from_str("future").unwrap())
            .expect("Group `future` should be present");
        assert_eq!(
            future,
            &[DependencyGroupSpecifier::Object(BTreeMap::from([
                ("include-group".to_string(), "bar".to_string()),
                ("unknown".to_string(), "value".to_string()),
            ]))]
        );
    }

    #[test]
    fn reject_colliding_optional_dependency_names() {
        let err = PyProjectToml::from_string(
            r#"
[project]
name = "example"
version = "1.0.0"

[project.optional-dependencies]
foo-bar = ["anyio"]
foo_bar = ["iniconfig"]
"#
            .to_string(),
            "pyproject.toml",
        )
        .unwrap_err();

        assert_snapshot!(err.to_string(), @r#"
        TOML parse error at line 6, column 1
          |
        6 | [project.optional-dependencies]
          | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
        duplicate normalized extra name `foo-bar`
        "#);
    }

    #[tokio::test]
    async fn independent_child_workspaces() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let options = DiscoveryOptions {
            stop_discovery_at: Some(root.to_path_buf()),
            ..DiscoveryOptions::default()
        };
        let cache = Cache::from_path(root.join(".uv-cache"));
        let workspace_cache = WorkspaceCache::default();

        root.child("pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            members = ["packages/*"]
            workspaces = ["services/*", "services/./alpha", "services/alpha/nested/*"]
            "#,
        )?;
        root.child("packages/shared/pyproject.toml").write_str(
            r#"
            [project]
            name = "shared"
            version = "1.0.0"
            "#,
        )?;
        root.child("services/alpha/pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            members = ["packages/*"]
            workspaces = ["nested/*"]
            "#,
        )?;
        root.child("services/alpha/packages/shared/pyproject.toml")
            .write_str(
                r#"
            [project]
            name = "shared"
            version = "2.0.0"
            "#,
            )?;
        root.child("services/beta/pyproject.toml")
            .write_str("[tool.uv.workspace]\n")?;
        root.child("services/alpha/nested/grandchild/pyproject.toml")
            .write_str("[tool.uv.workspace]\n")?;

        let workspace =
            Workspace::discover(root.as_ref(), &options, &cache, &workspace_cache).await?;
        assert_eq!(workspace.packages().len(), 1);
        assert_eq!(
            workspace.child_workspace_roots(&options, &cache).await?,
            vec![root.join("services/alpha"), root.join("services/beta")],
        );

        let alpha = Workspace::discover(
            &root.join("services/alpha"),
            &options,
            &cache,
            &workspace_cache,
        )
        .await?;
        assert_eq!(alpha.install_path(), &root.join("services/alpha"));
        assert_eq!(alpha.packages().len(), 1);
        assert_eq!(
            alpha.parent_workspace_root(&options, &cache).await?,
            Some(root.to_path_buf()),
        );
        assert_eq!(
            alpha.child_workspace_roots(&options, &cache).await?,
            vec![root.join("services/alpha/nested/grandchild")],
        );

        let member = ProjectWorkspace::discover(
            &root.join("services/alpha/packages/shared"),
            &options,
            &cache,
            &workspace_cache,
        )
        .await?;
        assert_eq!(member.workspace().install_path(), alpha.install_path());

        let grandchild = Workspace::discover(
            &root.join("services/alpha/nested/grandchild"),
            &options,
            &cache,
            &workspace_cache,
        )
        .await?;
        assert_eq!(
            grandchild.parent_workspace_root(&options, &cache).await?,
            Some(root.join("services/alpha")),
        );

        // The child remains its own workspace when upward discovery is limited to its root.
        let standalone_options = DiscoveryOptions {
            stop_discovery_at: Some(alpha.install_path().clone()),
            ..DiscoveryOptions::default()
        };
        assert_eq!(
            alpha
                .parent_workspace_root(&standalone_options, &cache)
                .await?,
            None,
        );
        Ok(())
    }

    #[tokio::test]
    async fn child_workspaces_require_explicit_roots() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let options = DiscoveryOptions {
            stop_discovery_at: Some(root.to_path_buf()),
            ..DiscoveryOptions::default()
        };
        let cache = Cache::from_path(root.join(".uv-cache"));
        let workspace_cache = WorkspaceCache::default();
        root.child("pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            workspaces = ["child"]
            "#,
        )?;
        root.child("child/pyproject.toml").write_str(
            r#"
            [project]
            name = "child"
            version = "1.0.0"
            "#,
        )?;

        let children_error = Workspace::discover(root.as_ref(), &options, &cache, &workspace_cache)
            .await
            .expect_err("registered children require explicit workspace roots");
        let child =
            Workspace::discover(&root.join("child"), &options, &cache, &workspace_cache).await?;
        let parent_error = child
            .parent_workspace_root(&options, &cache)
            .await
            .expect_err("an implicit project is not an independently locked child workspace");
        let root_escaped = regex::escape(root.to_string_lossy().as_ref());
        insta::with_settings!({filters => vec![(root_escaped.as_str(), "[ROOT]")]}, {
            assert_snapshot!(parent_error, @"Child workspace `[ROOT]/child` must contain a `tool.uv.workspace` table");
            assert_snapshot!(children_error, @"Child workspace `[ROOT]/child` must contain a `tool.uv.workspace` table");
        });
        Ok(())
    }

    #[tokio::test]
    async fn child_workspace_parent_lookup_ignores_siblings() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let options = DiscoveryOptions {
            stop_discovery_at: Some(root.to_path_buf()),
            ..DiscoveryOptions::default()
        };
        let cache = Cache::from_path(root.join(".uv-cache"));
        root.child("pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            workspaces = ["services/*"]
            "#,
        )?;
        root.child("services/good/pyproject.toml")
            .write_str("[tool.uv.workspace]\n")?;
        root.child("services/broken/pyproject.toml")
            .write_str("[invalid\n")?;
        root.child("uv.lock").write_str("invalid lockfile\n")?;

        let child = Workspace::discover(
            &root.join("services/good"),
            &options,
            &cache,
            &WorkspaceCache::default(),
        )
        .await?;
        assert_eq!(
            child.parent_workspace_root(&options, &cache).await?,
            Some(root.to_path_buf()),
        );
        Ok(())
    }

    #[tokio::test]
    async fn child_workspace_parent_lookup_ignores_unrelated_ancestors() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let options = DiscoveryOptions {
            stop_discovery_at: Some(root.to_path_buf()),
            ..DiscoveryOptions::default()
        };
        let cache = Cache::from_path(root.join(".uv-cache"));
        root.child("child/pyproject.toml")
            .write_str("[tool.uv.workspace]\n")?;
        let child = Workspace::discover(
            &root.join("child"),
            &options,
            &cache,
            &WorkspaceCache::default(),
        )
        .await?;

        for pyproject in [
            "[invalid\n",
            "[project]\nname = 42\n",
            "[project]\nname = 42\n[tool.uv.workspace]\nworkspaces = [\"elsewhere/*\"]\n",
            "[tool.uv.workspace]\nworkspaces = [\"../outside\"]\n",
        ] {
            root.child("pyproject.toml").write_str(pyproject)?;
            assert_eq!(child.parent_workspace_root(&options, &cache).await?, None,);
        }

        root.child("pyproject.toml")
            .write_str("[project]\nname = 42\n[tool.uv.workspace]\nworkspaces = [\"child\"]\n")?;
        let error = child
            .parent_workspace_root(&options, &cache)
            .await
            .expect_err("a matching parent must have valid project metadata");
        let root_escaped = regex::escape(root.to_string_lossy().as_ref());
        insta::with_settings!({filters => vec![(root_escaped.as_str(), "[ROOT]")]}, {
            assert_snapshot!(error, @"Failed to parse: `[ROOT]/pyproject.toml`");
        });
        Ok(())
    }

    #[tokio::test]
    async fn child_workspace_member_overlap() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let options = DiscoveryOptions {
            stop_discovery_at: Some(root.to_path_buf()),
            ..DiscoveryOptions::default()
        };
        let cache = Cache::from_path(root.join(".uv-cache"));
        root.child("pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            members = ["child/packages/*"]
            workspaces = ["child"]
            "#,
        )?;
        root.child("child/pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            members = ["packages/*"]
            "#,
        )?;
        root.child("child/packages/member/pyproject.toml")
            .write_str(
                r#"
            [project]
            name = "member"
            version = "1.0.0"
            "#,
            )?;

        let parent_error =
            Workspace::discover(root.as_ref(), &options, &cache, &WorkspaceCache::default())
                .await
                .expect_err("a parent cannot own packages beneath an independent child root");
        let child = Workspace::discover(
            &root.join("child"),
            &options,
            &cache,
            &WorkspaceCache::default(),
        )
        .await?;
        let child_error = child
            .parent_workspace_root(&options, &cache)
            .await
            .expect_err("upward lookup also validates ownership");
        let root_escaped = regex::escape(root.to_string_lossy().as_ref());
        insta::with_settings!({filters => vec![(root_escaped.as_str(), "[ROOT]")]}, {
            assert_snapshot!(parent_error, @"Workspace member `[ROOT]/child/packages/member` overlaps independently locked child workspace `[ROOT]/child`");
            assert_snapshot!(child_error, @"Workspace member `[ROOT]/child/packages/member` overlaps independently locked child workspace `[ROOT]/child`");
        });

        root.child("pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            members = ["child/packages/*"]
            exclude = ["child/packages/*"]
            workspaces = ["child"]
            "#,
        )?;
        assert_eq!(
            child.parent_workspace_root(&options, &cache).await?,
            Some(root.to_path_buf()),
        );
        Ok(())
    }

    #[tokio::test]
    async fn child_workspace_invalid_patterns() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let cache = Cache::from_path(root.join(".uv-cache"));
        let mut errors = Vec::new();
        for pattern in [".", "children/..", "../outside", "/outside"] {
            root.child("pyproject.toml").write_str(&format!(
                "[tool.uv.workspace]\nworkspaces = [{pattern:?}]\n"
            ))?;
            let error = Workspace::discover(
                root.as_ref(),
                &DiscoveryOptions::default(),
                &cache,
                &WorkspaceCache::default(),
            )
            .await
            .expect_err("child workspace patterns must stay below their root");
            errors.push(error.to_string());
        }
        let root_escaped = regex::escape(root.to_string_lossy().as_ref());
        insta::with_settings!({filters => vec![(root_escaped.as_str(), "[ROOT]")]}, {
            assert_json_snapshot!(errors, @r#"
            [
              "Child workspace pattern `.` must be a relative path below `[ROOT]`",
              "Child workspace pattern `children/..` must be a relative path below `[ROOT]`",
              "Child workspace pattern `../outside` must be a relative path below `[ROOT]`",
              "Child workspace pattern `/outside` must be a relative path below `[ROOT]`"
            ]
            "#);
        });
        Ok(())
    }

    #[tokio::test]
    async fn child_workspace_invalid_roots() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let options = DiscoveryOptions {
            stop_discovery_at: Some(root.to_path_buf()),
            ..DiscoveryOptions::default()
        };
        let cache = Cache::from_path(root.join(".uv-cache"));
        root.child("empty").create_dir_all()?;
        root.child("not-a-directory").write_str("file\n")?;
        let mut errors = Vec::new();
        for pattern in ["missing", "empty", "not-a-directory"] {
            root.child("pyproject.toml").write_str(&format!(
                "[tool.uv.workspace]\nworkspaces = [{pattern:?}]\n"
            ))?;
            let error =
                Workspace::discover(root.as_ref(), &options, &cache, &WorkspaceCache::default())
                    .await
                    .expect_err("each explicit child path must be a workspace root");
            errors.push(error.to_string());
            if pattern != "not-a-directory" {
                let existing_options = DiscoveryOptions {
                    members: MemberDiscovery::Existing,
                    ..options.clone()
                };
                let workspace = Workspace::discover(
                    root.as_ref(),
                    &existing_options,
                    &cache,
                    &WorkspaceCache::default(),
                )
                .await?;
                assert!(
                    workspace
                        .child_workspace_roots(&existing_options, &cache)
                        .await?
                        .is_empty()
                );
            }
        }
        let root_escaped = regex::escape(root.to_string_lossy().as_ref());
        insta::with_settings!({filters => vec![(root_escaped.as_str(), "[ROOT]")]}, {
            assert_json_snapshot!(errors, @r#"
            [
              "Child workspace `[ROOT]/missing` is missing a `pyproject.toml` (matches: `missing`)",
              "Child workspace `[ROOT]/empty` is missing a `pyproject.toml` (matches: `empty`)",
              "Child workspace path `[ROOT]/not-a-directory` is not a directory"
            ]
            "#);
        });
        Ok(())
    }

    #[tokio::test]
    async fn child_workspace_cache_boundary() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let cache = Cache::from_path(root.join("cache"));
        let options = DiscoveryOptions::default();
        let workspace_cache = WorkspaceCache::default();
        root.child("pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            workspaces = ["cache/clone", "services/*"]
            "#,
        )?;
        root.child("cache/clone/pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            workspaces = ["child"]
            "#,
        )?;
        root.child("cache/clone/child/pyproject.toml")
            .write_str("[tool.uv.workspace]\n")?;
        root.child("services/child/pyproject.toml")
            .write_str("[tool.uv.workspace]\n")?;

        let workspace =
            Workspace::discover(root.as_ref(), &options, &cache, &workspace_cache).await?;
        assert_eq!(
            workspace.child_workspace_roots(&options, &cache).await?,
            vec![root.join("services/child")],
        );
        let cached_child = Workspace::discover(
            &root.join("cache/clone/child"),
            &options,
            &cache,
            &workspace_cache,
        )
        .await?;
        assert_eq!(
            cached_child.parent_workspace_root(&options, &cache).await?,
            None,
        );
        let isolated = DiscoveryOptions {
            stop_discovery_at: Some(root.join("cache/clone")),
            ..DiscoveryOptions::default()
        };
        assert_eq!(
            cached_child
                .parent_workspace_root(&isolated, &cache)
                .await?,
            Some(root.join("cache/clone")),
        );
        Ok(())
    }

    #[tokio::test]
    async fn child_workspace_symlink_containment() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let options = DiscoveryOptions {
            stop_discovery_at: Some(root.to_path_buf()),
            ..DiscoveryOptions::default()
        };
        let cache = Cache::from_path(root.join(".uv-cache"));
        root.child("pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            workspaces = ["children/*", "aliases/*"]
            "#,
        )?;
        root.child("children/child/pyproject.toml")
            .write_str("[tool.uv.workspace]\n")?;
        root.child("aliases").create_dir_all()?;
        symlink(root.join("children/child"), root.join("aliases/child"))?;

        let workspace =
            Workspace::discover(root.as_ref(), &options, &cache, &WorkspaceCache::default())
                .await?;
        assert_eq!(
            workspace.child_workspace_roots(&options, &cache).await?,
            vec![root.join("children/child")],
        );
        let child = Workspace::discover(
            &root.join("aliases/child"),
            &options,
            &cache,
            &WorkspaceCache::default(),
        )
        .await?;
        assert_eq!(
            child.parent_workspace_root(&options, &cache).await?,
            Some(root.to_path_buf()),
        );

        symlink(root.as_ref(), root.join("aliases/self"))?;
        let error = workspace
            .child_workspace_roots(&options, &cache)
            .await
            .expect_err("a symlink cannot register the parent as its own child");
        let root_escaped = regex::escape(root.to_string_lossy().as_ref());
        insta::with_settings!({filters => vec![(root_escaped.as_str(), "[ROOT]")]}, {
            assert_snapshot!(error, @"Child workspace `[ROOT]` must be a strict descendant of `[ROOT]`");
        });
        Ok(())
    }

    #[tokio::test]
    async fn child_workspace_symlink_escape() -> Result<()> {
        let temporary = tempfile::TempDir::new()?;
        let root = ChildPath::new(fs_err::canonicalize(temporary.path())?);
        let outside_temporary = tempfile::TempDir::new()?;
        let outside = ChildPath::new(fs_err::canonicalize(outside_temporary.path())?);
        let options = DiscoveryOptions::default();
        let cache = Cache::from_path(root.join(".uv-cache"));
        root.child("pyproject.toml").write_str(
            r#"
            [tool.uv.workspace]
            workspaces = ["outside"]
            "#,
        )?;
        outside
            .child("pyproject.toml")
            .write_str("[tool.uv.workspace]\n")?;
        symlink(outside.as_ref(), root.join("outside"))?;

        let parent_error =
            Workspace::discover(root.as_ref(), &options, &cache, &WorkspaceCache::default())
                .await
                .expect_err("a registered child cannot escape its parent through a symlink");
        let child = Workspace::discover(
            &root.join("outside"),
            &options,
            &cache,
            &WorkspaceCache::default(),
        )
        .await?;
        let child_error = child
            .parent_workspace_root(&options, &cache)
            .await
            .expect_err("upward lookup checks the same canonical containment");
        let root_escaped = regex::escape(root.to_string_lossy().as_ref());
        let outside_escaped = regex::escape(outside.to_string_lossy().as_ref());
        let filters = vec![
            (root_escaped.as_str(), "[ROOT]"),
            (outside_escaped.as_str(), "[OUTSIDE]"),
        ];
        insta::with_settings!({filters => filters}, {
            assert_snapshot!(parent_error, @"Child workspace `[OUTSIDE]` must be a strict descendant of `[ROOT]`");
            assert_snapshot!(child_error, @"Child workspace `[OUTSIDE]` must be a strict descendant of `[ROOT]`");
        });
        Ok(())
    }

    #[tokio::test]
    async fn nested_workspace() -> Result<()> {
        let root = tempfile::TempDir::new()?;
        let root = ChildPath::new(root.path());

        // Create the root.
        root.child("pyproject.toml").write_str(
            r#"
            [project]
            name = "albatross"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["tqdm>=4,<5"]

            [tool.uv.workspace]
            members = ["packages/*"]
            "#,
        )?;

        // Create an included package (`seeds`).
        root.child("packages")
            .child("seeds")
            .child("pyproject.toml")
            .write_str(
                r#"
            [project]
            name = "seeds"
            version = "1.0.0"
            requires-python = ">=3.12"
            dependencies = ["idna==3.6"]

            [tool.uv.workspace]
            members = ["nested_packages/*"]
            "#,
            )?;

        let (error, root_escaped) = temporary_test(root.as_ref()).await.unwrap_err();
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_snapshot!(
                error,
            @"Workspace member `[ROOT]/packages/seeds` is a workspace root; register it in `tool.uv.workspace.workspaces` instead");
        });

        Ok(())
    }

    #[tokio::test]
    async fn duplicate_names() -> Result<()> {
        let root = tempfile::TempDir::new()?;
        let root = ChildPath::new(root.path());

        // Create the root.
        root.child("pyproject.toml").write_str(
            r#"
            [project]
            name = "albatross"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["tqdm>=4,<5"]

            [tool.uv.workspace]
            members = ["packages/*"]
            "#,
        )?;

        // Create an included package (`seeds`).
        root.child("packages")
            .child("seeds")
            .child("pyproject.toml")
            .write_str(
                r#"
            [project]
            name = "seeds"
            version = "1.0.0"
            requires-python = ">=3.12"
            dependencies = ["idna==3.6"]

            [tool.uv.workspace]
            members = ["nested_packages/*"]
            "#,
            )?;

        // Create an included package (`seeds2`).
        root.child("packages")
            .child("seeds2")
            .child("pyproject.toml")
            .write_str(
                r#"
            [project]
            name = "seeds"
            version = "1.0.0"
            requires-python = ">=3.12"
            dependencies = ["idna==3.6"]

            [tool.uv.workspace]
            members = ["nested_packages/*"]
            "#,
            )?;

        let (error, root_escaped) = temporary_test(root.as_ref()).await.unwrap_err();
        let filters = vec![(root_escaped.as_str(), "[ROOT]")];
        insta::with_settings!({filters => filters}, {
            assert_snapshot!(
                error,
            @"Two workspace members are both named `seeds`: `[ROOT]/packages/seeds` and `[ROOT]/packages/seeds2`");
        });

        Ok(())
    }
}
