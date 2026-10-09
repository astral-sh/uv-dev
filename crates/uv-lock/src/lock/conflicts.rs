use std::collections::{BTreeMap, BTreeSet};

use uv_configuration::{DependencyGroupsWithDefaults, ExtrasSpecification};
use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictKind, ConflictSet, ResolverMarkerEnvironment};

use crate::lock::installable::InstallableRootKind;
use crate::lock::reachability::ConflictRequests;
use crate::lock::{DependencyContext, LockErrorKind};
use crate::{Installable, LockError, implicit_constraints_marker};

/// Return the conditions under which selected packages, extras, and groups are requested.
///
/// Requests use effective declarations before conflict guards can erase resolved extra labels.
pub fn activated_conflicts<'lock>(
    target: &impl Installable<'lock>,
    extras: &ExtrasSpecification,
    groups: &DependencyGroupsWithDefaults,
    marker_environment: Option<&ResolverMarkerEnvironment>,
) -> Result<BTreeMap<ConflictItem, MarkerTree>, LockError> {
    let lock = target.lock();
    let modifiers = lock.dependency_modifiers()?;
    let root_marker = implicit_constraints_marker(
        lock.requires_python.to_marker_tree(),
        lock.supported_environments(),
    );
    let mut requests = ConflictRequests::new(lock);
    let mut activated = BTreeMap::<ConflictItem, MarkerTree>::new();
    for (name, kind) in target
        .roots()
        .map(|name| (name, InstallableRootKind::Production))
        .chain(
            target
                .group_root(groups)
                .map(|name| (name, InstallableRootKind::DependencyGroups)),
        )
    {
        let package = lock
            .find_by_name(name)
            .map_err(|_| LockErrorKind::MultipleRootPackages { name: name.clone() })?
            .ok_or_else(|| LockErrorKind::MissingRootPackage { name: name.clone() })?;
        let index = lock.by_id[&package.id];
        if kind == InstallableRootKind::Production && groups.prod() {
            activated.insert(ConflictItem::from(name.clone()), root_marker);
            requests.push(requests.root, index, None, root_marker);
            let conflict_extras = lock
                .conflicts()
                .iter()
                .flat_map(ConflictSet::iter)
                .filter(|item| item.package() == name)
                .filter_map(|item| match item.kind() {
                    ConflictKind::Extra(extra) => Some(extra),
                    ConflictKind::Project | ConflictKind::Group(_) => None,
                });
            for extra in extras
                .extra_names(
                    package
                        .optional_dependencies
                        .keys()
                        .chain(package.metadata.provides_extra.iter())
                        .chain(conflict_extras),
                )
                .collect::<BTreeSet<_>>()
            {
                activated.insert(
                    ConflictItem::from((name.clone(), extra.clone())),
                    root_marker,
                );
                requests.push(requests.root, index, Some(extra.clone()), root_marker);
            }
        }
        for (group, dependencies) in &package.dependency_groups {
            if !target.includes_group(Some(name), group, groups) {
                continue;
            }
            activated.insert(
                ConflictItem::from((name.clone(), group.clone())),
                root_marker,
            );
            let requirements = package.dependency_requirements(
                DependencyContext::Group(group),
                &modifiers,
                target.install_path(),
                lock.requires_python(),
            )?;
            for dependency in dependencies {
                let (marker, extras) =
                    dependency.activation(requirements.as_deref(), target.install_path())?;
                requests.push(
                    requests.root,
                    dependency.index,
                    None,
                    root_marker.and(marker),
                );
                for (extra, marker) in extras {
                    requests.push(
                        requests.root,
                        dependency.index,
                        Some(extra),
                        root_marker.and(marker),
                    );
                }
            }
        }
        // Empty groups still participate in declared conflicts.
        for group in package.metadata.dependency_groups.keys() {
            if target.includes_group(Some(name), group, groups) {
                activated.insert(
                    ConflictItem::from((name.clone(), group.clone())),
                    root_marker,
                );
            }
        }
    }
    for requirement in lock.requirements().iter().chain(
        lock.dependency_groups()
            .iter()
            .filter(|(group, _)| target.includes_group(None, group, groups))
            .flat_map(|(_, requirements)| requirements),
    ) {
        for package in lock.packages_for_name(&requirement.name) {
            let Some(marker) = lock.root_requirement_marker(requirement, package) else {
                continue;
            };
            let index = lock.by_id[&package.id];
            requests.push(requests.root, index, None, root_marker.and(marker));
            for extra in &requirement.extras {
                requests.push(
                    requests.root,
                    index,
                    Some(extra.clone()),
                    root_marker.and(marker),
                );
            }
        }
    }
    while let Some((index, extra, parent)) = requests.queue.pop_front() {
        let package = lock.package(index);
        let context = extra
            .as_ref()
            .map_or(DependencyContext::Production, DependencyContext::Extra);
        let requirements = package.dependency_requirements(
            context,
            &modifiers,
            target.install_path(),
            lock.requires_python(),
        )?;
        for dependency in context.dependencies(package) {
            let (marker, extras) =
                dependency.activation(requirements.as_deref(), target.install_path())?;
            requests.push(parent, dependency.index, None, marker);
            for (extra, marker) in extras {
                requests.push(parent, dependency.index, Some(extra), marker);
            }
        }
    }
    let known_conflicts = activated
        .iter()
        .map(|(item, marker)| (item.clone(), *marker))
        .collect();
    for request in requests.finish(&known_conflicts, marker_environment) {
        let (index, extra, parent_marker) = request?;
        let package = lock.package(index);
        if groups.prod() && lock.is_workspace_package(package) {
            activated
                .entry(ConflictItem::from(package.name().clone()))
                .and_modify(|marker| *marker = marker.or(parent_marker))
                .or_insert(parent_marker);
        }
        if let Some(extra) = &extra {
            activated
                .entry(ConflictItem::from((package.name().clone(), extra.clone())))
                .and_modify(|marker| *marker = marker.or(parent_marker))
                .or_insert(parent_marker);
        }
    }
    activated.retain(|_, marker| {
        marker_environment.is_none_or(|environment| marker.evaluate(environment.markers(), &[]))
    });
    Ok(activated)
}
