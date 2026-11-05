//! The compact build layout stores one package row per identity and overlays build-specific views.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use rustc_hash::FxHashMap;
use serde::Deserialize;

use uv_distribution_types::{RequiresPython, SimplifiedMarkerTree, satisfies_require_hashes};
use uv_macros::DeserializeUntagged;
use uv_normalize::{DefaultGroups, ExtraName, GroupName, PackageName};
use uv_pep508::MarkerTree;
use uv_pypi_types::HashDigest;
use uv_resolver_types::UniversalMarker;

use super::super::{
    COMPACT_BUILD_LOCK_VERSION, Dependency, DependencyWire, GroupMetadata, Lock, LockError,
    LockWire, Package, PackageId, PackageMetadata, PackageWire, Source, SourceDist, VERSION, Wheel,
};
use super::{
    BuildEncoding, BuildInput, BuildLocks, OrdinaryOwner, Universe, closure, error, missing,
    sort_universes,
};

/// Resolve references against distinct identities, even when several builds use the same row.
#[derive(Default)]
pub(in crate::lock) struct Identities<'a> {
    by_name: BTreeMap<&'a PackageName, Vec<&'a PackageId>>,
}

impl<'a> Identities<'a> {
    pub(in crate::lock) fn new(ids: impl IntoIterator<Item = &'a PackageId>) -> Self {
        let mut identities = Self::default();
        for id in ids {
            identities.by_name.entry(&id.name).or_default().push(id);
        }
        identities
    }

    pub(in crate::lock) fn named(&self, name: &PackageName) -> &[&'a PackageId] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    fn unique(&self) -> FxHashMap<PackageName, PackageId> {
        self.by_name
            .iter()
            .filter_map(|(name, ids)| {
                let [id] = ids.as_slice() else { return None };
                Some(((*name).clone(), (*id).clone()))
            })
            .collect()
    }

    fn resolve(&self, id: &QualifiedReference) -> Result<&'a PackageId, LockError> {
        let version = match &id.version {
            Some(ValueOrFalse::Value(version)) => Some(Some(version)),
            Some(ValueOrFalse::Clear(false)) => Some(None),
            Some(ValueOrFalse::Clear(true)) => {
                return Err(error("An unversioned package reference must use false"));
            }
            None => None,
        };
        let mut matches = self.named(&id.name).iter().copied().filter(|candidate| {
            version.is_none_or(|version| candidate.version.as_ref() == version)
                && id
                    .source
                    .as_ref()
                    .is_none_or(|source| &candidate.source == source)
        });
        match (matches.next(), matches.next()) {
            (Some(id), None) => Ok(id),
            (None, _) => Err(error(format!(
                "Unknown locked package reference `{}`",
                id.name
            ))),
            _ => Err(error(format!(
                "Ambiguous locked package reference `{}`",
                id.name
            ))),
        }
    }
}

#[derive(Clone, Debug, DeserializeUntagged)]
#[serde(
    untagged,
    expecting = "a package name or a qualified package reference"
)]
enum Reference {
    #[untagged(string)]
    Name(PackageName),
    #[untagged(map)]
    Qualified(QualifiedReference),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QualifiedReference {
    name: PackageName,
    version: Option<ValueOrFalse<uv_pep440::Version>>,
    source: Option<Source>,
    #[serde(default)]
    extra: BTreeSet<ExtraName>,
    #[serde(default)]
    marker: SimplifiedMarkerTree,
}

impl Reference {
    fn qualified(self) -> QualifiedReference {
        match self {
            Self::Name(name) => QualifiedReference {
                name,
                version: None,
                source: None,
                extra: BTreeSet::new(),
                marker: SimplifiedMarkerTree::default(),
            },
            Self::Qualified(reference) => reference,
        }
    }

    fn resolve(self, identities: &Identities<'_>) -> Result<PackageId, LockError> {
        let reference = self.qualified();
        if !reference.extra.is_empty() || !reference.marker.as_simplified_marker_tree().is_true() {
            return Err(error(format!(
                "Unexpected edge fields on package reference `{}`",
                reference.name
            )));
        }
        identities.resolve(&reference).cloned()
    }

    fn root(
        self,
        identities: &Identities<'_>,
        python: &RequiresPython,
    ) -> Result<Dependency, LockError> {
        let reference = self.qualified();
        let id = identities.resolve(&reference)?.clone();
        Ok(Dependency::new(
            python,
            id,
            reference.extra,
            reference.marker,
        ))
    }
}

#[derive(Clone, Debug, DeserializeUntagged)]
#[serde(
    untagged,
    expecting = "build inputs or false for an unrecorded declaration"
)]
enum Inputs {
    #[untagged(seq)]
    Known(BTreeSet<BuildInput>),
    #[untagged(bool)]
    Missing(bool),
}

impl Inputs {
    fn recorded(self) -> Result<Option<BTreeSet<BuildInput>>, LockError> {
        match self {
            Self::Known(inputs) => Ok(Some(inputs)),
            Self::Missing(false) => Ok(None),
            Self::Missing(true) => Err(error("Missing build inputs must be written as false")),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::lock) struct ClusterWire {
    #[serde(default)]
    marker: MarkerTree,
    roots: Vec<Reference>,
    owners: Vec<Reference>,
    inputs: Option<Inputs>,
    #[serde(default, rename = "owner")]
    owner_overrides: Vec<OwnerOverride>,
    #[serde(default, rename = "package")]
    packages: Vec<PackageOverride>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerOverride {
    #[serde(rename = "ref")]
    owner: Reference,
    marker: Option<MarkerTree>,
    inputs: Option<Inputs>,
}

/// Absent fields inherit from the common row. An explicit empty list replaces a nonempty list.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct PackageOverride {
    #[serde(rename = "ref")]
    package: Reference,
    sdist: Option<ValueOrFalse<SourceDist>>,
    wheels: Option<Vec<WheelReference>>,
    #[serde(rename = "resolution-markers")]
    fork_markers: Option<Vec<SimplifiedMarkerTree>>,
    dependencies: Option<Vec<DependencyWire>>,
    optional_dependencies: Option<BTreeMap<ExtraName, Vec<DependencyWire>>>,
    #[serde(rename = "dev-dependencies")]
    dependency_groups: Option<BTreeMap<GroupName, Vec<DependencyWire>>>,
    default_groups: Option<ValueOrFalse<DefaultGroups>>,
    group_requires_python: Option<BTreeMap<GroupName, GroupMetadata>>,
    metadata: Option<PackageMetadata>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum ValueOrFalse<T> {
    Value(T),
    Clear(bool),
}

impl<T> ValueOrFalse<T> {
    fn apply(self, value: &mut Option<T>) -> Result<(), LockError> {
        *value = match self {
            Self::Value(value) => Some(value),
            Self::Clear(false) => None,
            Self::Clear(true) => {
                return Err(error("An omitted package field must be written as false"));
            }
        };
        Ok(())
    }
}

#[derive(Clone, Debug, DeserializeUntagged)]
#[serde(untagged, expecting = "a wheel filename or a wheel record")]
enum WheelReference {
    #[untagged(string)]
    Filename(String),
    #[untagged(map)]
    Record(Wheel),
}

impl PackageOverride {
    fn apply(self, mut row: PackageWire) -> Result<PackageWire, LockError> {
        if let Some(sdist) = self.sdist {
            sdist.apply(&mut row.sdist)?;
        }
        if let Some(wheels) = self.wheels {
            row.wheels = wheels
                .into_iter()
                .map(|wheel| match wheel {
                    WheelReference::Record(wheel) => Ok(wheel),
                    WheelReference::Filename(filename) => {
                        let mut matches = row
                            .wheels
                            .iter()
                            .filter(|wheel| wheel.filename.to_string() == filename);
                        match (matches.next(), matches.next()) {
                            (Some(wheel), None) => Ok(wheel.clone()),
                            _ => Err(error(format!(
                                "Expected one common wheel `{filename}` for `{}`",
                                row.id
                            ))),
                        }
                    }
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(markers) = self.fork_markers {
            row.fork_markers = markers;
        }
        if let Some(deps) = self.dependencies {
            row.dependencies = deps;
        }
        if let Some(deps) = self.optional_dependencies {
            row.optional_dependencies = deps;
        }
        if let Some(deps) = self.dependency_groups {
            row.dependency_groups = deps;
        }
        if let Some(groups) = self.default_groups {
            groups.apply(&mut row.default_groups)?;
        }
        if let Some(groups) = self.group_requires_python {
            row.group_requires_python = groups;
        }
        if let Some(metadata) = self.metadata {
            row.metadata = metadata;
        }
        check_row(&row)?;
        Ok(row)
    }
}

fn check_row(row: &PackageWire) -> Result<(), LockError> {
    if row.build_roots.is_some()
        || row.build_inputs.is_some()
        || !row.universe.is_empty()
        || row
            .dependencies
            .iter()
            .chain(row.optional_dependencies.values().flatten())
            .chain(row.dependency_groups.values().flatten())
            .any(|dep| !dep.universe.is_empty())
    {
        return Err(error(format!(
            "Legacy build scope on compact package `{}`",
            row.id
        )));
    }
    Ok(())
}

pub(in crate::lock) fn decode(mut wire: LockWire) -> Result<Lock, LockError> {
    if wire.build_encoding.is_some() || !wire.build_universes.is_empty() {
        return Err(error("Legacy build scope on compact lock"));
    }

    let mut rows = BTreeMap::new();
    let mut runtime = Vec::new();
    for row in std::mem::take(&mut wire.packages) {
        check_row(&row)?;
        if !row.build_only {
            runtime.push(row.clone());
        }
        let id = row.id.clone();
        if rows.insert(id.clone(), row).is_some() {
            return Err(error(format!("Duplicate compact package `{id}`")));
        }
    }

    let identities = Identities::new(rows.keys());
    let names = identities.unique();
    let descriptors = std::mem::take(&mut wire.build_clusters);
    wire.packages = runtime;
    wire.version = VERSION;
    let mut lock = Lock::try_from(wire)?;
    let project = lock.build_project_domain();
    let environment = SimplifiedMarkerTree::default();
    let default = UniversalMarker::from_combined(environment.into_marker(&lock.requires_python));
    let unwire = |row: PackageWire| -> Result<Package, LockError> {
        let mut package = row.unwire(&lock.requires_python, environment, default, &names)?;
        package.normalize_dependencies()?;
        Ok(package)
    };
    let mut universes = Vec::new();
    let mut coverage = BTreeMap::<PackageId, MarkerTree>::new();
    for (index, descriptor) in descriptors.into_iter().enumerate() {
        if descriptor.owners.is_empty() {
            return Err(error("Empty locked build cluster"));
        }
        let roots = descriptor
            .roots
            .into_iter()
            .map(|root| root.root(&identities, &lock.requires_python))
            .collect::<Result<Vec<_>, _>>()?;
        let mut patches = BTreeMap::new();
        for patch in descriptor.packages {
            let id = patch.package.clone().resolve(&identities)?;
            let row = patch.apply(rows[&id].clone())?;
            if patches.insert(id.clone(), row).is_some() {
                return Err(error(format!(
                    "Duplicate compact package override for `{id}`"
                )));
            }
        }
        let packages = closure(&roots, |id| {
            unwire(
                patches
                    .remove(id)
                    .or_else(|| rows.get(id).cloned())
                    .ok_or_else(|| missing(id))?,
            )
        })?;
        if let Some((id, _)) = patches.first_key_value() {
            return Err(error(format!(
                "Unreachable compact package override for `{id}`"
            )));
        }
        let inputs = descriptor
            .inputs
            .map_or(Ok(Some(BTreeSet::new())), Inputs::recorded)?;
        let mut overrides = BTreeMap::new();
        for owner in descriptor.owner_overrides {
            let id = owner.owner.clone().resolve(&identities)?;
            if overrides.insert(id.clone(), owner).is_some() {
                return Err(error(format!(
                    "Duplicate compact owner override for `{id}`"
                )));
            }
        }
        for owner in descriptor.owners {
            let owner = owner.resolve(&identities)?;
            let owner_override = overrides.remove(&owner);
            let marker = owner_override
                .as_ref()
                .and_then(|owner| owner.marker)
                .unwrap_or(descriptor.marker);
            let domain = marker.and(project);
            if domain.is_false() || !domain.is_disjoint(descriptor.marker.negate()) {
                return Err(error(format!("Invalid compact owner marker for `{owner}`")));
            }
            let seen = coverage.entry(owner.clone()).or_insert(MarkerTree::FALSE);
            if !seen.is_disjoint(domain) {
                return Err(error(format!(
                    "Overlapping compact build coverage for `{owner}`"
                )));
            }
            *seen = seen.or(domain);
            let inputs = match owner_override.and_then(|owner| owner.inputs) {
                Some(inputs) => inputs.recorded()?,
                None => inputs.clone(),
            };
            let universe = Universe {
                cluster: Some(index),
                declaration: None,
                ordinary_owner: None,
                owner,
                marker: UniversalMarker::from_combined(marker),
                roots: roots.clone(),
                inputs,
                packages: packages.clone(),
            };
            // A narrower owner must see only the branches reachable in its own domain.
            universes.push(if marker == descriptor.marker {
                universe
            } else {
                universe.project(domain, &lock.requires_python)?
            });
        }
        if let Some((owner, _)) = overrides.first_key_value() {
            return Err(error(format!(
                "Unknown compact owner override for `{owner}`"
            )));
        }
    }
    let mut unreachable: BTreeSet<_> = rows
        .values()
        .filter(|row| row.build_only)
        .map(|row| &row.id)
        .collect();
    for id in universes.iter().flat_map(|owner| owner.packages.keys()) {
        unreachable.remove(id);
    }
    if let Some(id) = unreachable.first() {
        return Err(error(format!(
            "Unreachable compact build-only package `{id}`"
        )));
    }
    sort_universes(&mut universes);
    lock.version = COMPACT_BUILD_LOCK_VERSION;
    lock.builds = Some(BuildLocks {
        encoding: BuildEncoding::Dependency,
        universes,
    });
    lock.with_ordinary_build_roots()
}

/// Every environment selecting a registry owner must agree on the digest of its source archive.
/// Wheels and archive locations can differ across views of that package.
pub(super) fn bind_owners(
    runtime: &[Package],
    universes: &mut [Universe],
) -> Result<(), LockError> {
    let mut archives: BTreeMap<_, Option<HashDigest>> = universes
        .iter()
        .filter(|universe| matches!(universe.owner.source, Source::Registry(_)))
        .map(|universe| (universe.owner.clone(), None))
        .collect();
    for package in runtime.iter().chain(
        universes
            .iter()
            .flat_map(|universe| universe.packages.values()),
    ) {
        if let Some(known) = archives.get_mut(&package.id)
            && let Some(hash) = package
                .sdist
                .as_ref()
                .and_then(SourceDist::hash)
                .filter(|hash| satisfies_require_hashes(hash))
        {
            if known.as_ref().is_some_and(|known| known != hash) {
                return Err(error(format!(
                    "Ambiguous source archive digests for compact build owner `{}`",
                    package.id
                )));
            }
            known.get_or_insert_with(|| hash.clone());
        }
    }
    for universe in universes {
        universe.ordinary_owner = OrdinaryOwner::new(&universe.owner, None).or_else(|| {
            archives
                .get(&universe.owner)
                .and_then(|hash| hash.as_ref())
                .cloned()
                .map(OrdinaryOwner::Archive)
        });
    }
    Ok(())
}

/// Combine disjoint solves when a marker projection can reconstruct every owner's graph.
/// Different package artifacts require separate scopes.
pub(super) fn factor_domains(
    universes: &mut Vec<Universe>,
    project: MarkerTree,
    python: &RequiresPython,
) -> Result<(), LockError> {
    let mut clusters = BTreeMap::<_, Vec<_>>::new();
    for universe in std::mem::take(universes) {
        clusters.entry(universe.cluster).or_default().push(universe);
    }
    let mut families = BTreeMap::<Vec<PackageId>, Vec<Vec<Universe>>>::new();
    for cluster in clusters.into_values() {
        let owners = cluster.iter().map(|owner| owner.owner.clone()).collect();
        let family = families.entry(owners).or_default();
        let mut merged = None;
        for (index, previous) in family.iter().enumerate() {
            if let Some(combined) = combine_domains(previous, &cluster, project, python)? {
                merged = Some((index, combined));
                break;
            }
        }
        if let Some((index, combined)) = merged {
            family[index] = combined;
        } else {
            family.push(cluster);
        }
    }
    let mut by_roots = BTreeMap::<Vec<Dependency>, Vec<Vec<Universe>>>::new();
    for cluster in families.into_values().flatten() {
        let Some(first) = cluster.first() else {
            continue;
        };
        let candidates = by_roots.entry(first.roots.clone()).or_default();
        let mut merged = false;
        for candidate in candidates.iter_mut() {
            if let Some(combined) = combine_rosters(candidate, &cluster, project, python)? {
                *candidate = combined;
                merged = true;
                break;
            }
        }
        if !merged {
            candidates.push(cluster);
        }
    }
    universes.extend(by_roots.into_values().flatten().flatten().map(|mut owner| {
        owner.declaration = None;
        owner
    }));
    sort_universes(universes);
    Ok(())
}

fn combine_domains(
    left: &[Universe],
    right: &[Universe],
    project: MarkerTree,
    python: &RequiresPython,
) -> Result<Option<Vec<Universe>>, LockError> {
    let (Some(first), Some(second)) = (left.first(), right.first()) else {
        return Ok(None);
    };
    // The same owner identities can select different direct names when configured build inputs
    // match runtime. Keep those declaration families separate.
    if left.iter().zip(right).any(|(left, right)| {
        left.inputs != right.inputs
            || left.declaration.is_none()
            || left
                .declaration
                .as_ref()
                .map(|declaration| &declaration.names)
                != right
                    .declaration
                    .as_ref()
                    .map(|declaration| &declaration.names)
    }) || first.marker.has_conflict_marker()
        || second.marker.has_conflict_marker()
    {
        return Ok(None);
    }
    let first_domain = first.marker.combined().and(project);
    let second_domain = second.marker.combined().and(project);
    if !first_domain.is_disjoint(second_domain)
        || left.iter().any(|owner| {
            owner.marker != first.marker
                || owner.roots != first.roots
                || owner.packages != first.packages
        })
        || right.iter().any(|owner| {
            owner.marker != second.marker
                || owner.roots != second.roots
                || owner.packages != second.packages
        })
    {
        return Ok(None);
    }
    let domain = first_domain.or(second_domain);
    let roots = combine_edges(
        &first.roots,
        first_domain,
        &second.roots,
        second_domain,
        domain,
        python,
    );
    let ids: BTreeSet<_> = first
        .packages
        .keys()
        .chain(second.packages.keys())
        .collect();
    let mut packages = BTreeMap::new();
    for id in ids {
        let package = match (first.packages.get(id), second.packages.get(id)) {
            (Some(first), Some(second)) => {
                let Some(package) =
                    combine_package(first, first_domain, second, second_domain, domain, python)
                else {
                    return Ok(None);
                };
                package
            }
            (Some(package), None) => gated_package(package, first_domain, python),
            (None, Some(package)) => gated_package(package, second_domain, python),
            (None, None) => return Err(missing(id)),
        };
        packages.insert(id.clone(), package);
    }
    let mut representative = first.clone();
    representative.marker = UniversalMarker::from_combined(domain.restrict(project));
    representative.roots = roots;
    representative.packages = packages;
    for (original, domain) in [(first, first_domain), (second, second_domain)] {
        let projected = representative.clone().project(domain, python)?;
        if projected.roots != original.roots || projected.packages != original.packages {
            return Ok(None);
        }
    }
    Ok(Some(
        left.iter()
            .zip(right)
            .map(|(left, right)| Universe {
                owner: left.owner.clone(),
                inputs: left.inputs.clone(),
                declaration: left
                    .declaration
                    .clone()
                    .zip(right.declaration.as_ref())
                    .map(|(mut left, right)| {
                        left.representative &= right.representative;
                        if left.requirements != right.requirements {
                            left.requirements = None;
                        }
                        left
                    }),
                ..representative.clone()
            })
            .collect(),
    ))
}

fn combine_rosters(
    left: &[Universe],
    right: &[Universe],
    project: MarkerTree,
    python: &RequiresPython,
) -> Result<Option<Vec<Universe>>, LockError> {
    let Some(graph) = left.first() else {
        return Ok(None);
    };
    let left_marker = roster_marker(left.iter());
    let right_marker = roster_marker(right.iter());
    if left_marker.has_conflict_marker()
        || right_marker.has_conflict_marker()
        || !left_marker
            .combined()
            .and(project)
            .is_disjoint(right_marker.combined())
        || left
            .iter()
            .chain(right)
            .any(|owner| !same_graph(graph, owner))
    {
        return Ok(None);
    }

    let mut owners: BTreeMap<_, _> = left
        .iter()
        .map(|owner| (&owner.owner, (owner, None)))
        .collect();
    let mut shared = 0;
    let mut common_representative = false;
    for owner in right {
        let (other, second) = match owners.entry(&owner.owner) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                entry.insert((owner, None));
                continue;
            }
        };
        let Some((left, right)) = other.declaration.as_ref().zip(owner.declaration.as_ref()) else {
            return Ok(None);
        };
        if other.inputs != owner.inputs
            || left.requirements.is_none()
            || left.requirements != right.requirements
        {
            return Ok(None);
        }
        shared += 1;
        common_representative |= left.representative && right.representative;
        *second = Some(owner);
    }
    // A superset already accepted the whole roster. Incomparable rosters need a common
    // representative, whose unchanged declarations cover both accepted solves.
    if shared != left.len().min(right.len()) && !common_representative {
        return Ok(None);
    }

    let mut marker = left_marker;
    marker.or(right_marker);
    let mut has_full_owner = false;
    let mut projected = BTreeSet::new();
    let mut combined = Vec::with_capacity(owners.len());
    for (owner, other) in owners.into_values() {
        let mut owner = owner.clone();
        let mut representative = false;
        if let Some(other) = other {
            owner.marker.or(other.marker);
            representative = other
                .declaration
                .as_ref()
                .is_some_and(|declaration| declaration.representative);
        }
        if let Some(declaration) = &mut owner.declaration {
            declaration.representative &= representative;
        }
        if owner.marker == marker {
            has_full_owner = true;
        } else if projected.insert(owner.marker)
            && !same_graph(
                graph,
                &graph
                    .clone()
                    .project(owner.marker.combined().and(project), python)?,
            )
        {
            return Ok(None);
        }
        owner.cluster = graph.cluster;
        combined.push(owner);
    }
    Ok(has_full_owner.then_some(combined))
}

fn roster_marker<'a>(owners: impl IntoIterator<Item = &'a Universe>) -> UniversalMarker {
    let mut marker = UniversalMarker::FALSE;
    for owner in owners {
        marker.or(owner.marker);
    }
    marker
}

fn same_graph(left: &Universe, right: &Universe) -> bool {
    left.roots == right.roots && left.packages == right.packages
}

/// A shared descriptor can use a full-domain owner's graph if every narrower owner's view
/// can be reconstructed by the reader. This also recognizes such descriptors after loading.
pub(in crate::lock) fn shared_view<'a>(
    owners: &[&'a Universe],
    project: MarkerTree,
    python: &RequiresPython,
) -> Result<Option<&'a Universe>, LockError> {
    let marker = roster_marker(owners.iter().copied());
    let Some(graph) = owners.iter().copied().find(|owner| owner.marker == marker) else {
        return Ok(None);
    };
    if owners.iter().all(|owner| owner.marker == marker) {
        return Ok(None);
    }
    let mut projected = BTreeMap::new();
    for owner in owners {
        let view = if owner.marker == marker {
            graph
        } else {
            match projected.entry(owner.marker) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(
                    graph
                        .clone()
                        .project(owner.marker.combined().and(project), python)?,
                ),
            }
        };
        if !same_graph(view, owner) {
            return Ok(None);
        }
    }
    Ok(Some(graph))
}

fn gated_package(package: &Package, domain: MarkerTree, python: &RequiresPython) -> Package {
    let mut gated = package.clone();
    gated.fork_markers = gated_forks(package, domain, python);
    gated
}

fn gated_forks(
    package: &Package,
    domain: MarkerTree,
    python: &RequiresPython,
) -> Vec<UniversalMarker> {
    let normalize = |marker| {
        UniversalMarker::from_combined(
            SimplifiedMarkerTree::new(python, marker).into_marker(python),
        )
    };
    if package.fork_markers.is_empty() {
        vec![normalize(domain)]
    } else {
        package
            .fork_markers
            .iter()
            .map(|marker| marker.combined().and(domain))
            .filter(|marker| !marker.is_false())
            .map(normalize)
            .collect()
    }
}

fn combine_package(
    first: &Package,
    first_domain: MarkerTree,
    second: &Package,
    second_domain: MarkerTree,
    domain: MarkerTree,
    python: &RequiresPython,
) -> Option<Package> {
    if first.sdist != second.sdist
        || first.wheels != second.wheels
        || first.metadata != second.metadata
        || first.default_groups != second.default_groups
        || first.group_requires_python != second.group_requires_python
        || !first
            .optional_dependencies
            .keys()
            .eq(second.optional_dependencies.keys())
        || !first
            .dependency_groups
            .keys()
            .eq(second.dependency_groups.keys())
    {
        return None;
    }
    let mut package = first.clone();
    if !first.fork_markers.is_empty() || !second.fork_markers.is_empty() {
        package.fork_markers = gated_forks(first, first_domain, python);
        package
            .fork_markers
            .extend(gated_forks(second, second_domain, python));
        package.fork_markers.sort_unstable();
        package.fork_markers.dedup();
    }
    let combine = |first: &[Dependency], second: &[Dependency]| {
        combine_edges(first, first_domain, second, second_domain, domain, python)
    };
    package.dependencies = combine(&first.dependencies, &second.dependencies);
    for (extra, edges) in &mut package.optional_dependencies {
        *edges = combine(edges, &second.optional_dependencies[extra]);
    }
    for (group, edges) in &mut package.dependency_groups {
        *edges = combine(edges, &second.dependency_groups[group]);
    }
    Some(package)
}

fn combine_edges(
    first: &[Dependency],
    first_domain: MarkerTree,
    second: &[Dependency],
    second_domain: MarkerTree,
    domain: MarkerTree,
    python: &RequiresPython,
) -> Vec<Dependency> {
    let mut markers = BTreeMap::<_, MarkerTree>::new();
    for (edges, scope) in [(first, first_domain), (second, second_domain)] {
        for edge in edges {
            let active = edge.complexified_marker.combined().and(scope);
            let marker = markers
                .entry((&edge.package_id, &edge.extra))
                .or_insert(MarkerTree::FALSE);
            *marker = marker.or(active);
        }
    }
    markers
        .into_iter()
        .filter_map(|((id, extra), marker)| {
            let marker = marker.restrict(domain);
            (!marker.is_false()).then(|| {
                Dependency::new(
                    python,
                    id.clone(),
                    extra.clone(),
                    SimplifiedMarkerTree::new(python, marker),
                )
            })
        })
        .collect()
}
