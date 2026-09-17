use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

use owo_colors::OwoColorize;
use rustc_hash::{FxBuildHasher, FxHashMap};

use uv_cache::{Cache, Refresh};
use uv_client::BaseClientBuilder;
use uv_command_support::{ExitStatus, Printer, UvError};
use uv_configuration::{
    ActiveEnvironment, Concurrency, DependencyGroupsWithDefaults, DryRun, NoSources,
};
use uv_dispatch::UniversalState;
use uv_distribution_types::RequiresPython;
use uv_environment_operations::{
    ProjectEnvironmentPolicy, ProjectEnvironmentTarget, ProjectInterpreter,
};
use uv_git_types::GitOid;
use uv_lock::{Lock, Package, implicit_constraints_marker};
use uv_lock_operations::{
    LockError, LockMode, LockOperation, LockResult, LockTarget, MissingLockfileSource,
};
use uv_normalize::{GroupName, PackageName};
use uv_pep440::Version;
use uv_pep508::MarkerTree;
use uv_preview::{Preview, PreviewFeature};
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
    DiscoveryOptions, ResolvedWorkspaceGroup, VirtualProject, Workspace, WorkspaceCache,
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
    if lock.workspace_groups().is_empty() {
        return if let Some(name) = name {
            Err(ProjectError::MissingWorkspaceGroupLock(name.clone()))
        } else {
            Ok(lock)
        };
    }
    let members = if members.is_empty() {
        lock.members()
    } else {
        members
    };
    let name = name.or_else(|| {
        lock.workspace_groups()
            .iter()
            .find(|group| group.definition.default)
            .map(|group| &group.definition.name)
    });
    if let Some(name) = name {
        let selected = lock
            .select_workspace_group(name)?
            .ok_or_else(|| ProjectError::MissingWorkspaceGroupLock(name.clone()))?;
        if selected.select_workspace_members(members)?.is_none() {
            return Err(ProjectError::WorkspaceGroupTarget(name.clone()));
        }
        return Ok(selected);
    }
    let mut candidates = Vec::new();
    let mut covered = BTreeSet::new();
    for group in lock.workspace_groups() {
        let Some(candidate) = lock.select_workspace_group(&group.definition.name)? else {
            continue;
        };
        let available = candidate
            .packages()
            .iter()
            .map(Package::name)
            .collect::<BTreeSet<_>>();
        let contained = members
            .iter()
            .filter(|name| available.contains(name))
            .cloned()
            .collect::<BTreeSet<_>>();
        if contained.is_empty() {
            continue;
        }
        let Some(candidate) = candidate.select_workspace_members(&contained)? else {
            continue;
        };
        covered.extend(contained);
        candidates.push(candidate);
    }
    if covered != *members {
        return Err(ProjectError::WorkspaceGroupUncovered);
    }
    Lock::merge_workspace_resolutions(candidates)?.ok_or(ProjectError::WorkspaceGroupRequired)
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

/// Select group metadata for Python discovery from an existing lockfile.
pub(crate) fn command_workspace_group_from_lock(
    lock: &Lock,
    name: Option<&GroupName>,
    members: &BTreeSet<PackageName>,
) -> Result<Option<ResolvedWorkspaceGroup>, ProjectError> {
    if lock.workspace_groups().is_empty() {
        return if let Some(name) = name {
            Err(ProjectError::MissingWorkspaceGroupLock(name.clone()))
        } else {
            Ok(None)
        };
    }
    let members = if members.is_empty() {
        lock.members()
    } else {
        members
    };
    if let Some(group) = name
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
        })
    {
        return Ok(Some(ResolvedWorkspaceGroup {
            definition: group.definition.clone(),
            requires_python: group.effective_requires_python.clone(),
            environments: group.effective_environment(),
        }));
    }
    if let Some(name) = name {
        return Err(ProjectError::MissingWorkspaceGroupLock(name.clone()));
    }
    let selected = select_workspace_group_lock(lock.clone(), None, members)?;
    // This synthetic view is only used for interpreter discovery. Ordinary targeting
    // keeps the union of compatible contexts instead of choosing one by name.
    for group in lock.workspace_groups() {
        if let Some(candidate) = lock.select_workspace_group(&group.definition.name)?
            && candidate
                .packages()
                .iter()
                .any(|package| members.contains(package.name()))
        {
            let mut definition = group.definition.clone();
            definition.members.clone_from(members);
            definition.requires_python = None;
            definition.default = false;
            return Ok(Some(ResolvedWorkspaceGroup {
                definition,
                requires_python: selected.requires_python().clone(),
                environments: implicit_constraints_marker(
                    selected.requires_python().to_exact_marker_tree(),
                    selected.supported_environments(),
                ),
            }));
        }
    }
    Ok(None)
}

/// Select group metadata for Python discovery, using only the lock in frozen mode.
pub(crate) async fn command_workspace_group(
    workspace: &Workspace,
    name: Option<&GroupName>,
    members: &BTreeSet<PackageName>,
    frozen: Option<FrozenSource>,
    no_sources: &NoSources,
) -> Result<Option<ResolvedWorkspaceGroup>, ProjectError> {
    if let Some(frozen) = frozen {
        let lock = LockTarget::Workspace(workspace)
            .read_frozen(frozen.into())
            .await?;
        return command_workspace_group_from_lock(&lock, name, members);
    }
    let groups = workspace.workspace_groups_with_sources(no_sources)?;
    if let Some(name) = name {
        return groups
            .into_iter()
            .find(|group| group.definition.name == *name)
            .map(Some)
            .ok_or_else(|| {
                uv_workspace::WorkspaceError::from(
                    uv_workspace::WorkspaceErrorKind::UnknownWorkspaceGroup(name.clone()),
                )
                .into()
            });
    }
    if let Some(group) = groups.iter().find(|group| group.definition.default) {
        return Ok(Some(group.clone()));
    }
    let mut candidates = groups
        .iter()
        .filter(|group| members.is_subset(&group.definition.members))
        .cloned()
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        candidates = groups;
    }
    let Some(requires_python) =
        RequiresPython::union(candidates.iter().map(|group| &group.requires_python))
    else {
        return Ok(None);
    };
    let environments = candidates
        .iter()
        .fold(MarkerTree::FALSE, |environment, group| {
            environment.or(group.environments)
        });
    let Some(mut group) = candidates.into_iter().next() else {
        return Ok(None);
    };
    group.definition.members.clone_from(members);
    group.definition.requires_python = None;
    group.definition.default = false;
    group.requires_python = requires_python;
    group.environments = environments;
    Ok(Some(group))
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

    // Determine the lock mode.
    let interpreter;
    let mode = if let Some(frozen_source) = frozen {
        LockMode::Frozen(frozen_source.into())
    } else {
        interpreter = match target {
            LockTarget::Workspace(workspace) => {
                let workspace_groups =
                    workspace.workspace_groups_with_sources(&settings.sources)?;
                let grouped_workspace = (!workspace_groups.is_empty())
                    .then(|| workspace.with_workspace_groups(&workspace_groups));
                let workspace = grouped_workspace.as_ref().unwrap_or(workspace);
                // Don't enable dependency groups' requires-python for interpreter discovery.
                let groups = DependencyGroupsWithDefaults::none();
                let project_python = ProjectPythonRequest::from_request(
                    python.as_deref().map(PythonRequest::parse),
                    Some(workspace),
                    &groups,
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

    // Initialize any shared state.
    let state = UniversalState::default();

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
