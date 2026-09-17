use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use tracing::debug;

use uv_cache::{Cache, Refresh};
use uv_client::BaseClientBuilder;
use uv_configuration::{
    Concurrency, DependencyGroupsWithDefaults, ExtrasSpecificationWithDefaults, ForkStrategy,
    NoSources, ResolutionMode,
};
use uv_distribution_types::{
    IndexUrl, NameRequirementSpecification, Requirement, RequirementSource, RequiresPython,
};
use uv_lock::{
    Lock, LockError, LockedPackageIdentity, Package, WorkspaceAxisSelectionError,
    implicit_constraints_marker,
};
use uv_normalize::{DefaultGroups, PackageName};
use uv_pep440::{Version, VersionSpecifier, VersionSpecifiers};
use uv_pep508::{MarkerEnvironment, MarkerTree};
use uv_preview::{Preview, PreviewFeature};
use uv_pypi_types::{SupportedEnvironments, VerbatimParsedUrl};
use uv_python::Interpreter;
use uv_resolver::{Preference, ResolveError};
use uv_warnings::warn_user_once;
use uv_workspace::{
    DiscoveryOptions, MemberDiscovery, ResolvedWorkspaceAxes, VirtualProject, Workspace,
    WorkspaceAxisAssignment, WorkspaceAxisDomain, WorkspaceAxisError, WorkspaceAxisGroupMetadata,
    WorkspaceAxisName, WorkspaceAxisResolutionView, WorkspaceAxisSelection, WorkspaceCache,
    WorkspaceErrorKind,
};

use crate::commands::locked_requirements::upgrade_packages_for_lock;
use crate::commands::pip::loggers::{ResolveLogger, SummaryResolveLogger};
use crate::commands::pip::operations;
use crate::commands::project::lock::{
    LockMode, LockResult, do_lock, workspace_group_conflict, workspace_selection_members,
};
use crate::commands::project::lock_target::LockTarget;
use crate::commands::project::{ProjectError, UniversalState};
use crate::printer::Printer;
use crate::settings::{FrozenSource, ResolverSettings};

/// A command's concrete roots and potentially partial resolution-axis selection.
pub(crate) struct AxisCommandSelection {
    pub(crate) selection: WorkspaceAxisSelection,
    pub(crate) members: BTreeSet<PackageName>,
    /// Explicit roots for operations that target all members matching the selection.
    pub(crate) matching_members: Option<Vec<PackageName>>,
    /// A physical union used only to choose an interpreter before projecting the lock.
    pub(crate) workspace: Workspace,
}

impl AxisCommandSelection {
    pub(crate) fn select_lock(
        &self,
        lock: &Lock,
        environment: Option<&MarkerEnvironment>,
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<Lock, ProjectError> {
        if lock.workspace_axes().is_none() {
            return Err(ProjectError::MissingWorkspaceAxesResolution);
        }
        Ok(lock.select_workspace_axes_for_command(
            &self.selection,
            &self.members,
            environment,
            extras,
            groups,
        )?)
    }

    pub(crate) fn select_result(
        &self,
        result: LockResult,
        environment: Option<&MarkerEnvironment>,
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<LockResult, ProjectError> {
        Ok(match result {
            LockResult::Unchanged(lock) => {
                LockResult::Unchanged(self.select_lock(&lock, environment, extras, groups)?)
            }
            LockResult::Changed(previous, lock) => LockResult::Changed(
                previous.and_then(|lock| self.select_lock(&lock, environment, extras, groups).ok()),
                self.select_lock(&lock, environment, extras, groups)?,
            ),
        })
    }
}

/// Discover a frozen command's project without requiring every locked axis member to remain on
/// disk. Ordinary workspaces retain their normal discovery behavior and diagnostics.
pub(crate) async fn discover_frozen_workspace_axis_project(
    project_dir: &Path,
    options: &DiscoveryOptions,
    package: Option<&PackageName>,
    frozen: FrozenSource,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
) -> Result<VirtualProject, ProjectError> {
    let result = if let Some(package) = package {
        VirtualProject::discover_with_package(
            project_dir,
            options,
            cache,
            workspace_cache,
            package.clone(),
        )
        .await
    } else {
        VirtualProject::discover(project_dir, options, cache, workspace_cache).await
    };
    let error = match result {
        Ok(project) => return Ok(project),
        Err(error)
            if matches!(
                error.as_ref(),
                WorkspaceErrorKind::NoSuchMember(..)
                    | WorkspaceErrorKind::MissingPyprojectTomlMember(..)
            ) =>
        {
            error
        }
        Err(error) => return Err(error.into()),
    };
    let options = DiscoveryOptions {
        members: MemberDiscovery::Existing,
        ..options.clone()
    };
    let Ok(project) = VirtualProject::discover(project_dir, &options, cache, workspace_cache).await
    else {
        return Err(error.into());
    };
    let Ok(lock) = LockTarget::Workspace(project.workspace())
        .read_frozen(frozen.into())
        .await
    else {
        return Err(error.into());
    };
    let Some(axes) = lock.workspace_axes() else {
        return Err(error.into());
    };
    if let Some(package) = package {
        if !axes.model().members().contains(package) {
            return Err(error.into());
        }
        if project.workspace().packages().contains_key(package) {
            return Ok(VirtualProject::discover_with_package(
                project_dir,
                &options,
                cache,
                workspace_cache,
                package.clone(),
            )
            .await?);
        }
    }
    Ok(project)
}

/// Read axis-aware defaults before constructing a frozen command's group selection.
pub(crate) async fn command_workspace_axis_default_groups(
    project: &VirtualProject,
    packages: &[PackageName],
    frozen: Option<FrozenSource>,
) -> Result<Option<DefaultGroups>, ProjectError> {
    let Some(frozen) = frozen else {
        return Ok(None);
    };
    let lock = LockTarget::Workspace(project.workspace())
        .read_frozen(frozen.into())
        .await?;
    locked_workspace_axis_default_groups(project, &lock, packages)
}

/// A single explicitly selected package uses its locked defaults. Zero or multiple package
/// arguments use the current project's defaults, matching ordinary workspace commands.
pub(crate) fn locked_workspace_axis_default_groups(
    project: &VirtualProject,
    lock: &Lock,
    packages: &[PackageName],
) -> Result<Option<DefaultGroups>, ProjectError> {
    let Some(axes) = lock.workspace_axes() else {
        return Ok(None);
    };
    let member = if let [name] = packages {
        Some(name)
    } else {
        project.project_name()
    };
    let defaults = axes
        .group_metadata()
        .default_groups(member)
        .ok_or_else(|| {
            member.map_or_else(
                || {
                    WorkspaceAxisError::InvalidGroupMetadata(
                        "missing workspace defaults".to_owned(),
                    )
                },
                |member| WorkspaceAxisError::UnknownMember(member.clone()),
            )
        })?;
    Ok(Some(defaults.clone()))
}

/// Resolve command selectors before interpreter discovery. Frozen operations use only the locked
/// axis definitions and physical coverage, including when member files are no longer available.
pub(crate) async fn command_workspace_axes(
    project: &VirtualProject,
    assignments: &[WorkspaceAxisAssignment],
    packages: &[PackageName],
    all_packages: bool,
    all_matching_packages: bool,
    extras: &ExtrasSpecificationWithDefaults,
    groups: &DependencyGroupsWithDefaults,
    frozen: Option<FrozenSource>,
    no_sources: &NoSources,
) -> Result<Option<AxisCommandSelection>, ProjectError> {
    let workspace = project.workspace();
    if let Some(frozen) = frozen {
        let lock = LockTarget::Workspace(workspace)
            .read_frozen(frozen.into())
            .await?;
        return locked_command_workspace_axes(
            project,
            &lock,
            assignments,
            packages,
            all_packages,
            all_matching_packages,
            extras,
            groups,
        );
    }
    let Some(axes) = workspace.resolution_axes()? else {
        return if all_matching_packages || !assignments.is_empty() {
            Err(ProjectError::WorkspaceAxesNotConfigured)
        } else {
            Ok(None)
        };
    };

    let SelectedAxisRoots {
        mut selection,
        members,
        matching_members,
    } = select_roots(
        project,
        &axes,
        assignments,
        packages,
        all_packages,
        all_matching_packages,
    )?;
    let group_metadata = workspace.resolution_axis_group_metadata()?;
    let mut interpreter_roots = members.clone();
    if let Some(group_root) = group_metadata.group_root(&members, groups) {
        selection.merge(&axes.selection_for_members([group_root])?)?;
        interpreter_roots.insert(group_root.clone());
    }
    let domain = axes
        .domain()
        .restrict(&selection)
        .ok_or(WorkspaceAxisError::InvalidDomain)?;
    let environment = workspace
        .environment_for_domain(&axes, &domain, no_sources)?
        .environments
        .and(group_metadata.python_marker(&members, groups));
    let requires_python = RequiresPython::from_marker_tree(environment)
        .ok_or(WorkspaceAxisSelectionError::Incompatible)?;
    let scoped = workspace.with_resolution_axis_command_environment(
        interpreter_roots,
        requires_python,
        SupportedEnvironments::from_markers(vec![environment]),
    );
    Ok(Some(AxisCommandSelection {
        selection,
        matching_members,
        members,
        workspace: scoped,
    }))
}

/// Select from an already-read universal lock. Batch export uses this for each independent entry.
pub(crate) fn locked_command_workspace_axes(
    project: &VirtualProject,
    lock: &Lock,
    assignments: &[WorkspaceAxisAssignment],
    packages: &[PackageName],
    all_packages: bool,
    all_matching_packages: bool,
    extras: &ExtrasSpecificationWithDefaults,
    groups: &DependencyGroupsWithDefaults,
) -> Result<Option<AxisCommandSelection>, ProjectError> {
    let Some(axes) = lock.workspace_axes() else {
        return if all_matching_packages || !assignments.is_empty() {
            Err(ProjectError::WorkspaceAxesNotConfigured)
        } else {
            Ok(None)
        };
    };
    let SelectedAxisRoots {
        selection,
        members,
        matching_members,
    } = select_roots(
        project,
        axes.model(),
        assignments,
        packages,
        all_packages,
        all_matching_packages,
    )?;
    let environment =
        lock.workspace_axis_environment_for_command(&selection, &members, extras, groups)?;
    let requires_python =
        lock.workspace_axis_requires_python_for_command(&selection, &members, extras, groups)?;
    let mut interpreter_roots = members.clone();
    if let Some(group_root) = axes.group_metadata().group_root(&members, groups) {
        interpreter_roots.insert(group_root.clone());
    }
    let scoped = project
        .workspace()
        .with_resolution_axis_command_environment(
            interpreter_roots,
            requires_python,
            SupportedEnvironments::from_markers(vec![environment]),
        );
    Ok(Some(AxisCommandSelection {
        selection,
        matching_members,
        members,
        workspace: scoped,
    }))
}

/// Choose a build interpreter for several independent command selections without combining their
/// requested roots into a single installation target.
pub(crate) fn command_workspace_axes_python_view(
    workspace: &Workspace,
    assignments: &[WorkspaceAxisAssignment],
    no_sources: &NoSources,
) -> Result<Option<Workspace>, ProjectError> {
    let Some(axes) = workspace.resolution_axes()? else {
        return if assignments.is_empty() {
            Ok(None)
        } else {
            Err(ProjectError::WorkspaceAxesNotConfigured)
        };
    };
    let selection = WorkspaceAxisSelection::from_assignments(assignments.iter().cloned())?;
    axes.validate_selection(&selection)?;
    let domain = axes
        .domain()
        .restrict(&selection)
        .ok_or(WorkspaceAxisError::InvalidDomain)?;
    Ok(Some(
        workspace.resolution_axis_python_view(&axes, &domain, no_sources)?,
    ))
}

/// Choose an interpreter from the contexts in which one workspace member is available.
pub(crate) fn member_workspace_axes_python_view(
    workspace: &Workspace,
    member: &PackageName,
    no_sources: &NoSources,
) -> Result<Option<Workspace>, ProjectError> {
    let Some(axes) = workspace.resolution_axes()? else {
        return Ok(None);
    };
    let selection = axes.selection_for_members([member])?;
    let domain = axes
        .domain()
        .restrict(&selection)
        .ok_or(WorkspaceAxisError::InvalidDomain)?;
    Ok(Some(
        workspace.resolution_axis_python_view(&axes, &domain, no_sources)?,
    ))
}

struct SelectedAxisRoots {
    selection: WorkspaceAxisSelection,
    members: BTreeSet<PackageName>,
    matching_members: Option<Vec<PackageName>>,
}

fn select_roots(
    project: &VirtualProject,
    axes: &ResolvedWorkspaceAxes,
    assignments: &[WorkspaceAxisAssignment],
    packages: &[PackageName],
    all_packages: bool,
    all_matching_packages: bool,
) -> Result<SelectedAxisRoots, ProjectError> {
    let mut selection = WorkspaceAxisSelection::from_assignments(assignments.iter().cloned())?;
    axes.validate_selection(&selection)?;
    let matching =
        all_matching_packages || (!all_packages && packages.is_empty() && project.is_non_project());
    let members = if matching {
        matching_roots(axes, &selection)?
    } else if all_packages {
        // Frozen selection is defined by the lock, not the subset of member directories that is
        // currently present on disk.
        axes.members().clone()
    } else {
        workspace_selection_members(project, packages, false)
    };
    selection.merge(&axes.selection_for_members(&members)?)?;
    let matching_members = matching.then(|| members.iter().cloned().collect());
    Ok(SelectedAxisRoots {
        selection,
        members,
        matching_members,
    })
}

fn matching_roots(
    axes: &ResolvedWorkspaceAxes,
    selection: &WorkspaceAxisSelection,
) -> Result<BTreeSet<PackageName>, ProjectError> {
    let domain = axes
        .domain()
        .restrict(selection)
        .ok_or(WorkspaceAxisError::InvalidDomain)?;
    let possible = axes.possible_roots(&domain);
    let guaranteed = axes.guaranteed_roots(&domain);
    if possible != guaranteed {
        let unresolved = possible
            .difference(&guaranteed)
            .filter_map(|member| axes.assignments().get(member))
            .flat_map(WorkspaceAxisSelection::iter)
            .filter(|(axis, _)| domain.get(axis).is_some_and(|sections| sections.len() > 1))
            .map(|(axis, _)| axis.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return Err(WorkspaceAxisSelectionError::MatchingRoots { axes: unresolved }.into());
    }
    Ok(possible)
}

/// State shared by ordinary resolver invocations. Each invocation starts a fresh PubGrub solve;
/// metadata, build caches, and successful version preferences remain reusable.
struct AxisResolver<'a, 'client> {
    interpreter: &'a Interpreter,
    mode: LockMode<'a>,
    external: &'a [NameRequirementSpecification],
    base_preferences: &'a [Preference],
    refresh: Option<&'a Refresh>,
    settings: &'a ResolverSettings,
    client_builder: &'a BaseClientBuilder<'client>,
    state: &'a UniversalState,
    concurrency: &'a Concurrency,
    cache: &'a Cache,
    workspace_cache: &'a WorkspaceCache,
    printer: Printer,
    preview: Preview,
}

impl AxisResolver<'_, '_> {
    async fn resolve(
        &self,
        workspace: &Workspace,
        previous: Option<Lock>,
        constraints: impl IntoIterator<Item = NameRequirementSpecification>,
        preferences: &[Preference],
    ) -> Result<LockResult, ProjectError> {
        Box::pin(do_lock(
            LockTarget::Workspace(workspace),
            self.interpreter,
            previous,
            self.mode,
            None,
            self.external.iter().cloned().chain(constraints).collect(),
            self.base_preferences
                .iter()
                .chain(preferences)
                .cloned()
                .collect(),
            self.refresh,
            self.settings,
            self.client_builder,
            self.state,
            Box::new(SummaryResolveLogger),
            self.concurrency,
            self.cache,
            self.workspace_cache,
            self.printer,
            self.preview,
        ))
        .await
    }
}

struct ResolvedDomain {
    domain: WorkspaceAxisDomain,
    workspace: Workspace,
    lock: Lock,
    /// Same-context input choices that survived the primary solve and were not selected for
    /// upgrade. Optional alignment must not displace them.
    protected: BTreeMap<LockedPackageIdentity, MarkerTree>,
}

/// Cover the complete declared product with shared solves, refining declared axes only when a
/// conservative solve cannot represent all worlds in the current domain.
pub(super) async fn do_lock_workspace_axes(
    workspace: &Workspace,
    axes: ResolvedWorkspaceAxes,
    interpreter: &Interpreter,
    existing_lock: Option<Lock>,
    mode: LockMode<'_>,
    check_lockfile_contents: Option<String>,
    external: Vec<NameRequirementSpecification>,
    additional_preferences: Vec<Preference>,
    refresh: Option<&Refresh>,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    state: &UniversalState,
    logger: Box<dyn ResolveLogger>,
    concurrency: &Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> Result<LockResult, ProjectError> {
    let start = Instant::now();
    if !preview.is_enabled(PreviewFeature::WorkspaceResolutionAxes) {
        warn_user_once!(
            "Workspace resolution axes are experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::WorkspaceResolutionAxes
        );
    }
    let resolver = AxisResolver {
        interpreter,
        mode,
        external: &external,
        base_preferences: &additional_preferences,
        refresh,
        settings,
        client_builder,
        state,
        concurrency,
        cache,
        workspace_cache,
        printer,
        preview,
    };
    let expected_environment = workspace
        .environment_for_domain(&axes, &axes.domain(), &settings.sources)?
        .marker;
    let group_metadata = workspace.resolution_axis_group_metadata()?;
    if let Some(existing) = &existing_lock
        && settings.upgrade.is_none()
        && external.is_empty()
        && reuse_workspace_axes(
            &resolver,
            workspace,
            &axes,
            expected_environment,
            &group_metadata,
            existing,
        )
        .await?
    {
        let lock = existing.clone();
        logger.on_complete(lock.len(), start, printer)?;
        let unchanged = if let Some(contents) = &check_lockfile_contents {
            *contents == lock.to_toml()?
        } else {
            true
        };
        return Ok(if unchanged {
            LockResult::Unchanged(lock)
        } else {
            LockResult::Changed(existing_lock, lock)
        });
    }
    let mut pending = existing_lock
        .as_ref()
        .filter(|previous| {
            previous.resolution_mode() == settings.resolution
                && previous.fork_strategy() == settings.fork_strategy
        })
        .and_then(Lock::workspace_axes)
        .filter(|previous| {
            !settings.upgrade.is_all()
                && previous.model() == &axes
                && same_environment(previous.environment(), expected_environment)
        })
        .map(|previous| {
            // Keeping a valid partition lets each ordinary solve use its own previous choices.
            // A full upgrade can reconsider the partition as well as its versions.
            previous
                .contexts()
                .iter()
                .rev()
                .map(|context| context.domain.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![axes.domain()]);
    let mut resolutions = Vec::new();
    let mut preference_ledger = Vec::<(MarkerTree, Vec<Preference>)>::new();
    let mut changed = !settings.upgrade.is_none()
        || existing_lock.as_ref().is_none_or(|lock| {
            lock.resolution_mode() != settings.resolution
                || lock.fork_strategy() != settings.fork_strategy
                || lock
                    .workspace_axes()
                    .is_none_or(|previous| previous.model() != &axes)
        });
    while let Some(domain) = pending.pop() {
        if settings.resolution == ResolutionMode::LowestDirect
            && let Some(axis) = varying_root_axis(&axes, &domain)
        {
            // Directness is a property of a context's roots. Combining different root sets would
            // change lowest-direct into lowest-for-the-union even if a shared solve succeeds.
            let (left, right) = domain
                .split(&axis)
                .ok_or(WorkspaceAxisError::InvalidDomain)?;
            debug!(%axis, "Refining workspace resolution domain for lowest-direct roots");
            pending.push(right);
            pending.push(left);
            continue;
        }
        let (scoped, physical_environment) =
            match workspace.resolution_axis_view(&axes, &domain, &settings.sources)? {
                WorkspaceAxisResolutionView::Ready {
                    workspace,
                    environment,
                } => (*workspace, environment.environments),
                WorkspaceAxisResolutionView::Refine(axis) => {
                    let (left, right) = domain
                        .split(&axis)
                        .ok_or(WorkspaceAxisError::InvalidDomain)?;
                    debug!(%axis, "Refining workspace resolution domain for physical coverage");
                    pending.push(right);
                    pending.push(left);
                    continue;
                }
            };
        let previous = if let Some(existing) = &existing_lock {
            if existing.workspace_axes().is_some() {
                existing.workspace_axis_preferences(&domain)?
            } else if existing.workspace_groups().is_empty() {
                Some(existing.clone())
            } else {
                None
            }
        } else {
            None
        };
        let mut protected = previous
            .as_ref()
            .map(|previous| protected_choices(previous, settings))
            .unwrap_or_default();
        // The requires-python strategy intentionally allows newer Python lanes to choose newer
        // releases. Cross-lane suggestions must not silently replace that policy with `fewest`.
        let preferences = preference_ledger
            .iter()
            .filter(|(environment, _)| {
                settings.fork_strategy == ForkStrategy::Fewest
                    || same_environment(*environment, physical_environment)
            })
            .flat_map(|(_, preferences)| preferences.iter().cloned())
            .collect::<Vec<_>>();
        match resolver
            .resolve(&scoped, previous, std::iter::empty(), &preferences)
            .await
        {
            Ok(result) => {
                let result_changed = matches!(result, LockResult::Changed(..));
                let lock = result.into_lock();
                if let Err(error) = lock.validate_workspace_axis_cohort(&axes, &domain, workspace) {
                    if let Some(axis) = unavailable_member_axis(&axes, &domain, &error)
                        && let Some((left, right)) = domain.split(&axis)
                    {
                        debug!(%axis, "Refining workspace resolution domain for local member availability");
                        pending.push(right);
                        pending.push(left);
                        continue;
                    }
                    return Err(error.into());
                }
                changed |= result_changed;
                if !protected.is_empty() {
                    let selected = distribution_markers(&lock);
                    protected.retain(|identity, marker| {
                        *marker = marker
                            .and(selected.get(identity).copied().unwrap_or(MarkerTree::FALSE));
                        !marker.is_false()
                    });
                }
                if settings.resolution != ResolutionMode::LowestDirect {
                    let mut preferences = Vec::new();
                    extend_preferences(&mut preferences, &lock, workspace.install_path())?;
                    preference_ledger.push((physical_environment, preferences));
                }
                resolutions.push(ResolvedDomain {
                    domain,
                    workspace: scoped,
                    lock,
                    protected,
                });
            }
            Err(error) if workspace_group_conflict(&error) => {
                if let Some(axis) = conflicting_axis(workspace, &axes, &domain, &error)
                    && let Some((left, right)) = domain.split(&axis)
                {
                    debug!(%axis, "Refining incompatible workspace resolution domain");
                    pending.push(right);
                    pending.push(left);
                } else {
                    return Err(ProjectError::WorkspaceAxesResolution(
                        domain
                            .witness()
                            .map(|selection| selection.to_string())
                            .unwrap_or_default(),
                        Box::new(error),
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }

    if changed && settings.resolution != ResolutionMode::LowestDirect {
        reconcile(&resolver, &axes, &mut resolutions).await?;
    }
    let lock = Lock::from_workspace_axes_with_environment(
        axes,
        expected_environment,
        group_metadata,
        resolutions
            .into_iter()
            .map(|resolution| (resolution.domain, resolution.lock))
            .collect(),
    )?
    .ok_or(ProjectError::MissingWorkspaceAxesResolution)?;
    logger.on_complete(lock.len(), start, printer)?;
    let unchanged = if let Some(contents) = check_lockfile_contents {
        existing_lock.is_some() && contents == lock.to_toml()?
    } else if let Some(existing) = &existing_lock {
        existing.to_toml()? == lock.to_toml()?
    } else {
        false
    };
    Ok(if unchanged {
        LockResult::Unchanged(lock)
    } else {
        LockResult::Changed(existing_lock, lock)
    })
}

/// A valid lock's partition is stable input, not another optimization decision. Recheck its
/// ordinary cohorts before reusing it; any changed requirements return to shared-first solving.
async fn reuse_workspace_axes(
    resolver: &AxisResolver<'_, '_>,
    workspace: &Workspace,
    axes: &ResolvedWorkspaceAxes,
    expected_environment: MarkerTree,
    group_metadata: &WorkspaceAxisGroupMetadata,
    existing: &Lock,
) -> Result<bool, ProjectError> {
    let Some(previous) = existing.workspace_axes() else {
        return Ok(false);
    };
    let member_sources = workspace
        .packages()
        .iter()
        .map(|(name, member)| (name.clone(), member.root().clone()))
        .collect();
    if previous.model() != axes
        || previous.group_metadata() != group_metadata
        || !previous.matches_member_sources(workspace.install_path(), &member_sources)
        || !same_environment(previous.environment(), expected_environment)
    {
        return Ok(false);
    }
    for context in previous.contexts() {
        let WorkspaceAxisResolutionView::Ready {
            workspace: scoped, ..
        } = workspace.resolution_axis_view(axes, &context.domain, &resolver.settings.sources)?
        else {
            return Ok(false);
        };
        let Some(preferences) = existing.workspace_axis_preferences(&context.domain)? else {
            return Ok(false);
        };
        let previous_contents = preferences.to_toml()?;
        let result = resolver
            .resolve(&scoped, Some(preferences), std::iter::empty(), &[])
            .await;
        let lock = match result {
            Ok(result) => result.into_lock(),
            Err(error) if workspace_group_conflict(&error) => return Ok(false),
            Err(error) => return Err(error),
        };
        match lock.validate_workspace_axis_cohort(axes, &context.domain, workspace) {
            Ok(()) => {}
            Err(WorkspaceAxisError::IncompatibleMember { .. }) => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        if lock.to_toml()? != previous_contents {
            return Ok(false);
        }
    }
    Ok(true)
}

fn unavailable_member_axis(
    axes: &ResolvedWorkspaceAxes,
    domain: &WorkspaceAxisDomain,
    error: &WorkspaceAxisError,
) -> Option<WorkspaceAxisName> {
    let WorkspaceAxisError::IncompatibleMember { member, selection } = error else {
        return None;
    };
    axes.assignments()
        .get(member)?
        .iter()
        .find_map(|(axis, required)| {
            (selection.get(axis) != Some(required)
                && domain
                    .get(axis)
                    .is_some_and(|sections| sections.len() > 1 && sections.contains(required)))
            .then(|| axis.clone())
        })
}

fn varying_root_axis(
    axes: &ResolvedWorkspaceAxes,
    domain: &WorkspaceAxisDomain,
) -> Option<WorkspaceAxisName> {
    let possible = axes.possible_roots(domain);
    let guaranteed = axes.guaranteed_roots(domain);
    domain.iter().find_map(|(axis, sections)| {
        (sections.len() > 1
            && possible.difference(&guaranteed).any(|member| {
                axes.assignments()
                    .get(member)
                    .is_some_and(|selection| selection.get(axis).is_some())
            }))
        .then(|| axis.clone())
    })
}

/// Prefer selectors named by the structured incompatibility and the roots or policies that
/// introduced it. Unrelated axes remain symbolic even when their names sort before the conflict.
fn conflicting_axis(
    workspace: &Workspace,
    axes: &ResolvedWorkspaceAxes,
    domain: &WorkspaceAxisDomain,
    error: &ProjectError,
) -> Option<WorkspaceAxisName> {
    fn resolver_packages(error: &ResolveError, packages: &mut BTreeSet<PackageName>) {
        match error {
            ResolveError::Dependencies(source, package, ..) => {
                packages.insert(package.clone());
                resolver_packages(source, packages);
            }
            ResolveError::NoSolution(source) => packages.extend(source.packages().cloned()),
            ResolveError::ConflictingUrls { package_name, .. }
            | ResolveError::ConflictingIndexesForEnvironment { package_name, .. }
            | ResolveError::ConflictingIndexes(package_name, ..) => {
                packages.insert(package_name.clone());
            }
            _ => {}
        }
    }
    let mut packages = BTreeSet::new();
    match error {
        ProjectError::Operation(operations::Error::NoSolution { source, .. }) => {
            packages.extend(source.packages().cloned());
        }
        ProjectError::Operation(operations::Error::Resolve(source)) => {
            resolver_packages(source, &mut packages);
        }
        _ => {}
    }
    let possible = axes.possible_roots(domain);
    let related = possible
        .iter()
        .filter(|name| {
            packages.contains(*name)
                || workspace.packages().get(*name).is_some_and(|member| {
                    member
                        .project()
                        .dependencies
                        .iter()
                        .flatten()
                        .any(|requirement| {
                            requirement
                                .parse::<uv_pep508::Requirement<VerbatimParsedUrl>>()
                                .is_ok_and(|requirement| packages.contains(&requirement.name))
                        })
                })
        })
        .collect::<BTreeSet<_>>();
    let mut best = None;
    for (axis, allowed) in domain.iter().filter(|(_, sections)| sections.len() > 1) {
        let Some(sections) = axes.definitions().get(axis) else {
            continue;
        };
        let mut policy_hits = 0;
        let mut policies = 0;
        for section in allowed.iter().filter_map(|section| sections.get(section)) {
            policies += section.constraint_dependencies.len();
            policy_hits += section
                .constraint_dependencies
                .iter()
                .filter(|constraint| packages.contains(&constraint.name))
                .count();
        }
        let assigned = possible
            .iter()
            .filter(|member| {
                axes.assignments()
                    .get(*member)
                    .is_some_and(|selection| selection.get(axis).is_some())
            })
            .collect::<Vec<_>>();
        let root_hits = assigned
            .iter()
            .filter(|member| related.contains(**member))
            .count();
        if assigned.is_empty() && policies == 0 {
            continue;
        }
        let score = (
            policy_hits + root_hits,
            policy_hits,
            assigned.len() + policies,
        );
        if best.as_ref().is_none_or(|(_, previous)| score > *previous) {
            best = Some((axis.clone(), score));
        }
    }
    best.map(|(axis, _)| axis)
        .or_else(|| domain.first_splittable_axis().cloned())
}

fn extend_preferences(
    preferences: &mut Vec<Preference>,
    lock: &Lock,
    root: &Path,
) -> Result<(), LockError> {
    for package in lock.packages() {
        if let Some(version) = package.version()
            && let Some(index) = package.index(root)?
        {
            preferences.push(Preference::from_resolved(
                package.name().clone(),
                version.clone(),
                Some(index),
                package.fork_markers().to_vec(),
            ));
        }
    }
    Ok(())
}

fn protected_choices(
    lock: &Lock,
    settings: &ResolverSettings,
) -> BTreeMap<LockedPackageIdentity, MarkerTree> {
    if lock.resolution_mode() != settings.resolution
        || lock.fork_strategy() != settings.fork_strategy
        || settings.upgrade.is_all()
    {
        return BTreeMap::new();
    }
    // Dynamic local sources may not have a version preference, but their source identity is
    // still an input choice that an unrelated selective upgrade must retain.
    let upgrade = upgrade_packages_for_lock(lock, &settings.upgrade);
    distribution_markers(lock)
        .into_iter()
        .filter(|(identity, _)| !upgrade.contains(identity.name()))
        .collect()
}

/// The universal environments in which each distribution is a lockfile preference. Retain
/// conflict predicates as well as Python/platform markers so an identity cannot move between
/// ordinary resolver forks while still appearing to preserve an input pin.
fn distribution_markers(lock: &Lock) -> BTreeMap<LockedPackageIdentity, MarkerTree> {
    let environment = implicit_constraints_marker(
        lock.requires_python().to_exact_marker_tree(),
        lock.supported_environments(),
    );
    lock.packages()
        .iter()
        .map(|package| {
            let marker = if package.fork_markers().is_empty() {
                environment
            } else {
                package
                    .fork_markers()
                    .iter()
                    .fold(MarkerTree::FALSE, |marker, fork| marker.or(fork.combined()))
                    .and(environment)
            };
            (package.identity(), marker)
        })
        .collect()
}

/// Removing an unreachable dependency is harmless, but replacing an otherwise valid input pin
/// is not an alignment improvement during a selective update.
fn preserves_choices(lock: &Lock, protected: &BTreeMap<LockedPackageIdentity, MarkerTree>) -> bool {
    if protected.is_empty() {
        return true;
    }
    let selected = distribution_markers(lock);
    let mut present = BTreeMap::<PackageName, MarkerTree>::new();
    for (identity, marker) in &selected {
        present
            .entry(identity.name().clone())
            .and_modify(|previous| *previous = previous.or(*marker))
            .or_insert(*marker);
    }
    protected.iter().all(|(identity, marker)| {
        let present = present
            .get(identity.name())
            .copied()
            .unwrap_or(MarkerTree::FALSE);
        let selected = selected.get(identity).copied().unwrap_or(MarkerTree::FALSE);
        marker.and(present).and(selected.negate()).is_false()
    })
}

#[derive(Clone)]
struct RegistryCandidate {
    identity: LockedPackageIdentity,
    version: Version,
    index: IndexUrl,
}

/// A candidate is only suitable for cross-context alignment when its ordinary resolution already
/// uses a single registry identity. Existing Python/platform forks retain their normal policy.
fn registry_candidate(
    lock: &Lock,
    name: &PackageName,
    root: &Path,
) -> Result<Option<RegistryCandidate>, LockError> {
    let mut candidate: Option<RegistryCandidate> = None;
    for package in lock
        .packages()
        .iter()
        .filter(|package| package.name() == name)
    {
        let Some(version) = package.version() else {
            return Ok(None);
        };
        let Some(index) = package.index(root)? else {
            return Ok(None);
        };
        let identity = package.identity();
        if candidate
            .as_ref()
            .is_some_and(|candidate| candidate.identity != identity)
        {
            return Ok(None);
        }
        candidate = Some(RegistryCandidate {
            identity,
            version: version.clone(),
            index,
        });
    }
    Ok(candidate)
}

fn version_constraint(
    name: &PackageName,
    specifiers: impl IntoIterator<Item = VersionSpecifier>,
) -> NameRequirementSpecification {
    Requirement {
        name: name.clone(),
        extras: Box::default(),
        groups: Box::default(),
        marker: MarkerTree::TRUE,
        source: RequirementSource::Registry {
            specifier: specifiers.into_iter().collect::<VersionSpecifiers>(),
            index: None,
            conflict: None,
        },
        origin: None,
    }
    .into()
}

fn identity_count(resolutions: &[ResolvedDomain]) -> usize {
    resolutions
        .iter()
        .flat_map(|resolution| resolution.lock.packages())
        .map(Package::identity)
        .collect::<BTreeSet<_>>()
        .len()
}

fn same_environment(left: MarkerTree, right: MarkerTree) -> bool {
    left.and(right.negate()).is_false() && right.and(left.negate()).is_false()
}

/// Search beyond initially selected versions, accepting only a strict reduction in the number of
/// distribution identities. The solve and candidate budgets make this optimization predictable;
/// correctness and domain coverage do not depend on it succeeding.
async fn reconcile(
    resolver: &AxisResolver<'_, '_>,
    axes: &ResolvedWorkspaceAxes,
    resolutions: &mut [ResolvedDomain],
) -> Result<(), ProjectError> {
    const MAX_SOLVES: usize = 64;
    const MAX_CANDIDATES: usize = 8;

    if resolutions.len() < 2 {
        return Ok(());
    }
    let root = resolutions[0].workspace.install_path().clone();
    let mut identities = BTreeMap::<PackageName, BTreeSet<LockedPackageIdentity>>::new();
    for package in resolutions
        .iter()
        .flat_map(|resolution| resolution.lock.packages())
    {
        if !axes.members().contains(package.name()) {
            identities
                .entry(package.name().clone())
                .or_default()
                .insert(package.identity());
        }
    }
    let mut remaining = MAX_SOLVES;
    let mut score = identity_count(resolutions);
    for (name, identities) in identities {
        if identities.len() < 2 || remaining == 0 {
            continue;
        }
        let mut affected = Vec::new();
        let mut common_index = None;
        let mut common_environment = None;
        let mut eligible = true;
        for (index, resolution) in resolutions.iter().enumerate() {
            if !resolution
                .lock
                .packages()
                .iter()
                .any(|package| package.name() == &name)
            {
                continue;
            }
            let Some(candidate) = registry_candidate(&resolution.lock, &name, &root)? else {
                eligible = false;
                break;
            };
            let environment = implicit_constraints_marker(
                resolution.lock.requires_python().to_exact_marker_tree(),
                resolution.lock.supported_environments(),
            );
            if common_index
                .as_ref()
                .is_some_and(|index| index != &candidate.index)
                || (resolver.settings.fork_strategy != ForkStrategy::Fewest
                    && common_environment
                        .is_some_and(|previous| !same_environment(previous, environment)))
            {
                eligible = false;
                break;
            }
            common_index = Some(candidate.index.clone());
            common_environment = Some(environment);
            affected.push((index, candidate));
        }
        if !eligible || affected.len() < 2 {
            continue;
        }
        let fixed = affected
            .iter()
            .filter(|(index, candidate)| {
                resolutions[*index]
                    .protected
                    .contains_key(&candidate.identity)
            })
            .map(|(_, candidate)| candidate.identity.clone())
            .collect::<BTreeSet<_>>();
        if fixed.len() > 1 {
            continue;
        }
        let Some((anchor, mut candidate)) = affected
            .iter()
            .find(|(_, candidate)| {
                fixed
                    .first()
                    .is_none_or(|fixed| fixed == &candidate.identity)
            })
            .cloned()
        else {
            continue;
        };
        let mut rejected = BTreeSet::new();
        for attempt in 0..MAX_CANDIDATES {
            if remaining == 0 || (attempt > 0 && !fixed.is_empty()) {
                break;
            }
            if attempt > 0 {
                remaining -= 1;
                let constraint = version_constraint(
                    &name,
                    rejected
                        .iter()
                        .cloned()
                        .map(VersionSpecifier::not_equals_version),
                );
                let result = resolver
                    .resolve(
                        &resolutions[anchor].workspace,
                        Some(resolutions[anchor].lock.clone()),
                        [constraint],
                        &[],
                    )
                    .await;
                let lock = match result {
                    Ok(result) => result.into_lock(),
                    Err(error) if workspace_group_conflict(&error) => break,
                    Err(error) => {
                        debug!(package = %name, %error, "Stopping optional workspace resolution alignment");
                        return Ok(());
                    }
                };
                let Some(next) = registry_candidate(&lock, &name, &root)? else {
                    break;
                };
                if common_index.as_ref() != Some(&next.index) || rejected.contains(&next.version) {
                    break;
                }
                candidate = next;
            }

            let mut replacements = BTreeMap::new();
            let mut valid = true;
            for (index, selected) in &affected {
                if selected.identity == candidate.identity {
                    continue;
                }
                if remaining == 0 {
                    valid = false;
                    break;
                }
                remaining -= 1;
                let result = resolver
                    .resolve(
                        &resolutions[*index].workspace,
                        Some(resolutions[*index].lock.clone()),
                        [version_constraint(
                            &name,
                            [VersionSpecifier::equals_version(candidate.version.clone())],
                        )],
                        &[],
                    )
                    .await;
                let lock = match result {
                    Ok(result) => result.into_lock(),
                    Err(error) if workspace_group_conflict(&error) => {
                        valid = false;
                        break;
                    }
                    Err(error) => {
                        debug!(package = %name, %error, "Stopping optional workspace resolution alignment");
                        return Ok(());
                    }
                };
                match lock.validate_workspace_axis_cohort(
                    axes,
                    &resolutions[*index].domain,
                    &resolutions[*index].workspace,
                ) {
                    Ok(()) => {}
                    Err(WorkspaceAxisError::IncompatibleMember { .. }) => {
                        valid = false;
                        break;
                    }
                    Err(error) => return Err(error.into()),
                }
                if !preserves_choices(&lock, &resolutions[*index].protected)
                    || registry_candidate(&lock, &name, &root)?
                        .is_none_or(|selected| selected.identity != candidate.identity)
                {
                    valid = false;
                    break;
                }
                replacements.insert(*index, lock);
            }
            if valid {
                let next_score = resolutions
                    .iter()
                    .enumerate()
                    .flat_map(|(index, resolution)| {
                        replacements
                            .get(&index)
                            .unwrap_or(&resolution.lock)
                            .packages()
                    })
                    .map(Package::identity)
                    .collect::<BTreeSet<_>>()
                    .len();
                if next_score < score {
                    debug!(package = %name, version = %candidate.version, before = score, after = next_score, "Aligned workspace resolution contexts");
                    for (index, lock) in replacements {
                        resolutions[index].lock = lock;
                    }
                    score = next_score;
                    break;
                }
            }
            rejected.insert(candidate.version.clone());
        }
    }
    if remaining == 0 {
        debug!("Reached the workspace resolution alignment solve budget");
    }
    Ok(())
}
