//! Locked build environments. Universe zero denotes the runtime graph. Each subsequent
//! universe has its own source package, marker domain and set of direct build requirements.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

use futures::TryStreamExt;
use petgraph::Graph;
use petgraph::visit::EdgeRef;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use uv_client::RegistryClient;
use uv_configuration::BuildOptions;
use uv_distribution_types::{
    Edge, FirstParty, IndexLocations, Name, Node, Requirement, RequiresPython, Resolution,
    ResolvedDist, SimplifiedMarkerTree, SourceDist,
};
use uv_normalize::{ExtraName, PackageName};
use uv_pep508::{ExtraOperator, MarkerEnvironment, MarkerExpression, MarkerTree, MarkerValueExtra};
use uv_platform_tags::Tags;
use uv_redacted::SafeUrl;
use uv_resolver_types::{ResolutionGraphNode, ResolverOutput, UniversalMarker};

use super::{
    BUILD_LOCK_VERSION, Dependency, DependencyWire, HashedDist, Lock, LockError, LockErrorKind,
    LockWire, Package, PackageId, PackageWire, RegistrySource, ResolverManifest, Source, TagPolicy,
    absolute_path,
};

/// The lockfile representation of differences between resolved build environments.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuildEncoding {
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

/// A source package's part of a possibly shared build resolution.
pub struct BuildGraph<'a> {
    pub owner: &'a ResolvedDist,
    pub marker: UniversalMarker,
    pub requirements: &'a [Requirement],
    pub resolution: &'a ResolverOutput,
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
    packages: BTreeMap<PackageId, Package>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct UniverseWire {
    owner: PackageId,
    #[serde(default)]
    marker: MarkerTree,
    #[serde(default)]
    roots: Vec<DependencyWire>,
}

fn error(reason: impl Into<String>) -> LockError {
    LockErrorKind::BuildGraph(reason.into()).into()
}

impl Lock {
    /// Attach the resolved graphs for isolated source builds to the runtime lock.
    pub fn with_build_graphs<'a>(
        mut self,
        encoding: BuildEncoding,
        graphs: impl IntoIterator<Item = BuildGraph<'a>>,
        root: &Path,
        indexes: &IndexLocations,
    ) -> Result<Self, LockError> {
        let mut universes = Vec::new();
        let mut resolutions = FxHashMap::default();
        for graph in graphs {
            // Owners solved together share the same package table and artifact metadata.
            let build = match resolutions.entry(std::ptr::from_ref(graph.resolution)) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(Self::from_resolution(
                    graph.resolution,
                    ResolverManifest::default(),
                    root,
                    Vec::new(),
                    indexes,
                    false,
                )?),
            };
            let owner = PackageId {
                name: graph.owner.name().clone(),
                source: Source::from_resolved_dist(graph.owner, root)?,
                version: graph.owner.version().cloned(),
            };
            let mut roots = BTreeSet::new();
            for node in graph.resolution.graph.node_indices() {
                if let ResolutionGraphNode::Root = graph.resolution.graph[node] {
                    for edge in graph.resolution.graph.edges(node) {
                        let ResolutionGraphNode::Dist(dist) =
                            &graph.resolution.graph[edge.target()]
                        else {
                            continue;
                        };
                        for requirement in graph
                            .requirements
                            .iter()
                            .filter(|req| req.name == dist.name)
                        {
                            // A cluster can contain the same distribution with different extras.
                            // Activate only those named by this owner's direct requirement.
                            let id = PackageId::from_annotated_dist(dist, root)?;
                            if let Some(extra) = dist.kind.extra()
                                && !requirement.extras.contains(extra)
                            {
                                continue;
                            }
                            if !Self::package_satisfies_requirement(
                                &build.packages[build.by_id[&id].0],
                                requirement,
                                root,
                            )? {
                                continue;
                            }
                            let marker = edge.weight().combined().and(requirement.marker);
                            if !marker.is_false() {
                                roots.insert(Dependency::new(
                                    &self.requires_python,
                                    id,
                                    requirement.extras.iter().cloned().collect(),
                                    SimplifiedMarkerTree::new(&self.requires_python, marker),
                                ));
                            }
                        }
                    }
                }
            }
            let roots = roots.into_iter().collect::<Vec<_>>();
            let packages = closure(&roots, |id| {
                build
                    .by_id
                    .get(id)
                    .map(|index| build.package(*index).clone())
                    .ok_or_else(|| missing(id))
            })?;
            universes.push(Universe {
                owner,
                marker: graph.marker,
                roots,
                packages,
            });
        }
        universes.sort_by(|a, b| (&a.owner, a.marker).cmp(&(&b.owner, b.marker)));
        self.version = BUILD_LOCK_VERSION;
        self.builds = Some(BuildLocks {
            encoding,
            universes,
        });
        Ok(self)
    }

    /// A project that locks its builds verifies both its runtime and build archives at sync.
    /// Compute missing hashes when an index or local find-links directory omits them.
    pub async fn generate_missing_registry_hashes(
        &mut self,
        root: &Path,
        client: &RegistryClient,
        concurrency: usize,
    ) -> Result<(), LockError> {
        let Some(builds) = &mut self.builds else {
            return Ok(());
        };
        let mut jobs = FxHashMap::<_, Vec<_>>::default();
        let build_packages = builds
            .universes
            .iter_mut()
            .flat_map(|universe| universe.packages.values_mut());
        for package in self.packages.iter_mut().chain(build_packages) {
            let Source::Registry(index) = &package.id.source else {
                continue;
            };
            for wheel in &mut package.wheels {
                if wheel.hash.is_none() {
                    let file = wheel.to_registry_wheel(index, root)?.file;
                    let url = file.url.to_url().map_err(LockErrorKind::InvalidUrl)?;
                    jobs.entry(url).or_default().push(&mut wheel.hash);
                }
            }
            if let Some(sdist) = &mut package.sdist
                && sdist.hash().is_none()
            {
                let (url, metadata) = match sdist {
                    super::SourceDist::Url { url, metadata } => {
                        (url.to_url().map_err(LockErrorKind::InvalidUrl)?, metadata)
                    }
                    super::SourceDist::Path { path, metadata } => {
                        let RegistrySource::Path(index) = index else {
                            return Err(error(format!(
                                "Missing index directory for `{}`",
                                package.id
                            )));
                        };
                        let path = root.join(index).join(path);
                        let url = SafeUrl::from_file_path(&path).map_err(|()| {
                            LockErrorKind::PathToUrl {
                                path: path.into_boxed_path(),
                            }
                        })?;
                        (url, metadata)
                    }
                    super::SourceDist::Metadata { .. } => {
                        return Err(error(format!(
                            "Missing source archive location for `{}`",
                            package.id
                        )));
                    }
                };
                jobs.entry(url).or_default().push(&mut metadata.hash);
            }
        }

        // Shared artifacts have one download and one digest across all of their owners.
        futures::stream::iter(jobs.into_iter().map(Ok))
            .try_for_each_concurrent(concurrency, |(url, destinations)| async move {
                let digest = client
                    .hash_file(&url)
                    .await
                    .map_err(LockErrorKind::FileHash)?;
                for destination in destinations {
                    *destination = Some(digest.clone());
                }
                Ok(())
            })
            .await
    }

    /// The encoding to use when updating the recorded build environments.
    pub fn build_encoding(&self) -> Option<BuildEncoding> {
        self.builds.as_ref().map(|builds| builds.encoding)
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
                    if universes.len() != row.universes.len() {
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
            row.project(index + 1, &runtime.requires_python).unwire(
                &runtime.requires_python,
                environment,
                default,
                &names,
            )
        })?;
        universes.push(Universe {
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

#[cfg(test)]
mod tests {
    use std::error::Error;

    use indoc::formatdoc;
    use uv_configuration::{Constraints, DependencyModifiers};
    use uv_distribution_types::Dist;
    use uv_pypi_types::VerbatimParsedUrl;
    use uv_resolver_types::{AnnotatedDist, Options, PackageNodeKind, PackageRole};

    use super::*;

    fn dist(name: &str, version: &str) -> Result<AnnotatedDist, Box<dyn Error>> {
        let hash = if version == "1" { "1" } else { "2" }.repeat(64);
        let seed = Lock::from_toml(&formatdoc! {r#"
            version = 1
            requires-python = ">=3.11"
            [[package]]
            name = "{name}"
            version = "{version}"
            source = {{ registry = "https://example.org/simple" }}
            sdist = {{ url = "https://example.org/{name}-{version}.tar.gz", hash = "sha256:{hash}" }}
        "#})?;
        let source = seed.packages[0]
            .to_source_dist(&std::env::current_dir()?, FirstParty::No)?
            .expect("fixture sdist");
        Ok(AnnotatedDist {
            dist: ResolvedDist::Installable {
                dist: Arc::new(Dist::Source(source)),
                version: Some(version.parse()?),
            },
            name: name.parse()?,
            version: version.parse()?,
            kind: PackageNodeKind::Base,
            role: PackageRole::Dependency,
            marker: UniversalMarker::TRUE,
            hashes: Vec::new().into(),
            metadata: None,
        })
    }

    fn requirement(spec: &str) -> Result<Requirement, Box<dyn Error>> {
        Ok(spec
            .parse::<uv_pep508::Requirement<VerbatimParsedUrl>>()?
            .into())
    }

    /// Build one of two graphs which agree on C but select different versions of its dependency.
    fn graph(direct: &str, d: &str) -> Result<ResolverOutput, Box<dyn Error>> {
        let mut graph = Graph::new();
        let root = graph.add_node(ResolutionGraphNode::Root);
        let direct_node = graph.add_node(ResolutionGraphNode::Dist(dist(direct, "1")?));
        let c = graph.add_node(ResolutionGraphNode::Dist(dist("c", "1")?));
        let d = graph.add_node(ResolutionGraphNode::Dist(dist("d", d)?));
        graph.add_edge(root, direct_node, UniversalMarker::TRUE);
        graph.add_edge(root, c, UniversalMarker::TRUE);
        graph.add_edge(direct_node, d, UniversalMarker::TRUE);
        graph.add_edge(c, d, UniversalMarker::TRUE);
        Ok(ResolverOutput {
            graph,
            requires_python: RequiresPython::from_specifiers(">=3.11".parse()?),
            fork_markers: Vec::new(),
            diagnostics: Vec::new(),
            requirements: vec![requirement(direct)?, requirement("c==1")?],
            constraints: Constraints::default(),
            modifiers: DependencyModifiers::default(),
            options: Options::default(),
        })
    }

    fn round_trip(encoding: BuildEncoding) -> Result<String, Box<dyn Error>> {
        let root = std::env::current_dir()?;
        let _preview = uv_preview::test::with_features(&[]);
        let runtime = graph("a", "1")?;
        let build = graph("b", "2")?;
        let owner = dist("a", "1")?;
        let indexes = IndexLocations::default();
        let lock = Lock::from_resolution(
            &runtime,
            ResolverManifest::default(),
            &root,
            vec![],
            &indexes,
            false,
        )?
        .with_build_graphs(
            encoding,
            [BuildGraph {
                owner: &owner.dist,
                marker: UniversalMarker::TRUE,
                requirements: &build.requirements,
                resolution: &build,
            }],
            &root,
            &indexes,
        )?;
        let encoded = lock.to_toml()?;
        assert!(encoded.starts_with("version = 2\n"));
        let decoded = Lock::from_toml(&encoded)?;
        assert_eq!(lock, decoded);
        assert_eq!(decoded.to_toml()?, encoded);
        let runtime_c = decoded.find_by_name(&"c".parse()?)?.expect("runtime C");
        let build_c =
            &decoded.builds.as_ref().expect("build lock").universes[0].packages[&runtime_c.id];
        assert_eq!(
            runtime_c.dependencies[0]
                .package_id
                .version
                .as_ref()
                .expect("D version")
                .to_string(),
            "1"
        );
        assert_eq!(
            build_c.dependencies[0]
                .package_id
                .version
                .as_ref()
                .expect("D version")
                .to_string(),
            "2"
        );
        Ok(encoded)
    }

    #[test]
    fn build_conflict_marker_round_trip() -> Result<(), Box<dyn Error>> {
        let encoded = round_trip(BuildEncoding::ConflictMarker)?;
        assert!(encoded.contains("extra == 'uv-build-1'"));
        assert_eq!(encoded.matches("name = \"c\"\n").count(), 1);
        Ok(())
    }

    #[test]
    fn build_package_universe_round_trip() -> Result<(), Box<dyn Error>> {
        let encoded = round_trip(BuildEncoding::Package)?;
        assert_eq!(encoded.matches("name = \"c\"\n").count(), 2);
        Ok(())
    }

    #[test]
    fn build_dependency_universe_round_trip() -> Result<(), Box<dyn Error>> {
        let encoded = round_trip(BuildEncoding::Dependency)?;
        assert_eq!(encoded.matches("name = \"c\"\n").count(), 1);
        assert!(encoded.contains("universe = [1]"));
        Ok(())
    }
}
