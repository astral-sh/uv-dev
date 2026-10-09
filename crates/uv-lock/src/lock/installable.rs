use std::collections::VecDeque;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use either::Either;
use itertools::Itertools;
use petgraph::Graph;
use rustc_hash::{FxHashMap, FxHashSet};

use uv_configuration::{
    BuildOptions, DependencyGroupsWithDefaults, DependencyModifierScope, ExtrasSpecification,
    ExtrasSpecificationWithDefaults, InstallOptions,
};
use uv_distribution_types::{
    Edge, FirstParty, Node, Requirement, RequiresPython, Resolution, ResolvedDist,
};
use uv_normalize::{DefaultExtras, ExtraName, GroupName, PackageName};
use uv_pep508::MarkerTree;
use uv_platform_tags::Tags;
use uv_pypi_types::{ConflictItem, ConflictKind, ConflictSet, ResolverMarkerEnvironment};

use uv_resolver_types::universal_marker::{ActivatedConflictItems, resolve_activated_extras};
use uv_resolver_types::{ConflictMarker, UniversalMarker};
use uv_workspace::{RequiresPythonDeclaration, RequiresPythonSources};

use crate::lock::{
    Dependency, DependencyContext, DependencySelectionContext, HashedDist, LockErrorKind, Package,
    PackageIndex, SelectedDependency, TagPolicy, normalize_requirement,
};
use crate::{Lock, LockError, implicit_constraints_marker};

/// Intersect one root's Python requirements without applying another root's groups to it.
fn root_python_requirement(
    lock: &Lock,
    requirements: &RequiresPythonSources,
    root: Option<&PackageName>,
    inherited_group_root: Option<&PackageName>,
) -> Result<RequiresPython, LockError> {
    let specifiers = requirements
        .iter()
        .filter_map(|(source, specifiers)| match source {
            RequiresPythonDeclaration::Member(package, group) => (Some(package) == root
                || (group.is_some() && Some(package) == inherited_group_root))
                .then_some(specifiers),
            RequiresPythonDeclaration::Workspace(_) => Some(specifiers),
        });
    RequiresPython::intersection(
        std::iter::once(lock.requires_python().specifiers()).chain(specifiers),
    )
    .ok_or_else(|| LockErrorKind::DisjointWorkspaceRequiresPython.into())
}

fn newly_activated_extras<'lock>(
    dep: &'lock Dependency,
    activated_extras: &[(&'lock PackageName, &'lock ExtraName)],
) -> Vec<(&'lock PackageName, &'lock ExtraName)> {
    dep.extra
        .iter()
        .filter_map(|extra| {
            let key = (&dep.package_id.name, extra);
            (!activated_extras.contains(&key)).then_some(key)
        })
        .collect()
}

/// Record another condition under which a locked package and optional extra are reachable.
///
/// Returns `true` when the combined reachability changed.
fn add_reachability<'lock>(
    reachability: &mut FxHashMap<(PackageIndex, Option<&'lock ExtraName>), UniversalMarker>,
    key: (PackageIndex, Option<&'lock ExtraName>),
    marker: UniversalMarker,
) -> bool {
    match reachability.entry(key) {
        Entry::Occupied(mut entry) => {
            let mut combined = *entry.get();
            combined.or(marker);
            if combined == *entry.get() {
                false
            } else {
                entry.insert(combined);
                true
            }
        }
        Entry::Vacant(entry) => {
            entry.insert(marker);
            true
        }
    }
}

/// Resolve certain project and extra requests before evaluating sibling version guards.
fn resolve_conflict_activations<'lock>(
    lock: &'lock Lock,
    known_conflicts: &FxHashMap<ConflictItem, MarkerTree>,
    reachability: &FxHashMap<(PackageIndex, Option<&'lock ExtraName>), UniversalMarker>,
    marker_env: Option<&ResolverMarkerEnvironment>,
) -> FxHashMap<ConflictItem, MarkerTree> {
    let mut pending = FxHashMap::<ConflictItem, MarkerTree>::default();
    for ((index, extra), marker) in reachability {
        let package = lock.package(*index);
        let item = if let Some(extra) = extra {
            ConflictItem::from((package.name().clone(), (*extra).clone()))
        } else if lock.is_workspace_package(package) {
            ConflictItem::from(package.name().clone())
        } else {
            continue;
        };
        if !lock
            .conflicts()
            .contains(item.package(), item.kind().as_ref())
        {
            continue;
        }
        // Concrete Python and platform facts can make a recursive request certain.
        let marker = marker_env.map_or_else(
            || marker.combined(),
            |environment| {
                UniversalMarker::new(
                    MarkerTree::TRUE,
                    marker.conflict_for_environment(environment.markers()),
                )
                .combined()
            },
        );
        pending
            .entry(item)
            .and_modify(|current| *current = current.or(marker))
            .or_insert(marker);
    }

    let mut resolved = known_conflicts
        .iter()
        .filter(|(_, marker)| !UniversalMarker::from_combined(**marker).has_conflict_marker())
        .map(|(item, marker)| (item.clone(), *marker))
        .collect::<FxHashMap<_, _>>();
    for item in lock.conflicts().iter().flat_map(ConflictSet::iter) {
        if !pending.contains_key(item) {
            resolved.entry(item.clone()).or_insert(MarkerTree::FALSE);
        }
    }
    while !pending.is_empty() {
        let mut substitutions = resolved.clone();
        for item in pending.keys() {
            substitutions.entry(item.clone()).or_insert_with(|| {
                UniversalMarker::new(MarkerTree::TRUE, ConflictMarker::from_conflict_item(item))
                    .combined()
            });
        }
        let remaining = pending.len();
        pending.retain(|item, marker| {
            let marker = resolve_activated_extras(*marker, Some(item.package()), &substitutions);
            if UniversalMarker::from_combined(marker).has_conflict_marker() {
                return true;
            }
            resolved
                .entry(item.clone())
                .and_modify(|current| *current = current.or(marker))
                .or_insert(marker);
            false
        });
        // Every successful pass resolves a pending selection, so cycles cannot prevent termination.
        if pending.len() == remaining {
            break;
        }
    }
    resolved
}

/// Returns the dependencies a queued package contributes, either its own or those of one extra.
fn package_dependencies<'a>(
    package: &'a Package,
    extra: Option<&ExtraName>,
) -> impl Iterator<Item = &'a Dependency> {
    if let Some(extra) = extra {
        Either::Left(
            package
                .optional_dependencies
                .get(extra)
                .into_iter()
                .flatten(),
        )
    } else {
        Either::Right(package.dependencies.iter())
    }
}

/// Determines which dependencies are included from an install target root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InstallableRootKind {
    /// Include the root's production dependencies and selected dependency groups.
    Production,
    /// Include only the root's selected dependency groups.
    DependencyGroups,
}

pub trait Installable<'lock> {
    /// Return the root install path.
    fn install_path(&self) -> &'lock Path;

    /// Return the [`Lock`] to install.
    fn lock(&self) -> &'lock Lock;

    /// Return the [`PackageName`] of the root packages in the target.
    fn roots(&self) -> impl Iterator<Item = &PackageName>;

    /// Return the package whose dependency groups, but not production dependencies, are included.
    fn group_root(&self, _groups: &DependencyGroupsWithDefaults) -> Option<&PackageName> {
        None
    }

    /// Return whether a dependency group should be included for its owning package.
    ///
    /// A `None` package represents groups defined directly on a non-project workspace root.
    fn includes_group(
        &self,
        _package: Option<&PackageName>,
        group: &GroupName,
        groups: &DependencyGroupsWithDefaults,
    ) -> bool {
        groups.contains(group)
    }

    /// Collect the locked Python requirements for the selected members and dependency groups.
    fn workspace_python_requirements(
        &self,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<RequiresPythonSources, LockError> {
        let lock = self.lock();
        let mut group_requirements = RequiresPythonSources::new();
        for package in lock
            .non_root_workspace_packages()
            .filter(|package| groups.prod() && self.roots().any(|root| root == package.name()))
        {
            let requires_python = package.workspace_requires_python().ok_or_else(|| {
                LockErrorKind::MissingWorkspaceMemberPython(package.name().clone())
            })?;
            group_requirements.insert(
                RequiresPythonDeclaration::Member(package.name().clone(), None),
                requires_python.clone(),
            );
        }

        for name in self.roots() {
            let Ok(Some(package)) = lock.find_by_name(name) else {
                // Lockfile selection reports missing or ambiguous roots.
                continue;
            };
            if package.fork_markers().is_empty() {
                continue;
            }
            let marker = package.environment_marker();
            let Some(requirement) = RequiresPython::from_marker_tree(marker) else {
                if !marker.is_false() {
                    return Err(
                        LockErrorKind::UnrepresentableLockedRequiresPython(name.clone()).into(),
                    );
                }
                return Err(LockErrorKind::DisjointWorkspaceRequiresPython.into());
            };
            group_requirements
                .entry(RequiresPythonDeclaration::Member(name.clone(), None))
                .and_modify(|specifiers| {
                    *specifiers = specifiers
                        .iter()
                        .chain(requirement.specifiers().iter())
                        .cloned()
                        .collect();
                })
                .or_insert_with(|| requirement.specifiers().clone());
        }

        if let Some(members) = lock.member_group_metadata() {
            let group_root = self.group_root(groups);

            for (member, member_groups) in members {
                // The group root can contribute groups without being an install root.
                let is_install_root = self.roots().any(|root| root == member);
                if !is_install_root && group_root != Some(member) {
                    continue;
                }

                for (group, metadata) in member_groups {
                    if self.includes_group(Some(member), group, groups)
                        && let Some(requires_python) = &metadata.requires_python
                    {
                        group_requirements.insert(
                            RequiresPythonDeclaration::Member(member.clone(), Some(group.clone())),
                            requires_python.clone(),
                        );
                    }
                }
            }
        }

        for (group, metadata) in lock.workspace_group_metadata() {
            if self.includes_group(None, group, groups)
                && let Some(requires_python) = &metadata.requires_python
            {
                group_requirements.insert(
                    RequiresPythonDeclaration::Workspace(group.clone()),
                    requires_python.clone(),
                );
            }
        }

        Ok(group_requirements)
    }

    /// Retain each selected root's Python domain, including its own and inherited groups.
    fn selected_root_python_requirements(
        &self,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<BTreeMap<&PackageName, RequiresPython>, LockError> {
        let requirements = self.workspace_python_requirements(groups)?;
        let group_root = self.group_root(groups);
        self.roots()
            .chain(group_root)
            .map(|root| {
                root_python_requirement(self.lock(), &requirements, Some(root), group_root)
                    .map(|requires_python| (root, requires_python))
            })
            .collect()
    }

    /// Return the universal Python domain of the selected roots and their own dependency groups.
    fn export_python_requirement(
        &self,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<RequiresPython, LockError> {
        if self.roots().next().is_none() {
            let requirements = self.workspace_python_requirements(groups)?;
            return root_python_requirement(
                self.lock(),
                &requirements,
                None,
                self.group_root(groups),
            );
        }
        let requirements = self.selected_root_python_requirements(groups)?;
        RequiresPython::union(
            self.roots()
                .filter_map(|root| requirements.get(root))
                .map(RequiresPython::specifiers),
        )
        .ok_or_else(|| LockErrorKind::UnrepresentableExportRequiresPython.into())
    }

    /// Validate that selected non-root dependencies were resolved for the requested environment.
    fn validate_workspace_resolution(
        &self,
        extras: &ExtrasSpecification,
        groups: &DependencyGroupsWithDefaults,
        marker_environment: Option<&ResolverMarkerEnvironment>,
    ) -> Result<(), LockError> {
        let lock = self.lock();
        let roots = self
            .roots()
            .chain(self.group_root(groups))
            .collect::<FxHashSet<_>>();
        for package in lock
            .non_root_workspace_packages()
            .filter(|package| roots.contains(package.name()))
        {
            for group in package
                .dependency_groups()
                .keys()
                .chain(package.resolved_dependency_groups().keys())
            {
                if self.includes_group(Some(package.name()), group, groups) {
                    return Err(LockErrorKind::UnresolvedWorkspaceGroup {
                        package: package.name().clone(),
                        group: group.clone(),
                    }
                    .into());
                }
            }
        }

        if !groups.prod() {
            return Ok(());
        }
        let roots = self.roots().collect::<FxHashSet<_>>();
        let selected = lock
            .non_root_workspace_packages()
            .filter(|package| roots.contains(package.name()))
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Ok(());
        }
        let mut activated = roots
            .iter()
            .map(|name| ConflictItem::from((*name).clone()))
            .collect::<Vec<_>>();
        let group_root = self.group_root(groups);
        for package in lock.workspace_packages() {
            if roots.contains(package.name()) {
                for extra in extras.extra_names(
                    package
                        .provides_extras()
                        .iter()
                        .chain(package.optional_dependencies().keys()),
                ) {
                    activated.push(ConflictItem::from((package.name().clone(), extra.clone())));
                }
            }
            if roots.contains(package.name()) || group_root == Some(package.name()) {
                for group in package
                    .dependency_groups()
                    .keys()
                    .chain(package.resolved_dependency_groups().keys())
                {
                    if self.includes_group(Some(package.name()), group, groups) {
                        activated.push(ConflictItem::from((package.name().clone(), group.clone())));
                    }
                }
            }
        }
        let resolved = lock.resolved_workspace_reachability(self.install_path(), &activated)?;
        let requirements = marker_environment
            .is_none()
            .then(|| self.workspace_python_requirements(groups))
            .transpose()?;
        for package in selected {
            let domain = if let Some(requirements) = &requirements {
                let requires_python =
                    root_python_requirement(lock, requirements, Some(package.name()), group_root)?;
                implicit_constraints_marker(
                    requires_python.to_exact_marker_tree(),
                    lock.supported_environments(),
                )
            } else {
                MarkerTree::TRUE
            };
            let available = |marker: MarkerTree| {
                if let Some(environment) = marker_environment {
                    marker.evaluate(environment.markers(), &[])
                } else {
                    domain.and(marker.negate()).is_false()
                }
            };
            if groups.prod() {
                let marker = resolved
                    .get(&(package.name(), None))
                    .copied()
                    .unwrap_or(MarkerTree::FALSE);
                if !available(marker) {
                    return Err(
                        LockErrorKind::UnresolvedWorkspacePackage(package.name().clone()).into(),
                    );
                }
            }
            for extra in extras.extra_names(
                package
                    .provides_extras()
                    .iter()
                    .chain(package.optional_dependencies().keys()),
            ) {
                let marker = resolved
                    .get(&(package.name(), Some(extra)))
                    .copied()
                    .unwrap_or(MarkerTree::FALSE);
                if !available(marker) {
                    return Err(LockErrorKind::UnresolvedWorkspaceExtra {
                        package: package.name().clone(),
                        extra: extra.clone(),
                    }
                    .into());
                }
            }
        }
        Ok(())
    }

    /// Return workspace members reachable through the selected roots, extras, and groups.
    fn selected_workspace_members(
        &self,
        extras: &ExtrasSpecification,
        groups: &DependencyGroupsWithDefaults,
        requires_python: &RequiresPython,
        marker_env: Option<&ResolverMarkerEnvironment>,
    ) -> Result<BTreeMap<&'lock PackageName, MarkerTree>, LockError> {
        let lock = self.lock();
        let modifiers = lock.dependency_modifiers()?;
        let roots = self.roots().collect::<FxHashSet<_>>();
        let group_root = self.group_root(groups);
        let root_requirements = marker_env
            .is_none()
            .then(|| self.selected_root_python_requirements(groups))
            .transpose()?;
        let root_domain = |name: &PackageName| {
            root_requirements
                .as_ref()
                .and_then(|requirements| requirements.get(name))
                .map_or(MarkerTree::TRUE, RequiresPython::to_exact_marker_tree)
        };
        let root_marker = UniversalMarker::from_combined(implicit_constraints_marker(
            requires_python.to_exact_marker_tree(),
            lock.supported_environments(),
        ));
        let known_conflicts = lock
            .conflicts()
            .iter()
            .flat_map(ConflictSet::iter)
            .map(|item| {
                let selected = match item.kind() {
                    ConflictKind::Extra(extra) => {
                        (roots.contains(item.package()) && groups.prod() && extras.contains(extra))
                            .then_some(true)
                    }
                    ConflictKind::Group(group) => (roots.contains(item.package())
                        || group_root == Some(item.package()))
                    .then(|| self.includes_group(Some(item.package()), group, groups)),
                    ConflictKind::Project => {
                        (groups.prod() && roots.contains(item.package())).then_some(true)
                    }
                };
                let marker = selected.map_or_else(
                    || {
                        // Unresolved transitive selections remain possible until a path requests them.
                        UniversalMarker::new(
                            MarkerTree::TRUE,
                            ConflictMarker::from_conflict_item(item),
                        )
                        .combined()
                    },
                    |selected| {
                        if selected {
                            root_domain(item.package())
                        } else {
                            MarkerTree::FALSE
                        }
                    },
                );
                (item.clone(), marker)
            })
            .collect::<FxHashMap<_, _>>();
        let selected_conflicts = |parent: &Package, context: DependencyContext<'_>| {
            let mut selected = known_conflicts.clone();
            match context {
                DependencyContext::Production => {
                    selected.insert(ConflictItem::from(parent.name().clone()), MarkerTree::TRUE);
                }
                DependencyContext::Extra(extra) => {
                    selected.insert(ConflictItem::from(parent.name().clone()), MarkerTree::TRUE);
                    selected.insert(
                        ConflictItem::from((parent.name().clone(), extra.clone())),
                        MarkerTree::TRUE,
                    );
                }
                DependencyContext::Group(group) => {
                    selected.insert(
                        ConflictItem::from((parent.name().clone(), group.clone())),
                        MarkerTree::TRUE,
                    );
                }
            }
            selected
        };
        let dependency_marker = |dependency: &Dependency,
                                 requirements: Option<&[Requirement]>,
                                 parent: &Package,
                                 selected: &FxHashMap<ConflictItem, MarkerTree>|
         -> Result<UniversalMarker, LockError> {
            let marker = dependency.activation_marker(requirements, lock, self.install_path())?;
            // Keep selectors until dependency requests on sibling paths have been collected.
            let mut marker = resolve_activated_extras(marker, Some(parent.name()), selected);
            let target = lock.package(dependency.index);
            marker = marker.and(target.environment_marker());
            if marker_env.is_some_and(|environment| {
                !marker.without_extras().evaluate(environment.markers(), &[])
            }) {
                marker = MarkerTree::FALSE;
            }
            Ok(UniversalMarker::from_combined(marker))
        };
        let mut queue: VecDeque<(PackageIndex, Option<&ExtraName>)> = VecDeque::new();
        let mut reachability = FxHashMap::default();

        for (name, root_kind) in roots
            .iter()
            .copied()
            .map(|name| (name, InstallableRootKind::Production))
            .chain(group_root.map(|name| (name, InstallableRootKind::DependencyGroups)))
        {
            let Some(&index) = lock.workspace_members.get(name) else {
                continue;
            };
            let package = lock.package(index);
            let mut root_marker = root_marker;
            root_marker.and(UniversalMarker::from_combined(
                package.environment_marker().and(root_domain(name)),
            ));
            if root_kind == InstallableRootKind::Production && groups.prod() {
                if add_reachability(&mut reachability, (index, None), root_marker) {
                    queue.push_back((index, None));
                }
                for extra in extras.extra_names(package.optional_dependencies().keys()) {
                    if add_reachability(&mut reachability, (index, Some(extra)), root_marker) {
                        queue.push_back((index, Some(extra)));
                    }
                }
            }

            for (group, dependencies) in package.resolved_dependency_groups() {
                if !self.includes_group(Some(package.name()), group, groups) {
                    continue;
                }
                let context = DependencyContext::Group(group);
                let requirements = package.dependency_requirements(
                    context,
                    &modifiers,
                    self.install_path(),
                    lock.requires_python(),
                )?;
                let selected = selected_conflicts(package, context);
                for dependency in dependencies {
                    let mut marker = root_marker;
                    marker.and(dependency_marker(
                        dependency,
                        requirements.as_deref(),
                        package,
                        &selected,
                    )?);
                    if marker.is_false() {
                        continue;
                    }
                    if add_reachability(&mut reachability, (dependency.index, None), marker) {
                        queue.push_back((dependency.index, None));
                    }
                    for extra in dependency.extra() {
                        if add_reachability(
                            &mut reachability,
                            (dependency.index, Some(extra)),
                            marker,
                        ) {
                            queue.push_back((dependency.index, Some(extra)));
                        }
                    }
                }
            }
        }

        let requirements = lock.requirements().iter().filter(|_| groups.prod()).chain(
            lock.dependency_groups()
                .iter()
                .filter(|(group, _)| self.includes_group(None, group, groups))
                .flat_map(|(_, requirements)| requirements),
        );
        for requirement in modifiers.apply(DependencyModifierScope::Global, requirements) {
            let requirement = normalize_requirement(
                requirement.into_owned(),
                self.install_path(),
                lock.requires_python(),
            )?;
            for package in lock.packages_for_name(&requirement.name) {
                if !Lock::package_satisfies_requirement(package, &requirement, self.install_path())?
                {
                    continue;
                }
                let Some(marker) = lock.root_requirement_marker(&requirement, package) else {
                    continue;
                };
                let mut marker = UniversalMarker::from_combined(marker);
                marker.and(root_marker);
                if marker.is_false()
                    || marker_env.is_some_and(|environment| {
                        !marker.pep508().evaluate(environment.markers(), &[])
                    })
                {
                    continue;
                }
                let index = lock.by_id[&package.id];
                if add_reachability(&mut reachability, (index, None), marker) {
                    queue.push_back((index, None));
                }
                for extra in &requirement.extras {
                    if let Some((extra, _)) = package.optional_dependencies().get_key_value(extra)
                        && add_reachability(&mut reachability, (index, Some(extra)), marker)
                    {
                        queue.push_back((index, Some(extra)));
                    }
                }
            }
        }

        while let Some((index, extra)) = queue.pop_front() {
            let parent_marker = reachability[&(index, extra)];
            let package = lock.package(index);
            let context = extra.map_or(DependencyContext::Production, DependencyContext::Extra);
            let requirements = package.dependency_requirements(
                context,
                &modifiers,
                self.install_path(),
                lock.requires_python(),
            )?;
            let selected = selected_conflicts(package, context);
            for dependency in package_dependencies(package, extra) {
                let mut marker = parent_marker;
                marker.and(dependency_marker(
                    dependency,
                    requirements.as_deref(),
                    package,
                    &selected,
                )?);
                if marker.is_false() {
                    continue;
                }
                if add_reachability(&mut reachability, (dependency.index, None), marker) {
                    queue.push_back((dependency.index, None));
                }
                for extra in dependency.extra() {
                    if add_reachability(&mut reachability, (dependency.index, Some(extra)), marker)
                    {
                        queue.push_back((dependency.index, Some(extra)));
                    }
                }
            }
        }

        let activated =
            resolve_conflict_activations(lock, &known_conflicts, &reachability, marker_env);
        let members = reachability
            .into_iter()
            .filter_map(|((index, extra), marker)| {
                let package = lock.package(index);
                if extra.is_some() || !lock.is_workspace_package(package) {
                    return None;
                }
                let marker =
                    resolve_activated_extras(marker.combined(), Some(package.name()), &activated)
                        .without_extras();
                if marker.is_false()
                    || marker_env
                        .is_some_and(|environment| !marker.evaluate(environment.markers(), &[]))
                {
                    return None;
                }
                Some((package.name(), marker))
            })
            .fold(BTreeMap::new(), |mut members, (name, marker)| {
                members
                    .entry(name)
                    .and_modify(|existing: &mut MarkerTree| {
                        *existing = existing.or(marker);
                    })
                    .or_insert(marker);
                members
            });
        Ok(members)
    }

    /// Return the [`PackageName`] of the target, if available.
    fn project_name(&self) -> Option<&PackageName>;

    /// Convert the [`Lock`] to a [`Resolution`] using the given marker environment, tags, and root.
    fn to_resolution(
        &self,
        marker_env: &ResolverMarkerEnvironment,
        tags: &Tags,
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
        build_options: &BuildOptions,
        install_options: &InstallOptions,
    ) -> Result<Resolution, LockError> {
        self.validate_workspace_resolution(extras, groups, Some(marker_env))?;
        let resolve_root = |root_name: &PackageName| {
            self.lock()
                .find_by_name(root_name)
                .map_err(|_| LockErrorKind::MultipleRootPackages {
                    name: root_name.clone(),
                })?
                .ok_or_else(|| {
                    LockError::from(LockErrorKind::MissingRootPackage {
                        name: root_name.clone(),
                    })
                })
        };
        let roots = self
            .roots()
            .map(&resolve_root)
            .collect::<Result<Vec<_>, LockError>>()?;
        let group_root = self.group_root(groups).map(resolve_root).transpose()?;

        InstallableExt::to_resolution_from_packages(
            self,
            &roots,
            group_root,
            true,
            DependencySelectionContext::None,
            marker_env,
            tags,
            extras,
            groups,
            build_options,
            install_options,
        )
    }

    /// Create an installable [`Node`] from a [`Package`].
    fn installable_node(
        &self,
        package: &Package,
        tags: &Tags,
        marker_env: &ResolverMarkerEnvironment,
        build_options: &BuildOptions,
    ) -> Result<Node, LockError> {
        let tag_policy = TagPolicy::Required(tags);
        let HashedDist { dist, hashes } = package.to_dist(
            self.install_path(),
            tag_policy,
            build_options,
            marker_env,
            if self.lock().is_workspace_member(package) {
                FirstParty::Yes
            } else {
                FirstParty::No
            },
        )?;
        let version = package.version().cloned();
        let dist = ResolvedDist::Installable {
            dist: Arc::new(dist),
            version,
        };
        Ok(Node::Dist {
            dist,
            hashes,
            install: true,
        })
    }

    /// Create a non-installable [`Node`] from a [`Package`].
    fn non_installable_node(
        &self,
        package: &Package,
        tags: &Tags,
        marker_env: &ResolverMarkerEnvironment,
    ) -> Result<Node, LockError> {
        let HashedDist { dist, .. } = package.to_dist(
            self.install_path(),
            TagPolicy::Preferred(tags),
            &BuildOptions::default(),
            marker_env,
            FirstParty::No,
        )?;
        let version = package.version().cloned();
        let dist = ResolvedDist::Installable {
            dist: Arc::new(dist),
            version,
        };
        let hashes = package.hashes();
        Ok(Node::Dist {
            dist,
            hashes,
            install: false,
        })
    }

    /// Convert a lockfile entry to a graph [`Node`].
    fn package_to_node(
        &self,
        package: &Package,
        tags: &Tags,
        build_options: &BuildOptions,
        install_options: &InstallOptions,
        marker_env: &ResolverMarkerEnvironment,
    ) -> Result<Node, LockError> {
        if install_options.include_package(
            package.as_install_target(),
            self.project_name(),
            self.lock().workspace_members(),
        ) {
            self.installable_node(package, tags, marker_env, build_options)
        } else {
            self.non_installable_node(package, tags, marker_env)
        }
    }
}

/// Internal lock-to-resolution implementation shared by [`Installable`] and [`Lock`].
trait InstallableExt<'lock>: Installable<'lock> {
    /// Convert concrete locked packages to a [`Resolution`].
    ///
    /// `include_manifest` controls whether requirements attached directly to the lock target are
    /// included in addition to `roots`.
    fn to_resolution_from_packages(
        &self,
        roots: &[&Package],
        group_root: Option<&Package>,
        include_manifest: bool,
        selection_context: DependencySelectionContext<'lock>,
        marker_env: &ResolverMarkerEnvironment,
        tags: &Tags,
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
        build_options: &BuildOptions,
        install_options: &InstallOptions,
    ) -> Result<Resolution, LockError> {
        let size_guess = self.lock().packages.len();
        let mut petgraph = Graph::with_capacity(size_guess, size_guess);
        let mut inverse = vec![None; size_guess];

        let mut queue: VecDeque<(PackageIndex, Option<&ExtraName>)> = VecDeque::new();
        let mut seen = FxHashSet::default();
        let mut conflict_reachability = FxHashMap::default();
        let mut activated_projects: Vec<&PackageName> = vec![];
        let mut activated_extras: Vec<(&PackageName, &ExtraName)> = vec![];
        let mut activated_groups: Vec<(&PackageName, &GroupName)> = vec![];
        let has_conflicts = !self.lock().conflicts().is_empty();
        let validate_conflicts = !include_manifest && has_conflicts;
        let mut dependencies_for_conflict_validation = vec![];

        let root = petgraph.add_node(Node::Root);

        match selection_context {
            DependencySelectionContext::None => {}
            DependencySelectionContext::Production(project) => {
                activated_projects.push(project);
            }
            DependencySelectionContext::Group(project, group) => {
                activated_groups.push((project, group));
            }
        }

        // Determine the set of activated extras and groups, from the root.
        //
        // Extras activated by dependency groups (via `pkg[extra]` entries in the group) are
        // accumulated below, when we process the groups themselves. This ensures that when we
        // later evaluate conflict markers on transitive dependencies, self-extras enabled by an
        // active group are treated as enabled.
        //
        // TODO(zanieb): For completeness, the group-dep loop below still has two structural
        // soundness gaps. Neither is reachable through lockfiles the resolver currently
        // produces — they'd require a group-dep entry with a strict positive conflict marker
        // referencing an extra *other* than the entry's own self-extra, and the resolver
        // either emits self-extra markers (handled by `newly_activated_extras` below) or
        // markers with vacuously-true disjuncts. But the code should still handle them:
        //
        // 1. Ordering: an earlier group-dep entry whose marker references an extra activated
        //    by a later entry is evaluated with an incomplete `activated_extras` set.
        // 2. Transitive self-extras: a group dep `pkg[a]` where `pkg.optional_dependencies.a`
        //    includes `pkg[b]` only activates `(pkg, b)` during the first-pass traversal
        //    below, so any group dep whose marker needs `(pkg, b)` is evaluated too early.
        //
        // Fixing these correctly likely means iterating group-dep activation to a fixed point
        // or interleaving it with the first-pass traversal.
        if has_conflicts {
            for dist in roots.iter().copied() {
                // Track the activated extras.
                if groups.prod() {
                    activated_projects.push(&dist.id.name);
                    for extra in extras.extra_names(dist.optional_dependencies.keys()) {
                        activated_extras.push((&dist.id.name, extra));
                    }
                }
            }

            for dist in roots.iter().copied().chain(group_root) {
                for group in dist
                    .dependency_groups
                    .keys()
                    .filter(|group| self.includes_group(Some(&dist.id.name), group, groups))
                {
                    activated_groups.push((&dist.id.name, group));
                }
            }
        }

        // Initialize the workspace roots.
        let mut initialized_roots = vec![];
        for (dist, root_kind) in roots
            .iter()
            .copied()
            .map(|dist| (dist, InstallableRootKind::Production))
            .chain(group_root.map(|dist| (dist, InstallableRootKind::DependencyGroups)))
        {
            // Add the workspace package to the graph.
            let package_index = self.lock().by_id[&dist.id];
            let index = petgraph.add_node(
                if root_kind == InstallableRootKind::Production && groups.prod() {
                    self.package_to_node(dist, tags, build_options, install_options, marker_env)?
                } else {
                    self.non_installable_node(dist, tags, marker_env)?
                },
            );
            inverse[package_index.0] = Some(index);

            // Add an edge from the root.
            petgraph.add_edge(root, index, Edge::Prod);

            // Push the package onto the queue.
            initialized_roots.push((dist, package_index, index, root_kind));
        }

        // Add the workspace dependencies to the queue.
        for (dist, package_index, index, root_kind) in initialized_roots {
            if root_kind == InstallableRootKind::Production && groups.prod() {
                // Push its dependencies onto the queue.
                queue.push_back((package_index, None));
                add_reachability(
                    &mut conflict_reachability,
                    (package_index, None),
                    UniversalMarker::TRUE,
                );
                for extra in extras.extra_names(dist.optional_dependencies.keys()) {
                    queue.push_back((package_index, Some(extra)));
                    add_reachability(
                        &mut conflict_reachability,
                        (package_index, Some(extra)),
                        UniversalMarker::TRUE,
                    );
                }
            }

            // Add any dev dependencies.
            for (group, dep) in dist
                .dependency_groups
                .iter()
                .filter_map(|(group, deps)| {
                    if self.includes_group(Some(&dist.id.name), group, groups) {
                        Some(deps.iter().map(move |dep| (group, dep)))
                    } else {
                        None
                    }
                })
                .flatten()
            {
                if validate_conflicts && dep.complexified_marker.has_conflict_marker() {
                    dependencies_for_conflict_validation.push((dist, dep));
                }
                let additional_activated_extras = newly_activated_extras(dep, &activated_extras);
                if !dep.complexified_marker.evaluate(
                    marker_env,
                    activated_projects.iter().copied(),
                    activated_extras
                        .iter()
                        .chain(additional_activated_extras.iter())
                        .copied(),
                    activated_groups.iter().copied(),
                ) {
                    continue;
                }

                let dep_dist = self.lock().package(dep.index);

                // Add the package to the graph.
                let dep_index = match inverse[dep.index.0] {
                    None => {
                        let index = petgraph.add_node(self.package_to_node(
                            dep_dist,
                            tags,
                            build_options,
                            install_options,
                            marker_env,
                        )?);
                        inverse[dep.index.0] = Some(index);
                        index
                    }
                    Some(index) => {
                        // Critically, if the package is already in the graph, then it's a workspace
                        // member. If it was omitted due to, e.g., `--only-dev`, but is itself
                        // referenced as a development dependency, then we need to re-enable it.
                        let node = &mut petgraph[index];
                        if !groups.prod() || matches!(node, Node::Dist { install: false, .. }) {
                            *node = self.package_to_node(
                                dep_dist,
                                tags,
                                build_options,
                                install_options,
                                marker_env,
                            )?;
                        }
                        index
                    }
                };

                petgraph.add_edge(
                    index,
                    dep_index,
                    // This is OK because we are resolving to a resolution for
                    // a specific marker environment and set of extras/groups.
                    // So at this point, we know the extras/groups have been
                    // satisfied, so we can safely drop the conflict marker.
                    Edge::Dev(group.clone()),
                );

                // Persist any self-extras activated by this group dependency (e.g., a group
                // that references `pkg[extra]`). Without this, conflict markers on transitive
                // dependencies gated by the activated extra would not evaluate to `true`
                // during the graph traversals below.
                for key in additional_activated_extras {
                    activated_extras.push(key);
                }

                // Push its dependencies on the queue.
                add_reachability(
                    &mut conflict_reachability,
                    (dep.index, None),
                    dep.complexified_marker,
                );
                if seen.insert((dep.index, None)) {
                    queue.push_back((dep.index, None));
                }
                for extra in &dep.extra {
                    add_reachability(
                        &mut conflict_reachability,
                        (dep.index, Some(extra)),
                        dep.complexified_marker,
                    );
                    if seen.insert((dep.index, Some(extra))) {
                        queue.push_back((dep.index, Some(extra)));
                    }
                }
            }
        }

        if include_manifest {
            // Add any requirements that are exclusive to the workspace root (e.g., dependencies in
            // PEP 723 scripts).
            for dependency in self.lock().requirements() {
                if !dependency.marker.evaluate(marker_env, &[]) {
                    continue;
                }

                let root_name = &dependency.name;
                let dist = self
                    .lock()
                    .find_by_markers(root_name, marker_env)
                    .map_err(|_| LockErrorKind::MultipleRootPackages {
                        name: root_name.clone(),
                    })?
                    .ok_or_else(|| LockErrorKind::MissingRootPackage {
                        name: root_name.clone(),
                    })?;

                // Add the package to the graph.
                let package_index = self.lock().by_id[&dist.id];
                let index = match inverse[package_index.0] {
                    None => {
                        let index = petgraph.add_node(if groups.prod() {
                            self.package_to_node(
                                dist,
                                tags,
                                build_options,
                                install_options,
                                marker_env,
                            )?
                        } else {
                            self.non_installable_node(dist, tags, marker_env)?
                        });
                        inverse[package_index.0] = Some(index);
                        index
                    }
                    Some(index) => index,
                };

                // Add the edge.
                petgraph.add_edge(root, index, Edge::Prod);

                // Push its dependencies on the queue.
                add_reachability(
                    &mut conflict_reachability,
                    (package_index, None),
                    UniversalMarker::TRUE,
                );
                if seen.insert((package_index, None)) {
                    queue.push_back((package_index, None));
                }
                for extra in &dependency.extras {
                    add_reachability(
                        &mut conflict_reachability,
                        (package_index, Some(extra)),
                        UniversalMarker::TRUE,
                    );
                    if seen.insert((package_index, Some(extra))) {
                        queue.push_back((package_index, Some(extra)));
                    }
                }
            }

            // Add any dependency groups that are exclusive to the workspace root (e.g., dev
            // dependencies in non-project workspace roots).
            for (group, dependency) in self
                .lock()
                .dependency_groups()
                .iter()
                .filter_map(|(group, deps)| {
                    if self.includes_group(None, group, groups) {
                        Some(deps.iter().map(move |dep| (group, dep)))
                    } else {
                        None
                    }
                })
                .flatten()
            {
                if !dependency.marker.evaluate(marker_env, &[]) {
                    continue;
                }

                let root_name = &dependency.name;
                let dist = self
                    .lock()
                    .find_by_markers(root_name, marker_env)
                    .map_err(|_| LockErrorKind::MultipleRootPackages {
                        name: root_name.clone(),
                    })?
                    .ok_or_else(|| LockErrorKind::MissingRootPackage {
                        name: root_name.clone(),
                    })?;

                // Add the package to the graph.
                let package_index = self.lock().by_id[&dist.id];
                let index = match inverse[package_index.0] {
                    None => {
                        let index = petgraph.add_node(self.package_to_node(
                            dist,
                            tags,
                            build_options,
                            install_options,
                            marker_env,
                        )?);
                        inverse[package_index.0] = Some(index);
                        index
                    }
                    Some(index) => {
                        // Critically, if the package is already in the graph, then it's a workspace
                        // member. If it was omitted due to, e.g., `--only-dev`, but is itself
                        // referenced as a development dependency, then we need to re-enable it.
                        let node = &mut petgraph[index];
                        if !groups.prod() {
                            *node = self.package_to_node(
                                dist,
                                tags,
                                build_options,
                                install_options,
                                marker_env,
                            )?;
                        }
                        index
                    }
                };

                // Add the edge.
                petgraph.add_edge(root, index, Edge::Dev(group.clone()));

                // Persist any self-extras activated by this group dependency. Mirrors the
                // handling in the package-level `dependency_groups` loop above; without this,
                // conflict markers on transitive dependencies gated by the activated extra
                // would not evaluate to `true` during the graph traversals below.
                for extra in &dependency.extras {
                    let key = (&dist.id.name, extra);
                    if !activated_extras.contains(&key) {
                        activated_extras.push(key);
                    }
                }

                // Push its dependencies on the queue.
                add_reachability(
                    &mut conflict_reachability,
                    (package_index, None),
                    UniversalMarker::TRUE,
                );
                if seen.insert((package_index, None)) {
                    queue.push_back((package_index, None));
                }
                for extra in &dependency.extras {
                    add_reachability(
                        &mut conflict_reachability,
                        (package_index, Some(extra)),
                        UniversalMarker::TRUE,
                    );
                    if seen.insert((package_index, Some(extra))) {
                        queue.push_back((package_index, Some(extra)));
                    }
                }
            }
        }

        // Below, we traverse the dependency graph in a breadth first manner
        // twice. It's only in the second traversal that we actually build
        // up our resolution graph. In the first traversal, we accumulate all
        // activated extras. This includes the extras explicitly enabled on
        // the CLI (which were gathered above) and the extras enabled via
        // dependency specifications like `foo[extra]`. We need to do this
        // to correctly support conflicting extras.
        //
        // In particular, the way conflicting extras works is by forking the
        // resolver based on the extras that are declared as conflicting. But
        // this forking needs to be made manifest somehow in the lock file to
        // avoid multiple versions of the same package being installed into the
        // environment. This is why "conflict markers" were invented. For
        // example, you might have both `torch` and `torch+cpu` in your
        // dependency graph, where the latter is only enabled when the `cpu`
        // extra is enabled, and the former is specifically *not* enabled
        // when the `cpu` extra is enabled.
        //
        // In order to evaluate these conflict markers correctly, we need to
        // know whether the `cpu` extra is enabled when we visit the `torch`
        // dependency. If we think it's disabled, then we'll erroneously
        // include it if the extra is actually enabled. But in order to tell
        // if it's enabled, we need to traverse the entire dependency graph
        // first to inspect which extras are enabled!
        //
        // Of course, we don't need to do this at all if there aren't any
        // conflicts. In which case, we skip all of this and just do the one
        // traversal below.
        if has_conflicts {
            let mut activated_extras_set: BTreeSet<(&PackageName, &ExtraName)> =
                activated_extras.iter().copied().collect();
            let mut queue = queue.clone();
            let mut reachability = conflict_reachability;
            while let Some((package_index, extra)) = queue.pop_front() {
                let package = self.lock().package(package_index);
                let Some(parent_reachability) = reachability.get(&(package_index, extra)).copied()
                else {
                    continue;
                };
                for dep in package_dependencies(package, extra) {
                    let mut dep_reachability = dep.complexified_marker;
                    dep_reachability.and(parent_reachability);
                    let additional_activated_extras =
                        newly_activated_extras(dep, &activated_extras);
                    if !dep_reachability.evaluate(
                        marker_env,
                        activated_projects.iter().copied(),
                        activated_extras
                            .iter()
                            .chain(additional_activated_extras.iter())
                            .copied(),
                        activated_groups.iter().copied(),
                    ) {
                        continue;
                    }
                    // The dependency can still be visited provisionally before all activated
                    // extras are known. The second traversal below will exclude it once those
                    // extras are available. Crucially, `dep_reachability` includes the conditions
                    // required to reach the parent package: dependency markers may have been
                    // simplified under those conditions and cannot stand alone during this
                    // preliminary traversal. Otherwise, an unreachable package could activate an
                    // extra and cause the conflict check below to report a false positive.

                    for key in additional_activated_extras {
                        activated_extras_set.insert(key);
                        activated_extras.push(key);
                    }
                    // Push its dependencies on the queue.
                    if add_reachability(&mut reachability, (dep.index, None), dep_reachability) {
                        queue.push_back((dep.index, None));
                    }
                    for extra in &dep.extra {
                        if add_reachability(
                            &mut reachability,
                            (dep.index, Some(extra)),
                            dep_reachability,
                        ) {
                            queue.push_back((dep.index, Some(extra)));
                        }
                    }
                }
            }
            // At time of writing, it's somewhat expected that the set of
            // conflicting extras is pretty small. With that said, the
            // time complexity of the following routine is pretty gross.
            // Namely, `set.contains` is linear in the size of the set,
            // iteration over all conflicts is also obviously linear in
            // the number of conflicting sets and then for each of those,
            // we visit every possible pair of activated extra from above,
            // which is quadratic in the total number of extras enabled. I
            // believe the simplest improvement here, if it's necessary, is
            // to adjust the `Conflicts` internals to own these sorts of
            // checks. ---AG
            for set in self.lock().conflicts().iter() {
                for ((pkg1, extra1), (pkg2, extra2)) in
                    activated_extras_set.iter().tuple_combinations()
                {
                    if set.contains(pkg1, *extra1) && set.contains(pkg2, *extra2) {
                        return Err(LockErrorKind::ConflictingExtra {
                            package1: (*pkg1).clone(),
                            extra1: (*extra1).clone(),
                            package2: (*pkg2).clone(),
                            extra2: (*extra2).clone(),
                        }
                        .into());
                    }
                }
            }
        }

        // Unlike the traversals above, this one never activates an extra, so the activated set is
        // fixed for its duration and can be encoded once instead of once per dependency.
        let activated = ActivatedConflictItems::new(
            activated_projects.iter().copied(),
            activated_extras.iter().copied(),
            activated_groups.iter().copied(),
        );

        while let Some((package_index, extra)) = queue.pop_front() {
            let package = self.lock().package(package_index);
            for dep in package_dependencies(package, extra) {
                if validate_conflicts && dep.complexified_marker.has_conflict_marker() {
                    dependencies_for_conflict_validation.push((package, dep));
                }
                if !dep
                    .complexified_marker
                    .evaluate_activated(marker_env, &activated)
                {
                    continue;
                }

                let dep_dist = self.lock().package(dep.index);

                // Add the dependency to the graph.
                let dep_index = match inverse[dep.index.0] {
                    None => {
                        let index = petgraph.add_node(self.package_to_node(
                            dep_dist,
                            tags,
                            build_options,
                            install_options,
                            marker_env,
                        )?);
                        inverse[dep.index.0] = Some(index);
                        index
                    }
                    Some(index) => {
                        if matches!(&petgraph[index], Node::Dist { install: false, .. }) {
                            petgraph[index] = self.package_to_node(
                                dep_dist,
                                tags,
                                build_options,
                                install_options,
                                marker_env,
                            )?;
                        }
                        index
                    }
                };

                // Add the edge.
                let index = inverse[package_index.0].expect("queued package has a graph node");
                petgraph.add_edge(
                    index,
                    dep_index,
                    if let Some(extra) = extra {
                        Edge::Optional(extra.clone())
                    } else {
                        Edge::Prod
                    },
                );

                // Push its dependencies on the queue.
                if seen.insert((dep.index, None)) {
                    queue.push_back((dep.index, None));
                }
                for extra in &dep.extra {
                    if seen.insert((dep.index, Some(extra))) {
                        queue.push_back((dep.index, Some(extra)));
                    }
                }
            }
        }

        // Evaluate conflict markers from concrete roots, not from workspace members that depend on
        // them. Reject markers that still depend on conflict items outside the resulting subgraph.
        if !dependencies_for_conflict_validation.is_empty() {
            let subgraph_packages = inverse
                .iter()
                .enumerate()
                .filter_map(|(package_index, index)| {
                    index.map(|_| &self.lock().packages[package_index].id.name)
                })
                .collect::<FxHashSet<_>>();
            let selection_context_package = selection_context.package();

            // The environment and conflict state are shared by every dependency, so repeated
            // markers have the same result.
            let mut validated_markers = FxHashSet::default();
            for (package, dependency) in dependencies_for_conflict_validation {
                if !validated_markers.insert(dependency.complexified_marker) {
                    continue;
                }
                let mut marker = dependency.complexified_marker;
                for item in self.lock().conflicts().iter().flat_map(ConflictSet::iter) {
                    if selection_context_package != Some(item.package())
                        && !subgraph_packages.contains(item.package())
                    {
                        continue;
                    }

                    let active = match item.kind() {
                        ConflictKind::Project => activated_projects.contains(&item.package()),
                        ConflictKind::Extra(extra) => {
                            activated_extras.contains(&(item.package(), extra))
                        }
                        ConflictKind::Group(group) => {
                            activated_groups.contains(&(item.package(), group))
                        }
                    };
                    if active {
                        marker.assume_conflict_item(item);
                    } else {
                        marker.assume_not_conflict_item(item);
                    }
                }

                let conflict = marker.conflict_for_environment(marker_env);
                // All in-subgraph conflict items were resolved above, so a non-constant marker
                // still depends on a package outside the subgraph.
                if !conflict.is_constant() {
                    return Err(LockErrorKind::DependencyConflictOutsideSubgraph {
                        package: package.id.clone(),
                        dependency: dependency.package_id.clone(),
                    }
                    .into());
                }
            }
        }

        Ok(Resolution::new(petgraph))
    }
}

impl<'lock, T> InstallableExt<'lock> for T where T: Installable<'lock> + ?Sized {}

/// An [`Installable`] adapter for materializing concrete packages directly from a [`Lock`].
struct LockedPackages<'lock> {
    lock: &'lock Lock,
    install_path: &'lock Path,
    project_name: Option<&'lock PackageName>,
    roots: Vec<&'lock Package>,
}

impl<'lock> Installable<'lock> for LockedPackages<'lock> {
    fn install_path(&self) -> &'lock Path {
        self.install_path
    }

    fn lock(&self) -> &'lock Lock {
        self.lock
    }

    fn roots(&self) -> impl Iterator<Item = &PackageName> {
        self.roots.iter().map(|package| package.name())
    }

    fn project_name(&self) -> Option<&PackageName> {
        self.project_name
    }
}

impl Lock {
    /// Materialize a direct dependency selection from this lock.
    ///
    /// Like [`Self::to_resolution`], this materializes the selected dependency's subgraph. It also
    /// preserves the extras activated by the direct edge and the project production or group
    /// context used to select a conflict fork.
    pub fn to_resolution_from_dependency<'lock>(
        &'lock self,
        install_path: &'lock Path,
        dependency: &SelectedDependency<'lock>,
        project_name: Option<&'lock PackageName>,
        marker_env: &ResolverMarkerEnvironment,
        tags: &Tags,
        build_options: &BuildOptions,
        install_options: &InstallOptions,
    ) -> Result<Resolution, LockError> {
        let selected_package = dependency.package();
        let Some(index) = self.by_id.get(&selected_package.id) else {
            return Err(LockErrorKind::RootPackageMissingFromLock {
                id: selected_package.id.clone(),
            }
            .into());
        };
        let Some(package) = self.packages.get(index.0) else {
            return Err(LockErrorKind::RootPackageMissingFromLock {
                id: selected_package.id.clone(),
            }
            .into());
        };
        let extras = ExtrasSpecification::from_extra(dependency.extras().cloned().collect())
            .with_defaults(DefaultExtras::default());
        let groups = DependencyGroupsWithDefaults::none();

        LockedPackages {
            lock: self,
            install_path,
            project_name,
            roots: vec![package],
        }
        .to_resolution_from_packages(
            &[package],
            None,
            false,
            dependency.context(),
            marker_env,
            tags,
            &extras,
            &groups,
            build_options,
            install_options,
        )
    }

    /// Materialize the exact dependency subgraph reachable from concrete locked `roots`.
    ///
    /// Each root must be a [`Package`] from this lock. Unlike [`Installable::to_resolution`], this
    /// method does not include requirements or dependency groups attached directly to the lock
    /// manifest. Extras and dependency groups on the concrete roots are still included according
    /// to `extras` and `groups`.
    ///
    /// Conflict-marker evaluation starts from `roots` and their requested `extras` and `groups`,
    /// not from workspace members that depend on those roots. The method returns an error if a
    /// dependency marker still depends on a conflict item outside the resulting subgraph. Use
    /// [`Installable::to_resolution`] when materializing an existing lock target.
    ///
    /// `project_name` identifies the project for project-specific [`InstallOptions`] filters, if
    /// applicable. Callers are responsible for selecting roots that apply to `marker_env`.
    pub fn to_resolution<'lock>(
        &'lock self,
        install_path: &'lock Path,
        roots: impl IntoIterator<Item = &'lock Package>,
        project_name: Option<&'lock PackageName>,
        marker_env: &ResolverMarkerEnvironment,
        tags: &Tags,
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
        build_options: &BuildOptions,
        install_options: &InstallOptions,
    ) -> Result<Resolution, LockError> {
        let mut seen = FxHashSet::default();
        let mut concrete_roots = Vec::new();
        for root in roots {
            let Some(index) = self.by_id.get(&root.id) else {
                return Err(LockErrorKind::RootPackageMissingFromLock {
                    id: root.id.clone(),
                }
                .into());
            };
            if seen.insert(&root.id) {
                let Some(root) = self.packages.get(index.0) else {
                    return Err(LockErrorKind::RootPackageMissingFromLock {
                        id: root.id.clone(),
                    }
                    .into());
                };
                concrete_roots.push(root);
            }
        }

        let target = LockedPackages {
            lock: self,
            install_path,
            project_name,
            roots: concrete_roots,
        };
        target.validate_workspace_resolution(extras, groups, Some(marker_env))?;
        target.to_resolution_from_packages(
            &target.roots,
            None,
            false,
            DependencySelectionContext::None,
            marker_env,
            tags,
            extras,
            groups,
            build_options,
            install_options,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cmp::Ordering;
    use std::str::FromStr;
    use std::sync::LazyLock;

    use petgraph::visit::EdgeRef;
    use uv_configuration::{DependencyGroups, ExtrasSpecification};
    use uv_distribution_types::Name;
    use uv_normalize::{DefaultExtras, DefaultGroups};
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};
    use uv_platform_tags::{Arch, Os, Platform, TagsOptions};
    use uv_warnings::anstream;

    use super::*;

    static TAGS: LazyLock<Tags> = LazyLock::new(|| {
        Tags::from_env(
            Platform::new(
                Os::Macos {
                    major: 14,
                    minor: 0,
                },
                Arch::Aarch64,
            ),
            (3, 11),
            "cpython",
            (3, 11),
            TagsOptions::default(),
        )
        .expect("valid tags")
    });

    static DARWIN_MARKERS: LazyLock<ResolverMarkerEnvironment> =
        LazyLock::new(|| ResolverMarkerEnvironment::from(marker_environment("darwin", "Darwin")));

    static LINUX_MARKERS: LazyLock<ResolverMarkerEnvironment> =
        LazyLock::new(|| ResolverMarkerEnvironment::from(marker_environment("linux", "Linux")));

    fn marker_environment(
        sys_platform: &'static str,
        platform_system: &'static str,
    ) -> MarkerEnvironment {
        MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: "3.11.5",
            os_name: "posix",
            platform_machine: "arm64",
            platform_python_implementation: "CPython",
            platform_release: "23.0.0",
            platform_system,
            platform_version: "test",
            python_full_version: "3.11.5",
            python_version: "3.11",
            sys_platform,
        })
        .expect("valid marker environment")
    }

    fn lock() -> Lock {
        toml::from_str(
            r#"
version = 1
revision = 3
requires-python = ">=3.11"
resolution-markers = [
    "sys_platform == 'darwin'",
    "sys_platform != 'darwin'",
]

[manifest]
requirements = [{ name = "unrelated" }]

[[package]]
name = "dev-dependency"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/dev_dependency-1.0.0.tar.gz", hash = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }

[[package]]
name = "forked"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
resolution-markers = ["sys_platform == 'darwin'"]
sdist = { url = "https://example.com/forked-1.0.0.tar.gz", hash = "sha256:2222222222222222222222222222222222222222222222222222222222222222" }

[[package]]
name = "forked"
version = "2.0.0"
source = { registry = "https://example.com/simple" }
resolution-markers = ["sys_platform != 'darwin'"]
sdist = { url = "https://example.com/forked-2.0.0.tar.gz", hash = "sha256:3333333333333333333333333333333333333333333333333333333333333333" }

[[package]]
name = "optional-dependency"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/optional_dependency-1.0.0.tar.gz", hash = "sha256:4444444444444444444444444444444444444444444444444444444444444444" }

[[package]]
name = "root-a"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
dependencies = [
    { name = "forked", version = "1.0.0", source = { registry = "https://example.com/simple" }, marker = "sys_platform == 'darwin'" },
    { name = "forked", version = "2.0.0", source = { registry = "https://example.com/simple" }, marker = "sys_platform != 'darwin'" },
    { name = "shared" },
]
sdist = { url = "https://example.com/root_a-1.0.0.tar.gz", hash = "sha256:5555555555555555555555555555555555555555555555555555555555555555" }

[package.optional-dependencies]
feature = [{ name = "optional-dependency" }]

[package.dependency-groups]
dev = [{ name = "dev-dependency" }]

[package.metadata]
provides-extras = ["feature"]

[[package]]
name = "root-b"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
dependencies = [{ name = "shared" }]
sdist = { url = "https://example.com/root_b-1.0.0.tar.gz", hash = "sha256:6666666666666666666666666666666666666666666666666666666666666666" }

[[package]]
name = "shared"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/shared-1.0.0.tar.gz", hash = "sha256:7777777777777777777777777777777777777777777777777777777777777777" }

[[package]]
name = "unrelated"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/unrelated-1.0.0.tar.gz", hash = "sha256:8888888888888888888888888888888888888888888888888888888888888888" }
"#,
        )
        .expect("valid lock")
    }

    fn conflict_lock() -> Lock {
        toml::from_str(
            r#"
version = 1
revision = 3
requires-python = ">=3.11"
conflicts = [
    [
        { package = "tool", extra = "cpu" },
        { package = "tool", extra = "gpu" },
    ],
    [
        { package = "project", extra = "foo" },
        { package = "project", extra = "bar" },
    ],
]

[[package]]
name = "contextual-dependency"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/contextual_dependency-1.0.0.tar.gz", hash = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }

[[package]]
name = "contextual-tool"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
dependencies = [
    { name = "contextual-dependency", marker = "sys_platform == 'linux' or (sys_platform == 'darwin' and extra == 'extra-7-project-foo')" },
]
sdist = { url = "https://example.com/contextual_tool-1.0.0.tar.gz", hash = "sha256:2222222222222222222222222222222222222222222222222222222222222222" }

[[package]]
name = "cpu-backend"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/cpu_backend-1.0.0.tar.gz", hash = "sha256:3333333333333333333333333333333333333333333333333333333333333333" }

[[package]]
name = "gpu-backend"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/gpu_backend-1.0.0.tar.gz", hash = "sha256:4444444444444444444444444444444444444444444444444444444444444444" }

[[package]]
name = "project"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/project-1.0.0.tar.gz", hash = "sha256:5555555555555555555555555555555555555555555555555555555555555555" }

[package.optional-dependencies]
foo = []
bar = []

[package.metadata]
provides-extras = ["foo", "bar"]

[[package]]
name = "runtime"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
dependencies = [
    { name = "cpu-backend", marker = "extra == 'extra-4-tool-cpu'" },
    { name = "gpu-backend", marker = "extra == 'extra-4-tool-gpu'" },
]
sdist = { url = "https://example.com/runtime-1.0.0.tar.gz", hash = "sha256:6666666666666666666666666666666666666666666666666666666666666666" }

[[package]]
name = "tool"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
dependencies = [{ name = "runtime" }]
sdist = { url = "https://example.com/tool-1.0.0.tar.gz", hash = "sha256:7777777777777777777777777777777777777777777777777777777777777777" }

[package.optional-dependencies]
cpu = []
gpu = []

[package.metadata]
provides-extras = ["cpu", "gpu"]
"#,
        )
        .expect("valid lock")
    }

    fn dependency_selection_lock() -> Lock {
        toml::from_str(
            r#"
version = 1
revision = 3
requires-python = ">=3.11"
conflicts = [[
    { package = "project", group = "dev" },
    { package = "project", group = "other" },
]]

[[package]]
name = "contextual-dev-dependency"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/contextual_dev_dependency-1.0.0.tar.gz", hash = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }

[[package]]
name = "contextual-other-dependency"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/contextual_other_dependency-1.0.0.tar.gz", hash = "sha256:2222222222222222222222222222222222222222222222222222222222222222" }

[[package]]
name = "optional-dependency"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/optional_dependency-1.0.0.tar.gz", hash = "sha256:3333333333333333333333333333333333333333333333333333333333333333" }

[[package]]
name = "project"
version = "1.0.0"
source = { virtual = "." }

[package.dependency-groups]
dev = [{ name = "tool", extra = ["cli"] }]
other = [{ name = "tool" }]

[[package]]
name = "tool"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
dependencies = [
    { name = "contextual-dev-dependency", marker = "extra == 'group-7-project-dev'" },
    { name = "contextual-other-dependency", marker = "extra == 'group-7-project-other'" },
]
sdist = { url = "https://example.com/tool-1.0.0.tar.gz", hash = "sha256:4444444444444444444444444444444444444444444444444444444444444444" }

[package.optional-dependencies]
cli = [{ name = "optional-dependency" }]

[package.metadata]
provides-extras = ["cli"]
"#,
        )
        .expect("valid lock")
    }

    fn package<'lock>(lock: &'lock Lock, name: &str, version: &str) -> &'lock Package {
        lock.packages()
            .iter()
            .find(|package| {
                package.name().as_ref() == name
                    && package
                        .version()
                        .is_some_and(|package_version| package_version.to_string() == version)
            })
            .expect("locked package")
    }

    fn materialize(
        lock: &Lock,
        roots: &[&Package],
        marker_env: &ResolverMarkerEnvironment,
    ) -> Resolution {
        let extras = ExtrasSpecification::from_all_extras().with_defaults(DefaultExtras::default());
        let groups = DependencyGroups::from_all_groups().with_defaults(DefaultGroups::default());
        lock.to_resolution(
            Path::new("."),
            roots.iter().copied(),
            None,
            marker_env,
            &TAGS,
            &extras,
            &groups,
            &BuildOptions::default(),
            &InstallOptions::default(),
        )
        .expect("valid resolution")
    }

    fn materialize_with_extras(
        lock: &Lock,
        roots: &[&Package],
        marker_env: &ResolverMarkerEnvironment,
        extras: &ExtrasSpecification,
    ) -> Result<Resolution, LockError> {
        let extras = extras.with_defaults(DefaultExtras::default());
        let groups = DependencyGroupsWithDefaults::none();
        lock.to_resolution(
            Path::new("."),
            roots.iter().copied(),
            None,
            marker_env,
            &TAGS,
            &extras,
            &groups,
            &BuildOptions::default(),
            &InstallOptions::default(),
        )
    }

    fn materialize_selected_dependency(lock: &Lock, group: &str) -> Resolution {
        let project_name = PackageName::from_str("project").expect("valid package name");
        let dependency_name = PackageName::from_str("tool").expect("valid package name");
        let group = GroupName::from_str(group).expect("valid group name");
        let selection = lock
            .dependency_selection(
                Some(&project_name),
                &dependency_name,
                DARWIN_MARKERS.markers(),
            )
            .expect("unique dependency selection");
        let dependency = selection.group(&group).expect("group dependency");

        lock.to_resolution_from_dependency(
            Path::new("."),
            dependency,
            Some(&project_name),
            &DARWIN_MARKERS,
            &TAGS,
            &BuildOptions::default(),
            &InstallOptions::default(),
        )
        .expect("valid resolution")
    }

    #[test]
    fn unrelated_packages_do_not_change_dependency_identity() {
        let original = lock();
        let input = format!(
            "{}\n{}",
            original.to_toml().expect("valid lock TOML"),
            r#"
[[package]]
name = "aaa-unrelated"
version = "1.0.0"
source = { registry = "https://example.com/simple" }
"#
        );
        let extended = Lock::from_toml(&input).expect("valid extended lock");
        let original_root = package(&original, "root-a", "1.0.0");
        let extended_root = package(&extended, "root-a", "1.0.0");

        assert_eq!(original_root, extended_root);
        assert_eq!(
            original_root.dependencies.cmp(&extended_root.dependencies),
            Ordering::Equal,
        );
        assert_eq!(
            graph_snapshot(&materialize(&original, &[original_root], &DARWIN_MARKERS)),
            graph_snapshot(&materialize(&extended, &[extended_root], &DARWIN_MARKERS)),
        );
    }

    struct OverridingInstallable<'lock> {
        lock: &'lock Lock,
        root_name: &'lock PackageName,
        package_to_node_calls: Cell<usize>,
    }

    impl<'lock> Installable<'lock> for OverridingInstallable<'lock> {
        fn install_path(&self) -> &'lock Path {
            Path::new(".")
        }

        fn lock(&self) -> &'lock Lock {
            self.lock
        }

        fn roots(&self) -> impl Iterator<Item = &PackageName> {
            std::iter::once(self.root_name)
        }

        fn project_name(&self) -> Option<&PackageName> {
            None
        }

        fn package_to_node(
            &self,
            _package: &Package,
            _tags: &Tags,
            _build_options: &BuildOptions,
            _install_options: &InstallOptions,
            _marker_env: &ResolverMarkerEnvironment,
        ) -> Result<Node, LockError> {
            self.package_to_node_calls
                .set(self.package_to_node_calls.get() + 1);
            Ok(Node::Root)
        }
    }

    fn workspace_lock_with_unresolved_extra() -> Lock {
        Lock::from_toml(
            r#"
version = 1
revision = 3
requires-python = ">=3.11"

[manifest]
members = ["app"]
workspace-members = ["app", "shared"]

[[package]]
name = "app"
version = "1.0.0"
source = { virtual = "." }
dependencies = [{ name = "shared" }]

[[package]]
name = "shared"
version = "1.0.0"
source = { virtual = "shared" }

[package.metadata]
requires-python = ">=3.11"
provides-extras = ["feature"]
"#,
        )
        .expect("valid explicit-root lock")
    }

    #[test]
    fn concrete_workspace_conversion_rejects_unresolved_extra() {
        let lock = workspace_lock_with_unresolved_extra();
        let shared = package(&lock, "shared", "1.0.0");
        let install_path = std::env::current_dir().expect("absolute installation path");
        let extras = ExtrasSpecification::from_all_extras().with_defaults(DefaultExtras::default());
        let error = lock
            .to_resolution(
                &install_path,
                [shared],
                None,
                &LINUX_MARKERS,
                &TAGS,
                &extras,
                &DependencyGroupsWithDefaults::none(),
                &BuildOptions::default(),
                &InstallOptions::default(),
            )
            .expect_err("the requested non-root extra was not resolved");
        insta::assert_snapshot!(error, @"Extra `feature` for workspace member `shared` was not resolved for this selection");
    }

    #[test]
    fn installable_workspace_conversion_rejects_unresolved_extra() {
        let lock = workspace_lock_with_unresolved_extra();
        let target = OverridingInstallable {
            lock: &lock,
            root_name: package(&lock, "shared", "1.0.0").name(),
            package_to_node_calls: Cell::new(0),
        };
        let extras = ExtrasSpecification::from_all_extras().with_defaults(DefaultExtras::default());
        let error = target
            .to_resolution(
                &LINUX_MARKERS,
                &TAGS,
                &extras,
                &DependencyGroupsWithDefaults::none(),
                &BuildOptions::default(),
                &InstallOptions::default(),
            )
            .expect_err("the requested non-root extra was not resolved");
        insta::assert_snapshot!(error, @"Extra `feature` for workspace member `shared` was not resolved for this selection");
        assert_eq!(target.package_to_node_calls.get(), 0);
    }

    #[test]
    fn export_workspace_conversion_rejects_unresolved_extra() {
        let lock = workspace_lock_with_unresolved_extra();
        let target = OverridingInstallable {
            lock: &lock,
            root_name: package(&lock, "shared", "1.0.0").name(),
            package_to_node_calls: Cell::new(0),
        };
        let extras = ExtrasSpecification::from_all_extras().with_defaults(DefaultExtras::default());
        let groups = DependencyGroupsWithDefaults::none();
        let install_options = InstallOptions::default();
        let error = crate::RequirementsTxtExport::from_lock(
            &target,
            &[],
            &extras,
            &groups,
            false,
            None,
            false,
            &install_options,
        )
        .expect_err("the requested non-root extra was not resolved");
        insta::assert_snapshot!(error, @"Extra `feature` for workspace member `shared` was not resolved for this selection");
        let error = crate::PylockToml::from_lock(
            &target,
            Path::new("."),
            lock.requires_python().clone(),
            &[],
            &extras,
            &groups,
            false,
            None,
            &install_options,
        )
        .expect_err("the requested non-root extra was not resolved");
        insta::assert_snapshot!(error, @"Extra `feature` for workspace member `shared` was not resolved for this selection");
    }

    fn graph_snapshot(resolution: &Resolution) -> (Vec<String>, Vec<String>) {
        let graph = resolution.graph();
        let labels = graph
            .node_weights()
            .map(|node| match node {
                Node::Root => "root".to_string(),
                Node::Dist {
                    dist,
                    hashes,
                    install,
                } => format!(
                    "{}=={} (install: {install}, hashes: {})",
                    dist.name(),
                    dist.version()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "<dynamic>".to_string()),
                    hashes.iter().map(ToString::to_string).join(", ")
                ),
            })
            .collect::<Vec<_>>();
        let mut nodes = labels.clone();
        nodes.sort_unstable();
        let mut edges = graph
            .edge_references()
            .map(|edge| {
                format!(
                    "{} --{:?}--> {}",
                    labels[edge.source().index()],
                    edge.weight(),
                    labels[edge.target().index()]
                )
            })
            .collect::<Vec<_>>();
        edges.sort_unstable();
        (nodes, edges)
    }

    #[test]
    fn materializes_multiple_concrete_roots_with_shared_dependencies() {
        let lock = lock();
        let resolution = materialize(
            &lock,
            &[
                package(&lock, "root-a", "1.0.0"),
                package(&lock, "root-b", "1.0.0"),
            ],
            &DARWIN_MARKERS,
        );

        insta::with_settings!({
            filters => [(r"sha256:[0-9a-f]{64}", "sha256:[HASH]")],
        }, {
            insta::assert_debug_snapshot!(graph_snapshot(&resolution), @r#"
        (
            [
                "dev-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "forked==1.0.0 (install: true, hashes: sha256:[HASH])",
                "optional-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-b==1.0.0 (install: true, hashes: sha256:[HASH])",
                "shared==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
            [
                "root --Prod--> root-a==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root --Prod--> root-b==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Dev(GroupName(\"dev\"))--> dev-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Optional(ExtraName(\"feature\"))--> optional-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> forked==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> shared==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-b==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> shared==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
        )
            "#);
        });
    }

    #[test]
    fn materializes_group_root_referenced_by_production_dependency() {
        let lock = lock();
        let project = package(&lock, "root-a", "1.0.0");
        let group_root = package(&lock, "shared", "1.0.0");
        let extras = ExtrasSpecification::default().with_defaults(DefaultExtras::default());
        let groups = DependencyGroupsWithDefaults::none();

        let resolution = LockedPackages {
            lock: &lock,
            install_path: Path::new("."),
            project_name: Some(project.name()),
            roots: vec![project],
        }
        .to_resolution_from_packages(
            &[project],
            Some(group_root),
            false,
            DependencySelectionContext::None,
            &DARWIN_MARKERS,
            &TAGS,
            &extras,
            &groups,
            &BuildOptions::default(),
            &InstallOptions::default(),
        )
        .expect("valid resolution");

        assert!(
            resolution
                .distributions()
                .any(|distribution| distribution.name() == group_root.name())
        );
    }

    #[test]
    fn materializes_the_selected_universal_lock_fork() {
        let lock = lock();
        let root = package(&lock, "root-a", "1.0.0");
        let darwin = materialize(&lock, &[root], &DARWIN_MARKERS);
        let linux = materialize(&lock, &[root], &LINUX_MARKERS);
        let concrete_fork =
            materialize(&lock, &[package(&lock, "forked", "1.0.0")], &DARWIN_MARKERS);

        insta::with_settings!({
            filters => [(r"sha256:[0-9a-f]{64}", "sha256:[HASH]")],
        }, {
            insta::assert_debug_snapshot!(graph_snapshot(&darwin), @r#"
        (
            [
                "dev-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "forked==1.0.0 (install: true, hashes: sha256:[HASH])",
                "optional-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH])",
                "shared==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
            [
                "root --Prod--> root-a==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Dev(GroupName(\"dev\"))--> dev-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Optional(ExtraName(\"feature\"))--> optional-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> forked==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> shared==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
        )
        "#);
            insta::assert_debug_snapshot!(graph_snapshot(&linux), @r#"
        (
            [
                "dev-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "forked==2.0.0 (install: true, hashes: sha256:[HASH])",
                "optional-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH])",
                "shared==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
            [
                "root --Prod--> root-a==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Dev(GroupName(\"dev\"))--> dev-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Optional(ExtraName(\"feature\"))--> optional-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> forked==2.0.0 (install: true, hashes: sha256:[HASH])",
                "root-a==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> shared==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
        )
        "#);
            insta::assert_debug_snapshot!(graph_snapshot(&concrete_fork), @r#"
        (
            [
                "forked==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root",
            ],
            [
                "root --Prod--> forked==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
        )
            "#);
        });
    }

    #[test]
    fn materializes_conflicting_extras_within_the_synthetic_root() {
        let lock = conflict_lock();
        let extras =
            ExtrasSpecification::from_extra(vec!["cpu".parse().expect("valid extra name")]);
        let resolution = materialize_with_extras(
            &lock,
            &[package(&lock, "tool", "1.0.0")],
            &DARWIN_MARKERS,
            &extras,
        )
        .expect("conflict markers are resolved within the subgraph");

        insta::with_settings!({
            filters => [(r"sha256:[0-9a-f]{64}", "sha256:[HASH]")],
        }, {
            insta::assert_debug_snapshot!(graph_snapshot(&resolution), @r#"
        (
            [
                "cpu-backend==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root",
                "runtime==1.0.0 (install: true, hashes: sha256:[HASH])",
                "tool==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
            [
                "root --Prod--> tool==1.0.0 (install: true, hashes: sha256:[HASH])",
                "runtime==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> cpu-backend==1.0.0 (install: true, hashes: sha256:[HASH])",
                "tool==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> runtime==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
        )
        "#);
        });
    }

    #[test]
    fn materializes_selected_dependency_extras() {
        let resolution = materialize_selected_dependency(&dependency_selection_lock(), "dev");

        insta::with_settings!({
            filters => [(r"sha256:[0-9a-f]{64}", "sha256:[HASH]")],
        }, {
            insta::assert_debug_snapshot!(graph_snapshot(&resolution), @r#"
        (
            [
                "contextual-dev-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "optional-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root",
                "tool==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
            [
                "root --Prod--> tool==1.0.0 (install: true, hashes: sha256:[HASH])",
                "tool==1.0.0 (install: true, hashes: sha256:[HASH]) --Optional(ExtraName(\"cli\"))--> optional-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "tool==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> contextual-dev-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
        )
        "#);
        });
    }

    #[test]
    fn materializes_selected_dependency_project_conflict_context() {
        let resolution = materialize_selected_dependency(&dependency_selection_lock(), "other");

        insta::with_settings!({
            filters => [(r"sha256:[0-9a-f]{64}", "sha256:[HASH]")],
        }, {
            insta::assert_debug_snapshot!(graph_snapshot(&resolution), @r#"
        (
            [
                "contextual-other-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root",
                "tool==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
            [
                "root --Prod--> tool==1.0.0 (install: true, hashes: sha256:[HASH])",
                "tool==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> contextual-other-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
        )
        "#);
        });
    }

    #[test]
    fn rejects_conflicts_outside_the_synthetic_root() {
        let lock = conflict_lock();
        let root = package(&lock, "contextual-tool", "1.0.0");
        let extras = ExtrasSpecification::default();

        let error = materialize_with_extras(&lock, &[root], &DARWIN_MARKERS, &extras)
            .expect_err("Darwin dependency depends on the project extra");
        let error = error.to_string();
        let error = anstream::adapter::strip_str(&error);
        insta::assert_snapshot!(error, @"Cannot materialize dependency `contextual-dependency==1.0.0 @ registry+https://example.com/simple` of `contextual-tool==1.0.0 @ registry+https://example.com/simple` because its conflict marker depends on a package outside the selected subgraph");

        let linux = materialize_with_extras(&lock, &[root], &LINUX_MARKERS, &extras)
            .expect("the dependency is unconditional on Linux");
        insta::with_settings!({
            filters => [(r"sha256:[0-9a-f]{64}", "sha256:[HASH]")],
        }, {
            insta::assert_debug_snapshot!(graph_snapshot(&linux), @r#"
        (
            [
                "contextual-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "contextual-tool==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root",
            ],
            [
                "contextual-tool==1.0.0 (install: true, hashes: sha256:[HASH]) --Prod--> contextual-dependency==1.0.0 (install: true, hashes: sha256:[HASH])",
                "root --Prod--> contextual-tool==1.0.0 (install: true, hashes: sha256:[HASH])",
            ],
        )
        "#);
        });
    }

    #[test]
    fn installable_to_resolution_preserves_node_overrides() {
        let mut lock = lock();
        lock.manifest.requirements.clear();
        let target = OverridingInstallable {
            root_name: package(&lock, "root-a", "1.0.0").name(),
            lock: &lock,
            package_to_node_calls: Cell::new(0),
        };
        let extras = ExtrasSpecification::from_all_extras().with_defaults(DefaultExtras::default());
        let groups = DependencyGroups::from_all_groups().with_defaults(DefaultGroups::default());

        target
            .to_resolution(
                &DARWIN_MARKERS,
                &TAGS,
                &extras,
                &groups,
                &BuildOptions::default(),
                &InstallOptions::default(),
            )
            .expect("valid resolution");

        assert!(target.package_to_node_calls.get() > 0);
    }
}
