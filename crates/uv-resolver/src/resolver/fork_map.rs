use rustc_hash::FxHashMap;

use uv_distribution_types::{Requirement, RequirementSource};
use uv_normalize::PackageName;
use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictItemRef, ConflictKind};

use crate::ResolverEnvironment;
use crate::universal_marker::{ConflictMarker, UniversalMarker};

/// A set of package names associated with a given fork.
pub(crate) type ForkSet = ForkMap<()>;

/// A map from package names to their values for a given fork.
#[derive(Debug, Clone)]
pub(crate) struct ForkMap<T>(FxHashMap<PackageName, Vec<Entry<T>>>);

/// An entry in a [`ForkMap`].
#[derive(Debug, Clone)]
struct Entry<T> {
    value: T,
    scope: ForkScope,
}

/// The fork visibility of an entry.
#[derive(Debug, Clone, Eq, PartialEq)]
struct ForkScope {
    marker: MarkerTree,
    conflict: Option<ConflictItem>,
}

impl ForkScope {
    /// Derive the scope under which a requirement should be visible in forked resolution.
    ///
    /// Group conflicts are folded into the marker so group-scoped entries only appear in forks
    /// where that group is active.
    fn from_requirement(requirement: &Requirement) -> Self {
        let conflict = Self::conflict_for_requirement(requirement);
        let marker = conflict
            .as_ref()
            .filter(|conflict_item| matches!(conflict_item.kind(), ConflictKind::Group(_)))
            .map_or(requirement.marker, |conflict_item| {
                UniversalMarker::new(
                    requirement.marker.without_extras(),
                    ConflictMarker::from_conflict_item(conflict_item),
                )
                .combined()
            });
        Self { marker, conflict }
    }

    fn conflict_for_requirement(requirement: &Requirement) -> Option<ConflictItem> {
        let conflict = match &requirement.source {
            RequirementSource::Registry { conflict, .. } => conflict.clone(),
            RequirementSource::Url { .. }
            | RequirementSource::GitDirectory { .. }
            | RequirementSource::GitPath { .. }
            | RequirementSource::Path { .. }
            | RequirementSource::Directory { .. } => None,
        };
        conflict.or_else(|| requirement.scope.conflict_item())
    }

    /// Return the conflict item that further restricts this scope, if any.
    fn conflict(&self) -> Option<ConflictItemRef<'_>> {
        self.conflict.as_ref().map(ConflictItem::as_ref)
    }

    fn matches(&self, env: &ResolverEnvironment) -> bool {
        env.included_by_marker(self.marker)
            && self
                .conflict()
                .is_none_or(|conflict| env.included_by_group(conflict))
    }

    /// The full environment in which this entry is applicable.
    fn universal_marker(&self) -> UniversalMarker {
        UniversalMarker::new(
            self.marker,
            self.conflict
                .as_ref()
                .map_or(ConflictMarker::TRUE, ConflictMarker::from_conflict_item),
        )
    }
}

impl<T> Default for ForkMap<T> {
    fn default() -> Self {
        Self(FxHashMap::default())
    }
}

impl<T> ForkMap<T> {
    /// Associate a value with the [`Requirement`] in a given fork.
    pub(crate) fn add(&mut self, requirement: &Requirement, value: T) {
        self.0
            .entry(requirement.name.clone())
            .or_default()
            .push(Entry {
                value,
                scope: ForkScope::from_requirement(requirement),
            });
    }

    /// Returns `true` if the map contains any values for a package that are compatible with the
    /// given fork.
    pub(crate) fn contains(&self, package_name: &PackageName, env: &ResolverEnvironment) -> bool {
        self.0
            .get(package_name)
            .is_some_and(|values| values.iter().any(|entry| entry.scope.matches(env)))
    }

    /// Returns `true` if the map contains any values for a package.
    pub(crate) fn contains_key(&self, package_name: &PackageName) -> bool {
        self.0.contains_key(package_name)
    }

    /// Returns a list of values associated with a package that are compatible with the given fork.
    ///
    /// Compatibility implies that the markers on the requirement that contained this value
    /// are not disjoint with the given fork. Note that this does not imply that the requirement
    /// diverged in the given fork - values from overlapping forks may be combined.
    pub(crate) fn get(&self, package_name: &PackageName, env: &ResolverEnvironment) -> Vec<&T> {
        let Some(values) = self.0.get(package_name) else {
            return Vec::new();
        };
        values
            .iter()
            .filter(|entry| entry.scope.matches(env))
            .map(|entry| &entry.value)
            .collect()
    }

    /// Whether one value is imposed throughout the entire universal fork.
    ///
    /// Unlike [`Self::get`], mere marker overlap is insufficient. Every applicable value must
    /// agree, and their complete PEP 508 and conflict scopes must cover the fork.
    pub(crate) fn is_fixed(
        &self,
        package_name: &PackageName,
        env: &ResolverEnvironment,
        expected: &T,
    ) -> bool
    where
        T: PartialEq,
    {
        let Some(environment) = env.try_universal_markers() else {
            return false;
        };
        let Some(entries) = self.0.get(package_name) else {
            return false;
        };
        let mut covered = MarkerTree::FALSE;
        for entry in entries.iter().filter(|entry| entry.scope.matches(env)) {
            if &entry.value != expected {
                return false;
            }
            covered = covered.or(entry.scope.universal_marker().combined());
        }
        !covered.is_false() && environment.combined().is_disjoint(covered.negate())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;
    use std::str::FromStr;

    use uv_distribution_types::{RequirementScope, RequirementSource};
    use uv_normalize::{ExtraName, GroupName, PackageName};
    use uv_pep440::VersionSpecifiers;
    use uv_pep508::VerbatimUrl;

    use super::*;

    fn registry_requirement(name: PackageName, marker: MarkerTree) -> Requirement {
        Requirement {
            name,
            extras: Box::default(),
            groups: Box::default(),
            marker,
            source: RequirementSource::Registry {
                specifier: VersionSpecifiers::empty(),
                index: None,
                conflict: None,
            },
            scope: RequirementScope::Global,
            origin: None,
        }
    }

    #[test]
    fn fixed_values_require_complete_marker_coverage() -> Result<(), Box<dyn Error>> {
        let name: PackageName = "demo".parse()?;
        let env = ResolverEnvironment::universal(Vec::new());
        let mut requirement =
            registry_requirement(name.clone(), "python_version < '3.12'".parse()?);
        let mut map = ForkMap::default();
        map.add(&requirement, 1);
        assert!(!map.is_fixed(&name, &env, &1));

        requirement.marker = requirement.marker.negate();
        map.add(&requirement, 1);
        assert!(map.is_fixed(&name, &env, &1));
        assert!(!map.is_fixed(&name, &env, &2));

        map.add(&requirement, 2);
        assert!(!map.is_fixed(&name, &env, &1));
        Ok(())
    }

    #[test]
    fn fixed_values_include_conflict_scope() -> Result<(), Box<dyn Error>> {
        let name: PackageName = "demo".parse()?;
        let item = ConflictItem::from((
            "project".parse::<PackageName>()?,
            "feature".parse::<ExtraName>()?,
        ));
        let mut requirement = registry_requirement(name.clone(), MarkerTree::TRUE);
        requirement.source = RequirementSource::Registry {
            specifier: VersionSpecifiers::empty(),
            index: None,
            conflict: Some(item.clone()),
        };
        let mut map = ForkMap::default();
        map.add(&requirement, 1);
        let env = ResolverEnvironment::universal(Vec::new());
        assert!(!map.is_fixed(&name, &env, &1));

        let included = env
            .filter_by_group([Ok(item.clone())])
            .ok_or("included conflict scope should be valid")?;
        assert!(map.is_fixed(&name, &included, &1));
        let excluded = env
            .filter_by_group([Err(item)])
            .ok_or("excluded conflict scope should be valid")?;
        assert!(!map.is_fixed(&name, &excluded, &1));
        Ok(())
    }

    #[test]
    fn add_scopes_non_registry_requirements_without_origin() {
        let project_name = PackageName::from_str("workspace-root").unwrap();
        let group = GroupName::from_str("dev").unwrap();
        let package_name = PackageName::from_str("demo").unwrap();
        let conflict = ConflictItem::from((project_name.clone(), group.clone()));
        let requirement = Requirement {
            name: package_name.clone(),
            extras: Box::default(),
            groups: Box::default(),
            marker: MarkerTree::TRUE,
            source: RequirementSource::Directory {
                install_path: PathBuf::from("/tmp/demo").into_boxed_path(),
                editable: None,
                r#virtual: None,
                url: VerbatimUrl::parse_url("file:///tmp/demo").unwrap(),
            },
            scope: RequirementScope::Group {
                package: project_name,
                group,
            },
            origin: None,
        };

        let mut map = ForkMap::default();
        map.add(&requirement, ());

        assert!(map.contains(&package_name, &ResolverEnvironment::universal(vec![])));

        let env = ResolverEnvironment::universal(vec![])
            .filter_by_group([Err(conflict)])
            .unwrap();
        assert!(!map.contains(&package_name, &env));
    }
}
