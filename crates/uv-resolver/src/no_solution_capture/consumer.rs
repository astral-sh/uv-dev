//! Conservative classification against an independently constructed finite package inventory.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Bound;

use pubgrub::Ranges;
use serde::Serialize;
use uv_distribution_types::RequiresPython;
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::{
    EncodedVersion, LocalVersionSlice, MIN_VERSION, Operator, Version, VersionSpecifier,
    VersionSpecifiers, release_specifiers_to_ranges,
};
use uv_pep508::MarkerTree;

use super::NoSolutionEvidence;
use super::budget::{Budget, CaptureLimits, Resource, Stop};
use super::reader::CaptureReadError;
use super::wire::{
    CaptureMetadata, CaptureOperation, CaptureOptions, CaptureReason, CaptureScope, CaptureStatus,
    CapturedGraph, CapturedListing, CapturedMarker, CapturedNode, CapturedObservation,
    CapturedOperator, CapturedPackage, CapturedPython, CapturedPythonDomain, CapturedPythonSource,
    CapturedReasonKind, CapturedSource, VersionBound,
};

mod environment;
#[cfg(test)]
mod tests;

use environment::ExpectedEnvironments;

/// A bounded inventory supplied by the source-bound scenario checker, not by a captured graph.
///
/// Register every independently declared package and every name referenced by project or package
/// metadata. A reference to a missing name is an explicit empty inventory; encountering a name
/// only in a capture never creates one. This type does not establish that an index was served or
/// that a project is satisfiable. Those are separate obligations of the real caller.
pub struct ClosedWorldInventory {
    project: String,
    project_version: Version,
    project_extras: BTreeSet<String>,
    project_groups: BTreeSet<String>,
    registry: BTreeMap<String, RegistryPackage>,
    python: ExpectedPython,
    environments: Option<ExpectedEnvironments>,
    budget: Budget,
    failed: bool,
}

#[derive(Default)]
struct RegistryPackage {
    declared: bool,
    versions: BTreeSet<Version>,
    extras: BTreeSet<String>,
}

struct ExpectedPython {
    exact: Version,
    installed: RequiresPython,
    target: RequiresPython,
    ranges: Ranges<Version>,
}

impl fmt::Debug for ClosedWorldInventory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClosedWorldInventory")
            .field("project", &self.project)
            .field("registry_packages", &self.registry.len())
            .field("usage", &self.budget.usage)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl ClosedWorldInventory {
    /// Start an inventory for the generated directory project at its static `0.0.0` version.
    /// `interpreter` must be the independently queried version of the selected interpreter.
    pub fn new(
        project: &PackageName,
        requires_python: &VersionSpecifiers,
        interpreter: &Version,
    ) -> Result<Self, ClosedWorldNoSolutionError> {
        Self::with_limits(project, requires_python, interpreter, CaptureLimits::V1)
    }

    fn with_limits(
        project: &PackageName,
        requires_python: &VersionSpecifiers,
        interpreter: &Version,
        limits: CaptureLimits,
    ) -> Result<Self, ClosedWorldNoSolutionError> {
        let mut budget = Budget::new(limits);
        budget.charge(Resource::Packages, 1)?;
        let project = budget.string(project.as_ref())?;
        budget.version(interpreter)?;
        if !ordinary_version(interpreter) || interpreter.only_release() != *interpreter {
            return Err(unsupported("unsupported interpreter version"));
        }
        for specifier in requires_python.iter() {
            budget.charge(Resource::Intervals, 1)?;
            budget.version(specifier.version())?;
            if !ordinary_version(specifier.version()) {
                return Err(unsupported("unsupported project Python specifier"));
            }
        }
        // The workspace uses this existing release-only intersection to canonicalize its Python
        // policy. Charge its possible interval work before cloning the input specifiers.
        budget.work(requires_python.len().saturating_mul(requires_python.len()))?;
        let Some(target) = RequiresPython::intersection(std::iter::once(requires_python)) else {
            return Err(unsupported("empty project Python policy"));
        };
        let ranges = release_specifiers_to_ranges(target.specifiers().clone());
        let python3 = Ranges::from_range_bounds((
            Bound::Included(Version::new([3])),
            Bound::Excluded(Version::new([4])),
        ));
        if ranges.is_empty() || !ranges.subset_of(&python3) || !ranges.contains(interpreter) {
            return Err(unsupported("unsupported project Python policy"));
        }
        let exact = interpreter.clone().without_trailing_zeros();
        let installed = RequiresPython::greater_than_equal_version(&exact.only_release());
        Ok(Self {
            project,
            project_version: Version::new([0, 0, 0]),
            project_extras: BTreeSet::new(),
            project_groups: BTreeSet::new(),
            registry: BTreeMap::new(),
            python: ExpectedPython {
                exact,
                installed,
                target,
                ranges,
            },
            environments: None,
            budget,
            failed: false,
        })
    }

    /// Bind the configured supported environments of a fresh workspace lock.
    ///
    /// The ordered entries are independent project inputs, not forks learned from a lockfile or
    /// a capture. They must be disjoint ordinary marker domains that intersect the project Python
    /// policy.
    /// Satisfiability of the project over their union remains a separate caller obligation.
    pub fn set_supported_environments(
        &mut self,
        environments: &[MarkerTree],
    ) -> Result<(), ClosedWorldNoSolutionError> {
        self.mutate(|inventory| {
            if inventory.environments.is_some() {
                return Err(unsupported("supported environments were already supplied"));
            }
            inventory.environments = Some(ExpectedEnvironments::new(
                environments,
                &inventory.python.target,
                &mut inventory.budget,
            )?);
            Ok(())
        })
    }

    /// Register a declared optional dependency of the generated project.
    pub fn add_project_extra(
        &mut self,
        extra: &ExtraName,
    ) -> Result<(), ClosedWorldNoSolutionError> {
        self.mutate(|inventory| {
            inventory.budget.charge(Resource::AvailabilityEntries, 1)?;
            let extra = inventory.budget.string(extra.as_ref())?;
            if !inventory.project_extras.insert(extra) {
                return Err(unsupported("duplicate project extra"));
            }
            Ok(())
        })
    }

    /// Register a declared dependency group of the generated project.
    pub fn add_project_group(
        &mut self,
        group: &GroupName,
    ) -> Result<(), ClosedWorldNoSolutionError> {
        self.mutate(|inventory| {
            inventory.budget.charge(Resource::AvailabilityEntries, 1)?;
            let group = inventory.budget.string(group.as_ref())?;
            if !inventory.project_groups.insert(group) {
                return Err(unsupported("duplicate project dependency group"));
            }
            Ok(())
        })
    }

    /// Register one independently declared ordinary-registry package and every raw version.
    pub fn add_registry_package<'version>(
        &mut self,
        name: &PackageName,
        versions: impl IntoIterator<Item = &'version Version>,
    ) -> Result<(), ClosedWorldNoSolutionError> {
        self.mutate(|inventory| {
            inventory.reference_registry(name)?;
            let package = inventory
                .registry
                .get_mut(name.as_ref())
                .ok_or_else(|| unsupported("missing inventory entry"))?;
            if std::mem::replace(&mut package.declared, true) {
                return Err(unsupported("duplicate registry package"));
            }
            for version in versions {
                inventory.budget.charge(Resource::AvailabilityEntries, 1)?;
                inventory.budget.version(version)?;
                if !ordinary_version(version) {
                    return Err(unsupported(
                        "an inventory candidate is an internal sentinel",
                    ));
                }
                if !package.versions.insert(version.clone()) {
                    return Err(unsupported("duplicate registry version"));
                }
            }
            Ok(())
        })
    }

    /// Register a name referenced by independent project or distribution metadata.
    pub fn add_registry_reference(
        &mut self,
        name: &PackageName,
    ) -> Result<(), ClosedWorldNoSolutionError> {
        self.mutate(|inventory| inventory.reference_registry(name))
    }

    /// Register an extra declared or requested by independent metadata.
    ///
    /// This admits an identity, not an activation or a claim that every version declares it.
    pub fn add_registry_extra(
        &mut self,
        name: &PackageName,
        extra: &ExtraName,
    ) -> Result<(), ClosedWorldNoSolutionError> {
        self.mutate(|inventory| {
            inventory.reference_registry(name)?;
            inventory.budget.charge(Resource::AvailabilityEntries, 1)?;
            let extra = inventory.budget.string(extra.as_ref())?;
            inventory
                .registry
                .get_mut(name.as_ref())
                .ok_or_else(|| unsupported("missing inventory entry"))?
                .extras
                .insert(extra);
            Ok(())
        })
    }

    fn reference_registry(&mut self, name: &PackageName) -> Result<(), ClosedWorldNoSolutionError> {
        // Charge the spelling before comparing or looking it up, including repeated references.
        self.budget.atom(name.as_ref().len())?;
        if name.as_ref() == self.project {
            return Err(unsupported(
                "registry metadata targets the generated project",
            ));
        }
        if !self.registry.contains_key(name.as_ref()) {
            self.budget.charge(Resource::Packages, 1)?;
            self.registry
                .insert(name.to_string(), RegistryPackage::default());
        }
        Ok(())
    }

    fn mutate(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<(), ClosedWorldNoSolutionError>,
    ) -> Result<(), ClosedWorldNoSolutionError> {
        if self.failed {
            return Err(unsupported("inventory construction already failed"));
        }
        let result = operation(self);
        self.failed = result.is_err();
        result
    }

    fn classification_budget(&self) -> Result<Budget, ClosedWorldNoSolutionError> {
        if self.failed {
            return Err(unsupported("inventory construction failed"));
        }
        Ok(Budget {
            limits: self.budget.limits,
            usage: self.budget.usage,
        })
    }
}

fn ordinary_version(version: &Version) -> bool {
    if EncodedVersion::try_from(version).is_err()
        || matches!(version.local(), LocalVersionSlice::Max)
    {
        return false;
    }
    // The checked codec rejects unsupported combinations. The native sentinel setters also
    // require their valid combinations when clearing a component, so skip the forbidden cases.
    let mut ordinary = version.clone();
    if !version.is_pre() && !version.is_dev() {
        ordinary = ordinary.with_min(None);
    }
    if !version.is_post() && !version.is_dev() {
        ordinary = ordinary.with_max(None);
    }
    *version == ordinary
}

/// The original capture's typed failure and absent ranges agree with the admitted inventory.
///
/// This is not a verified PubGrub proof or a satisfiability certificate. The independent caller
/// must establish the whole-project witness and the actual command and served-source policy.
#[derive(Debug, Serialize)]
pub struct ClosedWorldNoSolution {
    grammar: &'static str,
    project: String,
    original_nodes: usize,
    external_leaves: usize,
    checked_no_versions: Vec<u32>,
}

/// The capture or its independently supplied inventory is outside the supported classifier.
#[derive(Debug)]
pub struct ClosedWorldNoSolutionError(ErrorKind);

#[derive(Debug)]
enum ErrorKind {
    Capture(CaptureReadError),
    Unsupported(&'static str),
    Limit(CaptureReason),
}

impl fmt::Display for ClosedWorldNoSolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::Capture(error) => fmt::Display::fmt(error, formatter),
            ErrorKind::Unsupported(reason) => {
                write!(
                    formatter,
                    "unsupported closed-world no-solution evidence: {reason}"
                )
            }
            ErrorKind::Limit(reason) => {
                write!(
                    formatter,
                    "closed-world evidence limit exceeded: {reason:?}"
                )
            }
        }
    }
}

impl std::error::Error for ClosedWorldNoSolutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            ErrorKind::Capture(error) => Some(error),
            ErrorKind::Unsupported(_) | ErrorKind::Limit(_) => None,
        }
    }
}

impl From<Stop> for ClosedWorldNoSolutionError {
    fn from(stop: Stop) -> Self {
        Self(ErrorKind::Limit(stop.reason))
    }
}

fn unsupported(reason: &'static str) -> ClosedWorldNoSolutionError {
    ClosedWorldNoSolutionError(ErrorKind::Unsupported(reason))
}

/// Classify a direct workspace lock's original no-solution capture against raw candidate data.
///
/// The caller must separately bind exit status, stdout, executable identity, strict index and
/// configuration policy, the actual served bytes, and a fresh whole-domain SAT certificate.
pub fn classify_closed_world_no_solution(
    bytes: &[u8],
    expected: &CaptureOptions,
    inventory: &ClosedWorldInventory,
) -> Result<ClosedWorldNoSolution, ClosedWorldNoSolutionError> {
    let evidence = NoSolutionEvidence::from_json(bytes, &expected.token)
        .map_err(|error| ClosedWorldNoSolutionError(ErrorKind::Capture(error)))?
        .0;
    if expected.scope != CaptureScope::Workspace
        || expected.operation != CaptureOperation::Write
        || evidence.scope != expected.scope
        || evidence.operation != expected.operation
        || evidence.metadata != expected.metadata
        || evidence.status != CaptureStatus::Complete
    {
        return Err(unsupported("the envelope is not the selected direct lock"));
    }
    // Both metadata representations have the same resolution and evidence contract.
    match expected.metadata {
        CaptureMetadata::Standard | CaptureMetadata::WithoutMetadata => {}
    }
    let Some(graph) = evidence.graph else {
        return Err(unsupported("the capture has no complete graph"));
    };
    let mut budget = inventory.classification_budget()?;
    // The checked reader proves these structural counters match the private graph. Reserve its
    // complete interval/text conversion cost before moving any encoded range into native values.
    budget.charge(Resource::Intervals, evidence.usage.intervals)?;
    budget.charge(Resource::TextBytes, evidence.usage.text_bytes)?;
    budget.work(evidence.usage.intervals)?;
    budget.work(evidence.usage.availability_entries)?;
    classify_graph(graph, inventory, &mut budget)
}

#[derive(Clone, Copy)]
enum AdmittedPackage<'inventory> {
    Root,
    Project,
    Registry(&'inventory RegistryPackage),
}

fn classify_graph(
    graph: CapturedGraph,
    inventory: &ClosedWorldInventory,
    budget: &mut Budget,
) -> Result<ClosedWorldNoSolution, ClosedWorldNoSolutionError> {
    if graph.index_authentication.unauthorized || graph.index_authentication.forbidden {
        return Err(unsupported("an index authentication failure was observed"));
    }
    if graph.workspace_members.as_slice() != [inventory.project.as_str()]
        || !graph.environment.include.is_empty()
        || !graph.environment.exclude.is_empty()
    {
        return Err(unsupported("unsupported workspace or conflict policy"));
    }
    let effective_python = check_python(
        &graph.original_python,
        &graph.effective_python,
        inventory,
        budget,
    )?;
    if let Some(environments) = &inventory.environments {
        environments.check(&graph, &effective_python, budget)?;
    } else if !graph.environment.initial_forks.is_empty() {
        return Err(unsupported("unsupported workspace or conflict policy"));
    }
    let root_package = graph.root_package as usize;
    let mut admitted = Vec::new();
    for (index, package) in graph.packages.iter().enumerate() {
        budget.work(1)?;
        let kind = match package {
            CapturedPackage::Root { name: None } if index == root_package => AdmittedPackage::Root,
            CapturedPackage::Root { .. }
            | CapturedPackage::Python { .. }
            | CapturedPackage::System { .. } => {
                return Err(unsupported("unsupported package identity"));
            }
            CapturedPackage::Package {
                name,
                extra,
                group,
                marker,
            } => admit_named_package(
                name,
                extra.as_deref(),
                group.as_deref(),
                *marker,
                &graph,
                inventory,
                budget,
            )?,
            CapturedPackage::Extra {
                name,
                extra,
                marker,
            } => admit_named_package(name, Some(extra), None, *marker, &graph, inventory, budget)?,
            CapturedPackage::Group {
                name,
                group,
                marker,
            } => admit_named_package(name, None, Some(group), *marker, &graph, inventory, budget)?,
            CapturedPackage::Marker { name, marker } => {
                if name == &inventory.project {
                    return Err(unsupported("unexpected generated-project marker proxy"));
                }
                admit_named_package(name, None, None, *marker, &graph, inventory, budget)?
            }
        };
        admitted.push(kind);
    }
    for observation in graph.observations {
        check_observation(observation, inventory, budget)?;
    }

    let original_nodes = graph.nodes.len();
    budget.work(graph.packages.len().saturating_mul(2))?;
    let mut rooted = vec![false; graph.packages.len()];
    let mut metadata = vec![false; graph.packages.len()];
    let mut proxy_edges = Vec::new();
    let mut checked_no_versions = Vec::new();
    let mut external_leaves = 0;
    for (node_id, node) in graph.nodes.into_iter().enumerate() {
        budget.work(1)?;
        match node {
            CapturedNode::NotRoot { package, version } => {
                external_leaves += 1;
                if package as usize != root_package || version.into_version() != *MIN_VERSION {
                    return Err(unsupported("unexpected synthetic-root leaf"));
                }
            }
            CapturedNode::NoVersions { package, range } => {
                external_leaves += 1;
                let range = range.encoded.into_ranges();
                let intervals = range.iter().count();
                let intersects = match admitted[package as usize] {
                    AdmittedPackage::Root => {
                        return Err(unsupported("synthetic-root absence claim"));
                    }
                    AdmittedPackage::Project => {
                        budget.work(intervals.saturating_add(1))?;
                        range.contains(&inventory.project_version)
                    }
                    AdmittedPackage::Registry(package) => {
                        // This is the native ordered membership operation, over every raw
                        // candidate. Neither the logical range nor candidate filtering is used.
                        budget.work(intervals.saturating_add(package.versions.len()))?;
                        range
                            .contains_many(package.versions.iter())
                            .any(|present| present)
                    }
                };
                if intersects {
                    return Err(unsupported("an absent range contains a raw candidate"));
                }
                checked_no_versions.push(
                    u32::try_from(node_id)
                        .map_err(|_| unsupported("derivation node identity overflow"))?,
                );
            }
            CapturedNode::FromDependencyOf {
                package,
                range,
                dependency,
                dependency_range,
            } => {
                external_leaves += 1;
                let parent = package as usize;
                let dependency = dependency as usize;
                match (admitted[parent], admitted[dependency]) {
                    (AdmittedPackage::Root, AdmittedPackage::Project) => {
                        let range = range.encoded.into_ranges();
                        let dependency_range = dependency_range.encoded.into_ranges();
                        if range.as_singleton() != Some(&*MIN_VERSION)
                            || dependency_range != Ranges::full()
                        {
                            return Err(unsupported("unexpected workspace-root dependency"));
                        }
                        rooted[dependency] = true;
                    }
                    (AdmittedPackage::Project, AdmittedPackage::Project) => {
                        if !project_proxy_dependency(
                            &graph.packages[parent],
                            &graph.packages[dependency],
                        ) || range.encoded.into_ranges().as_singleton()
                            != Some(&inventory.project_version)
                            || dependency_range.encoded.into_ranges().as_singleton()
                                != Some(&inventory.project_version)
                        {
                            return Err(unsupported("unexpected generated-project proxy edge"));
                        }
                        proxy_edges.push((parent, dependency));
                    }
                    (AdmittedPackage::Project, AdmittedPackage::Registry(_)) => {
                        if !is_package_identity(&graph.packages[parent])
                            || range.encoded.into_ranges().as_singleton()
                                != Some(&inventory.project_version)
                        {
                            return Err(unsupported("missing exact project metadata context"));
                        }
                        metadata[parent] = true;
                    }
                    (AdmittedPackage::Registry(_), AdmittedPackage::Registry(_)) => {}
                    (
                        AdmittedPackage::Root,
                        AdmittedPackage::Root | AdmittedPackage::Registry(_),
                    )
                    | (AdmittedPackage::Project, AdmittedPackage::Root)
                    | (
                        AdmittedPackage::Registry(_),
                        AdmittedPackage::Root | AdmittedPackage::Project,
                    ) => {
                        return Err(unsupported("unexpected dependency source context"));
                    }
                }
            }
            CapturedNode::Custom { .. } => {
                return Err(unsupported(
                    "custom resolver leaves are not inventory proofs",
                ));
            }
            CapturedNode::Derived { terms, .. } => budget.work(terms.len())?,
        }
    }
    // A project proxy only points to its corresponding Package identity, so these edges have
    // depth one after the direct synthetic-root edges. No graph search or proof rewrite is needed.
    for (parent, dependency) in proxy_edges {
        budget.work(1)?;
        if rooted[parent] {
            rooted[dependency] = true;
        }
    }
    let mut metadata_seen = false;
    for (index, package) in admitted.iter().enumerate() {
        match package {
            AdmittedPackage::Project => {
                if !rooted[index] {
                    return Err(unsupported("unrooted generated-project identity"));
                }
                metadata_seen |= metadata[index];
            }
            AdmittedPackage::Root | AdmittedPackage::Registry(_) => {}
        }
    }
    if !metadata_seen {
        return Err(unsupported(
            "the original graph does not show project metadata",
        ));
    }
    budget.atom(inventory.project.len())?;
    Ok(ClosedWorldNoSolution {
        grammar: "uv-closed-world-no-solution-v1",
        project: inventory.project.clone(),
        original_nodes,
        external_leaves,
        checked_no_versions,
    })
}

fn admit_named_package<'inventory>(
    name: &str,
    extra: Option<&str>,
    group: Option<&str>,
    marker: u32,
    graph: &CapturedGraph,
    inventory: &'inventory ClosedWorldInventory,
    budget: &mut Budget,
) -> Result<AdmittedPackage<'inventory>, ClosedWorldNoSolutionError> {
    budget.atom(name.len())?;
    if let Some(extra) = extra {
        budget.atom(extra.len())?;
    }
    if let Some(group) = group {
        budget.atom(group.len())?;
    }
    if name == inventory.project {
        let unconditional = match &graph.markers[marker as usize] {
            CapturedMarker::True => true,
            CapturedMarker::False
            | CapturedMarker::Version { .. }
            | CapturedMarker::String { .. } => false,
        };
        if !unconditional
            || extra.is_some_and(|extra| !inventory.project_extras.contains(extra))
            || group.is_some_and(|group| !inventory.project_groups.contains(group))
        {
            return Err(unsupported("unexpected generated-project selection"));
        }
        return Ok(AdmittedPackage::Project);
    }
    let Some(package) = inventory.registry.get(name) else {
        return Err(unsupported("package is outside the independent inventory"));
    };
    if group.is_some() || extra.is_some_and(|extra| !package.extras.contains(extra)) {
        return Err(unsupported("unsupported registry selection"));
    }
    Ok(AdmittedPackage::Registry(package))
}

fn is_package_identity(package: &CapturedPackage) -> bool {
    match package {
        CapturedPackage::Package { .. } => true,
        CapturedPackage::Root { .. }
        | CapturedPackage::Python { .. }
        | CapturedPackage::System { .. }
        | CapturedPackage::Extra { .. }
        | CapturedPackage::Group { .. }
        | CapturedPackage::Marker { .. } => false,
    }
}

fn project_proxy_dependency(parent: &CapturedPackage, dependency: &CapturedPackage) -> bool {
    let CapturedPackage::Package {
        name,
        extra,
        group,
        marker,
    } = dependency
    else {
        return false;
    };
    match parent {
        CapturedPackage::Extra {
            name: parent_name,
            extra: parent_extra,
            marker: parent_marker,
        } => {
            name == parent_name
                && group.is_none()
                && extra.as_ref().is_none_or(|extra| extra == parent_extra)
                && marker == parent_marker
        }
        CapturedPackage::Group {
            name: parent_name,
            group: parent_group,
            marker: parent_marker,
        } => {
            name == parent_name
                && extra.is_none()
                && group.as_ref() == Some(parent_group)
                && marker == parent_marker
        }
        CapturedPackage::Root { .. }
        | CapturedPackage::Python { .. }
        | CapturedPackage::System { .. }
        | CapturedPackage::Package { .. }
        | CapturedPackage::Marker { .. } => false,
    }
}

fn check_observation(
    observation: CapturedObservation,
    inventory: &ClosedWorldInventory,
    budget: &mut Budget,
) -> Result<(), ClosedWorldNoSolutionError> {
    budget.atom(observation.name.len())?;
    if observation.name == inventory.project {
        if observation.source != CapturedSource::Directory
            || !observation.has_url_policy
            || observation.has_index_policy
            || observation.listing != CapturedListing::NotApplicable
            || !observation.listed_versions.is_empty()
            || observation.known_versions.len() > 1
            || observation
                .known_versions
                .iter()
                .any(|version| version.to_version() != inventory.project_version)
            || observation.unavailable.is_some()
            || !observation.incomplete.is_empty()
        {
            return Err(unsupported("inconsistent generated-project observation"));
        }
        return Ok(());
    }
    let Some(package) = inventory.registry.get(observation.name.as_str()) else {
        return Err(unsupported(
            "observation is outside the independent inventory",
        ));
    };
    if observation.source != CapturedSource::Registry
        || observation.has_url_policy
        || observation.has_index_policy
        || !observation.incomplete.is_empty()
    {
        return Err(unsupported(
            "unsupported registry source or metadata availability",
        ));
    }
    match observation.listing {
        CapturedListing::NotFound => {
            if !package.versions.is_empty()
                || !observation.listed_versions.is_empty()
                || !observation.known_versions.is_empty()
                || !observation.unavailable.is_some_and(|reason| {
                    reason.kind == CapturedReasonKind::PackageNotFound
                        && reason.http_status.is_none()
                })
            {
                return Err(unsupported("inconsistent empty registry observation"));
            }
        }
        CapturedListing::Found => {
            if observation.unavailable.is_some() {
                return Err(unsupported("an unavailable package has a registry listing"));
            }
            let mut listed = BTreeSet::new();
            for version in observation.listed_versions {
                budget.work(1)?;
                let version = version.into_version();
                if !package.versions.contains(&version) || !listed.insert(version) {
                    return Err(unsupported("inconsistent registry version listing"));
                }
            }
            if listed != package.versions {
                return Err(unsupported(
                    "registry listing differs from the raw inventory",
                ));
            }
            let mut known = BTreeSet::new();
            for version in observation.known_versions {
                budget.work(1)?;
                let version = version.into_version();
                if !listed.contains(&version) || !known.insert(version) {
                    return Err(unsupported("inconsistent known registry version"));
                }
            }
        }
        CapturedListing::NoIndex
        | CapturedListing::Offline
        | CapturedListing::Unobserved
        | CapturedListing::NotApplicable => {
            return Err(unsupported("registry availability was not established"));
        }
    }
    Ok(())
}

fn check_python(
    original: &CapturedPython,
    effective: &CapturedPython,
    inventory: &ClosedWorldInventory,
    budget: &mut Budget,
) -> Result<RequiresPython, ClosedWorldNoSolutionError> {
    if original.source != CapturedPythonSource::RequiresPython
        || original.exact.to_version() != inventory.python.exact
        || !domain_matches(&original.installed, &inventory.python.installed)
        || !domain_matches(&original.target, &inventory.python.target)
        || effective.source != original.source
        || effective.exact != original.exact
        || !domain_matches(&effective.installed, &inventory.python.installed)
    {
        return Err(unsupported(
            "captured Python policy differs from the project",
        ));
    }
    budget.work(
        effective
            .target
            .specifiers
            .len()
            .saturating_mul(effective.target.specifiers.len()),
    )?;
    let specifiers = effective
        .target
        .specifiers
        .iter()
        .map(|specifier| {
            let version = specifier.version.to_version();
            if !ordinary_version(&version) {
                return Err(unsupported("unsupported effective Python specifier"));
            }
            VersionSpecifier::from_version(operator(specifier.operator), version)
                .map_err(|_| unsupported("unsupported effective Python specifier"))
        })
        .collect::<Result<VersionSpecifiers, _>>()?;
    let effective_policy = RequiresPython::from_specifiers(specifiers.clone());
    let effective_ranges = release_specifiers_to_ranges(specifiers);
    if !domain_matches(&effective.target, &effective_policy)
        || effective_ranges.is_empty()
        || !effective_ranges.subset_of(&inventory.python.ranges)
    {
        return Err(unsupported(
            "effective Python policy is outside the original domain",
        ));
    }
    Ok(effective_policy)
}

fn domain_matches(domain: &CapturedPythonDomain, expected: &RequiresPython) -> bool {
    bound_matches(&domain.lower, expected.range().lower().as_ref())
        && bound_matches(&domain.upper, expected.range().upper().as_ref())
        && domain.specifiers.len() == expected.specifiers().len()
        && domain
            .specifiers
            .iter()
            .zip(expected.specifiers().iter())
            .all(|(actual, expected)| {
                operator(actual.operator) == *expected.operator()
                    && actual.version.to_version() == *expected.version()
            })
}

fn bound_matches(actual: &VersionBound, expected: Bound<&Version>) -> bool {
    match (actual, expected) {
        (VersionBound::Unbounded, Bound::Unbounded) => true,
        (VersionBound::Included(actual), Bound::Included(expected))
        | (VersionBound::Excluded(actual), Bound::Excluded(expected)) => {
            actual.to_version() == *expected
        }
        (VersionBound::Unbounded, Bound::Included(_) | Bound::Excluded(_))
        | (VersionBound::Included(_), Bound::Unbounded | Bound::Excluded(_))
        | (VersionBound::Excluded(_), Bound::Unbounded | Bound::Included(_)) => false,
    }
}

fn operator(operator: CapturedOperator) -> Operator {
    match operator {
        CapturedOperator::Equal => Operator::Equal,
        CapturedOperator::EqualStar => Operator::EqualStar,
        CapturedOperator::ExactEqual => Operator::ExactEqual,
        CapturedOperator::NotEqual => Operator::NotEqual,
        CapturedOperator::NotEqualStar => Operator::NotEqualStar,
        CapturedOperator::TildeEqual => Operator::TildeEqual,
        CapturedOperator::LessThan => Operator::LessThan,
        CapturedOperator::LessThanEqual => Operator::LessThanEqual,
        CapturedOperator::GreaterThan => Operator::GreaterThan,
        CapturedOperator::GreaterThanEqual => Operator::GreaterThanEqual,
    }
}
