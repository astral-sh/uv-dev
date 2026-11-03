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
use serde::{Deserialize, Serialize};
use uv_client::RegistryClient;
use uv_configuration::BuildOptions;
use uv_distribution_types::{
    Edge, ExtraBuildRequires, FirstParty, IndexLocations, Name, Node, Requirement, RequiresPython,
    Resolution, ResolvedDist, SimplifiedMarkerTree, SourceDist,
};
use uv_normalize::{ExtraName, PackageName};
use uv_pep508::{
    ExtraOperator, MarkerEnvironment, MarkerExpression, MarkerTree, MarkerValueExtra,
    MarkerValueVersion,
};
use uv_platform_tags::Tags;
use uv_pypi_types::{HashAlgorithm, HashDigest};
use uv_resolver_types::{ResolutionGraphNode, ResolverOutput, UniversalMarker};

use super::build_inputs::BuildInput;
use super::{
    BUILD_LOCK_VERSION, Dependency, DependencyWire, HashedDist, Lock, LockError, LockErrorKind,
    LockWire, Package, PackageId, PackageWire, RegistrySource, ResolverManifest, Source,
    SourceDist as LockedSourceDist, TagPolicy, absolute_path,
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
    /// The backend-declared prefix of the roots, before any extra build dependencies.
    pub backend_requirements: &'a [Requirement],
    pub resolution: &'a ResolverOutput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BuildLocks {
    pub(super) encoding: BuildEncoding,
    pub(super) universes: Vec<Universe>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Universe {
    /// Provenance for an owner whose roots use the ordinary package graph.
    pub(super) ordinary_owner: Option<OrdinaryOwner>,
    pub(super) owner: PackageId,
    pub(super) marker: UniversalMarker,
    pub(super) roots: Vec<Dependency>,
    pub(super) inputs: Option<BTreeSet<BuildInput>>,
    packages: BTreeMap<PackageId, Package>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OrdinaryOwner {
    Archive(HashDigest),
    Directory,
}

impl OrdinaryOwner {
    fn new(owner: &PackageId, sdist: Option<&LockedSourceDist>) -> Option<Self> {
        if owner.source.mutable_build_directory().is_some() {
            return Some(Self::Directory);
        }
        let hash = sdist?.hash()?;
        (matches!(owner.source, Source::Registry(_)) && strong_hash(hash))
            .then(|| Self::Archive(hash.clone()))
    }
}

/// Direct roots authorized for a source build. The ordinary lock supplies
/// their versions, sources, outgoing edges and allowed artifacts.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OrdinaryRoot {
    name: PackageName,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    extra: BTreeSet<ExtraName>,
}

impl OrdinaryRoot {
    pub(super) fn from_dependencies(roots: &[Dependency]) -> Vec<Self> {
        roots
            .iter()
            .map(|d| Self {
                name: d.package_id.name.clone(),
                extra: d.extra.clone(),
            })
            .collect()
    }
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
    /// Iterate over packages selected for the runtime environment or an isolated build.
    pub(super) fn packages_in_all_environments(&self) -> impl Iterator<Item = &Package> {
        self.packages.iter().chain(
            self.builds
                .iter()
                .flat_map(|builds| &builds.universes)
                .flat_map(|universe| universe.packages.values()),
        )
    }

    /// Attach the resolved graphs for isolated source builds to the runtime lock.
    pub fn with_build_graphs<'a>(
        mut self,
        encoding: BuildEncoding,
        graphs: impl IntoIterator<Item = BuildGraph<'a>>,
        root: &Path,
        indexes: &IndexLocations,
        extras: &ExtraBuildRequires,
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
            let backend = if owner.source.mutable_build_directory().is_some() {
                graph.backend_requirements
            } else {
                &[]
            };
            // Keep complete declaration markers so a shared descriptor retains the
            // observations for every fork. Checked replay projects them onto the owner domain.
            let inputs = BuildInput::collect(
                backend,
                extras,
                &owner.name,
                root,
                &self.requires_python,
                UniversalMarker::TRUE,
            )?;
            universes.push(Universe {
                ordinary_owner: None,
                inputs: Some(inputs),
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
            if package
                .sdist
                .as_ref()
                .is_some_and(|sdist| sdist.hash().is_none())
                && let Some(SourceDist::Registry(source)) =
                    package.to_source_dist(root, FirstParty::No)?
                && let Some(sdist) = &mut package.sdist
            {
                let url = source
                    .file
                    .url
                    .to_url()
                    .map_err(LockErrorKind::InvalidUrl)?;
                let metadata = match sdist {
                    LockedSourceDist::Url { metadata, .. }
                    | LockedSourceDist::Path { metadata, .. }
                    | LockedSourceDist::Metadata { metadata } => metadata,
                };
                jobs.entry(url).or_default().push(&mut metadata.hash);
            }
            for wheel in &mut package.wheels {
                if wheel.hash.is_none() {
                    let file = wheel.to_registry_wheel(index, root)?.file;
                    let url = file.url.to_url().map_err(LockErrorKind::InvalidUrl)?;
                    jobs.entry(url).or_default().push(&mut wheel.hash);
                }
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

    /// The complete supported environment, including exclusions inside the Python range.
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

    /// Combine disjoint marker domains for an owner when it selects the same build environment
    /// throughout each domain. Keep artifact lists and dependency markers in the comparison.
    pub fn with_compacted_build_domains(mut self) -> Result<Self, LockError> {
        let project = self.build_project_domain();
        let Some(builds) = &mut self.builds else {
            return Ok(self);
        };
        let mut owners: BTreeMap<PackageId, Vec<Universe>> = BTreeMap::new();
        for universe in std::mem::take(&mut builds.universes) {
            let domain = universe.marker.combined().and(project);
            if domain.is_false() {
                continue;
            }
            let mut universe = universe.project(domain, &self.requires_python)?;
            universe.marker =
                UniversalMarker::from_combined(universe.marker.combined().restrict(project));
            let variants = owners.entry(universe.owner.clone()).or_default();
            if let Some(other) = variants.iter_mut().find(|other| {
                other.marker.combined().is_disjoint(domain)
                    && other.inputs.is_some()
                    && other.inputs == universe.inputs
                    && other.roots == universe.roots
                    && other.packages == universe.packages
            }) {
                other.marker.or(universe.marker);
            } else {
                variants.push(universe);
            }
        }
        builds.universes = owners
            .into_values()
            .flat_map(|mut variants| {
                variants.sort_by_key(|universe| universe.marker);
                variants
            })
            .collect();
        Ok(self)
    }

    /// The encoding to use when updating the recorded build environments.
    pub fn build_encoding(&self) -> Option<BuildEncoding> {
        self.builds.as_ref().map(|builds| builds.encoding)
    }

    /// Serialize the recorded environments in the selected encoding.
    #[must_use]
    pub fn with_build_encoding(mut self, encoding: BuildEncoding) -> Self {
        if let Some(builds) = &mut self.builds {
            builds.encoding = encoding;
        }
        self
    }

    /// Reuse ordinary packages only when reading the compact roots reconstructs the solved
    /// environment exactly, including every artifact and each outgoing dependency.
    pub fn with_ordinary_build_roots(mut self) -> Result<Self, LockError> {
        if self.version != BUILD_LOCK_VERSION {
            return Ok(self);
        }
        let Some(mut builds) = self.builds.take() else {
            return Ok(self);
        };
        let mut owner_artifacts = BTreeMap::new();
        for package in self
            .packages
            .iter()
            .chain(builds.universes.iter().flat_map(|u| u.packages.values()))
        {
            let files = (&package.sdist, &package.wheels);
            owner_artifacts
                .entry(&package.id)
                .and_modify(|known| {
                    if *known != Some(files) {
                        *known = None;
                    }
                })
                .or_insert(Some(files));
        }
        let proven: BTreeMap<_, _> = owner_artifacts
            .into_iter()
            .filter_map(|(id, files)| {
                let (sdist, _) = files?;
                Some((id.clone(), OrdinaryOwner::new(id, sdist.as_ref())?))
            })
            .collect();
        let mut counts = BTreeMap::new();
        for universe in &builds.universes {
            *counts.entry(universe.owner.clone()).or_insert(0usize) += 1;
        }
        let ordinary = OrdinaryGraph::new(&self);
        for universe in &mut builds.universes {
            let Some(provenance) = proven.get(&universe.owner) else {
                continue;
            };
            if counts[&universe.owner] != 1
                || !universe.marker.is_true()
                || universe.inputs.is_none()
            {
                continue;
            }
            let roots = OrdinaryRoot::from_dependencies(&universe.roots);
            if let Some(candidate) = ordinary.reconstruct(
                &universe.owner,
                provenance,
                &roots,
                universe.inputs.clone(),
            )? && candidate.roots == universe.roots
                && candidate.packages == universe.packages
            {
                universe.ordinary_owner = candidate.ordinary_owner;
            }
        }
        self.builds = Some(builds);
        Ok(self)
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
        if !universe.accepts_source(owner) {
            return Err(error(format!(
                "Source archive for `{}` does not match its locked build owner",
                universe.owner
            )));
        }
        universe.resolve(requirements, root, &environment, tags, options)
    }
}

/// The ordinary graph indexed by names with exactly one locked source and version.
struct OrdinaryGraph<'a> {
    lock: &'a Lock,
    domain: MarkerTree,
    unique: FxHashMap<&'a PackageName, Option<&'a Package>>,
}

impl<'a> OrdinaryGraph<'a> {
    fn new(lock: &'a Lock) -> Self {
        let mut unique = FxHashMap::default();
        for package in &lock.packages {
            unique
                .entry(&package.id.name)
                .and_modify(|candidate| *candidate = None)
                .or_insert(Some(package));
        }
        Self {
            lock,
            domain: lock.build_project_domain(),
            unique,
        }
    }

    fn reconstruct(
        &self,
        owner: &PackageId,
        provenance: &OrdinaryOwner,
        roots: &[OrdinaryRoot],
        inputs: Option<BTreeSet<BuildInput>>,
    ) -> Result<Option<Universe>, LockError> {
        let mut dependencies = Vec::with_capacity(roots.len());
        for root in roots {
            let Some(Some(package)) = self.unique.get(&root.name) else {
                return Ok(None);
            };
            dependencies.push(Dependency::new(
                &self.lock.requires_python,
                package.id.clone(),
                root.extra.clone(),
                SimplifiedMarkerTree::default(),
            ));
        }
        let packages = closure(&dependencies, |id| {
            self.lock
                .by_id
                .get(id)
                .map(|index| self.lock.package(*index).clone())
                .ok_or_else(|| missing(id))
        })?;
        let universe = Universe {
            ordinary_owner: Some(provenance.clone()),
            owner: owner.clone(),
            marker: UniversalMarker::TRUE,
            inputs,
            roots: dependencies,
            packages,
        }
        .project(self.domain, &self.lock.requires_python)?;
        for package in universe.packages.values() {
            if !matches!(package.id.source, Source::Registry(_))
                || !package.fork_markers.is_empty()
                || !matches!(self.unique.get(&package.id.name), Some(Some(_)))
                || (package.wheels.is_empty() && package.sdist.is_none())
                || package
                    .sdist
                    .as_ref()
                    .is_some_and(|s| s.hash().is_none_or(|hash| !strong_hash(hash)))
                || package
                    .wheels
                    .iter()
                    .any(|wheel| !wheel.hash.as_ref().is_some_and(strong_hash))
                || package
                    .dependencies
                    .iter()
                    .chain(package.optional_dependencies.values().flatten())
                    .chain(package.dependency_groups.values().flatten())
                    .any(|dep| !dep.simplified_marker.as_simplified_marker_tree().is_true())
            {
                return Ok(None);
            }
        }
        Ok(Some(universe))
    }
}

fn strong_hash(hash: &HashDigest) -> bool {
    match hash.algorithm() {
        HashAlgorithm::Md5 => false,
        HashAlgorithm::Sha256
        | HashAlgorithm::Sha384
        | HashAlgorithm::Sha512
        | HashAlgorithm::Blake2b256 => true,
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
    fn accepts_source(&self, owner: &SourceDist) -> bool {
        match &self.ordinary_owner {
            Some(OrdinaryOwner::Archive(hash)) => {
                matches!(owner, SourceDist::Registry(archive) if archive.file.hashes.as_slice().contains(hash))
            }
            // The caller has already matched the source directory against the lock root.
            Some(OrdinaryOwner::Directory) => matches!(owner, SourceDist::Directory(_)),
            None => true,
        }
    }

    /// Package markers are evaluated only after the owning build environment is selected.
    fn project(
        mut self,
        domain: MarkerTree,
        requires_python: &RequiresPython,
    ) -> Result<Self, LockError> {
        fn project_edges(
            edges: &mut Vec<Dependency>,
            domain: MarkerTree,
            requires_python: &RequiresPython,
        ) {
            edges.sort_unstable();
            edges.dedup_by(|later, earlier| {
                if earlier.package_id != later.package_id || earlier.extra != later.extra {
                    return false;
                }
                earlier.complexified_marker.or(later.complexified_marker);
                true
            });
            edges.retain_mut(|dep| {
                let marker = dep.complexified_marker.combined().restrict(domain);
                if marker.is_false() {
                    return false;
                }
                dep.simplified_marker = SimplifiedMarkerTree::new(requires_python, marker);
                dep.complexified_marker = UniversalMarker::from_combined(
                    dep.simplified_marker.into_marker(requires_python),
                );
                true
            });
        }
        project_edges(&mut self.roots, domain, requires_python);
        let pep508_domain = UniversalMarker::from_combined(domain).pep508();
        self.packages = closure(&self.roots, |id| {
            let mut package = self.packages.remove(id).ok_or_else(|| missing(id))?;
            // Fork preferences describe where this package ID applies. Its owner has already
            // selected a platform. Keep conflict-bearing preference sets in their original form.
            if !package.fork_markers.is_empty()
                && !package
                    .fork_markers
                    .iter()
                    .any(|marker| marker.has_conflict_marker())
            {
                let mut markers = Vec::new();
                for marker in &package.fork_markers {
                    let marker = marker.combined().restrict(pep508_domain);
                    if marker.is_true() {
                        markers.clear();
                        break;
                    }
                    markers.push(UniversalMarker::from_combined(
                        SimplifiedMarkerTree::new(requires_python, marker)
                            .into_marker(requires_python),
                    ));
                }
                markers.sort_unstable();
                markers.dedup();
                package.fork_markers = markers;
            }
            for edges in std::iter::once(&mut package.dependencies)
                .chain(package.optional_dependencies.values_mut())
                .chain(package.dependency_groups.values_mut())
            {
                project_edges(edges, domain, requires_python);
            }
            Ok(package)
        })?;
        Ok(self)
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
        for (index, universe) in self
            .universes
            .iter()
            .filter(|u| u.ordinary_owner.is_none())
            .enumerate()
        {
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
    let mut ordinary = BTreeMap::new();
    for mut package in std::mem::take(&mut wire.packages) {
        if let Some(roots) = package.build_roots.take() {
            let provenance =
                OrdinaryOwner::new(&package.id, package.sdist.as_ref()).ok_or_else(|| {
                    error(format!(
                        "Missing locked source provenance for ordinary build owner `{}`",
                        package.id
                    ))
                })?;
            let inputs = package.build_inputs.take().unwrap_or_default();
            if wire.version != BUILD_LOCK_VERSION
                || ordinary
                    .insert(package.id.clone(), (provenance, roots, inputs))
                    .is_some()
            {
                return Err(error(format!(
                    "Invalid ordinary build roots for `{}`",
                    package.id
                )));
            }
        } else if package.build_inputs.is_some() {
            return Err(error(format!(
                "Missing ordinary build roots for `{}`",
                package.id
            )));
        }
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
        if ordinary.contains_key(&descriptor.owner) {
            return Err(error(format!(
                "Ambiguous ordinary build owner `{}`",
                descriptor.owner
            )));
        }
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
            ordinary_owner: None,
            inputs: descriptor
                .inputs
                .or_else(|| (runtime.version == BUILD_LOCK_VERSION).then(BTreeSet::new)),
            owner: descriptor.owner,
            marker: UniversalMarker::from_combined(descriptor.marker),
            roots,
            packages,
        });
    }
    if !ordinary.is_empty() {
        let graph = OrdinaryGraph::new(&runtime);
        for (id, (provenance, roots, inputs)) in ordinary {
            if rows[&id]
                .windows(2)
                .any(|pair| pair[0].sdist != pair[1].sdist || pair[0].wheels != pair[1].wheels)
            {
                return Err(error(format!("Ambiguous ordinary build owner `{id}`")));
            }
            universes.push(
                graph
                    .reconstruct(&id, &provenance, &roots, Some(inputs))?
                    .ok_or_else(|| {
                        error(format!(
                            "Cannot reconstruct ordinary build roots for `{id}`"
                        ))
                    })?,
            );
        }
        universes.sort_by(|a, b| (&a.owner, a.marker).cmp(&(&b.owner, b.marker)));
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
    use uv_distribution_types::{Dist, ExtraBuildRequirement};
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

    fn build_lock(encoding: BuildEncoding) -> Result<Lock, Box<dyn Error>> {
        let build = graph("b", "2")?;
        lock_with_roots(encoding, &build, &build.requirements)
    }

    fn lock_with_roots(
        encoding: BuildEncoding,
        build: &ResolverOutput,
        roots: &[Requirement],
    ) -> Result<Lock, Box<dyn Error>> {
        let root = std::env::current_dir()?;
        let _preview = uv_preview::test::with_features(&[]);
        let runtime = graph("a", "1")?;
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
                backend_requirements: roots,
                requirements: roots,
                resolution: build,
            }],
            &root,
            &indexes,
            &ExtraBuildRequires::default(),
        )?;
        Ok(lock)
    }

    fn round_trip(encoding: BuildEncoding) -> Result<String, Box<dyn Error>> {
        let lock = build_lock(encoding)?;
        let encoded = lock.to_toml()?;
        assert!(encoded.starts_with("version = 4\n"));
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

    /// The owner selects C alone from a cluster that also solved B. C's transitive D
    /// must have the same pin and allowed artifacts in the ordinary graph.
    #[test]
    fn ordinary_build_roots() -> Result<(), Box<dyn Error>> {
        let roots = [requirement("c==1")?];
        let build = graph("b", "1")?;
        let explicit = lock_with_roots(BuildEncoding::Dependency, &build, &roots)?
            .with_compacted_build_domains()?;
        let compact = explicit.clone().with_ordinary_build_roots()?;
        let encoded = compact.to_toml()?;
        assert!(!encoded.contains("[[build-universes]]"));
        assert!(encoded.contains("build-roots = [{ name = \"c\" }]"));
        assert_eq!(Lock::from_toml(&encoded)?, compact);
        let inferred = &compact.builds.as_ref().expect("builds").universes[0];
        let declared = &explicit.builds.as_ref().expect("builds").universes[0];
        assert_eq!(inferred.roots, declared.roots);
        assert_eq!(inferred.packages, declared.packages);
        assert_eq!(inferred.packages.len(), 2);

        let owner = compact.find_by_name(&"a".parse()?)?.expect("source owner");
        let mut archive = owner
            .to_source_dist(&std::env::current_dir()?, FirstParty::No)?
            .expect("sdist");
        assert!(inferred.accepts_source(&archive));
        let SourceDist::Registry(claimed) = &mut archive else {
            return Err("expected a registry source".into());
        };
        claimed.file.hashes = vec![format!("sha256:{}", "7".repeat(64)).parse()?].into();
        assert!(!inferred.accepts_source(&archive));

        let mut different_hash = explicit;
        let universe = &mut different_hash.builds.as_mut().expect("builds").universes[0];
        let leaf = universe
            .packages
            .values_mut()
            .find(|p| p.id.name.as_ref() == "d")
            .expect("D");
        let Some(LockedSourceDist::Url { metadata, .. }) = &mut leaf.sdist else {
            return Err("expected a registry sdist".into());
        };
        metadata.hash = Some(format!("sha256:{}", "9".repeat(64)).parse()?);
        assert!(
            different_hash
                .with_ordinary_build_roots()?
                .to_toml()?
                .contains("[[build-universes]]")
        );

        let different_pin = graph("b", "2")?;
        let explicit = lock_with_roots(BuildEncoding::Dependency, &different_pin, &roots)?
            .with_compacted_build_domains()?
            .with_ordinary_build_roots()?;
        assert!(explicit.to_toml()?.contains("[[build-universes]]"));
        Ok(())
    }

    /// Platform forks can repeat roots and package preferences while selecting the same closure.
    /// Keep a fork separate when its transitive edge, sdist hash, or configured inputs differ.
    #[test]
    fn compact_build_domains() -> Result<(), Box<dyn Error>> {
        let mut lock = build_lock(BuildEncoding::Dependency)?;
        lock.requires_python = RequiresPython::from_specifiers(">=3.11,!=3.12.*".parse()?);
        let requires_python = &lock.requires_python;
        let universes = &mut lock.builds.as_mut().expect("build lock").universes;
        let base = universes.pop().expect("owner");
        let linux: MarkerTree = "sys_platform == 'linux'".parse()?;
        let windows: MarkerTree = "sys_platform == 'win32'".parse()?;
        let darwin: MarkerTree = "sys_platform == 'darwin'".parse()?;
        let netbsd: MarkerTree = "sys_platform == 'netbsd'".parse()?;
        let openbsd: MarkerTree = "sys_platform == 'openbsd'".parse()?;
        let cpython: MarkerTree = "implementation_name == 'cpython'".parse()?;
        let c = base
            .roots
            .iter()
            .find(|dep| dep.package_name().as_ref() == "c")
            .expect("C root")
            .package_id
            .clone();

        let mut on_linux = base.clone();
        on_linux.marker = UniversalMarker::from_combined(linux);
        let root = on_linux.roots.pop().expect("root");
        for implementation in [cpython, cpython.negate()] {
            on_linux.roots.push(Dependency::new(
                requires_python,
                root.package_id.clone(),
                root.extra.clone(),
                SimplifiedMarkerTree::new(requires_python, linux.and(implementation)),
            ));
        }
        on_linux.packages.get_mut(&c).expect("C").fork_markers =
            vec![UniversalMarker::from_combined(linux.and(cpython))];

        let mut on_windows = base.clone();
        on_windows.marker = UniversalMarker::from_combined(windows);
        on_windows.packages.get_mut(&c).expect("C").fork_markers =
            vec![UniversalMarker::from_combined(windows.and(cpython))];

        let mut on_darwin = base.clone();
        on_darwin.marker = UniversalMarker::from_combined(darwin);
        let package = on_darwin.packages.get_mut(&c).expect("C");
        package.fork_markers = vec![UniversalMarker::from_combined(cpython)];
        let Some(LockedSourceDist::Url { metadata, .. }) = &mut package.sdist else {
            return Err("expected a registry sdist".into());
        };
        metadata.hash = Some(format!("sha256:{}", "3".repeat(64)).parse()?);

        let mut on_netbsd = base.clone();
        on_netbsd.marker = UniversalMarker::from_combined(netbsd);
        let package = on_netbsd.packages.get_mut(&c).expect("C");
        package.fork_markers = vec![UniversalMarker::from_combined(cpython)];
        package.dependencies.clear();

        // A configured extra can repeat an existing build dependency on just one platform.
        let mut on_openbsd = base.clone();
        on_openbsd.marker = UniversalMarker::from_combined(openbsd);
        on_openbsd.packages.get_mut(&c).expect("C").fork_markers =
            vec![UniversalMarker::from_combined(cpython)];
        let extras = ExtraBuildRequires::from_iter([(
            base.owner.name.clone(),
            vec![ExtraBuildRequirement {
                requirement: requirement("c>=1")?,
                match_runtime: false,
            }],
        )]);
        on_openbsd.inputs = Some(BuildInput::collect(
            &[],
            &extras,
            &base.owner.name,
            &std::env::current_dir()?,
            requires_python,
            on_openbsd.marker,
        )?);

        let mut excluded = base.clone();
        excluded.marker = UniversalMarker::from_combined("python_version == '3.12'".parse()?);
        let mut outside = base;
        outside.marker = UniversalMarker::from_combined("python_version < '3.11'".parse()?);
        *universes = vec![
            on_linux, on_windows, on_darwin, on_netbsd, on_openbsd, excluded, outside,
        ];
        let compact = lock.with_compacted_build_domains()?;
        let domains = &compact.builds.as_ref().expect("compacted builds").universes;
        assert_eq!(domains.len(), 4);
        let shared = domains
            .iter()
            .find(|u| u.marker.combined() == linux.or(windows))
            .expect("shared Linux and Windows build");
        assert_eq!(shared.roots.len(), 2);
        assert_eq!(shared.packages[&c].fork_markers.len(), 1);
        assert_eq!(shared.packages[&c].dependencies.len(), 1);
        assert!(domains.iter().any(|u| u.marker.combined() == darwin));
        assert!(domains.iter().any(|u| u.marker.combined() == netbsd));
        assert!(domains.iter().any(|u| u.marker.combined() == openbsd));
        let encoded = compact.to_toml()?;
        assert_eq!(
            Lock::from_toml(&encoded)?
                .with_compacted_build_domains()?
                .to_toml()?,
            encoded
        );
        Ok(())
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
