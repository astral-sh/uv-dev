use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use itertools::Itertools;

use uv_configuration::{
    DependencyGroupsWithDefaults, ExtrasSpecificationWithDefaults, InstallOptions, Override,
};
use uv_distribution_types::{RequiresPython, SimplifiedMarkerTree};
use uv_fs::{PortablePathBuf, normalize_path};
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep508::{MarkerEnvironment, MarkerTree};
use uv_pypi_types::{
    ConflictItem, Conflicts, is_workspace_axis_context_extra, is_workspace_axis_extra,
    without_non_workspace_axis_markers, without_workspace_axis_markers,
    workspace_axis_context_extra, workspace_axis_context_id, workspace_axis_extra,
    workspace_axis_marker,
};
use uv_resolver_types::{ConflictMarker, UniversalMarker};
use uv_workspace::{
    ResolvedWorkspaceAxes, Workspace, WorkspaceAxisDependencyGroup, WorkspaceAxisDomain,
    WorkspaceAxisError, WorkspaceAxisGroupMetadata, WorkspaceAxisName, WorkspaceAxisSelection,
};

use super::export::workspace_axis_package_markers;
use super::workspace_axis_manifest::{WorkspaceAxisManifestKind, WorkspaceAxisManifestRoots};
use super::workspace_groups::{
    merge_dependencies, merge_package, package_environment, scope_dependencies,
};
use super::{
    Dependency, Installable, Lock, LockError, LockErrorKind, Package, PackageId, PackageWire,
    ResolverManifest, VERSION, WORKSPACE_AXES_VERSION, implicit_constraints_marker,
};

/// A solved product of axis sections and the ordinary environments it covers.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LockedWorkspaceAxisContext {
    pub id: u32,
    pub domain: WorkspaceAxisDomain,
    #[serde(default)]
    pub environment: MarkerTree,
    pub(super) manifest: ResolverManifest,
    pub(super) manifest_roots: WorkspaceAxisManifestRoots,
}

/// Validated axis definitions and the exact coverage of the solved workspace contexts.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "LockedWorkspaceAxesWire")]
pub struct LockedWorkspaceAxes {
    pub(super) model: ResolvedWorkspaceAxes,
    pub(super) member_paths: BTreeMap<PackageName, PortablePathBuf>,
    pub(super) group_metadata: WorkspaceAxisGroupMetadata,
    pub(super) environment: MarkerTree,
    pub(super) contexts: Vec<LockedWorkspaceAxisContext>,
}

/// Frozen group ownership and source identity for one projected command view.
///
/// This policy is intentionally not part of the ordinary lockfile format. It carries the
/// selector-aware lock's declarations across the final installation/export boundary, where live
/// workspace files may be missing or may have changed since the lock was created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceAxisCommandPolicy {
    group_metadata: WorkspaceAxisGroupMetadata,
    members: BTreeSet<PackageName>,
    member_paths: BTreeMap<PackageName, PortablePathBuf>,
}

impl WorkspaceAxisCommandPolicy {
    pub fn members(&self) -> &BTreeSet<PackageName> {
        &self.members
    }

    pub fn group_metadata(&self) -> &WorkspaceAxisGroupMetadata {
        &self.group_metadata
    }

    pub fn group_root(&self, groups: &DependencyGroupsWithDefaults) -> Option<&PackageName> {
        self.group_metadata.group_root(&self.members, groups)
    }

    pub fn includes_group(
        &self,
        owner: Option<&PackageName>,
        group: &GroupName,
        groups: &DependencyGroupsWithDefaults,
    ) -> bool {
        self.group_metadata
            .includes_group(&self.members, owner, group, groups)
    }

    pub(super) fn is_member_package(&self, package: &Package) -> bool {
        is_member_package(&self.member_paths, package)
    }

    /// Resolve a named workspace root by its locked local source, not just its package name.
    pub fn root_package<'lock>(
        &self,
        lock: &'lock Lock,
        name: &PackageName,
    ) -> Result<&'lock Package, LockError> {
        find_member_package(lock, &self.member_paths, name)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct LockedWorkspaceAxesWire {
    model: ResolvedWorkspaceAxes,
    member_paths: BTreeMap<PackageName, PortablePathBuf>,
    group_metadata: WorkspaceAxisGroupMetadata,
    environment: Vec<LockedWorkspaceAxisEnvironment>,
    #[serde(rename = "context")]
    contexts: Vec<LockedWorkspaceAxisContext>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct LockedWorkspaceAxisEnvironment {
    domain: WorkspaceAxisDomain,
    #[serde(default)]
    environment: MarkerTree,
}

impl TryFrom<LockedWorkspaceAxesWire> for LockedWorkspaceAxes {
    type Error = LockError;

    fn try_from(wire: LockedWorkspaceAxesWire) -> Result<Self, Self::Error> {
        let mut environment = MarkerTree::FALSE;
        for term in wire.environment {
            wire.model
                .validate_domain(&term.domain)
                .map_err(|error| invalid_axes(error.to_string()))?;
            let mut has_extras = false;
            term.environment.visit_extras_once(|_, _| has_extras = true);
            if has_extras {
                return Err(invalid_axes(
                    "promised environment terms must contain only physical markers",
                ));
            }
            environment = environment.or(wire
                .model
                .marker_for_domain(&term.domain)
                .and(term.environment));
        }
        let mut ids = BTreeSet::new();
        if wire.contexts.iter().any(|context| !ids.insert(context.id)) {
            return Err(invalid_axes("duplicate solved context identifier"));
        }
        Ok(Self {
            model: wire.model,
            member_paths: wire.member_paths,
            group_metadata: wire.group_metadata,
            environment,
            contexts: wire.contexts,
        })
    }
}

/// A selection that cannot be projected to an unambiguous ordinary lockfile.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceAxisSelectionError {
    #[error(transparent)]
    Definition(#[from] WorkspaceAxisError),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error("The lockfile does not contain workspace resolution axes; run `uv lock`")]
    MissingAxes,
    #[error("The selected workspace members have no compatible locked resolution context")]
    Incompatible,
    #[error(
        "The selected packages have different locked resolutions on unresolved {} {}; select a section with `--resolution-axis AXIS=SECTION`",
        if axes.len() == 1 { "axis" } else { "axes" },
        format_axes(axes)
    )]
    Ambiguous { axes: Vec<WorkspaceAxisName> },
    #[error(
        "The matching workspace members depend on unresolved {} {}; select a section with `--resolution-axis AXIS=SECTION`",
        if axes.len() == 1 { "axis" } else { "axes" },
        format_axes(axes)
    )]
    MatchingRoots { axes: Vec<WorkspaceAxisName> },
}

fn format_axes(axes: &[WorkspaceAxisName]) -> String {
    axes.iter().map(|axis| format!("`{axis}`")).join(", ")
}

impl LockedWorkspaceAxes {
    pub fn model(&self) -> &ResolvedWorkspaceAxes {
        &self.model
    }

    pub fn contexts(&self) -> &[LockedWorkspaceAxisContext] {
        &self.contexts
    }

    /// Return the declared dependency groups and their effective Python policy.
    pub fn group_metadata(&self) -> &WorkspaceAxisGroupMetadata {
        &self.group_metadata
    }

    /// Check whether the current workspace still declares exactly the locked local sources.
    pub fn matches_member_sources(
        &self,
        root: &Path,
        expected: &BTreeMap<PackageName, PathBuf>,
    ) -> bool {
        self.member_paths.len() == expected.len()
            && expected.iter().all(|(name, expected)| {
                self.member_paths.get(name).is_some_and(|actual| {
                    normalize_path(root.join(expected)).as_ref()
                        == normalize_path(root.join(actual.as_ref())).as_ref()
                })
            })
    }

    /// Return the complete correlated selector and physical environment promised by the lock.
    pub fn environment(&self) -> MarkerTree {
        self.environment
    }

    /// Restrict the declared product without assigning any omitted axis implicitly.
    pub fn selection_domain(
        &self,
        selection: &WorkspaceAxisSelection,
    ) -> Result<WorkspaceAxisDomain, WorkspaceAxisSelectionError> {
        self.model.validate_selection(selection)?;
        self.model
            .domain()
            .restrict(selection)
            .ok_or(WorkspaceAxisSelectionError::Incompatible)
    }

    /// Return the matching roots only if their identities no longer depend on an omitted axis.
    pub fn matching_roots(
        &self,
        selection: &WorkspaceAxisSelection,
    ) -> Result<BTreeSet<PackageName>, WorkspaceAxisSelectionError> {
        let domain = self.selection_domain(selection)?;
        let possible = self.model.possible_roots(&domain);
        let guaranteed = self.model.guaranteed_roots(&domain);
        if possible != guaranteed {
            let axes = domain
                .iter()
                .filter_map(|(axis, sections)| {
                    (sections.len() > 1
                        && possible.difference(&guaranteed).any(|member| {
                            self.model
                                .assignments()
                                .get(member)
                                .is_some_and(|selection| selection.get(axis).is_some())
                        }))
                    .then_some(axis.clone())
                })
                .collect();
            return Err(WorkspaceAxisSelectionError::MatchingRoots { axes });
        }
        Ok(possible)
    }

    /// Return the exact physical union available before selecting an interpreter.
    pub fn environment_for_selection(
        &self,
        selection: &WorkspaceAxisSelection,
        members: &BTreeSet<PackageName>,
    ) -> Result<MarkerTree, WorkspaceAxisSelectionError> {
        let domain = self.domain_for_members(selection, members)?;
        let environment = without_workspace_axis_markers(
            self.coverage().and(self.model.marker_for_domain(&domain)),
        );
        if environment.is_false() {
            return Err(WorkspaceAxisSelectionError::Incompatible);
        }
        Ok(environment)
    }

    /// Return the Python projection of the compatible locked contexts.
    pub fn requires_python_for_selection(
        &self,
        selection: &WorkspaceAxisSelection,
        members: &BTreeSet<PackageName>,
    ) -> Result<RequiresPython, WorkspaceAxisSelectionError> {
        RequiresPython::from_marker_tree(self.environment_for_selection(selection, members)?)
            .ok_or(WorkspaceAxisSelectionError::Incompatible)
    }

    fn domain_for_members(
        &self,
        selection: &WorkspaceAxisSelection,
        members: &BTreeSet<PackageName>,
    ) -> Result<WorkspaceAxisDomain, WorkspaceAxisSelectionError> {
        self.model.validate_selection(selection)?;
        let mut implied = self.model.selection_for_members(members)?;
        implied.merge(selection)?;
        self.selection_domain(&implied)
    }

    fn coverage(&self) -> MarkerTree {
        self.environment
    }

    pub(super) fn is_member_package(&self, package: &Package) -> bool {
        is_member_package(&self.member_paths, package)
    }

    fn context_environment(&self, context: &LockedWorkspaceAxisContext) -> MarkerTree {
        self.model
            .marker_for_domain(&context.domain)
            .and(context.environment)
    }

    /// Encode one marker relative to the persisted context domains. The private context atom
    /// prevents the marker formatter from expanding an unrelated Cartesian product into DNF.
    pub(super) fn wire_marker(
        &self,
        marker: MarkerTree,
        requires_python: &RequiresPython,
    ) -> Option<String> {
        if self.environment.and(marker.negate()).is_false() {
            return None;
        }
        let mut terms = Vec::new();
        for context in &self.contexts {
            let environment = self.context_environment(context);
            if marker.is_disjoint(environment) {
                continue;
            }
            let residual = requires_python.simplify_markers(marker.restrict(environment));
            let context_marker = format!("extra == '{}'", workspace_axis_context_extra(context.id));
            terms.push(if let Some(residual) = residual.try_to_string() {
                format!("{context_marker} and ({residual})")
            } else {
                context_marker
            });
        }
        match terms.as_slice() {
            [] => Some("python_version < '0'".to_string()),
            [term] => Some(term.clone()),
            _ => Some(terms.iter().map(|term| format!("({term})")).join(" or ")),
        }
    }

    pub(super) fn wire_markers(
        &self,
        markers: &[UniversalMarker],
        requires_python: &RequiresPython,
    ) -> Vec<String> {
        if markers
            .iter()
            .any(|marker| self.environment.and(marker.combined().negate()).is_false())
        {
            return Vec::new();
        }
        let mut markers = markers
            .iter()
            .filter_map(|marker| self.wire_marker(marker.combined(), requires_python))
            .collect::<Vec<_>>();
        markers.sort();
        markers.dedup();
        markers
    }

    /// Restore typed lock-context atoms before any ordinary resolver or installation code sees
    /// the marker. The declared context mapping supplies the full selector/physical correlation.
    pub(super) fn restore_wire_marker(&self, marker: MarkerTree) -> Result<MarkerTree, LockError> {
        let mut unknown = None;
        marker.visit_extras_once(|_, extra| {
            if is_workspace_axis_context_extra(extra)
                && workspace_axis_context_id(extra)
                    .is_none_or(|id| !self.contexts.iter().any(|context| context.id == id))
            {
                unknown = Some(extra.clone());
            }
        });
        if let Some(extra) = unknown {
            return Err(invalid_axes(format!("unknown context marker `{extra}`")));
        }
        let mut restored = MarkerTree::FALSE;
        for context in &self.contexts {
            let selected = workspace_axis_context_extra(context.id);
            let marker = marker
                .simplify_extras_with(|extra| *extra == selected)
                .simplify_not_extras_with(|extra| {
                    is_workspace_axis_context_extra(extra) && *extra != selected
                });
            restored = restored.or(marker
                .and(self.context_environment(context))
                .and(self.environment));
        }
        Ok(restored)
    }

    pub(super) fn restore_wire_package(
        &self,
        mut package: PackageWire,
        requires_python: &RequiresPython,
    ) -> Result<PackageWire, LockError> {
        if package.fork_markers.is_empty() {
            package
                .fork_markers
                .push(SimplifiedMarkerTree::new(requires_python, self.environment));
        } else {
            for marker in &mut package.fork_markers {
                *marker = SimplifiedMarkerTree::new(
                    requires_python,
                    self.restore_wire_marker(marker.into_marker(requires_python))?,
                );
            }
        }
        for dependencies in std::iter::once(&mut package.dependencies)
            .chain(package.optional_dependencies.values_mut())
            .chain(package.dependency_groups.values_mut())
        {
            for dependency in dependencies {
                dependency.marker = SimplifiedMarkerTree::new(
                    requires_python,
                    self.restore_wire_marker(dependency.marker.into_marker(requires_python))?,
                );
            }
        }
        Ok(package)
    }
}

impl Lock {
    /// Return the selector-aware workspace metadata recorded in this lockfile.
    pub fn workspace_axes(&self) -> Option<&LockedWorkspaceAxes> {
        self.workspace_axes.as_ref()
    }

    /// Return the authoritative group/source policy attached to a projected axis command.
    pub fn workspace_axis_command(&self) -> Option<&WorkspaceAxisCommandPolicy> {
        self.workspace_axis_command.as_ref()
    }

    pub(super) fn includes_install_target(
        &self,
        package: &Package,
        project_name: Option<&PackageName>,
        options: &InstallOptions,
    ) -> bool {
        if self
            .workspace_axis_command
            .as_ref()
            .is_some_and(|policy| !policy.is_member_package(package))
        {
            // Workspace/project filters classify a distribution by exact local identity. The
            // explicitly name-based package filters still apply to all distributions of that name.
            options.include_package(package.as_install_target(), None, &BTreeSet::new())
        } else {
            options.include_package(package.as_install_target(), project_name, self.members())
        }
    }

    pub(super) fn ensure_workspace_axes_selected(&self) -> Result<(), LockError> {
        if self.workspace_axes.is_some() {
            Err(LockErrorKind::WorkspaceAxesRequireSelection.into())
        } else {
            Ok(())
        }
    }

    /// Return whether this ordinary lock avoids every unavailable local workspace source.
    ///
    /// A registry or other distribution with the same normalized name is not a workspace member.
    pub fn satisfies_workspace_member_availability(
        &self,
        root: &Path,
        unavailable: &BTreeMap<PackageName, PathBuf>,
    ) -> bool {
        self.packages.iter().all(|package| {
            unavailable
                .get(&package.id.name)
                .zip(package.id.source.as_source_tree())
                .is_none_or(|(expected, actual)| {
                    normalize_path(root.join(expected)).as_ref()
                        != normalize_path(root.join(actual)).as_ref()
                })
        })
    }

    /// Check that every currently requested workspace root has its exact local source in the
    /// ordinary lock. A same-name registry or moved directory does not satisfy this requirement.
    pub fn satisfies_workspace_member_sources(
        &self,
        root: &Path,
        expected: &BTreeMap<PackageName, PathBuf>,
    ) -> bool {
        expected.iter().all(|(name, expected)| {
            self.packages_for_name(name).iter().any(|package| {
                package.id.source.as_source_tree().is_some_and(|actual| {
                    normalize_path(root.join(expected)).as_ref()
                        == normalize_path(root.join(actual)).as_ref()
                })
            })
        })
    }

    /// Combine successful ordinary solves while retaining their complete symbolic domains.
    ///
    /// This records the supplied solves' physical coverage as the lock's promise and infers
    /// unbounded dependency-group declarations from their graphs. Call
    /// [`Self::from_workspace_axes_with_environment`] when the workspace's complete environment is
    /// known independently, so missing Python or platform coverage is also rejected.
    pub fn from_workspace_axes(
        axes: ResolvedWorkspaceAxes,
        resolutions: Vec<(WorkspaceAxisDomain, Self)>,
    ) -> Result<Option<Self>, LockError> {
        let environment =
            resolutions
                .iter()
                .fold(MarkerTree::FALSE, |environment, (domain, lock)| {
                    environment.or(axes
                        .marker_for_domain(domain)
                        .and(ordinary_environment(lock)))
                });
        let group_metadata = inferred_group_metadata(&axes, &resolutions)?;
        Self::from_workspace_axes_with_environment(axes, environment, group_metadata, resolutions)
    }

    /// Combine ordinary solves only if they cover exactly the workspace's promised environment.
    pub fn from_workspace_axes_with_environment(
        axes: ResolvedWorkspaceAxes,
        expected_environment: MarkerTree,
        group_metadata: WorkspaceAxisGroupMetadata,
        mut resolutions: Vec<(WorkspaceAxisDomain, Self)>,
    ) -> Result<Option<Self>, LockError> {
        if resolutions.is_empty() {
            return Ok(None);
        }
        group_metadata
            .validate_members(axes.members())
            .map_err(|error| invalid_axes(error.to_string()))?;
        validate_axis_definition_predicates(&axes)?;
        resolutions.sort_by(|(left, _), (right, _)| left.cmp(right));
        let full_domain = axes.domain();
        if let Some(uncovered) =
            full_domain.first_uncovered(resolutions.iter().map(|(domain, _)| domain))
        {
            return Err(invalid_axes(format!(
                "resolution context `{uncovered}` was not solved"
            )));
        }
        let Some(requires_python) =
            RequiresPython::union(resolutions.iter().map(|(_, lock)| &lock.requires_python))
        else {
            return Ok(None);
        };
        let mut member_paths = BTreeMap::new();
        let mut contexts = Vec::with_capacity(resolutions.len());
        for (index, (domain, lock)) in resolutions.iter().enumerate() {
            axes.validate_domain(domain)
                .map_err(|error| invalid_axes(error.to_string()))?;
            if lock.workspace_axes.is_some()
                || lock.workspace_axis_command.is_some()
                || !lock.workspace_groups.is_empty()
            {
                return Err(invalid_axes("expected an ordinary solved lock"));
            }
            validate_ordinary_lock_predicates(lock)?;
            let environment = implicit_constraints_marker(
                lock.requires_python.to_exact_marker_tree(),
                &lock.supported_environments,
            );
            if environment.is_false() {
                return Err(invalid_axes("a solved context has no physical environment"));
            }
            let roots = axes.possible_roots(domain);
            for package in &lock.packages {
                if !lock.is_workspace_member(package)
                    || !roots.contains(&package.id.name)
                    || !axes.members().contains(&package.id.name)
                {
                    continue;
                }
                let Some(path) = package.id.source.as_source_tree() else {
                    continue;
                };
                let path = normalize_path(path);
                if let Some(previous) = member_paths.insert(
                    package.id.name.clone(),
                    PortablePathBuf::from(path.as_ref()),
                ) && previous.as_ref() != path.as_ref()
                {
                    return Err(invalid_axes(format!(
                        "workspace member `{}` has inconsistent source paths",
                        package.id.name
                    )));
                }
            }
            let mut manifest = lock.manifest.clone();
            manifest.members = roots;
            let manifest_roots = lock.authoritative_workspace_axis_manifest_roots()?;
            manifest_roots.validate(lock, &lock.manifest, environment, MarkerTree::TRUE)?;
            contexts.push(LockedWorkspaceAxisContext {
                id: u32::try_from(index)
                    .map_err(|_| invalid_axes("too many solved workspace contexts"))?,
                domain: domain.clone(),
                environment,
                manifest,
                manifest_roots,
            });
        }
        let metadata = LockedWorkspaceAxes {
            model: axes,
            member_paths,
            group_metadata,
            environment: expected_environment,
            contexts,
        };
        let first = &resolutions[0].1;
        let options = first.options.clone();
        let conflicts = first.conflicts.clone();
        let required_environments = first.required_environments.clone();
        let revision = first.revision;
        let mut manifest = first.manifest.clone();
        manifest.members.clone_from(metadata.model.members());
        let mut packages = BTreeMap::new();
        let mut fork_markers = BTreeSet::new();
        for ((domain, mut lock), context) in resolutions.into_iter().zip(&metadata.contexts) {
            if lock.options != options || lock.conflicts != conflicts {
                return Err(invalid_axes(
                    "cohorts have inconsistent resolver options or conflicts",
                ));
            }
            let scope = UniversalMarker::from_combined(
                metadata
                    .model
                    .marker_for_domain(&domain)
                    .and(context.environment),
            );
            let markers = if lock.fork_markers.is_empty() {
                vec![UniversalMarker::from_combined(context.environment)]
            } else {
                lock.fork_markers.clone()
            };
            for mut marker in markers {
                marker.and(scope);
                if !marker.is_false() {
                    fork_markers.insert(UniversalMarker::from_combined(marker.combined()));
                }
            }
            for package in &mut lock.packages {
                let marker = UniversalMarker::from_combined(package_environment(
                    package,
                    &lock.requires_python,
                ));
                let mut marker = marker;
                marker.and(scope);
                package.fork_markers = vec![marker];
                scope_dependencies(&mut package.dependencies, scope, &requires_python);
                for dependencies in package
                    .optional_dependencies
                    .values_mut()
                    .chain(package.dependency_groups.values_mut())
                {
                    scope_dependencies(dependencies, scope, &requires_python);
                }
            }
            let mut cohort = lock
                .packages
                .into_iter()
                .map(|package| (package.id.clone(), package))
                .collect::<BTreeMap<_, _>>();
            normalize_axis_graph(
                &mut cohort,
                &metadata,
                context,
                &conflicts,
                &requires_python,
            )
            .map_err(|error| invalid_axes(error.to_string()))?;
            for package in cohort.into_values() {
                if let Some(previous) = packages.get_mut(&package.id) {
                    validate_shared_package(previous, &package)?;
                    if previous.sdist.is_none() {
                        previous.sdist.clone_from(&package.sdist);
                    }
                }
                merge_package(&mut packages, package, &requires_python);
            }
        }
        let environment = without_workspace_axis_markers(metadata.coverage());
        let supported_environments = explicit_environment(&requires_python, environment);
        let mut lock = Self::new(
            WORKSPACE_AXES_VERSION,
            revision,
            packages.into_values().collect(),
            requires_python,
            options,
            manifest,
            conflicts,
            supported_environments,
            required_environments,
            fork_markers.into_iter().collect(),
        )?;
        lock.workspace_axes = Some(metadata);
        lock.validate_workspace_axes()?;
        Ok(Some(lock))
    }

    /// Project a selection conservatively across every locked extra and dependency group.
    pub fn select_workspace_axes(
        &self,
        selection: &WorkspaceAxisSelection,
        members: &BTreeSet<PackageName>,
        marker_environment: Option<&MarkerEnvironment>,
    ) -> Result<Self, WorkspaceAxisSelectionError> {
        self.project_workspace_axes(selection, members, marker_environment, None)
    }

    /// Project only the packages reachable by the current command's extras and groups.
    ///
    /// The result is an ordinary lock view for this command, not a replacement universal lock.
    pub fn select_workspace_axes_for_command(
        &self,
        selection: &WorkspaceAxisSelection,
        members: &BTreeSet<PackageName>,
        marker_environment: Option<&MarkerEnvironment>,
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<Self, WorkspaceAxisSelectionError> {
        let mut lock = self.project_workspace_axes(
            selection,
            members,
            marker_environment,
            Some((extras, groups)),
        )?;
        if let Some(axes) = &self.workspace_axes {
            lock.manifest.members = lock
                .packages
                .iter()
                .filter(|package| axes.is_member_package(package))
                .map(|package| package.id.name.clone())
                .collect();
            lock.workspace_axis_command = Some(WorkspaceAxisCommandPolicy {
                group_metadata: axes.group_metadata.clone(),
                members: members.clone(),
                member_paths: axes.member_paths.clone(),
            });
        }
        Ok(lock)
    }

    /// Return a command's compatible physical union before selecting an interpreter.
    ///
    /// Explicitly inherited workspace-root groups imply their owner's axis assignments, just as
    /// they do when projecting the command's installation plan.
    pub fn workspace_axis_environment_for_command(
        &self,
        selection: &WorkspaceAxisSelection,
        members: &BTreeSet<PackageName>,
        _extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<MarkerTree, WorkspaceAxisSelectionError> {
        let Some(axes) = &self.workspace_axes else {
            return if selection.is_empty() {
                Ok(ordinary_environment(self))
            } else {
                Err(WorkspaceAxisSelectionError::MissingAxes)
            };
        };
        let (selection, _) = self.axis_selection_for_command(axes, selection, members, groups)?;
        let environment = axes
            .environment_for_selection(&selection, members)?
            .and(axes.group_metadata.python_marker(members, groups));
        if environment.is_false() {
            return Err(WorkspaceAxisSelectionError::Incompatible);
        }
        Ok(environment)
    }

    /// Return the Python projection of a command's compatible physical union.
    pub fn workspace_axis_requires_python_for_command(
        &self,
        selection: &WorkspaceAxisSelection,
        members: &BTreeSet<PackageName>,
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<RequiresPython, WorkspaceAxisSelectionError> {
        RequiresPython::from_marker_tree(
            self.workspace_axis_environment_for_command(selection, members, extras, groups)?,
        )
        .ok_or(WorkspaceAxisSelectionError::Incompatible)
    }

    /// Check a successful ordinary solve before accepting it as an axis cohort.
    ///
    /// A resolver candidate can introduce an indirect local workspace dependency whose member
    /// assignments are incompatible with some worlds in the cohort. The structured witness lets
    /// the coordinator refine that cohort and retry without accepting an invalid universal lock.
    pub fn validate_workspace_axis_cohort(
        &self,
        axes: &ResolvedWorkspaceAxes,
        domain: &WorkspaceAxisDomain,
        workspace: &Workspace,
    ) -> Result<(), WorkspaceAxisError> {
        axes.validate_domain(domain)?;
        let mut member_paths = BTreeMap::new();
        for member in axes.members() {
            let Some(expected) = workspace.packages().get(member) else {
                return Err(WorkspaceAxisError::UnknownMember(member.clone()));
            };
            for package in self.packages_for_name(member) {
                let Some(path) = package.id.source.as_source_tree() else {
                    continue;
                };
                if normalize_path(workspace.install_path().join(path)).as_ref()
                    == normalize_path(expected.root()).as_ref()
                {
                    member_paths.insert(member.clone(), PortablePathBuf::from(path));
                }
            }
        }
        let roots = axes.possible_roots(domain);
        if let Some(member) = roots
            .iter()
            .find(|member| !member_paths.contains_key(*member))
        {
            return Err(WorkspaceAxisError::MissingMemberSource {
                member: member.clone(),
            });
        }
        let environment = ordinary_environment(self);
        let mut manifest = self.manifest.clone();
        manifest.members = roots;
        let context = LockedWorkspaceAxisContext {
            id: 0,
            domain: domain.clone(),
            environment,
            manifest,
            manifest_roots: self
                .authoritative_workspace_axis_manifest_roots()
                .map_err(|_| WorkspaceAxisError::MissingManifestRoots)?,
        };
        let axes = LockedWorkspaceAxes {
            model: axes.clone(),
            member_paths,
            group_metadata: workspace
                .resolution_axis_group_metadata()
                .map_err(|error| WorkspaceAxisError::InvalidGroupMetadata(error.to_string()))?,
            environment: axes.marker_for_domain(domain).and(environment),
            contexts: Vec::new(),
        };
        let mut packages = self
            .packages
            .iter()
            .cloned()
            .map(|package| (package.id.clone(), package))
            .collect();
        normalize_axis_graph(
            &mut packages,
            &axes,
            &context,
            &self.conflicts,
            &self.requires_python,
        )
    }

    /// Recover an ordinary lock as preferences for an arbitrary product of sections.
    ///
    /// This is deliberately weaker than installation projection: incompatible prior contexts can
    /// fall back to one deterministic context because version preferences are not requirements.
    pub fn workspace_axis_preferences(
        &self,
        domain: &WorkspaceAxisDomain,
    ) -> Result<Option<Self>, LockError> {
        let Some(axes) = &self.workspace_axes else {
            return Ok(None);
        };
        if axes.model.validate_domain(domain).is_err() {
            return Ok(None);
        }
        let mut candidates = Vec::new();
        for context in &axes.contexts {
            let Some(domain) = domain.intersection(&context.domain) else {
                continue;
            };
            let members = axes.model.possible_roots(&domain);
            if let Some(candidate) =
                self.project_axis_context(axes, context, &domain, &members, None, None)?
            {
                candidates.push(candidate);
            }
        }
        let fallback = candidates.first().cloned();
        Ok(Self::merge_workspace_axis_views(candidates, false)?.or(fallback))
    }

    fn project_workspace_axes(
        &self,
        selection: &WorkspaceAxisSelection,
        members: &BTreeSet<PackageName>,
        marker_environment: Option<&MarkerEnvironment>,
        command: Option<(
            &ExtrasSpecificationWithDefaults,
            &DependencyGroupsWithDefaults,
        )>,
    ) -> Result<Self, WorkspaceAxisSelectionError> {
        let Some(axes) = &self.workspace_axes else {
            return if selection.is_empty() {
                Ok(self.clone())
            } else {
                Err(WorkspaceAxisSelectionError::MissingAxes)
            };
        };
        let (selection, group_root) = if let Some((_, groups)) = command {
            self.axis_selection_for_command(axes, selection, members, groups)?
        } else {
            (selection.clone(), None)
        };
        let command_environment =
            command.map(|(_, groups)| axes.group_metadata.python_marker(members, groups));
        let domain = axes.domain_for_members(&selection, members)?;
        let conflict_world = UniversalMarker::new(
            MarkerTree::TRUE,
            ConflictMarker::from_conflicts(&self.conflicts),
        )
        .combined();
        let mut candidates = Vec::new();
        let mut plans = Vec::new();
        for context in &axes.contexts {
            let Some(candidate_domain) = domain.intersection(&context.domain) else {
                continue;
            };
            if marker_environment.is_some_and(|environment| {
                !context
                    .environment
                    .and(command_environment.unwrap_or(MarkerTree::TRUE))
                    .evaluate(environment, &[])
            }) {
                continue;
            }
            let Some(mut candidate) = self.project_axis_context(
                axes,
                context,
                &candidate_domain,
                members,
                group_root.as_ref(),
                command_environment,
            )?
            else {
                continue;
            };
            let plan = if let Some((extras, groups)) = command {
                let target = AxisInstallable {
                    lock: &candidate,
                    members,
                    axes,
                };
                let environment = ordinary_environment(&candidate);
                let plan = workspace_axis_package_markers(&target, extras, groups)?
                    .into_iter()
                    .map(|(package, marker)| (package.id.clone(), marker.and(environment)))
                    .collect::<BTreeMap<_, _>>();
                let group_root_id = group_root
                    .as_ref()
                    .map(|name| target.root_package(name).map(|package| package.id.clone()))
                    .transpose()?;
                retain_command_plan(&mut candidate, axes, members, group_root_id.as_ref(), &plan)?;
                plan
            } else {
                candidate
                    .packages
                    .iter()
                    .map(|package| {
                        (
                            package.id.clone(),
                            package_environment(package, &candidate.requires_python)
                                .and(ordinary_environment(&candidate)),
                        )
                    })
                    .collect()
            };
            for (previous_domain, previous_environment, previous_plan) in &plans {
                if !plans_agree(
                    previous_plan,
                    &plan,
                    *previous_environment,
                    ordinary_environment(&candidate),
                    marker_environment,
                    conflict_world,
                ) {
                    return Err(WorkspaceAxisSelectionError::Ambiguous {
                        axes: differing_axes(previous_domain, &candidate_domain),
                    });
                }
            }
            plans.push((candidate_domain, ordinary_environment(&candidate), plan));
            candidates.push(candidate);
        }
        if marker_environment.is_some() {
            return candidates
                .into_iter()
                .next()
                .ok_or(WorkspaceAxisSelectionError::Incompatible);
        }
        if candidates.len() == 1 {
            return candidates
                .pop()
                .ok_or(WorkspaceAxisSelectionError::Incompatible);
        }
        Self::merge_workspace_axis_views(candidates, true)?
            .ok_or(WorkspaceAxisSelectionError::Incompatible)
    }

    fn axis_selection_for_command(
        &self,
        axes: &LockedWorkspaceAxes,
        selection: &WorkspaceAxisSelection,
        members: &BTreeSet<PackageName>,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<(WorkspaceAxisSelection, Option<PackageName>), WorkspaceAxisSelectionError> {
        let group_root = AxisInstallable {
            lock: self,
            members,
            axes,
        }
        .group_root(groups)
        .cloned();
        let mut selection = selection.clone();
        if let Some(group_root) = &group_root {
            selection.merge(&axes.model.selection_for_members([group_root])?)?;
        }
        Ok((selection, group_root))
    }

    fn project_axis_context(
        &self,
        axes: &LockedWorkspaceAxes,
        context: &LockedWorkspaceAxisContext,
        domain: &WorkspaceAxisDomain,
        members: &BTreeSet<PackageName>,
        group_root: Option<&PackageName>,
        command_environment: Option<MarkerTree>,
    ) -> Result<Option<Self>, LockError> {
        let scope = axes
            .model
            .marker_for_domain(domain)
            .and(context.environment)
            .and(command_environment.unwrap_or(MarkerTree::TRUE));
        let environment = without_workspace_axis_markers(scope);
        let Some(requires_python) = RequiresPython::from_marker_tree(environment) else {
            return Ok(None);
        };
        let project = |marker: UniversalMarker| {
            UniversalMarker::from_combined(without_workspace_axis_markers(
                marker.combined().and(scope),
            ))
        };
        let mut packages = BTreeMap::new();
        for package in &self.packages {
            let mut package = package.clone();
            let marker = project(UniversalMarker::from_combined(package_environment(
                &package,
                &self.requires_python,
            )));
            if marker.is_false() {
                continue;
            }
            package.fork_markers = vec![marker];
            for dependencies in std::iter::once(&mut package.dependencies)
                .chain(package.optional_dependencies.values_mut())
                .chain(package.dependency_groups.values_mut())
            {
                for dependency in dependencies.iter_mut() {
                    dependency.complexified_marker = project(dependency.complexified_marker);
                    dependency.simplified_marker = SimplifiedMarkerTree::new(
                        &requires_python,
                        dependency.complexified_marker.combined(),
                    );
                }
                dependencies.retain(|dependency| !dependency.complexified_marker.is_false());
                merge_dependencies(dependencies, &requires_python);
            }
            packages.insert(package.id.clone(), package);
        }
        let mut roots = packages
            .values()
            .filter(|package| members.contains(&package.id.name) && axes.is_member_package(package))
            .map(|package| package.id.clone())
            .collect::<BTreeSet<_>>();
        if !members
            .iter()
            .all(|member| roots.iter().any(|id| &id.name == member))
        {
            return Ok(None);
        }
        if let Some(group_root) = group_root {
            let Some(package) = packages
                .values()
                .find(|package| &package.id.name == group_root && axes.is_member_package(package))
            else {
                return Ok(None);
            };
            roots.insert(package.id.clone());
        }
        let manifest_roots = context.manifest_roots.project(environment);
        retain_axis_reachable(&mut packages, &roots, &manifest_roots);
        let mut manifest = context.manifest.clone();
        manifest.members.clone_from(members);
        let fork_markers = self
            .fork_markers
            .iter()
            .copied()
            .map(project)
            .filter(|marker| !marker.is_false())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let supported_environments = explicit_environment(&requires_python, environment);
        let mut lock = Self::new(
            VERSION,
            self.revision,
            packages.into_values().collect(),
            requires_python,
            self.options.clone(),
            manifest,
            self.conflicts.clone(),
            supported_environments,
            self.required_environments.clone(),
            fork_markers,
        )?;
        lock.workspace_axis_manifest_roots = Some(manifest_roots);
        Ok(Some(lock))
    }

    pub(super) fn validate_workspace_axes(&self) -> Result<(), LockError> {
        let Some(axes) = &self.workspace_axes else {
            return if self.version == WORKSPACE_AXES_VERSION {
                Err(invalid_axes("version 3 requires workspace-axis metadata"))
            } else {
                Ok(())
            };
        };
        if self.version != WORKSPACE_AXES_VERSION {
            return Err(invalid_axes("workspace axes require lockfile version 3"));
        }
        if !self.workspace_groups.is_empty() {
            return Err(invalid_axes(
                "flat workspace groups and axes cannot be combined",
            ));
        }
        if axes.model.definitions().is_empty() || axes.contexts.is_empty() {
            return Err(invalid_axes(
                "axis definitions and solved contexts must be nonempty",
            ));
        }
        validate_axis_definition_predicates(&axes.model)?;
        validate_ordinary_manifest_predicates(&self.manifest)?;
        for context in &axes.contexts {
            validate_ordinary_manifest_predicates(&context.manifest)?;
            context.manifest_roots.validate(
                self,
                &context.manifest,
                context.environment,
                axes.model.marker_for_domain(&context.domain),
            )?;
        }
        for package in &self.packages {
            validate_package_declaration_predicates(package)?;
        }
        if axes.member_paths.keys().collect::<BTreeSet<_>>()
            != axes.model.members().iter().collect::<BTreeSet<_>>()
            || self.manifest.members != *axes.model.members()
            || axes.model.members().iter().any(|member| {
                !self
                    .packages_for_name(member)
                    .iter()
                    .any(|package| axes.is_member_package(package))
            })
        {
            return Err(invalid_axes(
                "locked member sources do not cover the declared members",
            ));
        }
        axes.group_metadata
            .validate_members(axes.model.members())
            .map_err(|error| invalid_axes(error.to_string()))?;
        let project_roots = axes
            .member_paths
            .iter()
            .filter_map(|(name, path)| {
                (normalize_path(path.as_ref()).as_ref() == Path::new("")).then_some(name)
            })
            .collect::<Vec<_>>();
        if project_roots
            != axes
                .group_metadata
                .project_root()
                .into_iter()
                .collect::<Vec<_>>()
        {
            return Err(invalid_axes(
                "dependency-group project root does not match its local source",
            ));
        }
        for package in &self.packages {
            if !axes.is_member_package(package) {
                continue;
            }
            let declared = &axes.group_metadata.members()[&package.id.name];
            if let Some(group) = package
                .dependency_groups
                .keys()
                .find(|group| !declared.contains_key(*group))
            {
                return Err(invalid_axes(format!(
                    "workspace member `{}` has no declaration for locked dependency group `{group}`",
                    package.id.name
                )));
            }
        }
        for context in &axes.contexts {
            if let Some(group) = context
                .manifest
                .dependency_groups
                .keys()
                .find(|group| !axes.group_metadata.root_groups().contains_key(*group))
            {
                return Err(invalid_axes(format!(
                    "the workspace root has no declaration for locked dependency group `{group}`"
                )));
            }
        }
        let full_domain = axes.model.domain();
        let legal_domain = axes.model.marker_for_domain(&full_domain);
        if axes.environment.is_false()
            || !axes.environment.and(legal_domain.negate()).is_false()
            || !legal_domain
                .and(axes.environment.only_extras().negate())
                .is_false()
        {
            return Err(invalid_axes(
                "the promised environment does not cover exactly the declared selector domain",
            ));
        }
        let mut covered = MarkerTree::FALSE;
        let mut context_ids = BTreeSet::new();
        let mut declared_atoms = BTreeSet::new();
        for (axis, sections) in axes.model.definitions().iter() {
            for (section, _) in sections.iter() {
                declared_atoms.insert(workspace_axis_extra(axis.as_str(), section.as_str()));
            }
        }
        let mut invalid_environment_extra = None;
        axes.environment.visit_extras_once(|_, extra| {
            if !declared_atoms.contains(extra) {
                invalid_environment_extra = Some(extra.clone());
            }
        });
        if let Some(extra) = invalid_environment_extra {
            return Err(invalid_axes(format!(
                "the promised environment contains undeclared selector `{extra}`"
            )));
        }
        for context in &axes.contexts {
            if !context_ids.insert(context.id) {
                return Err(invalid_axes("duplicate solved context identifier"));
            }
            axes.model
                .validate_domain(&context.domain)
                .map_err(|error| invalid_axes(error.to_string()))?;
            if context.manifest.members != axes.model.possible_roots(&context.domain) {
                return Err(invalid_axes(
                    "context roots do not match the declared member assignments",
                ));
            }
            let mut has_extras = false;
            context
                .environment
                .visit_extras_once(|_, _| has_extras = true);
            if context.environment.is_false() || has_extras {
                return Err(invalid_axes(
                    "context environments must contain only physical markers",
                ));
            }
            let scope = axes
                .model
                .marker_for_domain(&context.domain)
                .and(context.environment);
            if !covered.is_disjoint(scope) {
                return Err(invalid_axes("solved workspace contexts overlap"));
            }
            for (axis, sections) in axes.model.definitions().iter() {
                for (section, definition) in sections.iter() {
                    if let Some(requires_python) = &definition.requires_python {
                        let requirement = RequiresPython::from_specifiers(requires_python.clone())
                            .to_exact_marker_tree();
                        if !scope
                            .and(workspace_axis_marker(axis.as_str(), section.as_str()))
                            .and(requirement.negate())
                            .is_false()
                        {
                            return Err(invalid_axes(format!(
                                "context exceeds the Python requirement of `{axis}={section}`"
                            )));
                        }
                    }
                }
            }
            covered = covered.or(scope);
        }
        if let Some(uncovered) =
            full_domain.first_uncovered(axes.contexts.iter().map(|context| &context.domain))
        {
            return Err(invalid_axes(format!(
                "resolution context `{uncovered}` was not solved"
            )));
        }
        if !covered.and(axes.environment.negate()).is_false()
            || !axes.environment.and(covered.negate()).is_false()
        {
            return Err(invalid_axes(
                "solved contexts do not cover exactly the promised Python/platform environment",
            ));
        }
        let physical = without_workspace_axis_markers(axes.environment);
        let ordinary = ordinary_environment(self);
        if !physical.and(ordinary.negate()).is_false()
            || !ordinary.and(physical.negate()).is_false()
        {
            return Err(invalid_axes(
                "supported markers do not match the promised physical environment",
            ));
        }
        let mut undeclared = BTreeSet::new();
        for marker in self
            .fork_markers
            .iter()
            .copied()
            .chain(self.packages.iter().flat_map(|package| {
                package.fork_markers.iter().copied().chain(
                    package
                        .all_dependencies()
                        .map(|dependency| dependency.complexified_marker),
                )
            }))
        {
            marker.combined().visit_extras_once(|_, extra| {
                if is_workspace_axis_context_extra(extra)
                    || is_workspace_axis_extra(extra) && !declared_atoms.contains(extra)
                {
                    undeclared.insert(extra.clone());
                }
            });
        }
        if let Some(extra) = undeclared.first() {
            return Err(invalid_axes(format!("undeclared axis marker `{extra}`")));
        }
        let actual = without_non_workspace_axis_markers(
            self.fork_markers
                .iter()
                .fold(MarkerTree::FALSE, |marker, fork| marker.or(fork.combined())),
        );
        if !covered.and(actual.negate()).is_false() || !actual.and(covered.negate()).is_false() {
            return Err(invalid_axes(
                "resolution markers do not cover all solved contexts",
            ));
        }
        for context in &axes.contexts {
            let mut packages = self
                .packages
                .iter()
                .cloned()
                .map(|package| (package.id.clone(), package))
                .collect();
            normalize_axis_graph(
                &mut packages,
                axes,
                context,
                &self.conflicts,
                &self.requires_python,
            )
            .map_err(|error| invalid_axes(error.to_string()))?;
        }
        Ok(())
    }

    pub(super) fn workspace_axes_coverage(&self) -> Option<MarkerTree> {
        self.workspace_axes
            .as_ref()
            .map(LockedWorkspaceAxes::coverage)
    }
}

fn invalid_axes(reason: impl Into<String>) -> LockError {
    LockErrorKind::WorkspaceAxes(reason.into()).into()
}

fn is_member_package(
    member_paths: &BTreeMap<PackageName, PortablePathBuf>,
    package: &Package,
) -> bool {
    member_paths
        .get(&package.id.name)
        .zip(package.id.source.as_source_tree())
        .is_some_and(|(expected, actual)| {
            normalize_path(expected.as_ref()).as_ref() == normalize_path(actual).as_ref()
        })
}

fn find_member_package<'lock>(
    lock: &'lock Lock,
    member_paths: &BTreeMap<PackageName, PortablePathBuf>,
    name: &PackageName,
) -> Result<&'lock Package, LockError> {
    let mut candidates = lock
        .packages_for_name(name)
        .iter()
        .filter(|package| is_member_package(member_paths, package));
    let Some(package) = candidates.next() else {
        return Err(LockErrorKind::MissingRootPackage { name: name.clone() }.into());
    };
    if candidates.next().is_some() {
        return Err(LockErrorKind::MultipleRootPackages { name: name.clone() }.into());
    }
    Ok(package)
}

pub(super) fn validate_ordinary_predicate(marker: MarkerTree) -> Result<(), LockError> {
    let mut collision = None;
    marker.visit_extras_once(|_, extra| {
        if is_workspace_axis_extra(extra) || is_workspace_axis_context_extra(extra) {
            collision.get_or_insert_with(|| extra.clone());
        }
    });
    if let Some(extra) = collision {
        return Err(invalid_axes(format!(
            "ordinary dependency predicate uses private workspace selector `{extra}`; this extra spelling is not supported by workspace resolution axes"
        )));
    }
    Ok(())
}

fn validate_axis_definition_predicates(axes: &ResolvedWorkspaceAxes) -> Result<(), LockError> {
    for (_, sections) in axes.definitions().iter() {
        for (_, section) in sections.iter() {
            for requirement in &section.constraint_dependencies {
                validate_ordinary_predicate(requirement.marker)?;
            }
        }
    }
    Ok(())
}

fn validate_ordinary_manifest_predicates(manifest: &ResolverManifest) -> Result<(), LockError> {
    for requirement in manifest
        .requirements
        .iter()
        .chain(manifest.constraints.iter())
        .chain(manifest.dependency_groups.values().flatten())
        .chain(
            manifest
                .build_constraints
                .iter()
                .map(|requirement| &requirement.requirement),
        )
    {
        validate_ordinary_predicate(requirement.marker)?;
    }
    for entry in &manifest.overrides {
        match entry {
            Override::Requirement(requirement) => validate_ordinary_predicate(requirement.marker)?,
            Override::Package(package) => {
                for requirement in &package.dependencies {
                    validate_ordinary_predicate(requirement.marker)?;
                }
            }
        }
    }
    for metadata in &manifest.dependency_metadata {
        for requirement in &metadata.requires_dist {
            validate_ordinary_predicate(requirement.marker)?;
        }
    }
    Ok(())
}

fn validate_package_declaration_predicates(package: &Package) -> Result<(), LockError> {
    for requirement in package
        .metadata
        .requires_dist
        .iter()
        .chain(package.metadata.dependency_groups.values().flatten())
    {
        validate_ordinary_predicate(requirement.marker)?;
    }
    Ok(())
}

/// Ordinary solves must not already contain the private selector atoms added by this module.
/// User-defined extras with the same spelling remain legal outside selector-aware lockfiles.
fn validate_ordinary_lock_predicates(lock: &Lock) -> Result<(), LockError> {
    validate_ordinary_manifest_predicates(&lock.manifest)?;
    for marker in lock
        .supported_environments
        .iter()
        .chain(&lock.required_environments)
        .copied()
        .chain(lock.fork_markers.iter().map(|marker| marker.combined()))
    {
        validate_ordinary_predicate(marker)?;
    }
    for package in &lock.packages {
        validate_package_declaration_predicates(package)?;
        for marker in package
            .fork_markers
            .iter()
            .map(|marker| marker.combined())
            .chain(
                package
                    .all_dependencies()
                    .map(|dependency| dependency.complexified_marker.combined()),
            )
        {
            validate_ordinary_predicate(marker)?;
        }
    }
    Ok(())
}

/// Infer declarations available in ordinary lock graphs when no workspace source metadata was
/// supplied. Effective Python bounds and groups omitted from those graphs are not recoverable.
fn inferred_group_metadata(
    axes: &ResolvedWorkspaceAxes,
    resolutions: &[(WorkspaceAxisDomain, Lock)],
) -> Result<WorkspaceAxisGroupMetadata, LockError> {
    let mut members = axes
        .members()
        .iter()
        .cloned()
        .map(|member| (member, BTreeMap::new()))
        .collect::<BTreeMap<_, _>>();
    let mut project_root = None;
    let mut root_groups = BTreeMap::new();
    for (domain, lock) in resolutions {
        let roots = axes.possible_roots(domain);
        for package in &lock.packages {
            if !roots.contains(&package.id.name)
                || !lock.is_workspace_member(package)
                || package.id.source.as_source_tree().is_none()
            {
                continue;
            }
            if let Some(groups) = members.get_mut(&package.id.name) {
                for group in package.dependency_groups.keys() {
                    groups
                        .entry(group.clone())
                        .or_insert_with(WorkspaceAxisDependencyGroup::default);
                }
            }
        }
        if let Some(root) = lock.root().filter(|root| roots.contains(&root.id.name)) {
            if project_root
                .as_ref()
                .is_some_and(|previous| *previous != root.id.name)
            {
                return Err(invalid_axes("cohorts have inconsistent project roots"));
            }
            project_root = Some(root.id.name.clone());
        }
        for group in lock.manifest.dependency_groups.keys() {
            root_groups
                .entry(group.clone())
                .or_insert_with(WorkspaceAxisDependencyGroup::default);
        }
    }
    WorkspaceAxisGroupMetadata::from_parts(project_root, members, root_groups)
        .map_err(|error| invalid_axes(error.to_string()))
}

pub(super) fn ordinary_environment(lock: &Lock) -> MarkerTree {
    implicit_constraints_marker(
        lock.requires_python.to_exact_marker_tree(),
        &lock.supported_environments,
    )
}

fn explicit_environment(
    requires_python: &RequiresPython,
    environment: MarkerTree,
) -> Vec<MarkerTree> {
    if requires_python.simplify_markers(environment).is_true() {
        Vec::new()
    } else {
        vec![environment]
    }
}

fn validate_shared_package(previous: &Package, package: &Package) -> Result<(), LockError> {
    if previous.sdist.is_some() && package.sdist.is_some() && previous.sdist != package.sdist {
        return Err(invalid_axes(format!(
            "distribution `{}` has inconsistent source artifacts across contexts",
            package.id
        )));
    }
    if previous.metadata != package.metadata {
        return Err(invalid_axes(format!(
            "distribution `{}` has inconsistent declaration metadata across contexts",
            package.id
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AxisDependencyContext {
    Production,
    Extra(ExtraName),
    Group(GroupName),
}

pub(super) fn activation(conflicts: &Conflicts, item: &ConflictItem) -> MarkerTree {
    if conflicts.contains(item.package(), item.kind().as_ref()) {
        UniversalMarker::new(MarkerTree::TRUE, ConflictMarker::from_conflict_item(item)).combined()
    } else {
        MarkerTree::TRUE
    }
}

fn normalize_axis_graph(
    packages: &mut BTreeMap<PackageId, Package>,
    axes: &LockedWorkspaceAxes,
    context: &LockedWorkspaceAxisContext,
    conflicts: &Conflicts,
    requires_python: &RequiresPython,
) -> Result<(), WorkspaceAxisError> {
    let scope = axes
        .model
        .marker_for_domain(&context.domain)
        .and(context.environment);
    let conflict_world =
        UniversalMarker::new(MarkerTree::TRUE, ConflictMarker::from_conflicts(conflicts))
            .combined();
    let mut pending = VecDeque::new();
    let mut root_presence = BTreeMap::new();
    for package in packages.values() {
        if !axes.is_member_package(package) || !context.manifest.members.contains(&package.id.name)
        {
            continue;
        }
        let Some(guard) = axes.model.guard_for_member(&package.id.name) else {
            continue;
        };
        let available = scope
            .and(guard)
            .and(package_environment(package, requires_python));
        root_presence.insert(package.id.clone(), available);
        let project = activation(conflicts, &ConflictItem::from(package.id.name.clone()));
        pending.push_back((
            package.id.clone(),
            AxisDependencyContext::Production,
            available.and(project),
        ));
        for extra in package.optional_dependencies.keys() {
            pending.push_back((
                package.id.clone(),
                AxisDependencyContext::Extra(extra.clone()),
                available.and(project).and(activation(
                    conflicts,
                    &ConflictItem::from((package.id.name.clone(), extra.clone())),
                )),
            ));
        }
        for group in package.dependency_groups.keys() {
            pending.push_back((
                package.id.clone(),
                AxisDependencyContext::Group(group.clone()),
                available.and(activation(
                    conflicts,
                    &ConflictItem::from((package.id.name.clone(), group.clone())),
                )),
            ));
        }
    }
    for root in &context.manifest_roots.0 {
        for resolution in &root.resolutions {
            let kind = match &resolution.kind {
                WorkspaceAxisManifestKind::Production => AxisDependencyContext::Production,
                WorkspaceAxisManifestKind::Extra(extra) => {
                    AxisDependencyContext::Extra(extra.clone())
                }
                WorkspaceAxisManifestKind::Group(group) => {
                    AxisDependencyContext::Group(group.clone())
                }
            };
            for target in &resolution.targets {
                pending.push_back((
                    target.package.clone(),
                    kind.clone(),
                    scope.and(target.marker),
                ));
            }
        }
    }
    let mut reached = BTreeMap::<(PackageId, AxisDependencyContext), MarkerTree>::new();
    while let Some((id, kind, marker)) = pending.pop_front() {
        let Some(package) = packages.get(&id) else {
            continue;
        };
        let required = marker.and(conflict_world);
        if axes.is_member_package(package)
            && let Some(guard) = axes.model.guard_for_member(&id.name)
        {
            let incompatible = required.and(guard.negate());
            if let Some(selection) = context.domain.witness_for_marker(incompatible) {
                return Err(WorkspaceAxisError::IncompatibleMember {
                    member: id.name.clone(),
                    selection,
                });
            }
        }
        let active = required.and(package_environment(package, requires_python));
        if active.is_false() {
            continue;
        }
        let previous = reached
            .entry((id.clone(), kind.clone()))
            .or_insert(MarkerTree::FALSE);
        let active = previous.or(active);
        if active.and(previous.negate()).is_false() {
            continue;
        }
        *previous = active;
        let dependencies: &[Dependency] = match &kind {
            AxisDependencyContext::Production => &package.dependencies,
            AxisDependencyContext::Extra(extra) => package
                .optional_dependencies
                .get(extra)
                .map_or(&[], Vec::as_slice),
            AxisDependencyContext::Group(group) => package
                .dependency_groups
                .get(group)
                .map_or(&[], Vec::as_slice),
        };
        for dependency in dependencies {
            let mut marker = active.and(dependency.complexified_marker.combined());
            for extra in &dependency.extra {
                marker = marker.and(activation(
                    conflicts,
                    &ConflictItem::from((dependency.package_id.name.clone(), extra.clone())),
                ));
            }
            if marker.is_false() {
                continue;
            }
            pending.push_back((
                dependency.package_id.clone(),
                AxisDependencyContext::Production,
                marker,
            ));
            for extra in &dependency.extra {
                pending.push_back((
                    dependency.package_id.clone(),
                    AxisDependencyContext::Extra(extra.clone()),
                    marker,
                ));
            }
        }
    }
    for package in packages.values_mut() {
        let mut presence = root_presence
            .get(&package.id)
            .copied()
            .unwrap_or(MarkerTree::FALSE);
        for ((id, _), marker) in &reached {
            if *id == package.id {
                presence = presence.or(*marker);
            }
        }
        package.fork_markers = vec![UniversalMarker::from_combined(presence)];
        let restrict = |dependencies: &mut Vec<Dependency>, kind| {
            let marker = reached
                .get(&(package.id.clone(), kind))
                .copied()
                .unwrap_or(MarkerTree::FALSE);
            scope_dependencies(
                dependencies,
                UniversalMarker::from_combined(marker),
                requires_python,
            );
        };
        restrict(&mut package.dependencies, AxisDependencyContext::Production);
        for (extra, dependencies) in &mut package.optional_dependencies {
            restrict(dependencies, AxisDependencyContext::Extra(extra.clone()));
        }
        for (group, dependencies) in &mut package.dependency_groups {
            restrict(dependencies, AxisDependencyContext::Group(group.clone()));
        }
    }
    packages.retain(|_, package| package.fork_markers.iter().any(|marker| !marker.is_false()));
    Ok(())
}

fn retain_axis_reachable(
    packages: &mut BTreeMap<PackageId, Package>,
    roots: &BTreeSet<PackageId>,
    manifest_roots: &WorkspaceAxisManifestRoots,
) {
    let mut pending = roots.iter().cloned().collect::<Vec<_>>();
    pending.extend(
        manifest_roots
            .0
            .iter()
            .flat_map(|root| &root.resolutions)
            .flat_map(|resolution| &resolution.targets)
            .map(|target| target.package.clone()),
    );
    let mut reached = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if reached.insert(id.clone())
            && let Some(package) = packages.get(&id)
        {
            pending.extend(
                package
                    .all_dependencies()
                    .map(|dependency| dependency.package_id.clone()),
            );
        }
    }
    packages.retain(|id, _| reached.contains(id));
}

fn plans_agree(
    first: &BTreeMap<PackageId, MarkerTree>,
    second: &BTreeMap<PackageId, MarkerTree>,
    first_environment: MarkerTree,
    second_environment: MarkerTree,
    environment: Option<&MarkerEnvironment>,
    conflict_world: MarkerTree,
) -> bool {
    let overlap = first_environment
        .and(second_environment)
        .and(conflict_world);
    if overlap.is_false() {
        return true;
    }
    first.keys().chain(second.keys()).all(|id| {
        let first = first.get(id).copied().unwrap_or(MarkerTree::FALSE);
        let second = second.get(id).copied().unwrap_or(MarkerTree::FALSE);
        let difference = first
            .and(second.negate())
            .or(second.and(first.negate()))
            .and(overlap);
        environment.map_or_else(
            || difference.is_false(),
            |environment| !difference.without_extras().evaluate(environment, &[]),
        )
    })
}

fn differing_axes(
    first: &WorkspaceAxisDomain,
    second: &WorkspaceAxisDomain,
) -> Vec<WorkspaceAxisName> {
    first
        .iter()
        .filter_map(|(axis, sections)| (second.get(axis) != Some(sections)).then_some(axis.clone()))
        .collect()
}

struct AxisInstallable<'lock> {
    lock: &'lock Lock,
    members: &'lock BTreeSet<PackageName>,
    axes: &'lock LockedWorkspaceAxes,
}

impl<'lock> Installable<'lock> for AxisInstallable<'lock> {
    fn install_path(&self) -> &'lock Path {
        Path::new("")
    }

    fn lock(&self) -> &'lock Lock {
        self.lock
    }

    fn roots(&self) -> impl Iterator<Item = &PackageName> {
        self.members.iter()
    }

    fn root_package(&self, name: &PackageName) -> Result<&'lock Package, LockError> {
        find_member_package(self.lock, &self.axes.member_paths, name)
    }

    fn project_name(&self) -> Option<&PackageName> {
        if self.members.len() == 1 {
            self.members.first()
        } else {
            None
        }
    }

    fn group_root(&self, groups: &DependencyGroupsWithDefaults) -> Option<&PackageName> {
        self.axes.group_metadata.group_root(self.members, groups)
    }

    fn includes_group(
        &self,
        package: Option<&PackageName>,
        group: &GroupName,
        groups: &DependencyGroupsWithDefaults,
    ) -> bool {
        self.axes
            .group_metadata
            .includes_group(self.members, package, group, groups)
    }
}

fn retain_command_plan(
    lock: &mut Lock,
    axes: &LockedWorkspaceAxes,
    members: &BTreeSet<PackageName>,
    group_root: Option<&PackageId>,
    plan: &BTreeMap<PackageId, MarkerTree>,
) -> Result<(), LockError> {
    let environment = ordinary_environment(lock);
    let mut keep = plan.clone();
    for package in &lock.packages {
        if members.contains(&package.id.name) && axes.is_member_package(package)
            || group_root == Some(&package.id)
        {
            keep.entry(package.id.clone()).or_insert(environment);
        }
    }
    let mut packages = lock.packages.clone();
    packages.retain(|package| keep.contains_key(&package.id));
    for package in &mut packages {
        let parent = keep[&package.id];
        package.fork_markers = vec![UniversalMarker::from_combined(parent)];
        for dependencies in std::iter::once(&mut package.dependencies)
            .chain(package.optional_dependencies.values_mut())
            .chain(package.dependency_groups.values_mut())
        {
            dependencies.retain_mut(|dependency| {
                let Some(target) = keep.get(&dependency.package_id) else {
                    return false;
                };
                dependency
                    .complexified_marker
                    .and(UniversalMarker::from_combined(parent.and(*target)));
                dependency.simplified_marker = SimplifiedMarkerTree::new(
                    &lock.requires_python,
                    dependency.complexified_marker.combined(),
                );
                !dependency.complexified_marker.is_false()
            });
            merge_dependencies(dependencies, &lock.requires_python);
        }
    }
    let mut manifest_roots = lock.workspace_axis_manifest_roots.clone();
    if let Some(roots) = &mut manifest_roots {
        roots.retain_packages(&keep);
    }
    *lock = Lock::new(
        VERSION,
        lock.revision,
        packages,
        lock.requires_python.clone(),
        lock.options.clone(),
        lock.manifest.clone(),
        lock.conflicts.clone(),
        lock.supported_environments.clone(),
        lock.required_environments.clone(),
        lock.fork_markers.clone(),
    )?;
    lock.workspace_axis_manifest_roots = manifest_roots;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};

    use insta::assert_snapshot;

    use uv_configuration::{DependencyGroups, ExtrasSpecification, InstallOptions};
    use uv_distribution_types::SimplifiedMarkerTree;
    use uv_normalize::{DefaultExtras, DefaultGroups, PackageName};
    use uv_pep508::MarkerTree;
    use uv_pypi_types::workspace_axis_marker;
    use uv_resolver_types::UniversalMarker;
    use uv_workspace::{
        ResolvedWorkspaceAxes, WorkspaceAxes, WorkspaceAxisDomain, WorkspaceAxisGroupMetadata,
        WorkspaceAxisSelection,
    };

    use super::{
        AxisInstallable, Installable, Lock, WorkspaceAxisSelectionError,
        workspace_axis_package_markers,
    };

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn members(names: &[&str]) -> TestResult<BTreeSet<PackageName>> {
        names.iter().map(|name| Ok(name.parse()?)).collect()
    }

    fn selection(assignments: &[&str]) -> TestResult<WorkspaceAxisSelection> {
        Ok(WorkspaceAxisSelection::from_assignments(
            assignments
                .iter()
                .map(|assignment| assignment.parse())
                .collect::<Result<Vec<_>, _>>()?,
        )?)
    }

    fn checked_axes_lock(
        axes: ResolvedWorkspaceAxes,
        environment: MarkerTree,
        resolutions: Vec<(WorkspaceAxisDomain, Lock)>,
    ) -> TestResult<Option<Lock>> {
        let groups = super::inferred_group_metadata(&axes, &resolutions)?;
        Ok(Lock::from_workspace_axes_with_environment(
            axes,
            environment,
            groups,
            resolutions,
        )?)
    }

    fn sqlalchemy_axes() -> TestResult<ResolvedWorkspaceAxes> {
        let definitions: WorkspaceAxes = toml::from_str(
            r#"
[sqlalchemy]
v1 = { members = ["legacy"] }
v2 = { members = ["modern"] }
"#,
        )?;
        Ok(ResolvedWorkspaceAxes::from_parts(
            definitions,
            members(&["legacy", "modern", "shared"])?,
        )?)
    }

    fn ordinary_lock(
        names: &[&str],
        requires_python: &str,
        leaf_version: &str,
        leaf_index: &str,
        optional_shared: bool,
    ) -> TestResult<Lock> {
        let mut document = format!(
            "version = 1\nrevision = 3\nrequires-python = {requires_python:?}\n\n[manifest]\nmembers = {names:?}\n"
        );
        for name in names {
            let dependency = if optional_shared && *name == "shared" {
                "common-leaf"
            } else {
                "leaf"
            };
            writeln!(
                document,
                "\n[[package]]\nname = {name:?}\nversion = \"1.0.0\"\nsource = {{ virtual = {name:?} }}\ndependencies = [{{ name = {dependency:?} }}]"
            )?;
            if optional_shared && *name == "shared" {
                writeln!(
                    document,
                    "\n[package.optional-dependencies]\nfeature = [{{ name = \"leaf\" }}]\n\n[package.dev-dependencies]\ntest = [{{ name = \"leaf\" }}]"
                )?;
            }
        }
        writeln!(
            document,
            "\n[[package]]\nname = \"leaf\"\nversion = {leaf_version:?}\nsource = {{ registry = {leaf_index:?} }}"
        )?;
        if optional_shared {
            writeln!(
                document,
                "\n[[package]]\nname = \"common-leaf\"\nversion = \"1.0.0\"\nsource = {{ registry = \"https://pypi.org/simple\" }}"
            )?;
        }
        Ok(Lock::from_toml(&document)?)
    }

    fn sqlalchemy_lock(optional_shared: bool) -> TestResult<Lock> {
        let axes = sqlalchemy_axes()?;
        let full = axes.domain();
        let environment = axes
            .marker_for_domain(&full)
            .and("python_full_version >= '3.12' and python_full_version < '3.15'".parse()?);
        let first = full
            .restrict(&selection(&["sqlalchemy=v1"])?)
            .ok_or("missing first domain")?;
        let second = full
            .restrict(&selection(&["sqlalchemy=v2"])?)
            .ok_or("missing second domain")?;
        checked_axes_lock(
            axes,
            environment,
            vec![
                (
                    first,
                    ordinary_lock(
                        &["legacy", "shared"],
                        ">=3.12,<3.15",
                        "1.0.0",
                        "https://pypi.org/simple",
                        optional_shared,
                    )?,
                ),
                (
                    second,
                    ordinary_lock(
                        &["modern", "shared"],
                        ">=3.12,<3.15",
                        "2.0.0",
                        "https://pypi.org/simple",
                        optional_shared,
                    )?,
                ),
            ],
        )?
        .ok_or_else(|| "missing lock".into())
    }

    #[test]
    fn workspace_axis_lock_round_trip_and_selection() -> TestResult {
        let lock = sqlalchemy_lock(false)?;
        let serialized = lock.to_toml()?;
        let parsed = Lock::from_canonical_toml(&serialized)?;
        assert_eq!(serialized, parsed.to_toml()?);
        assert_eq!(serialized, Lock::from_toml(&serialized)?.to_toml()?);
        let selected = parsed.select_workspace_axes(
            &selection(&["sqlalchemy=v2"])?,
            &members(&["shared"])?,
            None,
        )?;
        let leaf = selected
            .find_by_name(&"leaf".parse()?)?
            .ok_or("missing leaf")?;
        assert_eq!(
            leaf.version().map(ToString::to_string).as_deref(),
            Some("2.0.0")
        );
        assert!(selected.workspace_axes().is_none());

        let error = parsed
            .select_workspace_axes(&selection(&[])?, &members(&["shared"])?, None)
            .expect_err("different overlapping plans require an axis selection");
        assert_snapshot!(error.to_string(), @"The selected packages have different locked resolutions on unresolved axis `sqlalchemy`; select a section with `--resolution-axis AXIS=SECTION`");
        Ok(())
    }

    #[test]
    fn workspace_axis_command_ignores_unrequested_extras_and_groups() -> TestResult {
        let lock = sqlalchemy_lock(true)?;
        let no_extras = ExtrasSpecification::default().with_defaults(DefaultExtras::default());
        let no_groups = DependencyGroups::default().with_defaults(DefaultGroups::default());
        let selected = lock.select_workspace_axes_for_command(
            &selection(&[])?,
            &members(&["shared"])?,
            None,
            &no_extras,
            &no_groups,
        )?;
        assert!(selected.find_by_name(&"common-leaf".parse()?)?.is_some());
        assert!(selected.find_by_name(&"leaf".parse()?)?.is_none());

        let extras = ExtrasSpecification::from_extra(vec!["feature".parse()?])
            .with_defaults(DefaultExtras::default());
        let groups =
            DependencyGroups::from_group("test".parse()?).with_defaults(DefaultGroups::default());
        assert!(matches!(
            lock.select_workspace_axes_for_command(
                &selection(&[])?,
                &members(&["shared"])?,
                None,
                &extras,
                &no_groups,
            ),
            Err(WorkspaceAxisSelectionError::Ambiguous { .. })
        ));
        assert!(matches!(
            lock.select_workspace_axes_for_command(
                &selection(&[])?,
                &members(&["shared"])?,
                None,
                &no_extras,
                &groups,
            ),
            Err(WorkspaceAxisSelectionError::Ambiguous { .. })
        ));
        Ok(())
    }

    #[test]
    fn workspace_axis_group_python_and_empty_declarations_round_trip() -> TestResult {
        let axes = sqlalchemy_axes()?;
        let full = axes.domain();
        let environment = axes
            .marker_for_domain(&full)
            .and("python_full_version >= '3.12' and python_full_version < '3.15'".parse()?);
        let first = full
            .restrict(&selection(&["sqlalchemy=v1"])?)
            .ok_or("missing first domain")?;
        let second = full
            .restrict(&selection(&["sqlalchemy=v2"])?)
            .ok_or("missing second domain")?;
        let first_lock = ordinary_lock(
            &["legacy", "shared"],
            ">=3.12,<3.15",
            "1.0.0",
            "https://pypi.org/simple",
            false,
        )?;
        let first_lock = Lock::from_toml(
            &first_lock
                .to_toml()?
                .replace("virtual = \"legacy\"", "virtual = \"\""),
        )?;
        let group_metadata: WorkspaceAxisGroupMetadata = toml::from_str(
            r#"
project-root = "legacy"
members = { legacy = { docs = { requires-python = ">=3.14" }, root-only = { requires-python = ">=3.13" } }, modern = {}, shared = { docs = {} } }
member-defaults = { legacy = ["root-only"], modern = [], shared = ["docs"] }
root-default-groups = ["root-only"]
"#,
        )?;
        let lock = Lock::from_workspace_axes_with_environment(
            axes,
            environment,
            group_metadata.clone(),
            vec![
                (first, first_lock),
                (
                    second,
                    ordinary_lock(
                        &["modern", "shared"],
                        ">=3.12,<3.15",
                        "2.0.0",
                        "https://pypi.org/simple",
                        false,
                    )?,
                ),
            ],
        )?
        .ok_or("missing lock")?;
        let serialized = lock.to_toml()?;
        let lock = Lock::from_canonical_toml(&serialized)?;
        assert_eq!(serialized, lock.to_toml()?);
        assert_eq!(
            lock.workspace_axes()
                .ok_or("missing axes")?
                .group_metadata(),
            &group_metadata
        );

        let requested = members(&["shared"])?;
        let no_extras = ExtrasSpecification::default().with_defaults(DefaultExtras::default());
        let root_only = DependencyGroups::from_group("root-only".parse()?)
            .with_defaults(DefaultGroups::default());
        let requires_python = lock.workspace_axis_requires_python_for_command(
            &selection(&[])?,
            &requested,
            &no_extras,
            &root_only,
        )?;
        assert_eq!(requires_python.to_string(), ">=3.13, <3.15");
        let selected = lock.select_workspace_axes_for_command(
            &selection(&[])?,
            &requested,
            None,
            &no_extras,
            &root_only,
        )?;
        assert_eq!(selected.requires_python().to_string(), ">=3.13, <3.15");
        let policy = selected
            .workspace_axis_command()
            .ok_or("missing projected command policy")?;
        assert_eq!(policy.members(), &requested);
        assert_eq!(policy.group_metadata(), &group_metadata);
        assert_eq!(
            policy.group_root(&root_only).map(PackageName::as_str),
            Some("legacy")
        );
        assert_eq!(
            policy
                .root_package(&selected, &"legacy".parse()?)?
                .id
                .source
                .as_source_tree(),
            Some(Path::new(""))
        );
        assert!(
            Lock::from_toml(&selected.to_toml()?)?
                .workspace_axis_command()
                .is_none()
        );
        assert_eq!(
            selected
                .find_by_name(&"leaf".parse()?)?
                .and_then(|package| package.version())
                .map(ToString::to_string)
                .as_deref(),
            Some("1.0.0")
        );

        // A declared empty member group shadows the root's stricter group, even though neither
        // group's existence can be reconstructed from a nonempty dependency edge.
        let docs =
            DependencyGroups::from_group("docs".parse()?).with_defaults(DefaultGroups::default());
        assert_eq!(
            lock.workspace_axis_requires_python_for_command(
                &selection(&[])?,
                &requested,
                &no_extras,
                &docs,
            )?
            .to_string(),
            ">=3.12, <3.15"
        );
        let only_docs = DependencyGroups::from_args(
            None,
            Vec::new(),
            Vec::new(),
            true,
            vec!["docs".parse()?],
            false,
        )
        .with_defaults(DefaultGroups::default());
        let selected = lock.select_workspace_axes_for_command(
            &selection(&[])?,
            &requested,
            None,
            &no_extras,
            &only_docs,
        )?;
        assert!(selected.find_by_name(&"leaf".parse()?)?.is_none());
        assert!(selected.workspace_axis_command().is_some());
        Ok(())
    }

    #[test]
    fn workspace_axis_availability_compares_local_source_identity() -> TestResult {
        let lock = |source: &str| -> TestResult<Lock> {
            Ok(Lock::from_toml(&format!(
                "version = 1\nrequires-python = \">=3.12\"\n[[package]]\nname = \"member\"\nversion = \"1.0.0\"\nsource = {{ {source} }}\n"
            ))?)
        };
        let unavailable = BTreeMap::from([("member".parse()?, PathBuf::from("local/member"))]);
        let local = lock("editable = 'local/./member'")?;
        let registry = lock("registry = 'https://pypi.org/simple'")?;
        let foreign = lock("directory = 'external/member'")?;
        assert!(
            !local.satisfies_workspace_member_availability(Path::new("/workspace"), &unavailable)
        );
        assert!(
            registry.satisfies_workspace_member_availability(Path::new("/workspace"), &unavailable)
        );
        assert!(
            foreign.satisfies_workspace_member_availability(Path::new("/workspace"), &unavailable)
        );
        assert!(local.satisfies_workspace_member_sources(Path::new("/workspace"), &unavailable));
        assert!(
            !registry.satisfies_workspace_member_sources(Path::new("/workspace"), &unavailable)
        );
        assert!(!foreign.satisfies_workspace_member_sources(Path::new("/workspace"), &unavailable));
        let name = "member".parse()?;
        assert_ne!(
            local
                .find_by_name(&name)?
                .ok_or("missing member")?
                .identity(),
            registry
                .find_by_name(&name)?
                .ok_or("missing member")?
                .identity()
        );
        assert_ne!(
            local
                .find_by_name(&name)?
                .ok_or("missing member")?
                .identity(),
            foreign
                .find_by_name(&name)?
                .ok_or("missing member")?
                .identity()
        );
        Ok(())
    }

    #[test]
    fn workspace_axis_named_roots_use_the_local_member_identity() -> TestResult {
        let universal = sqlalchemy_lock(false)?;
        let mut ordinary = Lock::from_toml(
            r#"
version = 1
requires-python = ">=3.12"
resolution-markers = ["python_version < '3.13'", "python_version >= '3.13'"]
[manifest]
members = ["shared"]
[[package]]
name = "shared"
version = "1.0.0"
source = { virtual = "shared" }
resolution-markers = ["python_version < '3.13'"]
[[package]]
name = "shared"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }
resolution-markers = ["python_version >= '3.13'"]
"#,
        )?;
        let requested = members(&["shared"])?;
        let target = AxisInstallable {
            lock: &ordinary,
            members: &requested,
            axes: universal.workspace_axes().ok_or("missing axes")?,
        };
        let name = "shared".parse()?;
        assert!(ordinary.find_by_name(&name).is_err());
        assert_eq!(
            target.root_package(&name)?.id.source.as_source_tree(),
            Some(Path::new("shared"))
        );

        let projected = universal.select_workspace_axes_for_command(
            &selection(&["sqlalchemy=v1"])?,
            &requested,
            None,
            &ExtrasSpecification::default().with_defaults(DefaultExtras::default()),
            &DependencyGroups::default().with_defaults(DefaultGroups::default()),
        )?;
        ordinary.workspace_axis_command = projected.workspace_axis_command;
        let local = ordinary
            .packages_for_name(&name)
            .iter()
            .find(|package| package.id.source.as_source_tree().is_some())
            .ok_or("missing local package")?;
        let foreign = ordinary
            .packages_for_name(&name)
            .iter()
            .find(|package| package.id.source.as_source_tree().is_none())
            .ok_or("missing registry package")?;
        assert!(ordinary.is_workspace_member(local));
        assert!(!ordinary.is_workspace_member(foreign));
        let omit_workspace = InstallOptions::new(
            false,
            false,
            true,
            false,
            false,
            false,
            Vec::new(),
            Vec::new(),
        );
        assert!(!ordinary.includes_install_target(local, Some(&name), &omit_workspace));
        assert!(ordinary.includes_install_target(foreign, Some(&name), &omit_workspace));
        let only_workspace = InstallOptions::new(
            false,
            false,
            false,
            true,
            false,
            false,
            Vec::new(),
            Vec::new(),
        );
        assert!(ordinary.includes_install_target(local, Some(&name), &only_workspace));
        assert!(!ordinary.includes_install_target(foreign, Some(&name), &only_workspace));
        Ok(())
    }

    #[test]
    fn workspace_axis_command_keeps_transitive_local_members_first_party() -> TestResult {
        let axes = sqlalchemy_axes()?;
        let full = axes.domain();
        let ordinary = |member: &str| -> TestResult<Lock> {
            Ok(Lock::from_toml(&format!(
                "version = 1\nrequires-python = \">=3.12\"\n[manifest]\nmembers = [{member:?}, \"shared\"]\n[[package]]\nname = {member:?}\nversion = \"1.0.0\"\nsource = {{ virtual = {member:?} }}\n[[package]]\nname = \"shared\"\nversion = \"1.0.0\"\nsource = {{ virtual = \"shared\" }}\ndependencies = [{{ name = {member:?} }}]\n"
            ))?)
        };
        let first = full
            .restrict(&selection(&["sqlalchemy=v1"])?)
            .ok_or("missing first domain")?;
        let second = full
            .restrict(&selection(&["sqlalchemy=v2"])?)
            .ok_or("missing second domain")?;
        let lock = Lock::from_workspace_axes(
            axes,
            vec![(first, ordinary("legacy")?), (second, ordinary("modern")?)],
        )?
        .ok_or("missing lock")?;
        let requested = members(&["shared"])?;
        let selected = lock.select_workspace_axes_for_command(
            &selection(&["sqlalchemy=v1"])?,
            &requested,
            None,
            &ExtrasSpecification::default().with_defaults(DefaultExtras::default()),
            &DependencyGroups::default().with_defaults(DefaultGroups::default()),
        )?;
        assert_eq!(selected.members(), &members(&["legacy", "shared"])?);
        assert_eq!(
            selected
                .workspace_axis_command()
                .ok_or("missing command policy")?
                .members(),
            &requested
        );
        let legacy = selected
            .find_by_name(&"legacy".parse()?)?
            .ok_or("missing transitive local member")?;
        assert!(selected.is_workspace_member(legacy));
        Ok(())
    }

    #[test]
    fn workspace_axis_unprojected_export_fails_closed() -> TestResult {
        let lock = sqlalchemy_lock(false)?;
        let requested = members(&["shared"])?;
        let target = AxisInstallable {
            lock: &lock,
            members: &requested,
            axes: lock.workspace_axes().ok_or("missing axes")?,
        };
        let error = workspace_axis_package_markers(
            &target,
            &ExtrasSpecification::default().with_defaults(DefaultExtras::default()),
            &DependencyGroups::default().with_defaults(DefaultGroups::default()),
        )
        .expect_err("raw selector-aware lock must not reach ordinary export");
        assert_snapshot!(error.to_string(), @"Workspace resolution axes must be selected before installation or export; use `--resolution-axis AXIS=SECTION`");
        Ok(())
    }

    #[test]
    fn workspace_axis_private_atoms_fail_closed_at_ordinary_boundaries() -> TestResult {
        let axes = sqlalchemy_axes()?;
        let domain = axes.domain();
        let mut ordinary = ordinary_lock(
            &["legacy", "modern", "shared"],
            ">=3.12",
            "1.0.0",
            "https://pypi.org/simple",
            false,
        )?;
        let requires_python = ordinary.requires_python.clone();
        let marker = workspace_axis_marker("sqlalchemy", "v1");
        let shared: PackageName = "shared".parse()?;
        let dependency = ordinary
            .packages
            .iter_mut()
            .find(|package| package.id.name == shared)
            .and_then(|package| package.dependencies.first_mut())
            .ok_or("missing shared dependency")?;
        dependency.complexified_marker = UniversalMarker::from_combined(marker);
        dependency.simplified_marker = SimplifiedMarkerTree::new(&requires_python, marker);
        // This remains a legal ordinary extra spelling outside a selector-aware lock.
        let ordinary = Lock::from_toml(&ordinary.to_toml()?)?;
        let error = Lock::from_workspace_axes(axes, vec![(domain, ordinary)])
            .expect_err("ordinary marker must not be reinterpreted as an axis");
        assert_snapshot!(error.to_string(), @"Invalid workspace resolution axes in lockfile: ordinary dependency predicate uses private workspace selector `uv-axis-a73716c616c6368656d79-s7631`; this extra spelling is not supported by workspace resolution axes");

        let lock = sqlalchemy_lock(false)?;
        let serialized = lock.to_toml()?;
        let mut value: toml::Value = toml::from_str(&serialized)?;
        let packages = value
            .get_mut("package")
            .and_then(toml::Value::as_array_mut)
            .ok_or("missing packages")?;
        let shared = packages
            .iter_mut()
            .find(|package| package.get("name").and_then(toml::Value::as_str) == Some("shared"))
            .and_then(toml::Value::as_table_mut)
            .ok_or("missing shared package")?;
        shared.insert(
            "metadata".to_string(),
            toml::from_str::<toml::Value>(
                "requires-dist = [{ name = 'leaf', specifier = '>=1', marker = \"extra == 'uv-axis-context-c0'\" }]",
            )?,
        );
        let error = Lock::from_toml(&toml::to_string(&value)?)
            .expect_err("declaration metadata must not contain private context atoms");
        assert!(error.to_string().contains("private workspace selector"));

        let unknown = serialized.replacen("uv-axis-context-c0", "uv-axis-context-c999", 1);
        assert_ne!(unknown, serialized);
        let error = Lock::from_toml(&unknown).expect_err("unknown context marker");
        assert!(error.to_string().contains("unknown context marker"));
        Ok(())
    }

    #[test]
    fn workspace_axis_reader_rejects_a_physical_coverage_hole() -> TestResult {
        let lock = sqlalchemy_lock(false)?;
        let serialized = lock.to_toml()?;
        let mut value: toml::Value = toml::from_str(&serialized)?;
        let contexts = value
            .get_mut("workspace-axes")
            .and_then(|axes| axes.get_mut("context"))
            .and_then(toml::Value::as_array_mut)
            .ok_or("missing contexts")?;
        let first = contexts.first_mut().ok_or("missing first context")?;
        first.as_table_mut().ok_or("invalid context")?.insert(
            "environment".to_string(),
            toml::Value::String(
                "python_full_version >= '3.12' and python_full_version < '3.14'".to_string(),
            ),
        );
        assert!(Lock::from_toml(&toml::to_string(&value)?).is_err());

        let mut corrupted = lock.clone();
        let axes = corrupted.workspace_axes.as_mut().ok_or("missing axes")?;
        axes.contexts[0].environment =
            "python_full_version >= '3.12' and python_full_version < '3.14'".parse()?;
        let error = corrupted
            .validate_workspace_axes()
            .expect_err("physical hole");
        assert_snapshot!(error.to_string(), @"Invalid workspace resolution axes in lockfile: solved contexts do not cover exactly the promised Python/platform environment");
        assert!(Lock::from_toml(&serialized.replacen("version = 3", "version = 1", 1)).is_err());
        Ok(())
    }

    #[test]
    fn workspace_axis_projection_retains_disjoint_python_worlds() -> TestResult {
        let definitions: WorkspaceAxes = toml::from_str(
            r#"
[python]
py312 = { members = ["legacy"], requires-python = "==3.12.*" }
py313 = { members = ["modern"], requires-python = "==3.13.*" }
"#,
        )?;
        let axes = ResolvedWorkspaceAxes::from_parts(
            definitions,
            members(&["legacy", "modern", "shared"])?,
        )?;
        let full = axes.domain();
        let first = full
            .restrict(&selection(&["python=py312"])?)
            .ok_or("missing first domain")?;
        let second = full
            .restrict(&selection(&["python=py313"])?)
            .ok_or("missing second domain")?;
        let environment = axes.marker_for_domain(&full).and(
            workspace_axis_marker("python", "py312")
                .and("python_version == '3.12'".parse()?)
                .or(workspace_axis_marker("python", "py313")
                    .and("python_version == '3.13'".parse()?)),
        );
        let lock = checked_axes_lock(
            axes,
            environment,
            vec![
                (
                    first,
                    ordinary_lock(
                        &["legacy", "shared"],
                        "==3.12.*",
                        "1.0.0",
                        "https://pypi.org/simple",
                        false,
                    )?,
                ),
                (
                    second,
                    ordinary_lock(
                        &["modern", "shared"],
                        "==3.13.*",
                        "2.0.0",
                        "https://pypi.org/simple",
                        false,
                    )?,
                ),
            ],
        )?
        .ok_or("missing lock")?;
        let selected =
            lock.select_workspace_axes(&selection(&[])?, &members(&["shared"])?, None)?;
        assert_eq!(selected.packages_for_name(&"leaf".parse()?).len(), 2);
        assert!(selected.workspace_axes().is_none());
        let selected = lock.select_workspace_axes(
            &selection(&["python=py312"])?,
            &members(&["shared"])?,
            None,
        )?;
        assert_eq!(selected.requires_python().to_string(), "==3.12.*");
        Ok(())
    }

    #[test]
    fn workspace_axis_incompatible_transitive_member_is_not_removed() -> TestResult {
        let axes = sqlalchemy_axes()?;
        let full = axes.domain();
        let lock = Lock::from_toml(
            r#"
version = 1
revision = 3
requires-python = ">=3.12"
[manifest]
members = ["legacy", "modern", "shared"]
[[package]]
name = "legacy"
version = "1.0.0"
source = { virtual = "legacy" }
[[package]]
name = "modern"
version = "1.0.0"
source = { virtual = "modern" }
[[package]]
name = "shared"
version = "1.0.0"
source = { virtual = "shared" }
dependencies = [{ name = "legacy" }]
"#,
        )?;
        let error = Lock::from_workspace_axes(axes, vec![(full, lock)])
            .expect_err("incompatible transitive member");
        assert_snapshot!(error.to_string(), @"Invalid workspace resolution axes in lockfile: Workspace member `legacy` is required outside its resolution-axis assignments in context `sqlalchemy=v2`");
        Ok(())
    }

    #[test]
    fn workspace_axis_compatible_products_have_compact_wire_markers() -> TestResult {
        let mut definitions = String::new();
        let mut names = vec!["shared".to_string()];
        for index in 0..24 {
            let left = format!("member-{index:02}-left");
            let right = format!("member-{index:02}-right");
            writeln!(
                definitions,
                "[axis-{index:02}]\nleft = {{ members = [{left:?}] }}\nright = {{ members = [{right:?}] }}\n"
            )?;
            names.push(left);
            names.push(right);
        }
        let names = names.iter().map(String::as_str).collect::<Vec<_>>();
        let axes =
            ResolvedWorkspaceAxes::from_parts(toml::from_str(&definitions)?, members(&names)?)?;
        let full = axes.domain();
        let environment = axes
            .marker_for_domain(&full)
            .and("python_full_version >= '3.12'".parse::<MarkerTree>()?);
        let lock = checked_axes_lock(
            axes,
            environment,
            vec![(
                full,
                ordinary_lock(&names, ">=3.12", "1.0.0", "https://pypi.org/simple", false)?,
            )],
        )?
        .ok_or("missing lock")?;
        let serialized = lock.to_toml()?;
        assert!(
            serialized.len() < 100_000,
            "compatible products must not enumerate worlds"
        );
        let parsed = Lock::from_canonical_toml(&serialized)?;
        assert_eq!(
            parsed
                .workspace_axes()
                .ok_or("missing axes")?
                .contexts()
                .len(),
            1
        );
        assert_eq!(serialized, parsed.to_toml()?);
        Ok(())
    }
}
