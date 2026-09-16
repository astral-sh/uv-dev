use std::fmt;
use std::io::{self, Write};
use std::ops::Bound;
use std::str::FromStr;

use rustc_hash::FxHashSet;
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::{EncodedVersion, MIN_VERSION, Version};

use super::NoSolutionEvidence;
use super::budget::{Budget, CaptureLimits, CaptureUsage, Stop};
use super::wire::*;

mod preflight;

#[cfg(test)]
mod tests;

/// An internal capture was not a supported, bounded envelope for this invocation.
#[derive(Debug)]
pub struct CaptureReadError(ReadErrorKind);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadErrorKind {
    InvalidJson,
    InvalidSchema,
    InvalidLimits,
    MismatchedRequest,
    Limit(CaptureReason),
    InvalidGraph(&'static str),
}

impl fmt::Display for CaptureReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            ReadErrorKind::InvalidJson => formatter.write_str("invalid capture JSON"),
            ReadErrorKind::InvalidSchema => formatter.write_str("unsupported capture schema"),
            ReadErrorKind::InvalidLimits => formatter.write_str("unsupported capture limits"),
            ReadErrorKind::MismatchedRequest => {
                formatter.write_str("capture request does not match")
            }
            ReadErrorKind::Limit(reason) => write!(formatter, "capture limit exceeded: {reason:?}"),
            ReadErrorKind::InvalidGraph(reason) => {
                write!(formatter, "invalid capture graph: {reason}")
            }
        }
    }
}

impl std::error::Error for CaptureReadError {}

impl From<Stop> for CaptureReadError {
    fn from(stop: Stop) -> Self {
        Self(ReadErrorKind::Limit(stop.reason))
    }
}

/// The bounded envelope could not be serialized, including its small failure status.
#[derive(Debug, thiserror::Error)]
pub enum CaptureWriteError {
    #[error("failed to serialize internal resolver capture")]
    Serialize(#[from] serde_json::Error),
    #[error("capture byte budget cannot hold its status envelope")]
    EnvelopeTooLarge,
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl BoundedBuffer {
    const fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        }
    }
}

impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("capture byte budget exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode(evidence: &EvidenceWire) -> Result<Vec<u8>, CaptureWriteError> {
    let mut buffer = BoundedBuffer::new(evidence.limits.json_bytes);
    match serde_json::to_writer(&mut buffer, evidence) {
        Ok(()) => Ok(buffer.bytes),
        Err(error) if !buffer.exceeded => Err(error.into()),
        Err(_) => {
            // Discard the incomplete serialization before preparing the small status envelope.
            drop(buffer);
            let truncated = EvidenceWire {
                schema: evidence.schema,
                request: evidence.request.clone(),
                producer_pid: evidence.producer_pid,
                command: evidence.command,
                scope: evidence.scope,
                operation: evidence.operation,
                metadata: evidence.metadata,
                terminal: evidence.terminal,
                status: CaptureStatus::Truncated,
                reason: Some(CaptureReason::JsonBytes),
                limits: evidence.limits,
                usage: evidence.usage,
                graph: None,
            };
            let mut buffer = BoundedBuffer::new(evidence.limits.json_bytes);
            match serde_json::to_writer(&mut buffer, &truncated) {
                Ok(()) => Ok(buffer.bytes),
                Err(_) if buffer.exceeded => Err(CaptureWriteError::EnvelopeTooLarge),
                Err(error) => Err(error.into()),
            }
        }
    }
}

pub(super) fn read(
    bytes: &[u8],
    token: &CaptureToken,
    limits: CaptureLimits,
) -> Result<NoSolutionEvidence, CaptureReadError> {
    let mut budget = preflight::check(bytes, token, limits)?;
    // Only the preflight is allowed to reach this private Deserialize implementation. It has
    // already bounded every sequence and decoded string atom, including adjacent-tag payloads.
    let evidence: EvidenceWire = serde_json::from_slice(bytes)
        .map_err(|_| CaptureReadError(ReadErrorKind::InvalidSchema))?;
    if let Some(graph) = &evidence.graph {
        validate_graph(graph, &mut budget)?;
    }
    Ok(NoSolutionEvidence(evidence))
}

fn invalid_graph(reason: &'static str) -> CaptureReadError {
    CaptureReadError(ReadErrorKind::InvalidGraph(reason))
}

fn usage_within(usage: CaptureUsage, limits: CaptureLimits, allowance: usize) -> bool {
    usage.derivation_nodes <= limits.derivation_nodes.saturating_add(allowance)
        && usage.packages <= limits.packages.saturating_add(allowance)
        && usage.terms <= limits.terms.saturating_add(allowance)
        && usage.intervals <= limits.intervals.saturating_add(allowance)
        && usage.marker_nodes <= limits.marker_nodes.saturating_add(allowance)
        && usage.marker_edges <= limits.marker_edges.saturating_add(allowance)
        && usage.availability_entries <= limits.availability_entries.saturating_add(allowance)
        && usage.max_version_components <= limits.version_components.saturating_add(allowance)
        && usage.max_atom_bytes <= limits.atom_bytes.saturating_add(allowance)
        && usage.text_bytes <= limits.text_bytes.saturating_add(allowance)
        && usage.work <= limits.work.saturating_add(allowance)
}

fn valid_name(name: &str) -> bool {
    PackageName::from_str(name).is_ok_and(|parsed| parsed.as_ref() == name)
}

fn valid_extra(extra: &str) -> bool {
    ExtraName::from_str(extra).is_ok_and(|parsed| parsed.as_ref() == extra)
}

fn valid_group(group: &str) -> bool {
    GroupName::from_str(group).is_ok_and(|parsed| parsed.as_ref() == group)
}

fn validate_graph(graph: &CapturedGraph, budget: &mut Budget) -> Result<(), CaptureReadError> {
    if graph.nodes.is_empty() || graph.root as usize != graph.nodes.len() - 1 {
        return Err(invalid_graph("root is not the final derivation node"));
    }
    let Some(CapturedPackage::Root { .. }) = graph.packages.get(graph.root_package as usize) else {
        return Err(invalid_graph("root package is not a root identity"));
    };
    let root_version = EncodedVersion::try_from(&*MIN_VERSION)
        .map_err(|_| invalid_graph("unsupported native root version"))?;
    if graph.root_version != root_version {
        return Err(invalid_graph("unexpected root version"));
    }

    let mut packages = FxHashSet::default();
    let mut names = FxHashSet::default();
    for package in &graph.packages {
        budget.work(1)?;
        if !packages.insert(package) {
            return Err(invalid_graph("duplicate package identity"));
        }
        if let Some(name) = package.name() {
            if !valid_name(name) {
                return Err(invalid_graph("non-normalized package name"));
            }
            names.insert(name);
        }
        match package {
            CapturedPackage::Package { extra, group, .. } => {
                if extra.is_some() && group.is_some() {
                    return Err(invalid_graph("package has both an extra and a group"));
                }
                if extra.as_deref().is_some_and(|extra| !valid_extra(extra))
                    || group.as_deref().is_some_and(|group| !valid_group(group))
                {
                    return Err(invalid_graph("non-normalized package selection"));
                }
            }
            CapturedPackage::Extra { extra, .. } if !valid_extra(extra) => {
                return Err(invalid_graph("non-normalized extra name"));
            }
            CapturedPackage::Group { group, .. } if !valid_group(group) => {
                return Err(invalid_graph("non-normalized group name"));
            }
            CapturedPackage::Root { .. }
            | CapturedPackage::Python { .. }
            | CapturedPackage::System { .. }
            | CapturedPackage::Extra { .. }
            | CapturedPackage::Group { .. }
            | CapturedPackage::Marker { .. } => {}
        }
    }

    let mut used_packages = vec![false; graph.packages.len()];
    used_packages[graph.root_package as usize] = true;
    for (index, node) in graph.nodes.iter().enumerate() {
        budget.work(1)?;
        let mut use_package = |package: u32| -> Result<(), CaptureReadError> {
            let Some(used) = used_packages.get_mut(package as usize) else {
                return Err(invalid_graph("invalid package reference"));
            };
            *used = true;
            Ok(())
        };
        match node {
            CapturedNode::NotRoot { package, .. }
            | CapturedNode::NoVersions { package, .. }
            | CapturedNode::Custom { package, .. } => use_package(*package)?,
            CapturedNode::FromDependencyOf {
                package,
                dependency,
                ..
            } => {
                use_package(*package)?;
                use_package(*dependency)?;
            }
            CapturedNode::Derived {
                cause1,
                cause2,
                terms,
            } => {
                if *cause1 as usize >= index || *cause2 as usize >= index {
                    return Err(invalid_graph("derivation is not in postorder"));
                }
                let mut term_packages = FxHashSet::default();
                for term in terms {
                    budget.work(1)?;
                    if !term_packages.insert(term.package) {
                        return Err(invalid_graph("duplicate signed term"));
                    }
                    use_package(term.package)?;
                }
            }
        }
    }
    if used_packages.iter().any(|used| !used) {
        return Err(invalid_graph("unreferenced package identity"));
    }

    let mut reached = vec![false; graph.nodes.len()];
    let mut pending = vec![graph.root];
    while let Some(index) = pending.pop() {
        budget.work(1)?;
        if std::mem::replace(&mut reached[index as usize], true) {
            continue;
        }
        if let CapturedNode::Derived { cause1, cause2, .. } = &graph.nodes[index as usize] {
            pending.push(*cause1);
            pending.push(*cause2);
        }
    }
    if reached.iter().any(|reached| !reached) {
        return Err(invalid_graph("unreachable derivation node"));
    }

    validate_markers(graph, budget)?;
    validate_selections(graph, budget)?;
    let mut observed = FxHashSet::default();
    for observation in &graph.observations {
        budget.work(1)?;
        if !names.contains(observation.name.as_str()) || !observed.insert(observation.name.as_str())
        {
            return Err(invalid_graph("unexpected or duplicate package observation"));
        }
        match (observation.source, observation.listing) {
            (
                CapturedSource::Registry | CapturedSource::ExplicitIndex,
                CapturedListing::NotApplicable,
            )
            | (
                CapturedSource::Archive
                | CapturedSource::Path
                | CapturedSource::Directory
                | CapturedSource::GitDirectory
                | CapturedSource::GitPath,
                CapturedListing::Found
                | CapturedListing::NotFound
                | CapturedListing::NoIndex
                | CapturedListing::Offline
                | CapturedListing::Unobserved,
            ) => {
                return Err(invalid_graph(
                    "listing state does not match its source kind",
                ));
            }
            _ => {}
        }
        if observation.listing != CapturedListing::Found && !observation.listed_versions.is_empty()
        {
            return Err(invalid_graph("unobserved listing contains versions"));
        }
        if let Some(reason) = &observation.unavailable {
            validate_reason(reason)?;
        }
        for fact in &observation.incomplete {
            budget.work(1)?;
            validate_reason(&fact.reason)?;
        }
    }
    if names.len() != observed.len() {
        return Err(invalid_graph("missing package observation"));
    }
    for node in &graph.nodes {
        if let CapturedNode::Custom { reason, .. } = node {
            validate_reason(reason)?;
        }
    }
    Ok(())
}

fn validate_reason(reason: &CapturedReason) -> Result<(), CaptureReadError> {
    let network = match reason.kind {
        CapturedReasonKind::PackageNetwork
        | CapturedReasonKind::VersionNetwork
        | CapturedReasonKind::MetadataNetwork => true,
        CapturedReasonKind::PackageNoIndex
        | CapturedReasonKind::PackageOffline
        | CapturedReasonKind::PackageNotFound
        | CapturedReasonKind::PackageInvalidMetadata
        | CapturedReasonKind::PackageInvalidStructure
        | CapturedReasonKind::VersionUnsatisfiableDependency
        | CapturedReasonKind::VersionIncompatibleSelfDependency
        | CapturedReasonKind::VersionIncompatibleDist
        | CapturedReasonKind::VersionInvalidMetadata
        | CapturedReasonKind::VersionInconsistentMetadata
        | CapturedReasonKind::VersionInvalidStructure
        | CapturedReasonKind::VersionOffline
        | CapturedReasonKind::VersionRequiresPython
        | CapturedReasonKind::MetadataOffline
        | CapturedReasonKind::MetadataInvalidMetadata
        | CapturedReasonKind::MetadataInconsistentMetadata
        | CapturedReasonKind::MetadataInvalidStructure
        | CapturedReasonKind::MetadataRequiresPython => false,
    };
    if network != reason.http_status.is_some()
        || reason
            .http_status
            .is_some_and(|status| !(100..=999).contains(&status))
    {
        return Err(invalid_graph("invalid availability reason status"));
    }
    Ok(())
}

fn validate_selections(graph: &CapturedGraph, budget: &mut Budget) -> Result<(), CaptureReadError> {
    let mut members = FxHashSet::default();
    for name in &graph.workspace_members {
        budget.work(1)?;
        if !valid_name(name) || !members.insert(name) {
            return Err(invalid_graph("invalid workspace member"));
        }
    }
    for selections in [&graph.environment.include, &graph.environment.exclude] {
        let mut unique = FxHashSet::default();
        for selection in selections {
            budget.work(1)?;
            if !unique.insert(selection) {
                return Err(invalid_graph("duplicate conflict selection"));
            }
            let valid = match selection {
                CapturedConflict::Project { package } => valid_name(package),
                CapturedConflict::Extra { package, extra } => {
                    valid_name(package) && valid_extra(extra)
                }
                CapturedConflict::Group { package, group } => {
                    valid_name(package) && valid_group(group)
                }
            };
            if !valid {
                return Err(invalid_graph("non-normalized conflict selection"));
            }
        }
    }
    Ok(())
}

fn validate_markers(graph: &CapturedGraph, budget: &mut Budget) -> Result<(), CaptureReadError> {
    for (index, marker) in graph.markers.iter().enumerate() {
        budget.work(1)?;
        match marker {
            CapturedMarker::True | CapturedMarker::False => {}
            CapturedMarker::Version { edges, .. } => {
                if edges.len() < 2 {
                    return Err(invalid_graph("marker decision has fewer than two edges"));
                }
                let mut previous_upper: Option<Bound<Version>> = None;
                for edge in edges {
                    budget.work(1)?;
                    if edge.child as usize >= index {
                        return Err(invalid_graph("marker is not in postorder"));
                    }
                    let ranges = edge.intervals.to_ranges();
                    let mut intervals = ranges.iter();
                    let Some((lower, upper)) = intervals.next() else {
                        return Err(invalid_graph("empty marker edge"));
                    };
                    if intervals.next().is_some() {
                        return Err(invalid_graph("marker edge has multiple intervals"));
                    }
                    if let Some(previous_upper) = &previous_upper {
                        if !adjacent_bounds(previous_upper.as_ref(), lower) {
                            return Err(invalid_graph("marker edges do not partition the domain"));
                        }
                    } else if !matches!(lower, Bound::Unbounded) {
                        return Err(invalid_graph("marker partition has no lower tail"));
                    }
                    previous_upper = Some(upper.cloned());
                }
                if !matches!(previous_upper, Some(Bound::Unbounded)) {
                    return Err(invalid_graph("marker partition has no upper tail"));
                }
            }
            CapturedMarker::String { edges, .. } => {
                if edges.len() < 2 {
                    return Err(invalid_graph("marker decision has fewer than two edges"));
                }
                let mut previous_upper: Option<Bound<&str>> = None;
                for edge in edges {
                    budget.work(1)?;
                    if edge.child as usize >= index {
                        return Err(invalid_graph("marker is not in postorder"));
                    }
                    let [interval] = edge.intervals.as_slice() else {
                        return Err(invalid_graph("marker edge is not one interval"));
                    };
                    let lower = string_bound(&interval.lower);
                    let upper = string_bound(&interval.upper);
                    if !nonempty_interval(lower, upper) {
                        return Err(invalid_graph("empty or reversed string marker edge"));
                    }
                    if let Some(previous_upper) = previous_upper {
                        if !adjacent_bounds(previous_upper, lower) {
                            return Err(invalid_graph("marker edges do not partition the domain"));
                        }
                    } else if !matches!(lower, Bound::Unbounded) {
                        return Err(invalid_graph("marker partition has no lower tail"));
                    }
                    previous_upper = Some(upper);
                }
                if !matches!(previous_upper, Some(Bound::Unbounded)) {
                    return Err(invalid_graph("marker partition has no upper tail"));
                }
            }
        }
    }

    let mut reached = vec![false; graph.markers.len()];
    let mut pending = Vec::new();
    for package in &graph.packages {
        if let Some(marker) = package.marker() {
            pending.push(marker);
        }
    }
    pending.push(graph.environment.marker);
    pending.extend(&graph.environment.initial_forks);
    pending.push(graph.original_python.target_marker);
    pending.push(graph.effective_python.target_marker);
    while let Some(index) = pending.pop() {
        budget.work(1)?;
        let Some(reached) = reached.get_mut(index as usize) else {
            return Err(invalid_graph("invalid marker reference"));
        };
        if std::mem::replace(reached, true) {
            continue;
        }
        match &graph.markers[index as usize] {
            CapturedMarker::True | CapturedMarker::False => {}
            CapturedMarker::Version { edges, .. } => {
                pending.extend(edges.iter().map(|edge| edge.child));
            }
            CapturedMarker::String { edges, .. } => {
                pending.extend(edges.iter().map(|edge| edge.child));
            }
        }
    }
    if reached.iter().any(|reached| !reached) {
        return Err(invalid_graph("unreachable marker node"));
    }
    Ok(())
}

fn string_bound(bound: &StringBound) -> Bound<&str> {
    match bound {
        StringBound::Unbounded => Bound::Unbounded,
        StringBound::Included(value) => Bound::Included(value),
        StringBound::Excluded(value) => Bound::Excluded(value),
    }
}

fn nonempty_interval<T: Ord + ?Sized>(lower: Bound<&T>, upper: Bound<&T>) -> bool {
    match (lower, upper) {
        (Bound::Included(lower), Bound::Included(upper)) => lower <= upper,
        (Bound::Included(lower) | Bound::Excluded(lower), Bound::Excluded(upper))
        | (Bound::Excluded(lower), Bound::Included(upper)) => lower < upper,
        (Bound::Unbounded, _) | (_, Bound::Unbounded) => true,
    }
}

fn adjacent_bounds<T: Eq + ?Sized>(upper: Bound<&T>, lower: Bound<&T>) -> bool {
    match (upper, lower) {
        (Bound::Included(upper), Bound::Excluded(lower))
        | (Bound::Excluded(upper), Bound::Included(lower)) => upper == lower,
        (Bound::Unbounded, _)
        | (_, Bound::Unbounded)
        | (Bound::Included(_), Bound::Included(_))
        | (Bound::Excluded(_), Bound::Excluded(_)) => false,
    }
}
