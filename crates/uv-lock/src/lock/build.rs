//! Locked build environments. Universe zero denotes the runtime graph. Each subsequent
//! universe has its own source package, marker domain and set of direct build requirements.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

use petgraph::Graph;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use uv_configuration::BuildOptions;
use uv_distribution_types::{
    Edge, FirstParty, Name, Node, Requirement, RequiresPython, Resolution, ResolvedDist,
    SimplifiedMarkerTree, SourceDist,
};
use uv_normalize::{ExtraName, PackageName};
use uv_pep508::{
    ExtraOperator, MarkerEnvironment, MarkerExpression, MarkerTree, MarkerValueExtra,
    MarkerValueVersion,
};
use uv_platform_tags::Tags;
use uv_resolver_types::UniversalMarker;

use super::build_inputs::BuildInput;
use super::{
    Dependency, DependencyWire, HashedDist, Lock, LockError, LockErrorKind, LockWire, Package,
    PackageId, PackageWire, RegistrySource, Source, TagPolicy, absolute_path,
};

/// The lockfile representation of differences between resolved build environments.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum BuildEncoding {
    ConflictMarker,
    Package,
    Dependency,
}

impl BuildEncoding {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::ConflictMarker => "conflict-marker",
            Self::Package => "package",
            Self::Dependency => "dependency",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BuildLocks {
    pub(super) encoding: BuildEncoding,
    pub(super) universes: Vec<Universe>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Universe {
    pub(super) owner: PackageId,
    pub(super) marker: UniversalMarker,
    pub(super) roots: Vec<Dependency>,
    pub(super) inputs: Option<BTreeSet<BuildInput>>,
    packages: BTreeMap<PackageId, Package>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct UniverseWire {
    owner: PackageId,
    #[serde(default)]
    marker: MarkerTree,
    #[serde(default)]
    roots: Vec<DependencyWire>,
    inputs: Option<BTreeSet<BuildInput>>,
}

fn error(reason: impl Into<String>) -> LockError {
    LockErrorKind::BuildGraph(reason.into()).into()
}

impl Lock {
    /// The supported environment, including exclusions within the Python range.
    pub(super) fn build_project_domain(&self) -> MarkerTree {
        let mut project =
            self.requires_python
                .specifiers()
                .iter()
                .fold(MarkerTree::TRUE, |domain, specifier| {
                    domain.and(MarkerTree::expression(MarkerExpression::Version {
                        key: MarkerValueVersion::PythonFullVersion,
                        specifier: specifier.clone(),
                    }))
                });
        if !self.supported_environments.is_empty() {
            let supported = self
                .supported_environments
                .iter()
                .copied()
                .fold(MarkerTree::FALSE, MarkerTree::or);
            project = project.and(supported);
        }
        project
    }

    /// Whether isolated builds must use the recorded build environments.
    pub fn locks_builds(&self) -> bool {
        self.builds.is_some()
    }

    /// Materialize a source build exclusively from the owner's locked roots and outgoing edges.
    pub fn resolve_build(
        &self,
        owner: &SourceDist,
        runtime: Option<&Resolution>,
        requirements: &[Requirement],
        root: &Path,
        markers: &MarkerEnvironment,
        tags: &Tags,
        options: &BuildOptions,
    ) -> Result<Resolution, LockError> {
        let environment = BuildMarkerEnvironment {
            markers,
            conflicts: runtime.map(Resolution::activated_conflicts),
        };
        let source = Source::from_source_dist(owner, root)?;
        let builds = self
            .builds
            .as_ref()
            .ok_or_else(|| error("Missing build lock"))?;
        let mut selected = None;
        for universe in &builds.universes {
            if universe.owner.name != *owner.name()
                || owner
                    .version()
                    .is_some_and(|v| universe.owner.version.as_ref() != Some(v))
            {
                continue;
            }
            // Deserializing a lock can change a file URL's preferred (absolute vs relative)
            // spelling. Editable installation also changes how a source tree is represented.
            let same_source = match (&source, &universe.owner.source) {
                (
                    Source::Registry(RegistrySource::Path(a)),
                    Source::Registry(RegistrySource::Path(b)),
                )
                | (Source::Path(a), Source::Path(b))
                | (
                    Source::Directory(a) | Source::Editable(a),
                    Source::Directory(b) | Source::Editable(b),
                ) => absolute_path(root, a)? == absolute_path(root, b)?,
                (a, b) => a == b,
            };
            if same_source
                && environment.matches(universe.marker)?
                && selected.replace(universe).is_some()
            {
                return Err(error(format!(
                    "Ambiguous locked build environment for `{}`",
                    owner.name()
                )));
            }
        }
        let universe = selected.ok_or_else(|| {
            error(format!(
                "No locked build environment for `{}`",
                owner.name()
            ))
        })?;
        universe.resolve(requirements, root, &environment, tags, options)
    }
}

fn missing(id: &PackageId) -> LockError {
    error(format!(
        "Missing package `{id}` in locked build environment"
    ))
}

/// Walk the universal closure, including each reachable marker branch.
fn closure(
    roots: &[Dependency],
    mut lookup: impl FnMut(&PackageId) -> Result<Package, LockError>,
) -> Result<BTreeMap<PackageId, Package>, LockError> {
    let mut packages = BTreeMap::new();
    let mut queue: Vec<_> = roots.iter().map(|d| d.package_id.clone()).collect();
    while let Some(id) = queue.pop() {
        if packages.contains_key(&id) {
            continue;
        }
        let package = lookup(&id)?;
        queue.extend(
            package
                .dependencies
                .iter()
                .chain(package.optional_dependencies.values().flatten())
                .chain(package.dependency_groups.values().flatten())
                .map(|dep| dep.package_id.clone()),
        );
        packages.insert(id, package);
    }
    Ok(packages)
}

/// Conflict items belong to the selected runtime environment, including during recursive builds.
struct BuildMarkerEnvironment<'a> {
    markers: &'a MarkerEnvironment,
    conflicts: Option<&'a [ExtraName]>,
}

impl BuildMarkerEnvironment<'_> {
    fn matches(&self, marker: UniversalMarker) -> Result<bool, LockError> {
        if let Some(conflicts) = self.conflicts {
            return Ok(marker.combined().evaluate(self.markers, conflicts));
        }
        // A universal lock freshness check may run before any runtime extras have been chosen.
        if marker.has_conflict_marker() && marker.pep508().evaluate(self.markers, &[]) {
            return Err(error(
                "Locked build requires a selection of conflicting runtime extras or groups",
            ));
        }
        Ok(marker.evaluate_no_extras(self.markers))
    }
}

impl Universe {
    /// Refresh mutable declarations before checking the registry sources they can select.
    pub(super) fn validation_packages(&self) -> impl Iterator<Item = &Package> {
        self.packages
            .values()
            .filter(|package| !package.id.source.is_immutable())
            .chain(
                self.packages
                    .values()
                    .filter(|package| package.id.source.is_immutable()),
            )
    }

    fn resolve(
        &self,
        requirements: &[Requirement],
        root: &Path,
        environment: &BuildMarkerEnvironment<'_>,
        tags: &Tags,
        options: &BuildOptions,
    ) -> Result<Resolution, LockError> {
        let mut graph = Graph::new();
        let root_index = graph.add_node(Node::Root);
        let mut queue = VecDeque::new();
        for req in requirements
            .iter()
            .filter(|req| req.evaluate_markers(Some(environment.markers), &[]))
        {
            let mut matched = false;
            for dep in &self.roots {
                if dep.package_id.name == req.name
                    && environment.matches(dep.complexified_marker)?
                    && req.extras.iter().all(|extra| dep.extra.contains(extra))
                    && Lock::package_satisfies_requirement(
                        &self.packages[&dep.package_id],
                        req,
                        root,
                    )?
                {
                    matched = true;
                }
            }
            if !matched {
                return Err(error(format!(
                    "Build requirement `{req}` is not locked for `{}`",
                    self.owner
                )));
            }
        }
        for dep in &self.roots {
            if environment.matches(dep.complexified_marker)? {
                queue.push_back((
                    root_index,
                    dep,
                    Edge::Prod,
                    dep.extra.iter().collect::<Vec<_>>(),
                ));
            }
        }
        let mut nodes = BTreeMap::new();
        let mut seen_extras = BTreeSet::new();
        while let Some((parent, edge, kind, extras)) = queue.pop_front() {
            let package = &self.packages[&edge.package_id];
            let index = match nodes.entry(&package.id.name) {
                std::collections::btree_map::Entry::Occupied(entry) => {
                    let (id, index) = *entry.get();
                    if id != &package.id {
                        return Err(error(format!(
                            "Multiple locked versions of `{}` selected for `{}`",
                            package.id.name, self.owner
                        )));
                    }
                    index
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let HashedDist { dist, hashes } = package.to_dist(
                        root,
                        TagPolicy::Required(tags),
                        options,
                        environment.markers,
                        FirstParty::No,
                    )?;
                    let node = graph.add_node(Node::Dist {
                        dist: ResolvedDist::Installable {
                            dist: Arc::new(dist),
                            version: package.id.version.clone(),
                        },
                        hashes,
                        install: true,
                    });
                    entry.insert((&package.id, node));
                    for dep in &package.dependencies {
                        if environment.matches(dep.complexified_marker)? {
                            queue.push_back((node, dep, Edge::Prod, dep.extra.iter().collect()));
                        }
                    }
                    node
                }
            };
            graph.add_edge(parent, index, kind);
            for extra in extras {
                if seen_extras.insert((&package.id, extra))
                    && let Some(deps) = package.optional_dependencies.get(extra)
                {
                    for dep in deps {
                        if environment.matches(dep.complexified_marker)? {
                            queue.push_back((
                                index,
                                dep,
                                Edge::Optional(extra.clone()),
                                dep.extra.iter().collect(),
                            ));
                        }
                    }
                }
            }
        }
        Ok(Resolution::new(graph))
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Section {
    Production,
    Extra(ExtraName),
    Group(uv_normalize::GroupName),
}

type ScopedEdges = BTreeMap<(Section, Dependency), Vec<usize>>;

pub(super) struct EncodedPackage {
    pub(super) package: Package,
    pub(super) universes: Vec<usize>,
    pub(super) build_only: bool,
    pub(super) edges: ScopedEdges,
}

impl BuildLocks {
    pub(super) fn packages(&self, runtime: &Lock) -> Vec<EncodedPackage> {
        let mut versions: BTreeMap<&PackageId, Vec<(usize, &Package)>> = BTreeMap::new();
        for package in &runtime.packages {
            versions.entry(&package.id).or_default().push((0, package));
        }
        for (index, universe) in self.universes.iter().enumerate() {
            for package in universe.packages.values() {
                versions
                    .entry(&package.id)
                    .or_default()
                    .push((index + 1, package));
            }
        }
        let mut encoded = Vec::new();
        for variants in versions.into_values() {
            // A package row can be shared wherever its artifacts and unscoped fields agree.
            let count = variants.len();
            let build_only = variants[0].0 != 0;
            let mut rows: Vec<EncodedPackage> = Vec::new();
            for (universe, variant) in variants {
                let mut package = variant.clone();
                let mut edges = Vec::new();
                if self.encoding != BuildEncoding::Package {
                    edges.extend(
                        package
                            .dependencies
                            .drain(..)
                            .map(|d| (Section::Production, d)),
                    );
                    for (extra, deps) in &mut package.optional_dependencies {
                        edges.extend(deps.drain(..).map(|d| (Section::Extra(extra.clone()), d)));
                    }
                    for (group, deps) in &mut package.dependency_groups {
                        edges.extend(deps.drain(..).map(|d| (Section::Group(group.clone()), d)));
                    }
                }
                let row = if let Some(row) = rows.iter_mut().find(|row| row.package == package) {
                    row
                } else {
                    rows.push(EncodedPackage {
                        package,
                        universes: Vec::new(),
                        build_only,
                        edges: BTreeMap::new(),
                    });
                    rows.last_mut().expect("just inserted a package")
                };
                row.universes.push(universe);
                for edge in edges {
                    row.edges.entry(edge).or_default().push(universe);
                }
            }
            for mut row in rows {
                let mut scoped = BTreeMap::new();
                for ((section, mut dep), universes) in std::mem::take(&mut row.edges) {
                    if universes != row.universes {
                        match self.encoding {
                            BuildEncoding::Package => {}
                            BuildEncoding::Dependency => {
                                scoped.insert((section.clone(), dep.clone()), universes);
                            }
                            BuildEncoding::ConflictMarker => {
                                let scope = universes
                                    .into_iter()
                                    .fold(MarkerTree::FALSE, |tree, index| {
                                        tree.or(universe_marker(index))
                                    });
                                dep.simplified_marker = SimplifiedMarkerTree::new(
                                    &runtime.requires_python,
                                    dep.complexified_marker.combined().and(scope),
                                );
                            }
                        }
                    }
                    match section {
                        Section::Production => row.package.dependencies.push(dep),
                        Section::Extra(extra) => row
                            .package
                            .optional_dependencies
                            .entry(extra)
                            .or_default()
                            .push(dep),
                        Section::Group(group) => row
                            .package
                            .dependency_groups
                            .entry(group)
                            .or_default()
                            .push(dep),
                    }
                }
                row.edges = scoped;
                if row.universes.len() == count {
                    row.universes.clear();
                }
                encoded.push(row);
            }
        }
        encoded
    }
}

fn universe_marker(index: usize) -> MarkerTree {
    let extra = ExtraName::from_owned(format!("uv-build-{index}")).expect("valid universe name");
    MarkerTree::expression(MarkerExpression::Extra {
        operator: ExtraOperator::Equal,
        name: MarkerValueExtra::Extra(extra),
    })
}

fn universe_index(extra: &ExtraName) -> Option<usize> {
    extra.as_ref().strip_prefix("uv-build-")?.parse().ok()
}

impl PackageWire {
    fn in_universe(&self, index: usize) -> bool {
        (!self.build_only || index != 0)
            && (self.universe.is_empty() || self.universe.contains(&index))
    }

    fn project(&self, index: usize, requires_python: &RequiresPython) -> Self {
        let mut package = self.clone();
        for deps in std::iter::once(&mut package.dependencies)
            .chain(package.optional_dependencies.values_mut())
            .chain(package.dependency_groups.values_mut())
        {
            deps.retain_mut(|dep| {
                if !dep.universe.is_empty() && !dep.universe.contains(&index) {
                    return false;
                }
                let marker = dep
                    .marker
                    .as_simplified_marker_tree()
                    .simplify_extras_with(|extra| universe_index(extra) == Some(index))
                    .simplify_not_extras_with(|extra| {
                        universe_index(extra).is_some_and(|other| other != index)
                    });
                if marker.is_false() {
                    return false;
                }
                dep.marker = SimplifiedMarkerTree::new(requires_python, marker);
                true
            });
        }
        package
    }
}

pub(super) fn decode(mut wire: LockWire, encoding: BuildEncoding) -> Result<Lock, LockError> {
    let descriptors = std::mem::take(&mut wire.build_universes);
    let mut names = FxHashMap::<PackageName, Option<PackageId>>::default();
    let mut rows: BTreeMap<PackageId, Vec<PackageWire>> = BTreeMap::new();
    for package in std::mem::take(&mut wire.packages) {
        names
            .entry(package.id.name.clone())
            .and_modify(|id| {
                if id.as_ref() != Some(&package.id) {
                    *id = None;
                }
            })
            .or_insert_with(|| Some(package.id.clone()));
        if package.in_universe(0) {
            wire.packages
                .push(package.project(0, &wire.requires_python));
        }
        rows.entry(package.id.clone()).or_default().push(package);
    }
    let names = names
        .into_iter()
        .filter_map(|(name, id)| Some((name, id?)))
        .collect();
    let mut runtime = Lock::try_from(wire)?;
    let environment = SimplifiedMarkerTree::default();
    let default = UniversalMarker::from_combined(environment.into_marker(&runtime.requires_python));
    let mut universes = Vec::new();
    for (index, descriptor) in descriptors.into_iter().enumerate() {
        let roots = descriptor
            .roots
            .into_iter()
            .map(|dep| dep.unwire(&runtime.requires_python, environment, default, &names))
            .collect::<Result<Vec<_>, _>>()?;
        let packages = closure(&roots, |id| {
            let mut candidates = rows
                .get(id)
                .into_iter()
                .flatten()
                .filter(|p| p.in_universe(index + 1));
            let row = candidates.next().ok_or_else(|| missing(id))?;
            if candidates.next().is_some() {
                return Err(error(format!(
                    "Duplicate package `{id}` in build universe {}",
                    index + 1
                )));
            }
            let mut package = row.project(index + 1, &runtime.requires_python).unwire(
                &runtime.requires_python,
                environment,
                default,
                &names,
            )?;
            package.normalize_dependencies()?;
            Ok(package)
        })?;
        universes.push(Universe {
            inputs: descriptor.inputs,
            owner: descriptor.owner,
            marker: UniversalMarker::from_combined(descriptor.marker),
            roots,
            packages,
        });
    }
    runtime.builds = Some(BuildLocks {
        encoding,
        universes,
    });
    Ok(runtime)
}
