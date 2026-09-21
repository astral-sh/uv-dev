//! Bounded comparison with independently configured initial marker forks.

use std::ops::Bound;

use pubgrub::Ranges;
use rustc_hash::FxHashSet;
use uv_distribution_types::RequiresPython;
use uv_pep440::{Version, VersionSpecifiers};
use uv_pep508::{MarkerExpression, MarkerTree, MarkerValueVersion};

use super::super::producer::collect_markers;
use super::super::wire::{
    CapturedGraph, CapturedMarker, StringBound, StringMarkerEdge, StringMarkerKey, VersionMarkerKey,
};
use super::{Budget, ClosedWorldNoSolutionError, Resource, unsupported};

pub(super) struct ExpectedEnvironments {
    markers: Vec<CapturedMarker>,
    forks: Vec<u32>,
    original_python: Option<u32>,
}

impl ExpectedEnvironments {
    pub(super) fn new(
        environments: &[MarkerTree],
        python: &RequiresPython,
        budget: &mut Budget,
    ) -> Result<Self, ClosedWorldNoSolutionError> {
        if environments.is_empty() {
            return Ok(Self {
                markers: Vec::new(),
                forks: Vec::new(),
                original_python: None,
            });
        }
        let full_python = full_python_marker(python.specifiers(), budget)?;
        let (markers, mut forks) = collect_markers(
            environments
                .iter()
                .copied()
                .chain([python.to_marker_tree(), full_python]),
            budget,
        )?;
        let full_python = forks.pop().expect("appended full Python marker");
        let original_python = forks.pop().expect("appended Python marker");
        let graph = MarkerGraph::new(&markers, budget)?;
        for (index, &fork) in forks.iter().enumerate() {
            if !graph.intersects(fork, false, &graph, full_python, false, budget)? {
                return Err(unsupported(
                    "a supported environment excludes the project Python domain",
                ));
            }
            for &previous in &forks[..index] {
                if graph.intersects(fork, false, &graph, previous, false, budget)? {
                    return Err(unsupported("supported environments are not disjoint"));
                }
            }
        }
        Ok(Self {
            markers,
            forks,
            original_python: Some(original_python),
        })
    }

    pub(super) fn check(
        &self,
        captured: &CapturedGraph,
        effective_python: &RequiresPython,
        budget: &mut Budget,
    ) -> Result<(), ClosedWorldNoSolutionError> {
        if captured.environment.initial_forks.len() != self.forks.len() {
            return Err(unsupported(
                "captured initial forks differ from the project",
            ));
        }
        let Some(original_python) = self.original_python else {
            return Ok(());
        };
        // The reader has bounded and checked the captured DAG. Reserve its nodes and edges before
        // adding decoded ranges or comparison states to the inventory's independent budget.
        budget.charge(Resource::MarkerNodes, captured.markers.len())?;
        for marker in &captured.markers {
            budget.charge(Resource::MarkerEdges, edge_count(marker))?;
        }
        let actual = MarkerGraph::new(&captured.markers, budget)?;
        let expected = MarkerGraph::new(&self.markers, budget)?;
        for (&actual_fork, &expected_fork) in
            captured.environment.initial_forks.iter().zip(&self.forks)
        {
            if !actual.equivalent(actual_fork, &expected, expected_fork, budget)? {
                return Err(unsupported(
                    "captured initial forks differ from the project",
                ));
            }
        }
        let full_python = full_python_marker(effective_python.specifiers(), budget)?;
        let (python_markers, python_roots) =
            collect_markers([effective_python.to_marker_tree(), full_python], budget)?;
        let python = MarkerGraph::new(&python_markers, budget)?;
        if !actual.equivalent(
            captured.original_python.target_marker,
            &expected,
            original_python,
            budget,
        )? || !actual.equivalent(
            captured.effective_python.target_marker,
            &python,
            python_roots[0],
            budget,
        )? {
            return Err(unsupported(
                "captured Python marker differs from its policy",
            ));
        }
        let mut contained = false;
        for &fork in &self.forks {
            if !actual.intersects(
                captured.environment.marker,
                false,
                &expected,
                fork,
                true,
                budget,
            )? {
                contained = true;
                break;
            }
        }
        if !contained
            || !actual.intersects(
                captured.environment.marker,
                false,
                &python,
                python_roots[1],
                false,
                budget,
            )?
        {
            return Err(unsupported(
                "failed effective environment is outside the certified domain",
            ));
        }
        Ok(())
    }
}

fn full_python_marker(
    specifiers: &VersionSpecifiers,
    budget: &mut Budget,
) -> Result<MarkerTree, ClosedWorldNoSolutionError> {
    // All expressions concern one version variable. Reserve their interval-combination work
    // before constructing the native marker, including exclusions inside the endpoint range.
    budget.work(specifiers.len().saturating_mul(specifiers.len()))?;
    let mut marker = MarkerTree::TRUE;
    for specifier in specifiers.iter() {
        budget.charge(Resource::Intervals, 1)?;
        budget.version(specifier.version())?;
        marker = marker.and(MarkerTree::expression(MarkerExpression::Version {
            key: MarkerValueVersion::PythonFullVersion,
            specifier: specifier.clone(),
        }));
    }
    Ok(marker)
}

// This is the native ordinary-marker variable order. Rejecting a non-ordered captured DAG lets
// the implication walker forget each variable after following its selected interval.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum Key {
    String(StringMarkerKey),
    Version(VersionMarkerKey),
}

impl Key {
    // Native marker algebra has cross-variable exclusions for these three OS identities. Until
    // those exclusions have a bounded wire representation, comparisons must use at most one of
    // the keys. `python_version` is not another correlated key: the producer's native marker
    // representation has already lowered it to `PythonFullVersion`.
    fn linked_os_bit(self) -> u8 {
        match self {
            Self::String(StringMarkerKey::OsName) => 1,
            Self::String(StringMarkerKey::SysPlatform) => 2,
            Self::String(StringMarkerKey::PlatformSystem) => 4,
            Self::String(
                StringMarkerKey::PlatformMachine
                | StringMarkerKey::PlatformPythonImplementation
                | StringMarkerKey::PlatformRelease
                | StringMarkerKey::PlatformVersion
                | StringMarkerKey::ImplementationName,
            )
            | Self::Version(_) => 0,
        }
    }
}

enum Decision<'a> {
    Terminal(bool),
    Version(VersionMarkerKey, Vec<(Ranges<Version>, u32)>),
    String(StringMarkerKey, &'a [StringMarkerEdge]),
}

impl Decision<'_> {
    fn key(&self) -> Option<Key> {
        match self {
            Self::Terminal(_) => None,
            Self::Version(key, _) => Some(Key::Version(*key)),
            Self::String(key, _) => Some(Key::String(*key)),
        }
    }

    fn children(&self) -> impl Iterator<Item = u32> + '_ {
        let version = match self {
            Self::Version(_, edges) => Some(edges.iter().map(|(_, child)| *child)),
            Self::Terminal(_) | Self::String(_, _) => None,
        };
        let string = match self {
            Self::String(_, edges) => Some(edges.iter().map(|edge| edge.child)),
            Self::Terminal(_) | Self::Version(_, _) => None,
        };
        version
            .into_iter()
            .flatten()
            .chain(string.into_iter().flatten())
    }
}

struct MarkerGraph<'a> {
    nodes: Vec<Decision<'a>>,
    linked_os_keys: Vec<u8>,
}

impl<'a> MarkerGraph<'a> {
    fn new(
        markers: &'a [CapturedMarker],
        budget: &mut Budget,
    ) -> Result<Self, ClosedWorldNoSolutionError> {
        let mut nodes: Vec<Decision<'a>> = Vec::with_capacity(markers.len());
        let mut linked_os_keys: Vec<u8> = Vec::with_capacity(markers.len());
        for marker in markers {
            budget.work(1)?;
            let node = match marker {
                CapturedMarker::True => Decision::Terminal(true),
                CapturedMarker::False => Decision::Terminal(false),
                CapturedMarker::Version { key, edges } => {
                    let mut decoded = Vec::with_capacity(edges.len());
                    for edge in edges {
                        budget.work(1)?;
                        let range = edge.intervals.to_ranges();
                        if range.iter().count() != 1 {
                            return Err(unsupported("unsupported marker interval partition"));
                        }
                        decoded.push((range, edge.child));
                    }
                    Decision::Version(*key, decoded)
                }
                CapturedMarker::String { key, edges } => {
                    for edge in edges {
                        budget.work(1)?;
                        if edge.intervals.len() != 1 {
                            return Err(unsupported("unsupported marker interval partition"));
                        }
                    }
                    Decision::String(*key, edges)
                }
            };
            let mut linked = 0;
            if let Some(key) = node.key() {
                linked = key.linked_os_bit();
                for child_id in node.children() {
                    budget.work(1)?;
                    let Some(child) = nodes.get(child_id as usize) else {
                        return Err(unsupported("marker decisions are not in postorder"));
                    };
                    if child.key().is_some_and(|child| child <= key) {
                        return Err(unsupported("marker variables are not ordered"));
                    }
                    linked |= linked_os_keys[child_id as usize];
                }
            }
            nodes.push(node);
            linked_os_keys.push(linked);
        }
        Ok(Self {
            nodes,
            linked_os_keys,
        })
    }

    fn equivalent(
        &self,
        left: u32,
        other: &Self,
        right: u32,
        budget: &mut Budget,
    ) -> Result<bool, ClosedWorldNoSolutionError> {
        Ok(!self.intersects(left, false, other, right, true, budget)?
            && !self.intersects(left, true, other, right, false, budget)?)
    }

    /// Decide whether two checked ordinary-marker DAGs overlap without building a new native DAG.
    /// Pair memoization and all edge joins consume the capture's fixed work/storage budget.
    fn intersects(
        &self,
        left: u32,
        negate_left: bool,
        other: &Self,
        right: u32,
        negate_right: bool,
        budget: &mut Budget,
    ) -> Result<bool, ClosedWorldNoSolutionError> {
        let (Some(left_keys), Some(right_keys)) = (
            self.linked_os_keys.get(left as usize),
            other.linked_os_keys.get(right as usize),
        ) else {
            return Err(unsupported("invalid marker root"));
        };
        if (*left_keys | *right_keys).count_ones() > 1 {
            return Err(unsupported("linked OS marker variables are unsupported"));
        }
        let mut pending = vec![(left, right)];
        let mut visited = FxHashSet::default();
        while let Some((left_id, right_id)) = pending.pop() {
            budget.work(1)?;
            if visited.contains(&(left_id, right_id)) {
                continue;
            }
            budget.charge(Resource::MarkerNodes, 1)?;
            visited.insert((left_id, right_id));
            let (Some(left), Some(right)) = (
                self.nodes.get(left_id as usize),
                other.nodes.get(right_id as usize),
            ) else {
                return Err(unsupported("invalid marker root"));
            };
            match (left, right) {
                (Decision::Terminal(value), _) if *value == negate_left => continue,
                (_, Decision::Terminal(value)) if *value == negate_right => continue,
                (Decision::Terminal(_), Decision::Terminal(_)) => return Ok(true),
                (Decision::Version(left_key, left), Decision::Version(right_key, right))
                    if left_key == right_key =>
                {
                    for (left_range, left_child) in left {
                        for (right_range, right_child) in right {
                            budget.work(1)?;
                            if !left_range.intersection(right_range).is_empty() {
                                pending.push((*left_child, *right_child));
                            }
                        }
                    }
                }
                (Decision::String(left_key, left), Decision::String(right_key, right))
                    if left_key == right_key =>
                {
                    for left in *left {
                        for right in *right {
                            budget.work(1)?;
                            let left_interval = &left.intervals[0];
                            let right_interval = &right.intervals[0];
                            if !ends_before(
                                string_bound(&left_interval.upper),
                                string_bound(&right_interval.lower),
                            ) && !ends_before(
                                string_bound(&right_interval.upper),
                                string_bound(&left_interval.lower),
                            ) {
                                pending.push((left.child, right.child));
                            }
                        }
                    }
                }
                _ if right.key().is_none()
                    || left.key().is_some_and(|left| Some(left) < right.key()) =>
                {
                    for child in left.children() {
                        budget.work(1)?;
                        pending.push((child, right_id));
                    }
                }
                _ => {
                    for child in right.children() {
                        budget.work(1)?;
                        pending.push((left_id, child));
                    }
                }
            }
        }
        Ok(false)
    }
}

fn edge_count(marker: &CapturedMarker) -> usize {
    match marker {
        CapturedMarker::True | CapturedMarker::False => 0,
        CapturedMarker::Version { edges, .. } => edges.len(),
        CapturedMarker::String { edges, .. } => edges.len(),
    }
}

fn string_bound(bound: &StringBound) -> Bound<&str> {
    match bound {
        StringBound::Unbounded => Bound::Unbounded,
        StringBound::Included(value) => Bound::Included(value),
        StringBound::Excluded(value) => Bound::Excluded(value),
    }
}

fn ends_before<T: Ord + ?Sized>(upper: Bound<&T>, lower: Bound<&T>) -> bool {
    match (upper, lower) {
        (Bound::Included(upper), Bound::Included(lower)) => upper < lower,
        (Bound::Included(upper) | Bound::Excluded(upper), Bound::Excluded(lower))
        | (Bound::Excluded(upper), Bound::Included(lower)) => upper <= lower,
        (Bound::Unbounded, _) | (_, Bound::Unbounded) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::no_solution_capture::budget::CaptureLimits;

    #[test]
    fn full_python_exclusions_bound_the_failed_environment() -> Result<(), Box<dyn Error>> {
        let python = RequiresPython::from_specifiers(">=3.12,<3.15,!=3.13.*".parse()?);
        let fork: MarkerTree = "sys_platform == 'darwin'".parse()?;
        let mut budget = Budget::new(CaptureLimits::V1);
        let expected = ExpectedEnvironments::new(&[fork], &python, &mut budget)?;
        let mut captured: CapturedGraph = serde_json::from_value(
            serde_json::from_slice::<serde_json::Value>(include_bytes!(
                "fixtures/group-proxy-no-solution.json"
            ))?["graph"]
                .clone(),
        )?;
        for (failed, admitted) in [
            (
                "sys_platform == 'darwin' and python_version == '3.12'",
                true,
            ),
            (
                "sys_platform == 'darwin' and python_version == '3.13'",
                false,
            ),
            (
                "sys_platform == 'darwin' and python_version == '3.14'",
                true,
            ),
        ] {
            let (markers, roots) = collect_markers(
                [fork, python.to_marker_tree(), failed.parse()?],
                &mut Budget::new(CaptureLimits::V1),
            )
            .map_err(ClosedWorldNoSolutionError::from)?;
            captured.markers = markers;
            captured.environment.initial_forks = vec![roots[0]];
            captured.original_python.target_marker = roots[1];
            captured.effective_python.target_marker = roots[1];
            captured.environment.marker = roots[2];
            let result = expected.check(&captured, &python, &mut Budget::new(CaptureLimits::V1));
            if admitted {
                result?;
            } else {
                assert!(
                    result
                        .expect_err("excluded Python interval")
                        .to_string()
                        .contains("outside the certified domain")
                );
            }
        }
        Ok(())
    }
}
