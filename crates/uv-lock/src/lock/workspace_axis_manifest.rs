use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use petgraph::visit::EdgeRef;

use uv_configuration::{Excludes, Overrides};
use uv_distribution_types::{
    IndexUrl, NameRequirementSpecification, Requirement, RequirementSource,
};
use uv_fs::normalize_path;
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep508::MarkerTree;
use uv_pypi_types::ConflictItem;
use uv_resolver_types::{ConflictMarker, ResolutionGraphNode, ResolverOutput, UniversalMarker};

use super::workspace_axes::{activation, ordinary_environment, validate_ordinary_predicate};
use super::workspace_groups::package_environment;
use super::{
    Lock, LockError, LockErrorKind, Package, PackageId, RegistrySource, ResolverManifest, Source,
};

/// Direct manifest edges chosen by a real ordinary resolution.
///
/// A package name and version do not identify a manifest root: another workspace context can
/// contain the same version from a different index or a local source. These entries also retain
/// the effective requirement after global overrides, which can change its source and extras.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(transparent)]
pub(super) struct WorkspaceAxisManifestRoots(pub(super) Vec<WorkspaceAxisManifestRoot>);

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct WorkspaceAxisManifestRoot {
    pub(super) group: Option<GroupName>,
    pub(super) requirement: Requirement,
    pub(super) resolutions: Vec<WorkspaceAxisManifestResolution>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct WorkspaceAxisManifestResolution {
    pub(super) requirement: Requirement,
    pub(super) kind: WorkspaceAxisManifestKind,
    /// The exact ordinary activation promised by the resolved root edges.
    #[serde(default, deserialize_with = "deserialize_manifest_marker")]
    pub(super) environment: MarkerTree,
    /// Normalized, root-relative source bindings for an explicitly sourced requirement. An
    /// unqualified registry requirement instead inherits the source selected by the resolver.
    pub(super) sources: Vec<Source>,
    pub(super) targets: Vec<WorkspaceAxisManifestTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum WorkspaceAxisManifestKind {
    Production,
    Extra(ExtraName),
    Group(GroupName),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct WorkspaceAxisManifestTarget {
    #[serde(deserialize_with = "deserialize_package_id")]
    pub(super) package: PackageId,
    #[serde(default, deserialize_with = "deserialize_manifest_marker")]
    pub(super) marker: MarkerTree,
}

fn deserialize_manifest_marker<'de, D>(deserializer: D) -> Result<MarkerTree, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // PEP 508 has no literal false expression. Keep an empty root activation as a Boolean instead
    // of serializing a synthetic Python bound that can admit prerelease versions when reparsed.
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum MarkerWire {
        Boolean(bool),
        Marker(MarkerTree),
    }

    Ok(
        match <MarkerWire as serde::Deserialize>::deserialize(deserializer)? {
            MarkerWire::Boolean(true) => MarkerTree::TRUE,
            MarkerWire::Boolean(false) => MarkerTree::FALSE,
            MarkerWire::Marker(marker) => marker,
        },
    )
}

fn deserialize_package_id<'de, D>(deserializer: D) -> Result<PackageId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "kebab-case", deny_unknown_fields)]
    struct PackageIdWire {
        name: PackageName,
        version: Option<uv_pep440::Version>,
        source: Source,
    }

    let wire = <PackageIdWire as serde::Deserialize>::deserialize(deserializer)?;
    Ok(PackageId {
        name: wire.name,
        version: wire.version,
        source: wire.source,
    })
}

/// A resolved manifest edge ready for an ordinary installation/export graph.
pub(super) struct WorkspaceAxisManifestDependency<'lock> {
    pub(super) group: Option<&'lock GroupName>,
    pub(super) package: &'lock Package,
    pub(super) extras: Vec<&'lock ExtraName>,
    pub(super) marker: UniversalMarker,
    pub(super) activated_group: Option<(&'lock PackageName, &'lock GroupName)>,
}

impl Lock {
    /// Record exact manifest-root identities from the resolver graph for an ordinary axis solve.
    ///
    /// Call this after [`Self::with_conflicts`]. The graph must be the [`ResolverOutput`] used to
    /// construct this lock, and `workspace_root` must be the same root passed to
    /// [`Self::from_resolution`]. This provenance is persisted only inside a v3 axis context.
    pub fn with_authoritative_workspace_axis_manifest_roots(
        mut self,
        resolution: &ResolverOutput,
        workspace_root: &Path,
    ) -> Result<Self, LockError> {
        if self.workspace_axes.is_some() || self.workspace_axis_command.is_some() {
            return Err(invalid_manifest_roots(
                "expected an ordinary resolver output",
            ));
        }
        if manifest_is_empty(&self.manifest) {
            self.workspace_axis_manifest_roots = Some(WorkspaceAxisManifestRoots::default());
            return Ok(self);
        }
        let environment = ordinary_environment(&self);
        let excludes = Excludes::from_entries(self.manifest.excludes.iter().cloned());
        let mut direct = BTreeMap::new();
        for edge in resolution.graph.edge_references() {
            let ResolutionGraphNode::Root = &resolution.graph[edge.source()] else {
                continue;
            };
            let ResolutionGraphNode::Dist(distribution) = &resolution.graph[edge.target()] else {
                return Err(invalid_manifest_roots(
                    "a resolver root points to another root",
                ));
            };
            let kind = match (&distribution.extra, &distribution.group) {
                (None, None) => WorkspaceAxisManifestKind::Production,
                (Some(extra), None) => WorkspaceAxisManifestKind::Extra(extra.clone()),
                (None, Some(group)) => WorkspaceAxisManifestKind::Group(group.clone()),
                (Some(_), Some(_)) => {
                    return Err(invalid_manifest_roots(
                        "a resolver root has both an extra and a group",
                    ));
                }
            };
            let id = PackageId::from_annotated_dist(distribution, workspace_root)?;
            if !self.by_id.contains_key(&id) {
                return Err(invalid_manifest_roots(format!(
                    "resolver root `{id}` is not in the ordinary lock"
                )));
            }
            let marker = edge
                .weight()
                .combined()
                .and(distribution.marker.combined())
                .and(environment);
            let by_id = direct
                .entry((id.name.clone(), kind))
                .or_insert_with(BTreeMap::new);
            let previous = by_id.entry(id).or_insert(MarkerTree::FALSE);
            *previous = previous.or(marker);
        }

        let mut roots = Vec::new();
        for (group, requirement) in manifest_requirements(&self.manifest) {
            let absolute = absolute_requirement(requirement.clone(), workspace_root);
            let mut resolutions = Vec::new();
            for effective in resolution.overrides.apply([&absolute]) {
                let relative = effective
                    .clone()
                    .into_owned()
                    .relative_to(workspace_root)
                    .map_err(LockErrorKind::RequirementRelativePath)?;
                for kind in requirement_kinds(&effective) {
                    let allowed = if excludes.contains(&effective.name) {
                        MarkerTree::FALSE
                    } else {
                        requirement_environment(&effective, &kind, &self.conflicts).and(environment)
                    };
                    let mut targets = Vec::new();
                    let mut covered = MarkerTree::FALSE;
                    let mut sources = BTreeSet::new();
                    for (id, marker) in direct
                        .get(&(effective.name.clone(), kind.clone()))
                        .into_iter()
                        .flatten()
                    {
                        if !version_matches(id, &effective)
                            || !source_matches_live(&id.source, &effective.source, workspace_root)?
                        {
                            continue;
                        }
                        let marker = marker.and(allowed);
                        if marker.is_false() {
                            continue;
                        }
                        covered = covered.or(marker);
                        if !inherits_source(&effective.source) {
                            sources.insert(id.source.clone());
                        }
                        targets.push(WorkspaceAxisManifestTarget {
                            package: id.clone(),
                            marker,
                        });
                    }
                    resolutions.push(WorkspaceAxisManifestResolution {
                        requirement: relative.clone(),
                        kind,
                        environment: covered,
                        sources: sources.into_iter().collect(),
                        targets,
                    });
                }
            }
            resolutions.sort_by(|left, right| {
                (&left.requirement, &left.kind).cmp(&(&right.requirement, &right.kind))
            });
            roots.push(WorkspaceAxisManifestRoot {
                group: group.cloned(),
                requirement: requirement.clone(),
                resolutions,
            });
        }
        let roots = WorkspaceAxisManifestRoots(roots);
        roots.validate(&self, &self.manifest, environment, MarkerTree::TRUE)?;
        self.workspace_axis_manifest_roots = Some(roots);
        Ok(self)
    }

    /// Return whether this ordinary solve has exact direct-root provenance for axis locking.
    /// An empty manifest is authoritative without another resolver run.
    pub fn has_authoritative_workspace_axis_manifest_roots(&self) -> bool {
        self.workspace_axis_manifest_roots.is_some() || manifest_is_empty(&self.manifest)
    }

    pub(super) fn authoritative_workspace_axis_manifest_roots(
        &self,
    ) -> Result<WorkspaceAxisManifestRoots, LockError> {
        self.workspace_axis_manifest_roots.clone().map_or_else(
            || {
                if manifest_is_empty(&self.manifest) {
                    Ok(WorkspaceAxisManifestRoots::default())
                } else {
                    Err(invalid_manifest_roots(
                        "the ordinary resolution is missing authoritative manifest-root identities",
                    ))
                }
            },
            Ok,
        )
    }

    /// Merge ordinary axis views without discarding their exact direct-root identities.
    pub(super) fn merge_workspace_axis_views(
        resolutions: Vec<Self>,
        require_all_roots: bool,
    ) -> Result<Option<Self>, LockError> {
        let maps = resolutions
            .iter()
            .map(Self::authoritative_workspace_axis_manifest_roots)
            .collect::<Result<Vec<_>, _>>()?;
        let merged = if require_all_roots {
            Self::merge_workspace_resolutions(resolutions)?
        } else {
            Self::merge_workspace_group_preferences(resolutions)?
        };
        let Some(mut lock) = merged else {
            return Ok(None);
        };
        let environment = ordinary_environment(&lock);
        let packages = lock
            .packages
            .iter()
            .map(|package| {
                (
                    package.id.clone(),
                    package_environment(package, &lock.requires_python),
                )
            })
            .collect();
        let mut maps = maps.into_iter();
        let Some(first) = maps.next() else {
            return Ok(None);
        };
        let mut roots = first.project(environment);
        for incoming in maps {
            if roots.merge(incoming.project(environment)).is_err() {
                // A preference merge may cross source/override declaration changes. It can
                // fall back to one real prior context, but must not invent a direct-root map.
                return Ok(None);
            }
        }
        roots.retain_packages(&packages);
        lock.workspace_axis_manifest_roots = Some(roots);
        Ok(Some(lock))
    }

    /// Return exact direct dependencies for a projected axis view. `None` selects the ordinary
    /// v1/v2 manifest path; `Some([])` is an authoritative empty direct-root set.
    pub(super) fn workspace_axis_manifest_dependencies(
        &self,
    ) -> Result<Option<Vec<WorkspaceAxisManifestDependency<'_>>>, LockError> {
        let Some(roots) = self.workspace_axis_manifest_roots.as_ref() else {
            return Ok(None);
        };
        let mut dependencies = Vec::new();
        for root in &roots.0 {
            for resolution in &root.resolutions {
                for target in &resolution.targets {
                    let Some(index) = self.by_id.get(&target.package) else {
                        return Err(invalid_manifest_roots(format!(
                            "manifest root `{}` is not in the projected lock",
                            target.package
                        )));
                    };
                    let package = self.package(*index);
                    let marker = UniversalMarker::from_combined(target.marker);
                    match &resolution.kind {
                        WorkspaceAxisManifestKind::Production => {
                            dependencies.push(WorkspaceAxisManifestDependency {
                                group: root.group.as_ref(),
                                package,
                                extras: Vec::new(),
                                marker,
                                activated_group: None,
                            });
                        }
                        WorkspaceAxisManifestKind::Extra(extra) => {
                            dependencies.push(WorkspaceAxisManifestDependency {
                                group: root.group.as_ref(),
                                package,
                                extras: vec![extra],
                                marker,
                                activated_group: None,
                            });
                        }
                        WorkspaceAxisManifestKind::Group(group) => {
                            for dependency in
                                package.dependency_groups.get(group).into_iter().flatten()
                            {
                                let mut marker = marker;
                                marker.and(dependency.complexified_marker);
                                dependencies.push(WorkspaceAxisManifestDependency {
                                    group: root.group.as_ref(),
                                    package: self.package(dependency.index),
                                    extras: dependency.extra.iter().collect(),
                                    marker,
                                    activated_group: Some((&package.id.name, group)),
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(Some(dependencies))
    }
}

impl WorkspaceAxisManifestRoots {
    pub(super) fn validate(
        &self,
        lock: &Lock,
        manifest: &ResolverManifest,
        environment: MarkerTree,
        selector_scope: MarkerTree,
    ) -> Result<(), LockError> {
        let expected = manifest_requirements(manifest)
            .map(|(group, requirement)| (group.cloned(), requirement.clone()))
            .collect::<BTreeSet<_>>();
        let actual = self
            .0
            .iter()
            .map(|root| (root.group.clone(), root.requirement.clone()))
            .collect::<BTreeSet<_>>();
        if actual.len() != self.0.len() || actual != expected {
            return Err(invalid_manifest_roots(
                "manifest-root declaration keys do not match the cohort manifest",
            ));
        }
        let overrides = Overrides::from_entries(manifest.overrides.iter().cloned().collect())
            .map_err(|error| invalid_manifest_roots(error.to_string()))?;
        let excludes = Excludes::from_entries(manifest.excludes.iter().cloned());
        let conflict_world = UniversalMarker::new(
            MarkerTree::TRUE,
            ConflictMarker::from_conflicts(&lock.conflicts),
        )
        .combined();
        for root in &self.0 {
            let expected = overrides
                .apply([&root.requirement])
                .flat_map(|requirement| {
                    requirement_kinds(&requirement)
                        .map(move |kind| (requirement.clone().into_owned(), kind))
                })
                .collect::<BTreeSet<_>>();
            let actual = root
                .resolutions
                .iter()
                .map(|resolution| (resolution.requirement.clone(), resolution.kind.clone()))
                .collect::<BTreeSet<_>>();
            if actual.len() != root.resolutions.len() || actual != expected {
                return Err(invalid_manifest_roots(format!(
                    "effective root keys for `{}` do not match its overrides",
                    root.requirement.name
                )));
            }
            for resolution in &root.resolutions {
                validate_ordinary_predicate(resolution.requirement.marker)?;
                validate_ordinary_predicate(resolution.environment)?;
                let allowed = if excludes.contains(&resolution.requirement.name) {
                    MarkerTree::FALSE
                } else {
                    requirement_environment(
                        &resolution.requirement,
                        &resolution.kind,
                        &lock.conflicts,
                    )
                    .and(environment)
                    .and(conflict_world)
                };
                let promised = resolution.environment.and(conflict_world);
                // Ordinary conflict forks can make a root conditional on selected extras or
                // projects. Their exact graph-derived predicate is persisted, while its physical
                // projection must still cover every applicable Python/platform environment.
                if !promised.and(allowed.negate()).is_false()
                    || !same_markers(promised.without_extras(), allowed.without_extras())
                {
                    return Err(invalid_manifest_roots(format!(
                        "root activation for `{}` does not cover its declared environments",
                        resolution.requirement.name
                    )));
                }
                let sources = resolution.sources.iter().collect::<BTreeSet<_>>();
                if sources.len() != resolution.sources.len()
                    || inherits_source(&resolution.requirement.source) && !sources.is_empty()
                {
                    return Err(invalid_manifest_roots(
                        "invalid normalized root source bindings",
                    ));
                }
                let mut reached_sources = BTreeSet::new();
                let mut targets = BTreeSet::new();
                let mut covered = MarkerTree::FALSE;
                for target in &resolution.targets {
                    validate_ordinary_predicate(target.marker)?;
                    if target.marker.is_false() || !targets.insert(&target.package) {
                        return Err(invalid_manifest_roots(
                            "duplicate or empty resolved manifest-root target",
                        ));
                    }
                    let Some(index) = lock.by_id.get(&target.package) else {
                        return Err(invalid_manifest_roots(format!(
                            "manifest root `{}` is not in the lock",
                            target.package
                        )));
                    };
                    let package = lock.package(*index);
                    if !version_matches(&target.package, &resolution.requirement)
                        || !source_matches_locked(
                            &target.package.source,
                            &resolution.requirement.source,
                            &sources,
                        )?
                    {
                        return Err(invalid_manifest_roots(format!(
                            "manifest root `{}` does not satisfy its effective requirement",
                            target.package
                        )));
                    }
                    let required = selector_scope.and(target.marker).and(conflict_world);
                    if !required
                        .and(package_environment(package, &lock.requires_python).negate())
                        .is_false()
                    {
                        return Err(invalid_manifest_roots(format!(
                            "manifest root `{}` exceeds its locked availability",
                            target.package
                        )));
                    }
                    covered = covered.or(target.marker);
                    if !inherits_source(&resolution.requirement.source) {
                        reached_sources.insert(&target.package.source);
                    }
                }
                if reached_sources != sources {
                    return Err(invalid_manifest_roots(
                        "normalized root sources do not match the resolved targets",
                    ));
                }
                if !same_markers(covered.and(conflict_world), promised) {
                    return Err(invalid_manifest_roots(format!(
                        "resolved targets do not cover the activation of `{}`",
                        resolution.requirement.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// Restrict graph provenance alongside a projected ordinary lock.
    pub(super) fn project(&self, environment: MarkerTree) -> Self {
        let mut roots = self.clone();
        for root in &mut roots.0 {
            for resolution in &mut root.resolutions {
                resolution.environment = resolution.environment.and(environment);
                for target in &mut resolution.targets {
                    target.marker = target.marker.and(environment);
                }
                resolution
                    .targets
                    .retain(|target| !target.marker.is_false());
                resolution.retain_sources();
            }
        }
        roots
    }

    pub(super) fn retain_packages(&mut self, packages: &BTreeMap<PackageId, MarkerTree>) {
        for root in &mut self.0 {
            for resolution in &mut root.resolutions {
                for target in &mut resolution.targets {
                    target.marker = target.marker.and(
                        packages
                            .get(&target.package)
                            .copied()
                            .unwrap_or(MarkerTree::FALSE),
                    );
                }
                resolution
                    .targets
                    .retain(|target| !target.marker.is_false());
                resolution.environment = resolution
                    .targets
                    .iter()
                    .fold(MarkerTree::FALSE, |marker, target| marker.or(target.marker));
                resolution.retain_sources();
            }
        }
    }

    pub(super) fn merge(&mut self, other: Self) -> Result<(), LockError> {
        for incoming in other.0 {
            let Some(root) = self.0.iter_mut().find(|root| {
                root.group == incoming.group && root.requirement == incoming.requirement
            }) else {
                return Err(invalid_manifest_roots(
                    "projected manifests have different root declarations",
                ));
            };
            for incoming in incoming.resolutions {
                let Some(resolution) = root.resolutions.iter_mut().find(|resolution| {
                    resolution.requirement == incoming.requirement
                        && resolution.kind == incoming.kind
                }) else {
                    return Err(invalid_manifest_roots(
                        "projected manifests have different effective root declarations",
                    ));
                };
                resolution.environment = resolution.environment.or(incoming.environment);
                for incoming in incoming.targets {
                    if let Some(target) = resolution
                        .targets
                        .iter_mut()
                        .find(|target| target.package == incoming.package)
                    {
                        target.marker = target.marker.or(incoming.marker);
                    } else {
                        resolution.targets.push(incoming);
                    }
                }
                resolution
                    .targets
                    .sort_by(|left, right| left.package.cmp(&right.package));
                resolution.sources.extend(incoming.sources);
                resolution.sources.sort();
                resolution.sources.dedup();
            }
        }
        Ok(())
    }
}

impl WorkspaceAxisManifestResolution {
    fn retain_sources(&mut self) {
        self.sources.retain(|source| {
            self.targets
                .iter()
                .any(|target| target.package.source == *source)
        });
    }
}

fn manifest_is_empty(manifest: &ResolverManifest) -> bool {
    manifest.requirements.is_empty() && manifest.dependency_groups.values().all(BTreeSet::is_empty)
}

fn manifest_requirements(
    manifest: &ResolverManifest,
) -> impl Iterator<Item = (Option<&GroupName>, &Requirement)> {
    manifest
        .requirements
        .iter()
        .map(|requirement| (None, requirement))
        .chain(
            manifest
                .dependency_groups
                .iter()
                .flat_map(|(group, requirements)| {
                    requirements
                        .iter()
                        .map(move |requirement| (Some(group), requirement))
                }),
        )
}

fn requirement_kinds(
    requirement: &Requirement,
) -> impl Iterator<Item = WorkspaceAxisManifestKind> + use<> {
    let mut kinds = BTreeSet::new();
    if requirement.groups.is_empty() {
        kinds.insert(WorkspaceAxisManifestKind::Production);
        kinds.extend(
            requirement
                .extras
                .iter()
                .cloned()
                .map(WorkspaceAxisManifestKind::Extra),
        );
    } else {
        kinds.extend(
            requirement
                .groups
                .iter()
                .cloned()
                .map(WorkspaceAxisManifestKind::Group),
        );
    }
    kinds.into_iter()
}

fn requirement_environment(
    requirement: &Requirement,
    kind: &WorkspaceAxisManifestKind,
    conflicts: &uv_pypi_types::Conflicts,
) -> MarkerTree {
    let mut marker = requirement.marker.simplify_not_extras_with(|_| true);
    if let RequirementSource::Registry {
        conflict: Some(item),
        ..
    } = &requirement.source
    {
        marker = marker.and(activation(conflicts, item));
    }
    match kind {
        WorkspaceAxisManifestKind::Production => {}
        WorkspaceAxisManifestKind::Extra(extra) => {
            marker = marker.and(activation(
                conflicts,
                &ConflictItem::from((requirement.name.clone(), extra.clone())),
            ));
        }
        WorkspaceAxisManifestKind::Group(group) => {
            marker = marker.and(activation(
                conflicts,
                &ConflictItem::from((requirement.name.clone(), group.clone())),
            ));
        }
    }
    marker
}

fn absolute_requirement(requirement: Requirement, root: &Path) -> Requirement {
    NameRequirementSpecification {
        requirement,
        hashes: Vec::new(),
    }
    .into_absolute(root)
    .requirement
}

fn version_matches(id: &PackageId, requirement: &Requirement) -> bool {
    id.name == requirement.name
        && requirement
            .source
            .version_specifiers()
            .zip(id.version.as_ref())
            .is_none_or(|(specifier, version)| specifier.contains(version))
}

fn inherits_source(source: &RequirementSource) -> bool {
    matches!(source, RequirementSource::Registry { index: None, .. })
}

fn source_matches_live(
    source: &Source,
    requirement: &RequirementSource,
    root: &Path,
) -> Result<bool, LockError> {
    if inherits_source(requirement) {
        return Ok(true);
    }
    source.satisfies_requirement_source(requirement, root)
}

fn source_matches_locked(
    source: &Source,
    requirement: &RequirementSource,
    bindings: &BTreeSet<&Source>,
) -> Result<bool, LockError> {
    if inherits_source(requirement) {
        return Ok(true);
    }
    if !bindings.contains(source) {
        return Ok(false);
    }
    // Explicit file-index and local-path declarations may retain an absolute user spelling while
    // package identities use workspace-relative paths. Their exact source binding was computed
    // with the live workspace root; a frozen reader must not reinterpret it using its own cwd.
    match (source, requirement) {
        (
            Source::Registry(RegistrySource::Path(_)),
            RequirementSource::Registry {
                index: Some(index), ..
            },
        ) if matches!(index.url, IndexUrl::Path(_)) => Ok(true),
        (Source::Path(actual), RequirementSource::Path { install_path, .. }) => {
            Ok(actual.is_absolute() != install_path.is_absolute()
                || normalize_path(actual.as_ref()).as_ref()
                    == normalize_path(install_path.as_ref()).as_ref())
        }
        (
            Source::Directory(actual) | Source::Editable(actual) | Source::Virtual(actual),
            RequirementSource::Directory {
                install_path,
                editable,
                r#virtual,
                ..
            },
        ) => {
            let same_path = actual.is_absolute() != install_path.is_absolute()
                || normalize_path(actual.as_ref()).as_ref()
                    == normalize_path(install_path.as_ref()).as_ref();
            let root_virtual = matches!(source, Source::Virtual(_))
                && normalize_path(install_path.as_ref()).as_ref() == Path::new("");
            Ok(same_path
                && matches!(source, Source::Editable(_)) == editable.unwrap_or(false)
                && (matches!(source, Source::Virtual(_)) == r#virtual.unwrap_or(false)
                    || root_virtual))
        }
        _ => source.satisfies_requirement_source(requirement, Path::new("")),
    }
}

fn same_markers(left: MarkerTree, right: MarkerTree) -> bool {
    left.and(right.negate()).is_false() && right.and(left.negate()).is_false()
}

fn invalid_manifest_roots(reason: impl Into<String>) -> LockError {
    LockErrorKind::WorkspaceAxes(format!("invalid manifest roots: {}", reason.into())).into()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::sync::Arc;

    use insta::assert_snapshot;
    use petgraph::Graph;
    use uv_configuration::{Constraints, DependencyGroups, ExtrasSpecification, Overrides};
    use uv_distribution_filename::SourceDistExtension;
    use uv_distribution_types::{
        DirectorySourceDist, Dist, File, FileLocation, FirstParty, RegistrySourceDist,
        ResolvedDist, SourceDist, UrlString,
    };
    use uv_normalize::{DefaultExtras, DefaultGroups, PackageName};
    use uv_pep508::VerbatimUrl;
    use uv_pypi_types::HashDigests;
    use uv_redacted::DisplaySafeUrl;
    use uv_resolver_types::{
        AnnotatedDist, Options, ResolutionGraphNode, ResolverOutput, UniversalMarker,
    };
    use uv_workspace::{ResolvedWorkspaceAxes, WorkspaceAxes, WorkspaceAxisSelection};

    use super::{Lock, Package, Source, ordinary_environment};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// Construct only the direct resolver edges needed by the provenance builder. Integration
    /// tests exercise the complete resolver; this fixture isolates the lock's identity contract.
    fn direct_resolution(lock: &Lock, root_path: &Path) -> TestResult<ResolverOutput> {
        let mut graph = Graph::new();
        let root = graph.add_node(ResolutionGraphNode::Root);
        let environment = UniversalMarker::from_combined(ordinary_environment(lock));
        for package in &lock.packages {
            let distribution = annotated(package, root_path, environment)?;
            let target = graph.add_node(ResolutionGraphNode::Dist(distribution));
            graph.add_edge(root, target, environment);
        }
        Ok(ResolverOutput {
            graph,
            requires_python: lock.requires_python.clone(),
            fork_markers: lock.fork_markers.clone(),
            diagnostics: Vec::new(),
            requirements: Vec::new(),
            constraints: Constraints::default(),
            overrides: Overrides::from_entries(lock.manifest.overrides.iter().cloned().collect())?,
            options: Options::default(),
        })
    }

    fn annotated(
        package: &Package,
        root: &Path,
        marker: UniversalMarker,
    ) -> TestResult<AnnotatedDist> {
        let version = package
            .version()
            .cloned()
            .ok_or("fixture version is missing")?;
        let name = package.name().clone();
        let dist = match &package.id.source {
            Source::Registry(_) => {
                let filename = format!("{name}-{version}.tar.gz");
                let url = DisplaySafeUrl::parse(&format!("https://files.example.org/{filename}"))?;
                Dist::Source(SourceDist::Registry(RegistrySourceDist {
                    name: name.clone(),
                    version: version.clone(),
                    file: Box::new(File {
                        dist_info_metadata: None,
                        filename: filename.into(),
                        hashes: HashDigests::empty(),
                        requires_python: None,
                        size: None,
                        upload_time_utc_ms: None,
                        url: FileLocation::AbsoluteUrl(UrlString::from(url)),
                        yanked: None,
                    }),
                    ext: SourceDistExtension::TarGz,
                    index: package.index(root)?.ok_or("fixture index is missing")?,
                    wheels: Vec::new(),
                    size_is_authoritative: false,
                }))
            }
            Source::Directory(path) | Source::Editable(path) | Source::Virtual(path) => {
                let install_path = root.join(path);
                Dist::Source(SourceDist::Directory(DirectorySourceDist {
                    name: name.clone(),
                    url: VerbatimUrl::from_absolute_path(&install_path)?
                        .with_force_relative(path.is_relative()),
                    install_path: install_path.into_boxed_path(),
                    editable: Some(matches!(&package.id.source, Source::Editable(_))),
                    r#virtual: Some(matches!(&package.id.source, Source::Virtual(_))),
                    first_party: FirstParty::Yes,
                }))
            }
            Source::Git(..) | Source::Direct(..) | Source::Path(..) => {
                return Err("unsupported fixture source".into());
            }
        };
        Ok(AnnotatedDist {
            dist: ResolvedDist::Installable {
                dist: Arc::new(dist),
                version: Some(version.clone()),
            },
            name,
            version,
            extra: None,
            group: None,
            hashes: HashDigests::empty(),
            metadata: None,
            marker,
        })
    }

    fn manifest_lock() -> TestResult<Lock> {
        let definitions: WorkspaceAxes = toml::from_str(
            r#"
[python]
py312 = { members = ["legacy"], requires-python = "==3.12.*" }
py313 = { members = ["modern"], requires-python = "==3.13.*" }
"#,
        )?;
        let members = ["legacy", "modern", "shared"]
            .into_iter()
            .map(str::parse)
            .collect::<Result<BTreeSet<PackageName>, _>>()?;
        let axes = ResolvedWorkspaceAxes::from_parts(definitions, members)?;
        let root = std::env::current_dir()?;
        let mut cohorts = Vec::new();
        for (section, member, python) in [
            ("py312", "legacy", "==3.12.*"),
            ("py313", "modern", "==3.13.*"),
        ] {
            let mut text = format!(
                r#"
version = 1
requires-python = {python:?}
[manifest]
members = [{member:?}, "shared"]
overrides = [{{ name = "override-root", specifier = "==2.0.0", marker = "python_version >= '3.13'" }}]
[manifest.dependency-groups]
tooling = [
    {{ name = "legacy", specifier = "==0.1.0", index = "https://pypi.org/simple", marker = "python_version >= '3.13'" }},
    {{ name = "override-root", specifier = "==1.0.0", marker = "python_version >= '3.13'" }},
]
[[package]]
name = {member:?}
version = "0.1.0"
source = {{ virtual = {member:?} }}
[[package]]
name = "shared"
version = "0.1.0"
source = {{ virtual = "shared" }}
"#
            );
            if section == "py313" {
                text.push_str(
                    r#"
[[package]]
name = "legacy"
version = "0.1.0"
source = { registry = "https://pypi.org/simple" }
[[package]]
name = "override-root"
version = "2.0.0"
source = { registry = "https://pypi.org/simple" }
"#,
                );
            }
            let lock = Lock::from_toml(&text)?;
            assert!(!lock.has_authoritative_workspace_axis_manifest_roots());
            let resolution = direct_resolution(&lock, &root)?;
            let lock = lock.with_authoritative_workspace_axis_manifest_roots(&resolution, &root)?;
            assert!(lock.has_authoritative_workspace_axis_manifest_roots());
            let selection =
                WorkspaceAxisSelection::from_assignments([format!("python={section}").parse()?])?;
            let domain = axes
                .domain()
                .restrict(&selection)
                .ok_or("fixture domain is missing")?;
            cohorts.push((domain, lock));
        }
        Ok(Lock::from_workspace_axes(axes, cohorts)?.ok_or("fixture axes lock is missing")?)
    }

    #[test]
    fn workspace_axis_manifest_roots_preserve_sources_and_overrides() -> TestResult {
        let lock = manifest_lock()?;
        let serialized = lock.to_toml()?;
        let parsed = Lock::from_toml(&serialized)?;
        assert_eq!(parsed.to_toml()?, serialized);
        assert_eq!(
            Lock::from_canonical_toml(&serialized)?.to_toml()?,
            serialized
        );
        let axes = parsed.workspace_axes().ok_or("missing axes")?;
        assert!(axes.contexts[0].manifest_roots.0.iter().all(|root| {
            root.resolutions
                .iter()
                .all(|resolution| resolution.environment.is_false())
        }));

        let selected = parsed.select_workspace_axes_for_command(
            &WorkspaceAxisSelection::default(),
            &["modern".parse()?].into_iter().collect(),
            None,
            &ExtrasSpecification::default().with_defaults(DefaultExtras::default()),
            &DependencyGroups::from_group("tooling".parse()?)
                .with_defaults(DefaultGroups::default()),
        )?;
        let dependencies = selected
            .workspace_axis_manifest_dependencies()?
            .ok_or("missing authoritative roots")?;
        let legacy = dependencies
            .iter()
            .find(|dependency| dependency.package.name().as_str() == "legacy")
            .ok_or("missing registry root")?;
        assert!(matches!(legacy.package.id.source, Source::Registry(_)));
        let override_root = dependencies
            .iter()
            .find(|dependency| dependency.package.name().as_str() == "override-root")
            .ok_or("missing overridden root")?;
        assert_eq!(
            override_root
                .package
                .version()
                .map(ToString::to_string)
                .as_deref(),
            Some("2.0.0")
        );
        assert!(
            !Lock::from_toml(&selected.to_toml()?)?
                .has_authoritative_workspace_axis_manifest_roots()
        );
        Ok(())
    }

    fn active_resolution<'a>(
        document: &'a mut toml::Value,
        name: &str,
    ) -> TestResult<&'a mut toml::Value> {
        let contexts = document["workspace-axes"]["context"]
            .as_array_mut()
            .ok_or("missing contexts")?;
        let roots = contexts[1]["manifest-roots"]
            .as_array_mut()
            .ok_or("missing manifest roots")?;
        let root = roots
            .iter_mut()
            .find(|root| root["requirement"]["name"].as_str() == Some(name))
            .ok_or("missing manifest root")?;
        Ok(&mut root["resolutions"][0])
    }

    #[test]
    fn workspace_axis_reader_rejects_corrupt_manifest_roots() -> TestResult {
        let serialized = manifest_lock()?.to_toml()?;
        let mut document: toml::Value = toml::from_str(&serialized)?;
        document["workspace-axes"]["context"][0]["manifest-roots"]
            .as_array_mut()
            .ok_or("missing roots")?
            .pop();
        let error = Lock::from_toml(&toml::to_string(&document)?)
            .expect_err("missing original declaration");
        assert_snapshot!(error.to_string(), @"Invalid workspace resolution axes in lockfile: invalid manifest roots: manifest-root declaration keys do not match the cohort manifest");

        let mut document: toml::Value = toml::from_str(&serialized)?;
        active_resolution(&mut document, "override-root")?["requirement"]["specifier"] =
            toml::Value::String("==1.0.0".to_owned());
        let error =
            Lock::from_toml(&toml::to_string(&document)?).expect_err("changed effective override");
        assert_snapshot!(error.to_string(), @"Invalid workspace resolution axes in lockfile: invalid manifest roots: effective root keys for `override-root` do not match its overrides");

        let mut document: toml::Value = toml::from_str(&serialized)?;
        active_resolution(&mut document, "override-root")?["targets"] =
            toml::Value::Array(Vec::new());
        let error =
            Lock::from_toml(&toml::to_string(&document)?).expect_err("missing resolved target");
        assert_snapshot!(error.to_string(), @"Invalid workspace resolution axes in lockfile: invalid manifest roots: resolved targets do not cover the activation of `override-root`");

        let mut document: toml::Value = toml::from_str(&serialized)?;
        let source: toml::Value = toml::from_str("virtual = 'legacy'")?;
        active_resolution(&mut document, "legacy")?["targets"][0]["package"]["source"] = source;
        let error = Lock::from_toml(&toml::to_string(&document)?)
            .expect_err("different existing source identity");
        assert_snapshot!(error.to_string(), @"Invalid workspace resolution axes in lockfile: invalid manifest roots: manifest root `legacy==0.1.0 @ virtual+legacy` does not satisfy its effective requirement");
        Ok(())
    }
}
