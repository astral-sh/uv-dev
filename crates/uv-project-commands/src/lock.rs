use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

use owo_colors::OwoColorize;
use rustc_hash::{FxBuildHasher, FxHashMap};

use uv_cache::{Cache, Refresh};
use uv_client::BaseClientBuilder;
use uv_command_support::{ExitStatus, Printer, UvError};
use uv_configuration::{ActiveEnvironment, Concurrency, DependencyGroupsWithDefaults, DryRun};
use uv_dispatch::UniversalState;
use uv_distribution_types::RequiresPython;
use uv_environment_operations::{
    ProjectEnvironmentPolicy, ProjectEnvironmentTarget, ProjectInterpreter,
    discover_workspace_groups,
};
use uv_git_types::GitOid;
use uv_lock::{Lock, Package, WorkspaceGroupSelectionError, implicit_constraints_marker};
use uv_lock_operations::{
    LockError, LockMode, LockOperation, LockResult, LockTarget, MissingLockfileSource,
};
use uv_normalize::{GroupName, PackageName};
use uv_pep440::Version;
use uv_pep508::MarkerTree;
use uv_preview::{Preview, PreviewFeature};
use uv_pypi_types::SupportedEnvironments;
use uv_python_discovery::ConfigDiscovery;
use uv_python_discovery::ProjectPythonRequest;
use uv_python_discovery::PythonDownloadReporter;
use uv_python_discovery::ScriptInterpreter;
use uv_python_discovery::init_script_python_requirement;
use uv_python_types::{PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest};
use uv_resolve_operations::loggers::DefaultResolveLogger;
use uv_scripts::Pep723Script;
use uv_settings::{FrozenSource, LockCheck, PythonInstallMirrors, ResolverSettings};
use uv_warnings::warn_user;
use uv_workspace::{
    DiscoveryOptions, ProvisionalWorkspaceGroup, ResolvedWorkspaceGroup, VirtualProject, Workspace,
    WorkspaceCache, WorkspaceGroup, WorkspaceResolution,
};

use crate::{ProjectError, ScriptPath};

pub(crate) fn select_workspace_group_result(
    result: LockResult,
    name: Option<&GroupName>,
    members: &BTreeSet<PackageName>,
) -> Result<LockResult, ProjectError> {
    Ok(match result {
        LockResult::Unchanged(lock) => {
            LockResult::Unchanged(select_workspace_group_lock(lock, name, members)?)
        }
        LockResult::Changed(previous, lock) => {
            let previous = match previous {
                Some(previous) if !previous.workspace_groups().is_empty() => {
                    select_workspace_group_lock(previous, name, members).ok()
                }
                previous => previous,
            };
            LockResult::Changed(previous, select_workspace_group_lock(lock, name, members)?)
        }
    })
}

pub(crate) fn select_workspace_group_lock(
    lock: Lock,
    name: Option<&GroupName>,
    members: &BTreeSet<PackageName>,
) -> Result<Lock, ProjectError> {
    if name.is_none() && lock.workspace_groups().is_empty() {
        return Ok(lock);
    }
    Ok(lock.select_workspace_context(name, members)?)
}

/// The ordinary project target, before applying a named workspace-group default.
pub(crate) fn workspace_selection_members(
    project: &VirtualProject,
    packages: &[PackageName],
    all_packages: bool,
) -> BTreeSet<PackageName> {
    if all_packages || (packages.is_empty() && project.is_non_project()) {
        project.workspace().packages().keys().cloned().collect()
    } else if packages.is_empty() {
        project.project_name().into_iter().cloned().collect()
    } else {
        packages.iter().cloned().collect()
    }
}

/// Select the same Python domain that synchronization uses for the command's project target.
pub(crate) fn workspace_for_project_groups(
    project: &VirtualProject,
    packages: &[PackageName],
    all_packages: bool,
    groups: &[ResolvedWorkspaceGroup],
) -> Result<Workspace, ProjectError> {
    let members = workspace_selection_members(project, packages, all_packages);
    let workspace_target = all_packages || (packages.is_empty() && project.is_non_project());
    let selection = command_workspace_group(
        project.workspace(),
        None,
        Some(&members),
        workspace_target,
        groups,
    )?;
    Ok(workspace_for_group_selection(
        project.workspace(),
        &members,
        workspace_target,
        selection.as_ref(),
    ))
}

/// Scope a workspace to selected roots, applying a named default to whole-workspace targets.
pub(crate) fn workspace_for_group_selection(
    workspace: &Workspace,
    members: &BTreeSet<PackageName>,
    workspace_target: bool,
    selection: Option<&CommandWorkspaceSelection>,
) -> Workspace {
    let Some(selection) = selection else {
        return workspace.clone();
    };
    let members = if workspace_target && selection.name.is_some() {
        &selection.members
    } else {
        members
    };
    selection.scoped_workspace(workspace, members)
}

/// Select workspace members when only a lockfile is available.
pub(crate) fn lockfile_selection_members(
    lock: &Lock,
    project_name: Option<&PackageName>,
    packages: &[PackageName],
    all_packages: bool,
) -> BTreeSet<PackageName> {
    if all_packages || (packages.is_empty() && project_name.is_none()) {
        let mut members = lock.members().clone();
        if members.is_empty()
            && let Some(root) = lock.root()
        {
            members.insert(root.name().clone());
        }
        members
    } else if packages.is_empty() {
        project_name.iter().copied().cloned().collect()
    } else {
        packages.iter().cloned().collect()
    }
}

/// The interpreter domain of selected roots, with an optional named context.
pub(crate) struct CommandWorkspaceSelection {
    pub name: Option<GroupName>,
    pub members: BTreeSet<PackageName>,
    requires_python: RequiresPython,
    environments: MarkerTree,
    selected_lock: Option<Lock>,
}

impl CommandWorkspaceSelection {
    /// Reuse the graph projected while deriving a frozen selection's Python domain.
    pub(crate) fn take_selected_lock(&mut self) -> Option<Lock> {
        self.selected_lock.take()
    }

    pub(crate) fn scoped_workspace(
        &self,
        workspace: &Workspace,
        members: &BTreeSet<PackageName>,
    ) -> Workspace {
        workspace.with_resolution(WorkspaceResolution {
            roots: members
                .iter()
                .cloned()
                .map(|name| (name, self.environments))
                .collect(),
            requires_python: self.requires_python.clone(),
            environments: SupportedEnvironments::from_markers(vec![self.environments]),
        })
    }
}

/// Select group metadata for Python discovery from an existing lockfile.
///
/// Whole-group commands replace their ordinary roots with the named or default group's roots;
/// explicit member selections retain their narrower activation domain.
pub(crate) fn command_workspace_group_from_lock(
    lock: &Lock,
    name: Option<&GroupName>,
    members: Option<&BTreeSet<PackageName>>,
    use_group_roots: bool,
) -> Result<Option<CommandWorkspaceSelection>, ProjectError> {
    if lock.workspace_groups().is_empty() {
        return if let Some(name) = name {
            Err(WorkspaceGroupSelectionError::Missing(name.clone()).into())
        } else {
            Ok(None)
        };
    }
    let group = name
        .and_then(|name| {
            lock.workspace_groups()
                .iter()
                .find(|group| group.definition.name == *name)
        })
        .or_else(|| {
            name.is_none()
                .then(|| {
                    lock.workspace_groups()
                        .iter()
                        .find(|group| group.definition.default)
                })
                .flatten()
        });
    if let Some(name) = name
        && group.is_none()
    {
        return Err(WorkspaceGroupSelectionError::Missing(name.clone()).into());
    }
    let members = if use_group_roots {
        group.map(|group| &group.definition.members).or(members)
    } else {
        members.or_else(|| group.map(|group| &group.definition.members))
    };
    let Some(members) = members else {
        return Ok(None);
    };
    let members = if members.is_empty() {
        lock.members()
    } else {
        members
    };
    let name = group.map(|group| &group.definition.name);
    let selected = lock.select_workspace_context(name, members)?;
    Ok(Some(CommandWorkspaceSelection {
        name: name.cloned(),
        members: group.map_or_else(|| members.clone(), |group| group.definition.members.clone()),
        requires_python: selected.requires_python().clone(),
        environments: implicit_constraints_marker(
            selected.requires_python().to_exact_marker_tree(),
            selected.supported_environments(),
        ),
        selected_lock: Some(selected),
    }))
}

/// Select completed group metadata for Python discovery with the command's root selection.
pub(crate) fn command_workspace_group(
    workspace: &Workspace,
    name: Option<&GroupName>,
    members: Option<&BTreeSet<PackageName>>,
    use_group_roots: bool,
    groups: &[ResolvedWorkspaceGroup],
) -> Result<Option<CommandWorkspaceSelection>, ProjectError> {
    select_command_workspace_group(
        workspace,
        name,
        members,
        use_group_roots,
        &groups
            .iter()
            .map(|group| CommandGroupDomain {
                definition: group.definition(),
                requires_python: group.requires_python(),
                environments: group.environments(),
                member_environments: group.member_environments(),
            })
            .collect::<Vec<_>>(),
    )
}

/// Select an upper-bound domain for metadata probes and commands that skip synchronization.
pub(crate) fn provisional_command_workspace_group(
    workspace: &Workspace,
    name: Option<&GroupName>,
    members: Option<&BTreeSet<PackageName>>,
    use_group_roots: bool,
    groups: &[ProvisionalWorkspaceGroup],
) -> Result<Option<CommandWorkspaceSelection>, ProjectError> {
    select_command_workspace_group(
        workspace,
        name,
        members,
        use_group_roots,
        &groups
            .iter()
            .map(|group| CommandGroupDomain {
                definition: group.definition(),
                requires_python: group.requires_python(),
                environments: group.environments(),
                member_environments: group.member_environments(),
            })
            .collect::<Vec<_>>(),
    )
}

struct CommandGroupDomain<'a> {
    definition: &'a WorkspaceGroup,
    requires_python: &'a RequiresPython,
    environments: MarkerTree,
    member_environments: &'a BTreeMap<PackageName, MarkerTree>,
}

impl CommandGroupDomain<'_> {
    fn selection(
        &self,
        members: Option<&BTreeSet<PackageName>>,
    ) -> Result<CommandWorkspaceSelection, ProjectError> {
        let (requires_python, environments) = if let Some(members) = members {
            let environments = members
                .iter()
                .fold(self.environments, |environment, member| {
                    // Production reachability can omit members selected through extras or groups.
                    // Their membership and complete activation domain are checked by lock projection.
                    environment.and(
                        self.member_environments
                            .get(member)
                            .copied()
                            .unwrap_or(self.environments),
                    )
                });
            let requires_python =
                RequiresPython::from_marker_tree(environments).ok_or_else(|| {
                    WorkspaceGroupSelectionError::Target(self.definition.name.clone())
                })?;
            (requires_python, environments)
        } else {
            (self.requires_python.clone(), self.environments)
        };
        Ok(CommandWorkspaceSelection {
            name: Some(self.definition.name.clone()),
            members: self.definition.members.clone(),
            requires_python,
            environments,
            selected_lock: None,
        })
    }
}

fn select_command_workspace_group(
    workspace: &Workspace,
    name: Option<&GroupName>,
    members: Option<&BTreeSet<PackageName>>,
    use_group_roots: bool,
    groups: &[CommandGroupDomain<'_>],
) -> Result<Option<CommandWorkspaceSelection>, ProjectError> {
    if let Some(name) = name {
        let group = groups
            .iter()
            .find(|group| group.definition.name == *name)
            .ok_or_else(|| {
                uv_workspace::WorkspaceError::from(
                    uv_workspace::WorkspaceErrorKind::UnknownWorkspaceGroup(name.clone()),
                )
            })?;
        return group
            .selection(if use_group_roots { None } else { members })
            .map(Some);
    }
    if let Some(group) = groups.iter().find(|group| group.definition.default) {
        return group
            .selection(if use_group_roots { None } else { members })
            .map(Some);
    }
    if groups.is_empty() {
        return Ok(None);
    }
    let all_environments = groups.iter().fold(MarkerTree::FALSE, |environment, group| {
        environment.or(group.environments)
    });
    let environments = if let Some(members) = members {
        let mut environments = MarkerTree::TRUE;
        for member in members {
            let supported = groups
                .iter()
                .filter_map(|group| group.member_environments.get(member).copied())
                .fold(MarkerTree::FALSE, MarkerTree::or);
            // Python inference follows production dependencies. The complete lock also includes
            // extras and dependency groups, so only lock projection can reject target membership.
            environments = environments.and(if supported.is_false() {
                all_environments
            } else {
                supported
            });
        }
        environments
    } else {
        all_environments
    };
    let requires_python = RequiresPython::from_marker_tree(environments)
        .ok_or(WorkspaceGroupSelectionError::Ambiguous)?;
    Ok(Some(CommandWorkspaceSelection {
        name: None,
        members: members
            .cloned()
            .unwrap_or_else(|| workspace.packages().keys().cloned().collect()),
        requires_python,
        environments,
        selected_lock: None,
    }))
}

/// Resolve the project requirements into a lockfile.
pub async fn lock(
    project_dir: &Path,
    lock_check: LockCheck,
    frozen: Option<FrozenSource>,
    dry_run: DryRun,
    refresh: Refresh,
    python: Option<String>,
    install_mirrors: PythonInstallMirrors,
    settings: ResolverSettings,
    client_builder: BaseClientBuilder<'_>,
    script: Option<ScriptPath>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    concurrency: Concurrency,
    config_discovery: ConfigDiscovery,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> anyhow::Result<ExitStatus> {
    // If necessary, initialize the PEP 723 script.
    let script = match script {
        Some(ScriptPath::Path(path)) => {
            let reporter = PythonDownloadReporter::single(printer);
            let requires_python = init_script_python_requirement(
                python.as_deref(),
                &install_mirrors,
                project_dir,
                false,
                python_preference,
                python_arch,
                python_downloads,
                config_discovery,
                &client_builder,
                cache,
                &reporter,
            )
            .await?;
            Some(Pep723Script::init(&path, requires_python.specifiers()).await?)
        }
        Some(ScriptPath::Script(script)) => Some(script),
        None => None,
    };

    // Find the project requirements.
    let workspace;
    let target = if let Some(script) = script.as_ref() {
        LockTarget::Script(script)
    } else {
        workspace = VirtualProject::discover(
            project_dir,
            &DiscoveryOptions::default(),
            cache,
            workspace_cache,
        )
        .await?;
        LockTarget::Workspace(workspace.workspace())
    };

    // Share discovered metadata with the lock operation.
    let state = UniversalState::default();

    // Determine the lock mode.
    let interpreter;
    let mode = if let Some(frozen_source) = frozen {
        LockMode::Frozen(frozen_source.into())
    } else {
        interpreter = match target {
            LockTarget::Workspace(workspace) => {
                let workspace_groups = discover_workspace_groups(
                    workspace,
                    project_dir,
                    python.as_deref(),
                    lock_check,
                    &settings,
                    &client_builder,
                    &state,
                    &BTreeSet::new(),
                    python_preference,
                    python_arch,
                    python_downloads,
                    &install_mirrors,
                    &concurrency,
                    config_discovery,
                    cache,
                    workspace_cache,
                    printer,
                    preview,
                )
                .await?;
                let grouped_workspace = (!workspace_groups.is_empty())
                    .then(|| workspace.with_workspace_groups(&workspace_groups))
                    .transpose()?;
                let workspace = grouped_workspace.as_ref().unwrap_or(workspace);
                // Don't enable dependency groups' requires-python for interpreter discovery.
                let groups = DependencyGroupsWithDefaults::none();
                let project_python = ProjectPythonRequest::from_request(
                    python.as_deref().map(PythonRequest::parse),
                    Some(workspace),
                    &groups,
                    &settings.sources,
                    project_dir,
                    config_discovery,
                )
                .await?;
                ProjectInterpreter::discover(
                    ProjectEnvironmentTarget::from(workspace),
                    project_python,
                    &client_builder,
                    python_preference,
                    python_arch,
                    python_downloads,
                    &install_mirrors,
                    ProjectEnvironmentPolicy::Optional,
                    ActiveEnvironment::Ignore,
                    cache,
                    printer,
                )
                .await?
                .into_interpreter()
            }
            LockTarget::Script(script) => ScriptInterpreter::discover(
                script.into(),
                python.as_deref().map(PythonRequest::parse),
                &client_builder,
                python_preference,
                python_arch,
                python_downloads,
                &install_mirrors,
                false,
                config_discovery,
                ActiveEnvironment::Ignore,
                cache,
                printer,
            )
            .await?
            .into_interpreter(),
        };

        if let LockCheck::Enabled(lock_check) = lock_check {
            LockMode::Locked(&interpreter, lock_check)
        } else if dry_run.enabled() {
            LockMode::DryRun(&interpreter)
        } else {
            LockMode::Write(&interpreter)
        }
    };

    // Perform the lock operation.
    match Box::pin(
        LockOperation::new(
            mode,
            &settings,
            &client_builder,
            &state,
            Box::new(DefaultResolveLogger),
            &concurrency,
            cache,
            workspace_cache,
            printer,
            preview,
        )
        .with_refresh(&refresh)
        .with_lockfile_contents_check(
            matches!(&refresh, Refresh::All(..))
                && preview.is_enabled(PreviewFeature::LockfileFormatCheck),
        )
        .execute(target),
    )
    .await
    {
        Ok(lock) => {
            if let Some(frozen_source) = frozen {
                warn_user!(
                    "The lockfile at `uv.lock` was only checked for validity, not whether it is up-to-date, because {} was provided; use `--check` instead",
                    MissingLockfileSource::from(frozen_source)
                );
            }

            if dry_run.enabled() {
                // In `--dry-run` mode, show all changes.
                if let LockResult::Changed(previous, lock) = &lock {
                    let mut changed = false;
                    for event in LockEvent::detect_changes(previous.as_ref(), lock, dry_run) {
                        changed = true;
                        writeln!(printer.stderr(), "{event}")?;
                    }

                    // If we didn't report any version changes, but the lockfile changed, report back.
                    if !changed {
                        writeln!(printer.stderr(), "{}", "Lockfile changes detected".bold())?;
                    }
                } else {
                    writeln!(
                        printer.stderr(),
                        "{}",
                        "No lockfile changes detected".bold()
                    )?;
                }
            } else {
                if let LockResult::Changed(Some(previous), lock) = &lock {
                    for event in LockEvent::detect_changes(Some(previous), lock, dry_run) {
                        writeln!(printer.stderr(), "{event}")?;
                    }
                }
            }

            Ok(ExitStatus::Success)
        }
        // Lock mismatches from `--check`/`--locked` are expected validation failures.
        Err(err @ (LockError::LockMismatch(..) | LockError::LockFormat(..))) => {
            Err(UvError::user(err).into())
        }
        Err(err) => Err(UvError::from(err).into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct LockEventVersion<'lock> {
    /// The version of the package, or `None` if the package has a dynamic version.
    version: Option<&'lock Version>,
    /// The short Git SHA of the package, if it was installed from a Git repository.
    sha: Option<&'lock str>,
}

impl<'lock> From<&'lock Package> for LockEventVersion<'lock> {
    fn from(value: &'lock Package) -> Self {
        Self {
            version: value.version(),
            sha: value.git_sha().map(GitOid::as_tiny_str),
        }
    }
}

impl std::fmt::Display for LockEventVersion<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.version, self.sha) {
            (Some(version), Some(sha)) => write!(f, "v{version} ({sha})"),
            (Some(version), None) => write!(f, "v{version}"),
            (None, Some(sha)) => write!(f, "(dynamic) ({sha})"),
            (None, None) => write!(f, "(dynamic)"),
        }
    }
}

/// A modification to a lockfile.
#[derive(Debug, Clone)]
pub(super) enum LockEvent<'lock> {
    Update(
        DryRun,
        PackageName,
        BTreeSet<LockEventVersion<'lock>>,
        BTreeSet<LockEventVersion<'lock>>,
    ),
    Add(DryRun, PackageName, BTreeSet<LockEventVersion<'lock>>),
    Remove(DryRun, PackageName, BTreeSet<LockEventVersion<'lock>>),
}

impl<'lock> LockEvent<'lock> {
    /// Detect the change events between an (optional) existing and updated lockfile.
    pub(super) fn detect_changes(
        existing_lock: Option<&'lock Lock>,
        new_lock: &'lock Lock,
        dry_run: DryRun,
    ) -> impl Iterator<Item = Self> {
        // Identify the package-versions in the existing lockfile.
        let mut existing_packages: FxHashMap<&PackageName, BTreeSet<LockEventVersion>> =
            if let Some(existing_lock) = existing_lock {
                existing_lock.packages().iter().fold(
                    FxHashMap::with_capacity_and_hasher(
                        existing_lock.packages().len(),
                        FxBuildHasher,
                    ),
                    |mut acc, package| {
                        acc.entry(package.name())
                            .or_default()
                            .insert(LockEventVersion::from(package));
                        acc
                    },
                )
            } else {
                FxHashMap::default()
            };

        // Identify the package-versions in the updated lockfile.
        let mut new_packages: FxHashMap<&PackageName, BTreeSet<LockEventVersion>> =
            new_lock.packages().iter().fold(
                FxHashMap::with_capacity_and_hasher(new_lock.packages().len(), FxBuildHasher),
                |mut acc, package| {
                    acc.entry(package.name())
                        .or_default()
                        .insert(LockEventVersion::from(package));
                    acc
                },
            );

        let names = existing_packages
            .keys()
            .chain(new_packages.keys())
            .map(|name| (*name).clone())
            .collect::<BTreeSet<_>>();

        names.into_iter().filter_map(move |name| {
            match (existing_packages.remove(&name), new_packages.remove(&name)) {
                (Some(existing_versions), Some(new_versions)) => {
                    if existing_versions != new_versions {
                        Some(Self::Update(dry_run, name, existing_versions, new_versions))
                    } else {
                        None
                    }
                }
                (Some(existing_versions), None) => {
                    Some(Self::Remove(dry_run, name, existing_versions))
                }
                (None, Some(new_versions)) => Some(Self::Add(dry_run, name, new_versions)),
                (None, None) => {
                    unreachable!("The key `{name}` should exist in at least one of the maps");
                }
            }
        })
    }

    pub(super) fn package(&self) -> &PackageName {
        match self {
            Self::Update(_, package, ..)
            | Self::Add(_, package, ..)
            | Self::Remove(_, package, ..) => package,
        }
    }
}

impl std::fmt::Display for LockEvent<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Update(dry_run, name, existing_versions, new_versions) => {
                let existing_versions = existing_versions
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let new_versions = new_versions
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");

                write!(
                    f,
                    "{} {name} {existing_versions} -> {new_versions}",
                    if dry_run.enabled() {
                        "Update"
                    } else {
                        "Updated"
                    }
                    .green()
                    .bold()
                )
            }
            Self::Add(dry_run, name, new_versions) => {
                let new_versions = new_versions
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");

                write!(
                    f,
                    "{} {name} {new_versions}",
                    if dry_run.enabled() { "Add" } else { "Added" }
                        .green()
                        .bold()
                )
            }
            Self::Remove(dry_run, name, existing_versions) => {
                let existing_versions = existing_versions
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");

                write!(
                    f,
                    "{} {name} {existing_versions}",
                    if dry_run.enabled() {
                        "Remove"
                    } else {
                        "Removed"
                    }
                    .red()
                    .bold()
                )
            }
        }
    }
}
