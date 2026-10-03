use std::collections::BTreeSet;

use uv_configuration::DependencyGroups;
use uv_distribution_types::Requirement;
use uv_normalize::{DefaultGroups, ExtraName, GroupName, PackageName};
use uv_pypi_types::ResolverMarkerEnvironment;

use super::installable::{Installable, InstallableRootKind};
use super::walk::LockWalker;
use super::{Dependency, LockError, LockErrorKind};

/// A lock traversal that follows every conflict selection compatible with one Python environment.
struct EnvironmentReachability<'lock, 'env> {
    walker: LockWalker<'lock>,
    marker_environment: &'env ResolverMarkerEnvironment,
}

impl<'lock, 'env> EnvironmentReachability<'lock, 'env> {
    fn new(lock: &'lock super::Lock, marker_environment: &'env ResolverMarkerEnvironment) -> Self {
        Self {
            walker: LockWalker::new(lock),
            marker_environment,
        }
    }

    fn push_dependency(&mut self, dependency: &'lock Dependency) {
        if dependency
            .complexified_marker
            .pep508()
            .evaluate(self.marker_environment, &[])
        {
            self.walker.push_dependency(dependency);
        }
    }

    fn push_requirement(&mut self, requirement: &'lock Requirement) {
        let lock = self.walker.lock();
        for (index, package) in lock.packages_for_requirement(requirement) {
            if !lock
                .root_requirement_marker(requirement, package)
                .is_some_and(|marker| marker.evaluate(self.marker_environment, &[]))
            {
                continue;
            }
            self.walker.push_package(index, &requirement.extras);
        }
    }

    fn package_names(mut self) -> BTreeSet<PackageName> {
        let mut names = BTreeSet::new();
        while let Some(visit) = self.walker.pop() {
            names.insert(visit.package.name().clone());
            for dependency in visit.dependencies {
                self.push_dependency(dependency);
            }
        }
        names
    }
}

/// A direct dependency section in a project or lock target.
#[derive(Debug, Clone, Copy)]
pub enum DependencySection<'a> {
    /// Project dependencies or lock-level requirements.
    Production,
    /// An optional dependency group.
    Optional(&'a ExtraName),
    /// A dependency group.
    Group(&'a GroupName),
}

/// Return package names required by any declaration in the target.
///
/// The traversal preserves package IDs and activated extras until the final name projection. It
/// evaluates PEP 508 markers for the active interpreter while treating every conflict selection
/// as potentially active. This conservative union prevents a removal from deleting a package
/// owned by any compatible combination of declared project extras or groups.
pub fn reachable_declared_package_names<'lock>(
    target: &impl Installable<'lock>,
    marker_environment: &ResolverMarkerEnvironment,
) -> Result<BTreeSet<PackageName>, LockError> {
    let lock = target.lock();
    let mut reachability = EnvironmentReachability::new(lock, marker_environment);

    let groups = DependencyGroups::from_all_groups().with_defaults(DefaultGroups::default());
    for (root_name, root_kind) in target.roots_with_kind(&groups) {
        let root = lock
            .find_by_name(root_name)
            .map_err(|_| LockErrorKind::MultipleRootPackages {
                name: root_name.clone(),
            })?
            .ok_or_else(|| LockErrorKind::MissingRootPackage {
                name: root_name.clone(),
            })?;
        if root_kind == InstallableRootKind::Production {
            reachability
                .walker
                .push_package(lock.by_id[&root.id], root.optional_dependencies().keys());
        }
        for dependency in root
            .resolved_dependency_groups()
            .iter()
            .filter(|(group, _)| target.includes_group(Some(root.name()), group, &groups))
            .flat_map(|(_, dependencies)| dependencies)
        {
            reachability.push_dependency(dependency);
        }
    }
    for requirement in target.root_requirements(&groups) {
        reachability.push_requirement(requirement);
    }

    Ok(reachability.package_names())
}

/// Return package names required by the named direct dependencies in one project section.
pub fn reachable_direct_dependency_names<'lock>(
    target: &impl Installable<'lock>,
    marker_environment: &ResolverMarkerEnvironment,
    section: DependencySection<'_>,
    names: &BTreeSet<PackageName>,
) -> Result<BTreeSet<PackageName>, LockError> {
    let lock = target.lock();
    let mut reachability = EnvironmentReachability::new(lock, marker_environment);

    if let Some(project_name) = target.project_name() {
        let project = lock
            .find_by_name(project_name)
            .map_err(|_| LockErrorKind::MultipleRootPackages {
                name: project_name.clone(),
            })?
            .ok_or_else(|| LockErrorKind::MissingRootPackage {
                name: project_name.clone(),
            })?;
        let dependencies = match section {
            DependencySection::Production => project.dependencies(),
            DependencySection::Optional(extra) => project
                .optional_dependencies()
                .get(extra)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            DependencySection::Group(group) => project
                .resolved_dependency_groups()
                .get(group)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        };
        for dependency in dependencies
            .iter()
            .filter(|dependency| names.contains(&dependency.package_id.name))
        {
            reachability.push_dependency(dependency);
        }
    } else {
        let requirements = match section {
            DependencySection::Production => Some(lock.requirements()),
            DependencySection::Optional(_) => None,
            DependencySection::Group(group) => lock.dependency_groups().get(group),
        };
        for requirement in requirements
            .into_iter()
            .flatten()
            .filter(|requirement| names.contains(&requirement.name))
        {
            reachability.push_requirement(requirement);
        }
    }

    Ok(reachability.package_names())
}
