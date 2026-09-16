use std::collections::BTreeSet;
use std::ops::Bound;
use std::sync::Arc;

use arcstr::ArcStr;
use papaya::HashMap;
use pubgrub::{DerivationTree, External, Ranges, Term};
use rustc_hash::{FxHashMap, FxHashSet};

use uv_distribution_types::RequiresPython;
use uv_normalize::PackageName;
use uv_pep440::{
    EncodedVersion, EncodedVersionRanges, LocalSegment, LocalVersionSlice, MIN_VERSION, Operator,
    Version,
};
use uv_pep508::{
    CanonicalMarkerValueString, CanonicalMarkerValueVersion, MarkerTree, MarkerTreeKind,
};
use uv_pypi_types::{ConflictItem, ConflictKind, ParsedUrl};

use crate::PythonRequirement;
use crate::error::ErrorTree;
use crate::fork_indexes::ForkIndexes;
use crate::fork_urls::ForkUrls;
use crate::pubgrub::{PubGrubPackage, PubGrubPackageInner, PubGrubPython, Range};
use crate::python_requirement::PythonRequirementSource;
use crate::resolver::{
    InMemoryIndex, Indexes, MetadataUnavailable, ResolverEnvironment, UnavailablePackage,
    UnavailableReason, UnavailableVersion, Urls, VersionsResponse,
};

use super::NoSolutionEvidence;
use super::budget::{Budget, CaptureLimits, Resource, Stop};
use super::wire::*;

/// Read-only state at the final failed fork, before any diagnostic tree transformations.
pub(crate) struct CaptureContext<'a> {
    pub error: &'a ErrorTree,
    pub project: Option<&'a PackageName>,
    pub workspace_members: &'a BTreeSet<PackageName>,
    pub environment: &'a ResolverEnvironment,
    pub original_python: &'a PythonRequirement,
    pub effective_python: &'a PythonRequirement,
    pub index: &'a InMemoryIndex,
    pub urls: &'a Urls,
    pub indexes: &'a Indexes,
    pub fork_urls: &'a ForkUrls,
    pub fork_indexes: &'a ForkIndexes,
    pub known_versions: &'a FxHashMap<PackageName, Arc<[Version]>>,
    pub unavailable_packages: &'a HashMap<PackageName, UnavailablePackage>,
    pub incomplete_packages: &'a HashMap<PackageName, HashMap<Version, MetadataUnavailable>>,
}

impl CaptureOptions {
    pub(crate) fn capture(&self, context: CaptureContext<'_>) -> NoSolutionEvidence {
        self.capture_with_limits(context, CaptureLimits::V1)
    }

    fn capture_with_limits(
        &self,
        context: CaptureContext<'_>,
        limits: CaptureLimits,
    ) -> NoSolutionEvidence {
        let mut collector = Collector::new(limits);
        let result = collector.graph(context);
        let (status, reason, graph) = match result {
            Ok(graph) => (CaptureStatus::Complete, None, Some(graph)),
            Err(stop) => (stop.status, Some(stop.reason), None),
        };
        NoSolutionEvidence(EvidenceWire {
            schema: 1,
            request: self.token.request().to_owned(),
            producer_pid: self.token.producer_pid(),
            command: CaptureCommand::Lock,
            scope: self.scope,
            operation: self.operation,
            metadata: self.metadata,
            terminal: CaptureTerminal::NoSolution,
            status,
            reason,
            limits,
            usage: collector.budget.usage,
            graph,
        })
    }
}

struct Collector {
    budget: Budget,
    packages: Vec<CapturedPackage>,
    package_ids: FxHashMap<PubGrubPackage, u32>,
    names: BTreeSet<PackageName>,
    markers: Vec<CapturedMarker>,
    marker_ids: FxHashMap<MarkerTree, u32>,
}

impl Collector {
    fn new(limits: CaptureLimits) -> Self {
        Self {
            budget: Budget::new(limits),
            packages: Vec::new(),
            package_ids: FxHashMap::default(),
            names: BTreeSet::new(),
            markers: Vec::new(),
            marker_ids: FxHashMap::default(),
        }
    }

    fn graph(&mut self, context: CaptureContext<'_>) -> Result<CapturedGraph, Stop> {
        if let Some(project) = context.project {
            self.budget.check_atom(project.as_ref().len())?;
        }
        let root_package =
            self.package(&PubGrubPackageInner::Root(context.project.cloned()).into())?;
        let root_version = self.version(&MIN_VERSION)?;
        let (nodes, root) = self.derivation(context.error)?;
        let environment = self.environment(context.environment)?;
        let original_python = self.python(context.original_python)?;
        let effective_python = self.python(context.effective_python)?;
        let mut workspace_members = Vec::new();
        for name in context.workspace_members {
            workspace_members.push(self.budget.string(name.as_ref())?);
        }
        let observations = self.observations(&context)?;
        Ok(CapturedGraph {
            root,
            root_package,
            root_version,
            nodes,
            packages: std::mem::take(&mut self.packages),
            markers: std::mem::take(&mut self.markers),
            workspace_members,
            environment,
            original_python,
            effective_python,
            observations,
        })
    }

    fn derivation(&mut self, root: &ErrorTree) -> Result<(Vec<CapturedNode>, u32), Stop> {
        enum Task<'a> {
            Visit(&'a ErrorTree),
            Finish(&'a ErrorTree),
        }

        let mut nodes = Vec::new();
        let mut ids = FxHashMap::default();
        let mut active = FxHashSet::default();
        let mut pending = vec![Task::Visit(root)];
        while let Some(task) = pending.pop() {
            self.budget.work(1)?;
            match task {
                Task::Visit(node) => {
                    let key = std::ptr::from_ref(node);
                    if ids.contains_key(&key) {
                        continue;
                    }
                    if active.contains(&key) {
                        return Err(Stop::unsupported(CaptureReason::InvalidNativeGraph));
                    }
                    self.budget.charge(Resource::DerivationNodes, 1)?;
                    active.insert(key);
                    pending.push(Task::Finish(node));
                    if let DerivationTree::Derived(derived) = node {
                        pending.push(Task::Visit(&derived.cause2));
                        pending.push(Task::Visit(&derived.cause1));
                    }
                }
                Task::Finish(node) => {
                    let captured = match node {
                        DerivationTree::External(external) => match external {
                            External::NotRoot(package, version) => CapturedNode::NotRoot {
                                package: self.package(package)?,
                                version: self.version(version)?,
                            },
                            External::NoVersions(package, range) => CapturedNode::NoVersions {
                                package: self.package(package)?,
                                range: self.range(range)?,
                            },
                            External::FromDependencyOf(
                                package,
                                range,
                                dependency,
                                dependency_range,
                            ) => CapturedNode::FromDependencyOf {
                                package: self.package(package)?,
                                range: self.range(range)?,
                                dependency: self.package(dependency)?,
                                dependency_range: self.range(dependency_range)?,
                            },
                            External::Custom(package, range, reason) => CapturedNode::Custom {
                                package: self.package(package)?,
                                range: self.range(range)?,
                                reason: reason.into(),
                            },
                        },
                        DerivationTree::Derived(derived) => {
                            let mut terms = Vec::new();
                            for (package, term) in &derived.terms {
                                self.budget.charge(Resource::Terms, 1)?;
                                self.budget.work(1)?;
                                let (positive, range) = match term {
                                    Term::Positive(range) => (true, range),
                                    Term::Negative(range) => (false, range),
                                };
                                terms.push(CapturedTerm {
                                    package: self.package(package)?,
                                    positive,
                                    range: self.range(range)?,
                                });
                            }
                            CapturedNode::Derived {
                                cause1: *ids.get(&Arc::as_ptr(&derived.cause1)).ok_or_else(
                                    || Stop::unsupported(CaptureReason::InvalidNativeGraph),
                                )?,
                                cause2: *ids.get(&Arc::as_ptr(&derived.cause2)).ok_or_else(
                                    || Stop::unsupported(CaptureReason::InvalidNativeGraph),
                                )?,
                                terms,
                            }
                        }
                    };
                    let id = u32::try_from(nodes.len())
                        .map_err(|_| Stop::truncated(CaptureReason::DerivationNodes))?;
                    nodes.push(captured);
                    let key = std::ptr::from_ref(node);
                    active.remove(&key);
                    ids.insert(key, id);
                }
            }
        }
        let root = *ids
            .get(&std::ptr::from_ref(root))
            .ok_or_else(|| Stop::unsupported(CaptureReason::InvalidNativeGraph))?;
        Ok((nodes, root))
    }

    fn package(&mut self, package: &PubGrubPackage) -> Result<u32, Stop> {
        self.budget.work(1)?;
        // Bound names before hashing the native package identity.
        if let Some(name) = package.name() {
            self.budget.check_atom(name.as_ref().len())?;
        }
        if let Some(extra) = package.extra() {
            self.budget.check_atom(extra.as_ref().len())?;
        }
        if let Some(group) = package.group() {
            self.budget.check_atom(group.as_ref().len())?;
        }
        if let Some(id) = self.package_ids.get(package) {
            return Ok(*id);
        }
        self.budget.charge(Resource::Packages, 1)?;
        let captured = match &**package {
            PubGrubPackageInner::Root(name) => CapturedPackage::Root {
                name: name
                    .as_ref()
                    .map(|name| self.budget.string(name.as_ref()))
                    .transpose()?,
            },
            PubGrubPackageInner::Python(kind) => CapturedPackage::Python {
                kind: match kind {
                    PubGrubPython::Installed => CapturedPythonKind::Installed,
                    PubGrubPython::Target => CapturedPythonKind::Target,
                },
            },
            PubGrubPackageInner::System(name) => CapturedPackage::System {
                name: self.budget.string(name.as_ref())?,
            },
            PubGrubPackageInner::Package {
                name,
                extra,
                group,
                marker,
            } => CapturedPackage::Package {
                name: self.budget.string(name.as_ref())?,
                extra: extra
                    .as_ref()
                    .map(|extra| self.budget.string(extra.as_ref()))
                    .transpose()?,
                group: group
                    .as_ref()
                    .map(|group| self.budget.string(group.as_ref()))
                    .transpose()?,
                marker: self.marker(*marker)?,
            },
            PubGrubPackageInner::Extra {
                name,
                extra,
                marker,
            } => CapturedPackage::Extra {
                name: self.budget.string(name.as_ref())?,
                extra: self.budget.string(extra.as_ref())?,
                marker: self.marker(*marker)?,
            },
            PubGrubPackageInner::Group {
                name,
                group,
                marker,
            } => CapturedPackage::Group {
                name: self.budget.string(name.as_ref())?,
                group: self.budget.string(group.as_ref())?,
                marker: self.marker(*marker)?,
            },
            PubGrubPackageInner::Marker { name, marker } => CapturedPackage::Marker {
                name: self.budget.string(name.as_ref())?,
                marker: self.marker(*marker)?,
            },
        };
        if let Some(name) = package.name() {
            self.names.insert(name.clone());
        }
        let id = u32::try_from(self.packages.len())
            .map_err(|_| Stop::truncated(CaptureReason::Packages))?;
        self.packages.push(captured);
        self.package_ids.insert(package.clone(), id);
        Ok(id)
    }

    fn marker(&mut self, root: MarkerTree) -> Result<u32, Stop> {
        enum Task {
            Visit(MarkerTree),
            Finish(MarkerTree),
        }

        let mut active = FxHashSet::default();
        let mut pending = vec![Task::Visit(root)];
        while let Some(task) = pending.pop() {
            self.budget.work(1)?;
            match task {
                Task::Visit(marker) => {
                    if self.marker_ids.contains_key(&marker) {
                        continue;
                    }
                    if active.contains(&marker) {
                        return Err(Stop::unsupported(CaptureReason::InvalidNativeGraph));
                    }
                    self.budget.charge(Resource::MarkerNodes, 1)?;
                    active.insert(marker);
                    pending.push(Task::Finish(marker));
                    match marker.kind() {
                        MarkerTreeKind::True | MarkerTreeKind::False => {}
                        MarkerTreeKind::Version(node) => {
                            for (_, child) in node.edges() {
                                self.budget.charge(Resource::MarkerEdges, 1)?;
                                self.budget.work(1)?;
                                pending.push(Task::Visit(child));
                            }
                        }
                        MarkerTreeKind::String(node) => {
                            for (_, child) in node.children() {
                                self.budget.charge(Resource::MarkerEdges, 1)?;
                                self.budget.work(1)?;
                                pending.push(Task::Visit(child));
                            }
                        }
                        MarkerTreeKind::In(_) => {
                            return Err(Stop::unsupported(CaptureReason::UnsupportedMarkerIn));
                        }
                        MarkerTreeKind::Contains(_) => {
                            return Err(Stop::unsupported(
                                CaptureReason::UnsupportedMarkerContains,
                            ));
                        }
                        MarkerTreeKind::List(_) => {
                            return Err(Stop::unsupported(CaptureReason::UnsupportedMarkerList));
                        }
                        MarkerTreeKind::Extra(_) => {
                            return Err(Stop::unsupported(CaptureReason::UnsupportedMarkerExtra));
                        }
                    }
                }
                Task::Finish(marker) => {
                    let captured = match marker.kind() {
                        MarkerTreeKind::True => CapturedMarker::True,
                        MarkerTreeKind::False => CapturedMarker::False,
                        MarkerTreeKind::Version(node) => {
                            let mut edges = Vec::new();
                            for (range, child) in node.edges() {
                                self.budget.work(1)?;
                                edges.push(VersionMarkerEdge {
                                    intervals: self.version_ranges(range)?,
                                    child: *self.marker_ids.get(&child).ok_or_else(|| {
                                        Stop::unsupported(CaptureReason::InvalidNativeGraph)
                                    })?,
                                });
                            }
                            CapturedMarker::Version {
                                key: node.key().into(),
                                edges,
                            }
                        }
                        MarkerTreeKind::String(node) => {
                            let mut edges = Vec::new();
                            for (range, child) in node.children() {
                                self.budget.work(1)?;
                                edges.push(StringMarkerEdge {
                                    intervals: self.string_ranges(range)?,
                                    child: *self.marker_ids.get(&child).ok_or_else(|| {
                                        Stop::unsupported(CaptureReason::InvalidNativeGraph)
                                    })?,
                                });
                            }
                            CapturedMarker::String {
                                key: node.key().into(),
                                edges,
                            }
                        }
                        MarkerTreeKind::In(_)
                        | MarkerTreeKind::Contains(_)
                        | MarkerTreeKind::List(_)
                        | MarkerTreeKind::Extra(_) => {
                            return Err(Stop::unsupported(CaptureReason::InvalidNativeGraph));
                        }
                    };
                    let id = u32::try_from(self.markers.len())
                        .map_err(|_| Stop::truncated(CaptureReason::MarkerNodes))?;
                    self.markers.push(captured);
                    active.remove(&marker);
                    self.marker_ids.insert(marker, id);
                }
            }
        }
        self.marker_ids
            .get(&root)
            .copied()
            .ok_or_else(|| Stop::unsupported(CaptureReason::InvalidNativeGraph))
    }

    fn preflight_version(&mut self, version: &Version) -> Result<(), Stop> {
        self.budget.work(1)?;
        let release = version.release();
        self.budget.components(release.len())?;
        self.budget.decimal(version.epoch())?;
        for component in &*release {
            self.budget.decimal(*component)?;
        }
        if let Some(pre) = version.pre() {
            self.budget.decimal(pre.number)?;
        }
        for component in [
            version.post(),
            version.dev(),
            Version::min(version),
            Version::max(version),
        ]
        .into_iter()
        .flatten()
        {
            self.budget.decimal(component)?;
        }
        match version.local() {
            LocalVersionSlice::Segments(segments) => {
                self.budget.components(segments.len())?;
                for segment in segments {
                    match segment {
                        LocalSegment::String(value) => self.budget.atom(value.len())?,
                        LocalSegment::Number(value) => self.budget.decimal(*value)?,
                    }
                }
            }
            LocalVersionSlice::Max => {}
        }
        Ok(())
    }

    fn version(&mut self, version: &Version) -> Result<EncodedVersion, Stop> {
        self.preflight_version(version)?;
        EncodedVersion::try_from(version)
            .map_err(|_| Stop::unsupported(CaptureReason::UnsupportedVersion))
    }

    fn version_ranges(&mut self, ranges: &Ranges<Version>) -> Result<EncodedVersionRanges, Stop> {
        for (lower, upper) in ranges.iter() {
            self.budget.charge(Resource::Intervals, 1)?;
            self.budget.work(1)?;
            for bound in [lower, upper] {
                if let Bound::Included(version) | Bound::Excluded(version) = bound {
                    self.preflight_version(version)?;
                }
            }
        }
        EncodedVersionRanges::try_from(ranges)
            .map_err(|_| Stop::unsupported(CaptureReason::UnsupportedRange))
    }

    fn range(&mut self, range: &Range<Version>) -> Result<CapturedRange, Stop> {
        self.budget.work(1)?;
        Ok(CapturedRange {
            encoded: self.version_ranges(range.encoded_versions())?,
            logical: range
                .canonical_versions()
                .map(|versions| self.version_ranges(versions))
                .transpose()?,
        })
    }

    fn string_bound(&mut self, bound: Bound<&ArcStr>) -> Result<StringBound, Stop> {
        Ok(match bound {
            Bound::Unbounded => StringBound::Unbounded,
            Bound::Included(value) => StringBound::Included(self.budget.string(value.as_str())?),
            Bound::Excluded(value) => StringBound::Excluded(self.budget.string(value.as_str())?),
        })
    }

    fn string_ranges(&mut self, ranges: &Ranges<ArcStr>) -> Result<Vec<StringInterval>, Stop> {
        let mut intervals = Vec::new();
        for (lower, upper) in ranges.iter() {
            self.budget.charge(Resource::Intervals, 1)?;
            self.budget.work(1)?;
            intervals.push(StringInterval {
                lower: self.string_bound(lower)?,
                upper: self.string_bound(upper)?,
            });
        }
        Ok(intervals)
    }

    fn environment(&mut self, env: &ResolverEnvironment) -> Result<CapturedEnvironment, Stop> {
        let Some((marker, initial_forks, include, exclude)) = env.capture_parts() else {
            return Err(Stop::unsupported(CaptureReason::SpecificEnvironment));
        };
        let marker = self.marker(marker)?;
        let mut forks = Vec::new();
        for fork in initial_forks {
            self.budget.work(1)?;
            forks.push(self.marker(*fork)?);
        }
        let mut captured_include = Vec::new();
        for item in include {
            captured_include.push(self.conflict(item)?);
        }
        let mut captured_exclude = Vec::new();
        for item in exclude {
            captured_exclude.push(self.conflict(item)?);
        }
        Ok(CapturedEnvironment {
            marker,
            initial_forks: forks,
            include: captured_include,
            exclude: captured_exclude,
        })
    }

    fn conflict(&mut self, item: &ConflictItem) -> Result<CapturedConflict, Stop> {
        self.budget.work(1)?;
        let package = self.budget.string(item.package().as_ref())?;
        Ok(match item.kind() {
            ConflictKind::Project => CapturedConflict::Project { package },
            ConflictKind::Extra(extra) => CapturedConflict::Extra {
                package,
                extra: self.budget.string(extra.as_ref())?,
            },
            ConflictKind::Group(group) => CapturedConflict::Group {
                package,
                group: self.budget.string(group.as_ref())?,
            },
        })
    }

    fn python(&mut self, python: &PythonRequirement) -> Result<CapturedPython, Stop> {
        Ok(CapturedPython {
            source: match python.source() {
                PythonRequirementSource::PythonVersion => CapturedPythonSource::PythonVersion,
                PythonRequirementSource::RequiresPython => CapturedPythonSource::RequiresPython,
                PythonRequirementSource::Interpreter => CapturedPythonSource::Interpreter,
            },
            exact: self.version(python.exact())?,
            installed: self.python_domain(python.installed())?,
            target: self.python_domain(python.target())?,
            target_marker: self.marker(python.to_marker_tree())?,
        })
    }

    fn version_bound(&mut self, bound: Bound<&Version>) -> Result<VersionBound, Stop> {
        Ok(match bound {
            Bound::Unbounded => VersionBound::Unbounded,
            Bound::Included(version) => VersionBound::Included(self.version(version)?),
            Bound::Excluded(version) => VersionBound::Excluded(self.version(version)?),
        })
    }

    fn python_domain(&mut self, python: &RequiresPython) -> Result<CapturedPythonDomain, Stop> {
        self.budget.charge(Resource::Intervals, 1)?;
        self.budget.work(1)?;
        let lower = self.version_bound(python.range().lower().0.as_ref())?;
        let upper = self.version_bound(python.range().upper().0.as_ref())?;
        let mut specifiers = Vec::new();
        for specifier in python.specifiers().iter() {
            self.budget.work(1)?;
            specifiers.push(CapturedSpecifier {
                operator: (*specifier.operator()).into(),
                version: self.version(specifier.version())?,
            });
        }
        Ok(CapturedPythonDomain {
            lower,
            upper,
            specifiers,
        })
    }

    fn observations(
        &mut self,
        context: &CaptureContext<'_>,
    ) -> Result<Vec<CapturedObservation>, Stop> {
        let mut observations = Vec::new();
        // Move the already length-checked name set out to avoid cloning it for this walk.
        let names = std::mem::take(&mut self.names);
        let unavailable = context.unavailable_packages.pin();
        let incomplete = context.incomplete_packages.pin();
        for name in names {
            self.budget.charge(Resource::AvailabilityEntries, 1)?;
            self.budget.work(1)?;
            let source = if let Some(url) = context.fork_urls.get(&name) {
                match &url.parsed_url {
                    ParsedUrl::Archive(_) => CapturedSource::Archive,
                    ParsedUrl::Path(_) => CapturedSource::Path,
                    ParsedUrl::Directory(_) => CapturedSource::Directory,
                    ParsedUrl::GitDirectory(_) => CapturedSource::GitDirectory,
                    ParsedUrl::GitPath(_) => CapturedSource::GitPath,
                }
            } else if context.fork_indexes.get(&name).is_some() {
                CapturedSource::ExplicitIndex
            } else {
                CapturedSource::Registry
            };
            let response = match source {
                CapturedSource::Registry => context.index.implicit().get(&name),
                CapturedSource::ExplicitIndex => {
                    let index = context
                        .fork_indexes
                        .get(&name)
                        .ok_or_else(|| Stop::unsupported(CaptureReason::InvalidNativeGraph))?;
                    // The lookup hashes the URL. Bound that operation without copying or
                    // serializing a URL, its credentials, or any original spelling.
                    self.budget.check_atom(index.url().url().as_str().len())?;
                    context
                        .index
                        .explicit()
                        .get(&(name.clone(), index.url().clone()))
                }
                CapturedSource::Archive
                | CapturedSource::Path
                | CapturedSource::Directory
                | CapturedSource::GitDirectory
                | CapturedSource::GitPath => None,
            };
            let mut listed_versions = Vec::new();
            let listing = match response.as_deref() {
                Some(VersionsResponse::Found(maps)) => {
                    for map in maps {
                        self.budget.work(1)?;
                        for (version, _) in map.iter(&Ranges::full()) {
                            self.budget.charge(Resource::AvailabilityEntries, 1)?;
                            listed_versions.push(self.version(version)?);
                        }
                    }
                    CapturedListing::Found
                }
                Some(VersionsResponse::NotFound) => CapturedListing::NotFound,
                Some(VersionsResponse::NoIndex) => CapturedListing::NoIndex,
                Some(VersionsResponse::Offline) => CapturedListing::Offline,
                None if matches!(
                    source,
                    CapturedSource::Registry | CapturedSource::ExplicitIndex
                ) =>
                {
                    CapturedListing::Unobserved
                }
                None => CapturedListing::NotApplicable,
            };
            let mut known_versions = Vec::new();
            if let Some(versions) = context.known_versions.get(&name) {
                for version in &**versions {
                    self.budget.charge(Resource::AvailabilityEntries, 1)?;
                    known_versions.push(self.version(version)?);
                }
            }
            let mut incomplete_versions = Vec::new();
            if let Some(versions) = incomplete.get(&name) {
                for (version, reason) in &versions.pin() {
                    self.budget.charge(Resource::AvailabilityEntries, 1)?;
                    incomplete_versions.push(CapturedMetadataFact {
                        version: self.version(version)?,
                        reason: reason.into(),
                    });
                }
            }
            observations.push(CapturedObservation {
                name: self.budget.string(name.as_ref())?,
                source,
                has_url_policy: context.urls.any_url(&name)
                    || context.fork_urls.contains_key(&name),
                has_index_policy: context.indexes.contains_key(&name)
                    || context.fork_indexes.get(&name).is_some(),
                listing,
                listed_versions,
                known_versions,
                unavailable: unavailable.get(&name).map(CapturedReason::from),
                incomplete: incomplete_versions,
            });
        }
        Ok(observations)
    }
}

impl From<CanonicalMarkerValueVersion> for VersionMarkerKey {
    fn from(key: CanonicalMarkerValueVersion) -> Self {
        match key {
            CanonicalMarkerValueVersion::ImplementationVersion => Self::ImplementationVersion,
            CanonicalMarkerValueVersion::PythonFullVersion => Self::PythonFullVersion,
        }
    }
}

impl From<CanonicalMarkerValueString> for StringMarkerKey {
    fn from(key: CanonicalMarkerValueString) -> Self {
        match key {
            CanonicalMarkerValueString::OsName => Self::OsName,
            CanonicalMarkerValueString::SysPlatform => Self::SysPlatform,
            CanonicalMarkerValueString::PlatformSystem => Self::PlatformSystem,
            CanonicalMarkerValueString::PlatformMachine => Self::PlatformMachine,
            CanonicalMarkerValueString::PlatformPythonImplementation => {
                Self::PlatformPythonImplementation
            }
            CanonicalMarkerValueString::PlatformRelease => Self::PlatformRelease,
            CanonicalMarkerValueString::PlatformVersion => Self::PlatformVersion,
            CanonicalMarkerValueString::ImplementationName => Self::ImplementationName,
        }
    }
}

impl From<Operator> for CapturedOperator {
    fn from(operator: Operator) -> Self {
        match operator {
            Operator::Equal => Self::Equal,
            Operator::EqualStar => Self::EqualStar,
            Operator::ExactEqual => Self::ExactEqual,
            Operator::NotEqual => Self::NotEqual,
            Operator::NotEqualStar => Self::NotEqualStar,
            Operator::TildeEqual => Self::TildeEqual,
            Operator::LessThan => Self::LessThan,
            Operator::LessThanEqual => Self::LessThanEqual,
            Operator::GreaterThan => Self::GreaterThan,
            Operator::GreaterThanEqual => Self::GreaterThanEqual,
        }
    }
}

impl CapturedReason {
    const fn plain(kind: CapturedReasonKind) -> Self {
        Self {
            kind,
            http_status: None,
        }
    }

    fn network(kind: CapturedReasonKind, status: reqwest::StatusCode) -> Self {
        Self {
            kind,
            http_status: Some(status.as_u16()),
        }
    }
}

impl From<&UnavailableReason> for CapturedReason {
    fn from(reason: &UnavailableReason) -> Self {
        match reason {
            UnavailableReason::Package(reason) => reason.into(),
            UnavailableReason::Version(reason) => reason.into(),
        }
    }
}

impl From<&UnavailablePackage> for CapturedReason {
    fn from(reason: &UnavailablePackage) -> Self {
        match reason {
            UnavailablePackage::NoIndex => Self::plain(CapturedReasonKind::PackageNoIndex),
            UnavailablePackage::Offline => Self::plain(CapturedReasonKind::PackageOffline),
            UnavailablePackage::NotFound => Self::plain(CapturedReasonKind::PackageNotFound),
            UnavailablePackage::InvalidMetadata(_) => {
                Self::plain(CapturedReasonKind::PackageInvalidMetadata)
            }
            UnavailablePackage::InvalidStructure(_) => {
                Self::plain(CapturedReasonKind::PackageInvalidStructure)
            }
            UnavailablePackage::Network(status) => {
                Self::network(CapturedReasonKind::PackageNetwork, *status)
            }
        }
    }
}

impl From<&UnavailableVersion> for CapturedReason {
    fn from(reason: &UnavailableVersion) -> Self {
        match reason {
            UnavailableVersion::UnsatisfiableDependency(_) => {
                Self::plain(CapturedReasonKind::VersionUnsatisfiableDependency)
            }
            UnavailableVersion::IncompatibleSelfDependency(_) => {
                Self::plain(CapturedReasonKind::VersionIncompatibleSelfDependency)
            }
            UnavailableVersion::IncompatibleDist(_) => {
                Self::plain(CapturedReasonKind::VersionIncompatibleDist)
            }
            UnavailableVersion::InvalidMetadata => {
                Self::plain(CapturedReasonKind::VersionInvalidMetadata)
            }
            UnavailableVersion::InconsistentMetadata => {
                Self::plain(CapturedReasonKind::VersionInconsistentMetadata)
            }
            UnavailableVersion::InvalidStructure => {
                Self::plain(CapturedReasonKind::VersionInvalidStructure)
            }
            UnavailableVersion::Offline => Self::plain(CapturedReasonKind::VersionOffline),
            UnavailableVersion::RequiresPython(_) => {
                Self::plain(CapturedReasonKind::VersionRequiresPython)
            }
            UnavailableVersion::Network(status) => {
                Self::network(CapturedReasonKind::VersionNetwork, *status)
            }
        }
    }
}

impl From<&MetadataUnavailable> for CapturedReason {
    fn from(reason: &MetadataUnavailable) -> Self {
        match reason {
            MetadataUnavailable::Offline => Self::plain(CapturedReasonKind::MetadataOffline),
            MetadataUnavailable::InvalidMetadata(_) => {
                Self::plain(CapturedReasonKind::MetadataInvalidMetadata)
            }
            MetadataUnavailable::InconsistentMetadata(_) => {
                Self::plain(CapturedReasonKind::MetadataInconsistentMetadata)
            }
            MetadataUnavailable::InvalidStructure(_) => {
                Self::plain(CapturedReasonKind::MetadataInvalidStructure)
            }
            MetadataUnavailable::RequiresPython(..) => {
                Self::plain(CapturedReasonKind::MetadataRequiresPython)
            }
            MetadataUnavailable::Network(status) => {
                Self::network(CapturedReasonKind::MetadataNetwork, *status)
            }
        }
    }
}
