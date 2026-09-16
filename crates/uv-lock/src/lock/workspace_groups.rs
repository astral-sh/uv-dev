use std::collections::{BTreeMap, BTreeSet};

use uv_distribution_types::{RequiresPython, SimplifiedMarkerTree};
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep508::MarkerTree;
use uv_resolver_types::{ConflictMarker, UniversalMarker};
use uv_workspace::{ResolvedWorkspaceGroup, WorkspaceGroup};

use super::{
    Dependency, Lock, LockError, Package, PackageId, ResolverManifest, VERSION,
    WORKSPACE_GROUPS_VERSION,
};

/// The definition and effective Python domain of a locked workspace group.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LockedWorkspaceGroup {
    #[serde(flatten)]
    pub definition: WorkspaceGroup,
    pub effective_requires_python: RequiresPython,
    #[serde(default)]
    pub environment: Option<MarkerTree>,
}

impl LockedWorkspaceGroup {
    pub fn effective_environment(&self) -> MarkerTree {
        self.environment
            .unwrap_or_else(|| self.effective_requires_python.to_exact_marker_tree())
    }
}

impl From<ResolvedWorkspaceGroup> for LockedWorkspaceGroup {
    fn from(group: ResolvedWorkspaceGroup) -> Self {
        Self {
            definition: group.definition,
            effective_requires_python: group.requires_python,
            environment: Some(group.environments),
        }
    }
}

impl Lock {
    /// Return the workspace contexts recorded in this lockfile.
    pub fn workspace_groups(&self) -> &[LockedWorkspaceGroup] {
        &self.workspace_groups
    }

    /// Combine successful workspace-group forks into one package graph.
    pub fn from_workspace_groups(
        groups: Vec<ResolvedWorkspaceGroup>,
        resolutions: Vec<(Vec<GroupName>, Self)>,
    ) -> Result<Option<Self>, LockError> {
        let Some(requires_python) =
            RequiresPython::union(groups.iter().map(|group| &group.requires_python))
        else {
            return Ok(None);
        };
        let mut resolutions = resolutions.into_iter();
        let Some((first_names, first)) = resolutions.next() else {
            return Ok(None);
        };
        let mut manifest = first.manifest.clone();
        manifest.members = groups
            .iter()
            .flat_map(|group| group.definition.members.iter().cloned())
            .collect();
        let options = first.options.clone();
        let conflicts = first.conflicts.clone();
        let mut supported_environments = MarkerTree::FALSE;
        let required_environments = first.required_environments.clone();
        let revision = first.revision;
        let mut packages = BTreeMap::<PackageId, Package>::new();
        let mut fork_markers = BTreeSet::new();

        for (names, lock) in std::iter::once((first_names, first)).chain(resolutions) {
            if lock.supported_environments.is_empty() {
                supported_environments = MarkerTree::TRUE;
            } else {
                for environment in &lock.supported_environments {
                    supported_environments = supported_environments.or(*environment);
                }
            }
            let mut scope = UniversalMarker::from_combined(MarkerTree::FALSE);
            for group in &groups {
                if names.contains(&group.definition.name) {
                    let mut marker = UniversalMarker::workspace_group(&group.definition.name);
                    marker.and(UniversalMarker::from_combined(group.environments));
                    scope.or(marker);
                }
            }
            let markers = if lock.fork_markers.is_empty() {
                vec![UniversalMarker::new(
                    lock.requires_python.to_marker_tree(),
                    ConflictMarker::TRUE,
                )]
            } else {
                lock.fork_markers.clone()
            };
            for mut marker in markers {
                marker.and(scope);
                if !marker.is_false() {
                    fork_markers.insert(marker);
                }
            }
            for mut package in lock.packages {
                if package.fork_markers.is_empty() {
                    package.fork_markers.push(UniversalMarker::new(
                        lock.requires_python.to_marker_tree(),
                        ConflictMarker::TRUE,
                    ));
                }
                for marker in &mut package.fork_markers {
                    marker.and(scope);
                }
                scope_dependencies(&mut package.dependencies, scope, &requires_python);
                for dependencies in package
                    .optional_dependencies
                    .values_mut()
                    .chain(package.dependency_groups.values_mut())
                {
                    scope_dependencies(dependencies, scope, &requires_python);
                }
                merge_package(&mut packages, package, &requires_python);
            }
        }
        let supported_environments = if requires_python
            .simplify_markers(supported_environments)
            .is_true()
        {
            vec![]
        } else {
            vec![supported_environments]
        };
        let fork_markers =
            canonical_workspace_markers(&fork_markers.into_iter().collect::<Vec<_>>(), &groups);
        normalize_workspace_graph(&mut packages, &groups, &manifest, &requires_python);
        let mut lock = Self::new(
            WORKSPACE_GROUPS_VERSION,
            revision,
            packages.into_values().collect(),
            requires_python,
            options,
            manifest,
            conflicts,
            supported_environments,
            required_environments,
            fork_markers,
        )?;
        lock.workspace_groups = groups.into_iter().map(LockedWorkspaceGroup::from).collect();
        Ok(Some(lock))
    }

    /// Select a workspace context without resolving or consulting package metadata.
    pub fn select_workspace_group(&self, name: &GroupName) -> Result<Option<Self>, LockError> {
        let Some(group) = self
            .workspace_groups
            .iter()
            .find(|group| group.definition.name == *name)
        else {
            return Ok(None);
        };
        let requires_python = group.effective_requires_python.clone();
        let environment = group.effective_environment();
        let select_markers = |markers: &[UniversalMarker]| {
            markers
                .iter()
                .copied()
                .map(|marker| {
                    let mut marker = marker.select_workspace_group(name);
                    marker.and(UniversalMarker::from_combined(environment));
                    marker
                })
                .filter(|marker| !marker.is_false())
                .collect::<Vec<_>>()
        };
        let mut packages = Vec::new();
        for package in &self.packages {
            let mut package = package.clone();
            package.fork_markers = select_markers(&package.fork_markers);
            if package.fork_markers.is_empty() {
                continue;
            }
            select_dependencies(
                &mut package.dependencies,
                name,
                environment,
                &requires_python,
            );
            for dependencies in package
                .optional_dependencies
                .values_mut()
                .chain(package.dependency_groups.values_mut())
            {
                select_dependencies(dependencies, name, environment, &requires_python);
            }
            if package.fork_markers.iter().any(|marker| marker.is_true()) {
                package.fork_markers.clear();
            }
            packages.push(package);
        }
        let mut manifest = self.manifest.clone();
        manifest.members.clone_from(&group.definition.members);
        retain_reachable(&mut packages, &group.definition.members, &manifest);
        let fork_markers = select_markers(&self.fork_markers);
        Self::new(
            VERSION,
            self.revision,
            packages,
            requires_python,
            self.options.clone(),
            manifest,
            self.conflicts.clone(),
            if self.supported_environments.is_empty() {
                vec![environment]
            } else {
                self.supported_environments
                    .iter()
                    .map(|marker| marker.and(environment))
                    .filter(|marker| !marker.is_false())
                    .collect()
            },
            self.required_environments.clone(),
            fork_markers,
        )
        .map(Some)
    }

    /// Retain an ordinary project target within an already selected workspace context.
    pub fn select_workspace_members(
        &self,
        members: &BTreeSet<PackageName>,
    ) -> Result<Option<Self>, LockError> {
        if !members
            .iter()
            .all(|name| self.packages.iter().any(|package| &package.id.name == name))
        {
            return Ok(None);
        }
        let mut selected = self.clone();
        retain_reachable(&mut selected.packages, members, &selected.manifest);
        selected.manifest.members.clone_from(members);
        Self::new(
            selected.version,
            selected.revision,
            selected.packages,
            selected.requires_python,
            selected.options,
            selected.manifest,
            selected.conflicts,
            selected.supported_environments,
            selected.required_environments,
            selected.fork_markers,
        )
        .map(Some)
    }

    /// Merge ordinary-target views when their package choices agree in overlapping environments.
    pub fn merge_workspace_resolutions(resolutions: Vec<Self>) -> Result<Option<Self>, LockError> {
        Self::merge_workspace_contexts(resolutions, true)
    }

    /// Reconstruct compatible prior contexts as preferences for another shared resolution.
    pub fn merge_workspace_group_preferences(
        resolutions: Vec<Self>,
    ) -> Result<Option<Self>, LockError> {
        Self::merge_workspace_contexts(resolutions, false)
    }

    fn merge_workspace_contexts(
        resolutions: Vec<Self>,
        require_all_roots: bool,
    ) -> Result<Option<Self>, LockError> {
        let mut member_environments = BTreeMap::<PackageName, MarkerTree>::new();
        for lock in &resolutions {
            let environment = super::implicit_constraints_marker(
                lock.requires_python.to_exact_marker_tree(),
                &lock.supported_environments,
            );
            for member in &lock.manifest.members {
                let active = lock
                    .packages_for_name(member)
                    .iter()
                    .fold(MarkerTree::FALSE, |active, package| {
                        active.or(package_environment(package, &lock.requires_python))
                    })
                    .and(environment);
                let supported = member_environments
                    .entry(member.clone())
                    .or_insert(MarkerTree::FALSE);
                *supported = supported.or(active);
            }
        }
        // Every ordinary target is required, even when its compatible contexts differ.
        let environment = if require_all_roots {
            member_environments
                .values()
                .fold(MarkerTree::TRUE, |environment, member| {
                    environment.and(*member)
                })
        } else {
            member_environments
                .values()
                .fold(MarkerTree::FALSE, |environment, member| {
                    environment.or(*member)
                })
        };
        let Some(requires_python) = RequiresPython::from_marker_tree(environment) else {
            return Ok(None);
        };
        let mut resolutions = resolutions.into_iter();
        let Some(first) = resolutions.next() else {
            return Ok(None);
        };
        let revision = first.revision;
        let mut manifest = first.manifest.clone();
        manifest.members = member_environments.into_keys().collect();
        let options = first.options.clone();
        let conflicts = first.conflicts.clone();
        let required_environments = first.required_environments.clone();
        let mut packages = BTreeMap::<PackageId, Package>::new();
        let mut supported_environments = MarkerTree::FALSE;
        let mut fork_markers = BTreeSet::new();
        for lock in std::iter::once(first).chain(resolutions) {
            let environment = environment.and(super::implicit_constraints_marker(
                lock.requires_python.to_exact_marker_tree(),
                &lock.supported_environments,
            ));
            if environment.is_false() {
                continue;
            }
            supported_environments = supported_environments.or(environment);
            let mut previous = BTreeMap::<&PackageName, Vec<(&PackageId, MarkerTree)>>::new();
            for package in packages.values() {
                previous
                    .entry(&package.id.name)
                    .or_default()
                    .push((&package.id, package_environment(package, &requires_python)));
            }
            for package in &lock.packages {
                let marker = package_environment(package, &lock.requires_python).and(environment);
                if previous.get(&package.id.name).is_some_and(|choices| {
                    choices
                        .iter()
                        .any(|(id, previous)| **id != package.id && !previous.is_disjoint(marker))
                }) {
                    return Ok(None);
                }
            }
            let scope = UniversalMarker::from_combined(environment);
            if lock.fork_markers.is_empty() {
                fork_markers.insert(scope);
            } else {
                for mut marker in lock.fork_markers {
                    marker.and(scope);
                    if !marker.is_false() {
                        fork_markers.insert(marker);
                    }
                }
            }
            for mut package in lock.packages {
                if package.fork_markers.is_empty() {
                    package.fork_markers.push(scope);
                }
                for marker in &mut package.fork_markers {
                    marker.and(scope);
                }
                package.fork_markers.retain(|marker| !marker.is_false());
                if package.fork_markers.is_empty() {
                    continue;
                }
                scope_dependencies(&mut package.dependencies, scope, &requires_python);
                for dependencies in package
                    .optional_dependencies
                    .values_mut()
                    .chain(package.dependency_groups.values_mut())
                {
                    scope_dependencies(dependencies, scope, &requires_python);
                }
                merge_package(&mut packages, package, &requires_python);
            }
        }
        Self::new(
            VERSION,
            revision,
            packages.into_values().collect(),
            requires_python,
            options,
            manifest,
            conflicts,
            vec![supported_environments],
            required_environments,
            remove_redundant_markers(&fork_markers),
        )
        .map(Some)
    }
}

/// Canonicalize each selector independently so projecting and recombining contexts is stable.
fn canonical_workspace_markers(
    markers: &[UniversalMarker],
    groups: &[ResolvedWorkspaceGroup],
) -> Vec<UniversalMarker> {
    let mut canonical = BTreeSet::new();
    for group in groups {
        let selected = markers
            .iter()
            .map(|marker| {
                let mut marker = marker.select_workspace_group(&group.definition.name);
                marker.and(UniversalMarker::from_combined(group.environments));
                marker
            })
            .filter(|marker| !marker.is_false())
            .collect::<BTreeSet<_>>();
        for mut marker in remove_redundant_markers(&selected) {
            marker.and(UniversalMarker::workspace_group(&group.definition.name));
            canonical.insert(marker);
        }
    }
    canonical.into_iter().collect()
}

fn remove_redundant_markers(markers: &BTreeSet<UniversalMarker>) -> Vec<UniversalMarker> {
    markers
        .iter()
        .copied()
        .filter(|marker| {
            !markers.iter().any(|other| {
                marker != other && marker.combined().implies(other.combined()).is_true()
            })
        })
        .collect()
}

fn package_environment(package: &Package, requires_python: &RequiresPython) -> MarkerTree {
    if package.fork_markers.is_empty() {
        requires_python.to_exact_marker_tree()
    } else {
        package
            .fork_markers
            .iter()
            .fold(MarkerTree::FALSE, |marker, fork| marker.or(fork.combined()))
    }
}

fn normalize_workspace_graph(
    packages: &mut BTreeMap<PackageId, Package>,
    groups: &[ResolvedWorkspaceGroup],
    manifest: &ResolverManifest,
    requires_python: &RequiresPython,
) {
    let contexts = groups
        .iter()
        .map(|group| {
            let mut pending = packages
                .values()
                .filter_map(|package| {
                    root_marker(package, &group.definition.members, manifest)
                        .map(|marker| (package.id.clone(), marker.and(group.environments)))
                })
                .collect::<Vec<_>>();
            let mut reached = BTreeMap::<PackageId, MarkerTree>::new();
            while let Some((id, active)) = pending.pop() {
                let Some(package) = packages.get(&id) else {
                    continue;
                };
                let available =
                    UniversalMarker::from_combined(package_environment(package, requires_python))
                        .select_workspace_group(&group.definition.name)
                        .combined();
                let active = active.and(available);
                let previous = reached.entry(id).or_insert(MarkerTree::FALSE);
                let active = previous.or(active);
                if active == *previous {
                    continue;
                }
                *previous = active;
                for dependency in package.all_dependencies() {
                    let marker = active.and(
                        dependency
                            .complexified_marker
                            .select_workspace_group(&group.definition.name)
                            .combined(),
                    );
                    if !marker.is_false() {
                        pending.push((dependency.package_id.clone(), marker));
                    }
                }
            }
            (group, reached)
        })
        .collect::<Vec<_>>();
    for package in packages.values_mut() {
        let scopes = contexts
            .iter()
            .filter_map(|(group, reached)| {
                reached
                    .get(&package.id)
                    .filter(|marker| !marker.is_false())
                    .map(|marker| {
                        let mut scope = UniversalMarker::workspace_group(&group.definition.name);
                        scope.and(UniversalMarker::from_combined(*marker));
                        (&group.definition.name, scope)
                    })
            })
            .collect::<Vec<_>>();
        package.fork_markers = scopes.iter().map(|(_, scope)| *scope).collect();
        for dependencies in std::iter::once(&mut package.dependencies)
            .chain(package.optional_dependencies.values_mut())
            .chain(package.dependency_groups.values_mut())
        {
            for dependency in dependencies.iter_mut() {
                let mut marker = UniversalMarker::FALSE;
                for (name, scope) in &scopes {
                    let mut selected = dependency.complexified_marker.select_workspace_group(name);
                    selected.and(*scope);
                    marker.or(selected);
                }
                dependency.complexified_marker = marker;
                dependency.simplified_marker =
                    SimplifiedMarkerTree::new(requires_python, marker.combined());
            }
            dependencies.retain(|dependency| !dependency.complexified_marker.is_false());
            merge_dependencies(dependencies, requires_python);
        }
    }
    packages.retain(|_, package| !package.fork_markers.is_empty());
}

fn root_marker(
    package: &Package,
    members: &BTreeSet<PackageName>,
    manifest: &ResolverManifest,
) -> Option<MarkerTree> {
    let mut marker = if members.contains(&package.id.name) {
        MarkerTree::TRUE
    } else {
        MarkerTree::FALSE
    };
    for requirement in manifest
        .requirements
        .iter()
        .chain(manifest.dependency_groups.values().flatten())
    {
        if requirement.name == package.id.name
            && requirement
                .source
                .version_specifiers()
                .zip(package.id.version.as_ref())
                .is_none_or(|(specifiers, version)| specifiers.contains(version))
        {
            marker = marker.or(requirement.marker);
        }
    }
    (!marker.is_false()).then_some(marker)
}

fn merge_package(
    packages: &mut BTreeMap<PackageId, Package>,
    package: Package,
    requires_python: &RequiresPython,
) {
    match packages.entry(package.id.clone()) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(package);
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            let target = entry.get_mut();
            target.fork_markers.extend(package.fork_markers);
            target.fork_markers.sort();
            target.fork_markers.dedup();
            for wheel in package.wheels {
                if !target.wheels.contains(&wheel) {
                    target.wheels.push(wheel);
                }
            }
            target.dependencies.extend(package.dependencies);
            merge_dependencies(&mut target.dependencies, requires_python);
            for (extra, dependencies) in package.optional_dependencies {
                let target = target.optional_dependencies.entry(extra).or_default();
                target.extend(dependencies);
                merge_dependencies(target, requires_python);
            }
            for (group, dependencies) in package.dependency_groups {
                let target = target.dependency_groups.entry(group).or_default();
                target.extend(dependencies);
                merge_dependencies(target, requires_python);
            }
        }
    }
}

fn retain_reachable(
    packages: &mut Vec<Package>,
    members: &BTreeSet<PackageName>,
    manifest: &ResolverManifest,
) {
    let mut pending = packages
        .iter()
        .filter(|package| root_marker(package, members, manifest).is_some())
        .map(|package| package.id.clone())
        .collect::<Vec<_>>();
    let by_id = packages
        .iter()
        .map(|package| (&package.id, package))
        .collect::<BTreeMap<_, _>>();
    let mut reachable = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if reachable.insert(id.clone())
            && let Some(package) = by_id.get(&id)
        {
            pending.extend(
                package
                    .all_dependencies()
                    .map(|dependency| dependency.package_id.clone()),
            );
        }
    }
    packages.retain(|package| reachable.contains(&package.id));
}

fn scope_dependencies(
    dependencies: &mut Vec<Dependency>,
    scope: UniversalMarker,
    requires_python: &RequiresPython,
) {
    for dependency in dependencies.iter_mut() {
        dependency.complexified_marker.and(scope);
        dependency.simplified_marker =
            SimplifiedMarkerTree::new(requires_python, dependency.complexified_marker.combined());
    }
    dependencies.retain(|dependency| !dependency.complexified_marker.is_false());
    merge_dependencies(dependencies, requires_python);
}

fn select_dependencies(
    dependencies: &mut Vec<Dependency>,
    name: &GroupName,
    environment: MarkerTree,
    requires_python: &RequiresPython,
) {
    for dependency in dependencies.iter_mut() {
        dependency.complexified_marker =
            dependency.complexified_marker.select_workspace_group(name);
        dependency
            .complexified_marker
            .and(UniversalMarker::from_combined(environment));
        dependency.simplified_marker =
            SimplifiedMarkerTree::new(requires_python, dependency.complexified_marker.combined());
    }
    dependencies.retain(|dependency| !dependency.complexified_marker.is_false());
    merge_dependencies(dependencies, requires_python);
}

fn merge_dependencies(dependencies: &mut Vec<Dependency>, requires_python: &RequiresPython) {
    let mut merged = BTreeMap::<(PackageId, BTreeSet<ExtraName>), UniversalMarker>::new();
    for dependency in dependencies.drain(..) {
        merged
            .entry((dependency.package_id, dependency.extra))
            .and_modify(|marker| marker.or(dependency.complexified_marker))
            .or_insert(dependency.complexified_marker);
    }
    dependencies.extend(merged.into_iter().map(|((package_id, extras), marker)| {
        Dependency::new(
            requires_python,
            package_id,
            extras,
            SimplifiedMarkerTree::new(requires_python, marker.combined()),
        )
    }));
}

#[cfg(test)]
mod tests {
    use super::Lock;

    #[test]
    fn workspace_group_lock_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let lock = Lock::from_toml(
            r#"
version = 2
revision = 3
requires-python = ">=3.12,<3.15"
resolution-markers = ["python_full_version < '3.13' and extra == 'workspace-main'", "extra == 'workspace-next'"]

[[workspace-group]]
name = "main"
members = ["app"]
effective-requires-python = "==3.12.*"
default = true

[[workspace-group]]
name = "next"
members = ["app"]
effective-requires-python = ">=3.12,<3.15"

[manifest]
members = ["app"]

[[package]]
name = "app"
version = "0.1.0"
source = { virtual = "." }
resolution-markers = ["python_full_version < '3.13' and extra == 'workspace-main'", "extra == 'workspace-next'"]
dependencies = [
  { name = "leaf", version = "1.0.0", source = { registry = "https://pypi.org/simple" }, marker = "extra == 'workspace-main'" },
  { name = "leaf", version = "2.0.0", source = { registry = "https://pypi.org/simple" }, marker = "extra == 'workspace-next'" },
]

[[package]]
name = "leaf"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }
resolution-markers = ["python_full_version < '3.13' and extra == 'workspace-main'"]

[[package]]
name = "leaf"
version = "2.0.0"
source = { registry = "https://pypi.org/simple" }
resolution-markers = ["extra == 'workspace-next'"]
"#,
        )?;
        let serialized = lock.to_toml()?;
        assert_eq!(serialized, Lock::from_toml(&serialized)?.to_toml()?);
        let selected = lock
            .select_workspace_group(&"next".parse()?)?
            .ok_or("missing group")?;
        let leaf = selected
            .find_by_name(&"leaf".parse()?)?
            .ok_or("missing leaf")?;
        assert_eq!(
            leaf.version().map(ToString::to_string).as_deref(),
            Some("2.0.0")
        );
        Ok(())
    }
}
