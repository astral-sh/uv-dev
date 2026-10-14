use std::collections::BTreeSet;
use std::collections::hash_map::Entry;

use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::{Direction, Graph};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};

use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictItemRef, Conflicts, Inference};

use crate::ResolutionGraphNode;
use crate::universal_marker::UniversalMarker;

/// Collect all nodes without incoming edges, including the virtual root and virtual dev
/// dependency packages, since the graph's edges don't cover dev dependencies.
fn root_nodes<Node, Edge>(graph: &Graph<Node, Edge>) -> Vec<NodeIndex> {
    graph.externals(Direction::Incoming).collect()
}

/// Determine the markers under which a package is reachable in the dependency tree.
///
/// The algorithm is a variant of Dijkstra's algorithm for not totally ordered distances:
/// Whenever we find a shorter distance to a node (a marker that is not a subset of the existing
/// marker), we re-queue the node and update all its children. This implicitly handles cycles,
/// whenever we re-reach a node through a cycle the marker we have is a more
/// specific marker/longer path, so we don't update the node and don't re-queue it.
pub fn marker_reachability<Marker: Boolean + Copy + PartialEq, Node, Edge: Reachable<Marker>>(
    graph: &Graph<Node, Edge>,
    fork_markers: &[Edge],
) -> FxHashMap<NodeIndex, Marker> {
    // Note that we build including the virtual packages due to how we propagate markers through
    // the graph, even though we then only read the markers for base packages.
    let mut reachability = FxHashMap::with_capacity_and_hasher(graph.node_count(), FxBuildHasher);

    let mut queue = root_nodes(graph);

    // The root nodes are always applicable, unless the user has restricted resolver
    // environments with `tool.uv.environments`.
    let root_markers = fork_markers
        .iter()
        .map(Reachable::marker)
        .reduce(|mut combined, marker| {
            combined.or(marker);
            combined
        })
        .unwrap_or_else(Edge::true_marker);
    for root_index in &queue {
        reachability.insert(*root_index, root_markers);
    }

    // Propagate all markers through the graph, so that the eventual marker for each node is the
    // union of the markers of each path we can reach the node by.
    while let Some(parent_index) = queue.pop() {
        let marker = reachability[&parent_index];
        for child_edge in graph.edges_directed(parent_index, Direction::Outgoing) {
            // The marker for all paths to the child through the parent.
            let mut child_marker = child_edge.weight().marker();
            child_marker.and(marker);
            let child_index = child_edge.target();
            match reachability.entry(child_index) {
                Entry::Occupied(mut existing) => {
                    child_marker.or(*existing.get());
                    // Re-queue only if this path expands the child's marker.
                    if child_marker == *existing.get() {
                        continue;
                    }
                    existing.insert(child_marker);
                }
                Entry::Vacant(vacant) => {
                    vacant.insert(child_marker);
                }
            }
            queue.push(child_index);
        }
    }

    reachability
}

/// Traverse the given dependency graph and propagate activated markers.
///
/// For example, given an edge like `foo[x1] -> bar`, then it is known that
/// `x1` is activated. This in turn can be used to simplify any downstream
/// conflict markers with `extra == "x1"` in them (by replacing `extra == "x1"`
/// with `true`).
pub fn simplify_conflict_markers(
    conflicts: &Conflicts,
    graph: &mut Graph<ResolutionGraphNode, UniversalMarker>,
) {
    // Do nothing if there are no declared conflicts. Without any declared
    // conflicts, we know we have no conflict markers and thus nothing to
    // simplify by determining which extras are activated at different points
    // in the dependency graph.
    if conflicts.is_empty() {
        return;
    }

    // Unrelated extras and groups cannot simplify a conflict marker. Tracking
    // them enumerates distinct paths through large workspaces unnecessarily.
    let relevant: FxHashSet<ConflictItemRef<'_>> = conflicts
        .iter()
        .flat_map(|set| set.iter().map(ConflictItem::as_ref))
        .collect();

    // The set of activated extras and groups for each node. The ROOT nodes
    // don't have any extras/groups activated.
    let mut activated: FxHashMap<NodeIndex, Vec<FxHashSet<ConflictItemRef<'_>>>> =
        FxHashMap::default();
    let unactivated = [FxHashSet::default()];

    let mut queue = root_nodes(graph);

    while let Some(parent_index) = queue.pop() {
        if let Some(item) = graph[parent_index]
            .activated_conflict_item()
            .filter(|item| relevant.contains(item))
        {
            for set in activated
                .entry(parent_index)
                .or_insert_with(|| vec![FxHashSet::default()])
            {
                set.insert(item);
            }
        }
        for child in graph.neighbors_directed(parent_index, Direction::Outgoing) {
            // A self-edge cannot add anything to the node's own activation state.
            if child == parent_index {
                continue;
            }
            activated.entry(child).or_default();
            let [source, Some(existing)] = activated.get_disjoint_mut([&parent_index, &child])
            else {
                unreachable!("Child activation state was initialized");
            };
            let mut change = false;
            for set in source.map_or(unactivated.as_slice(), |sets| sets.as_slice()) {
                if !existing.contains(set) {
                    existing.push(set.clone());
                    change = true;
                }
            }
            if change {
                queue.push(child);
            }
        }
    }

    let mut inferences: FxHashMap<NodeIndex, Vec<BTreeSet<Inference>>> = FxHashMap::default();
    for (node_id, sets) in activated {
        inferences.insert(
            node_id,
            sets.into_iter()
                .map(|set| infer_conflicts(conflicts, set))
                .collect(),
        );
    }

    // Inferences are not always sufficient to disambiguate edges to packages with the same name.
    let ambiguous_edges = find_ambiguous_edges(graph, inferences.keys().copied());
    for edge_index in (0..graph.edge_count()).map(EdgeIndex::new) {
        let (from_index, _) = graph.edge_endpoints(edge_index).unwrap();
        let Some(inference_sets) = inferences.get(&from_index) else {
            continue;
        };
        if ambiguous_edges.contains(&edge_index) {
            continue;
        }
        // If not all possible paths (represented by our inferences)
        // satisfy the conflict marker on this edge, then we can't make any
        // simplifications. Namely, because it follows that out inferences
        // aren't always true. Some of them may sometimes be false.
        let all_paths_satisfied = inference_sets.iter().all(|set| {
            let included = set.iter().filter(|inference| inference.included);
            let extras = included
                .clone()
                .filter_map(|inference| Some((inference.item.package(), inference.item.extra()?)));
            let groups = included
                .filter_map(|inference| Some((inference.item.package(), inference.item.group()?)));
            // Notably, the marker must be possible to satisfy with the extras and groups alone.
            // For example, when `a` and `b` conflict, this marker does not simplify:
            // ```
            // (platform_machine == 'x86_64' and extra == 'extra-5-foo-b') or extra == 'extra-5-foo-a'
            // ````
            graph[edge_index].evaluate_only_extras(extras, groups)
        });
        if all_paths_satisfied {
            graph[edge_index].assume_inferences(inference_sets.iter().flatten());
        } else {
            graph[edge_index].unify_inference_sets(inference_sets);
        }
    }
}

/// Infer included and excluded conflict items from the items activated on a path.
fn infer_conflicts(
    conflicts: &Conflicts,
    activated: FxHashSet<ConflictItemRef<'_>>,
) -> BTreeSet<Inference> {
    if activated.is_empty() {
        return BTreeSet::new();
    }

    let mut inferences = BTreeSet::new();
    for conflict_set in conflicts.iter() {
        let mut active = conflict_set
            .iter()
            .filter(|item| activated.contains(&item.as_ref()));
        let Some(first) = active.next() else {
            continue;
        };
        // When multiple items are active, each is excluded by another active item.
        let multiple_active = active.next().is_some();
        inferences.extend(
            conflict_set
                .iter()
                .filter(|item| multiple_active || *item != first)
                .map(|item| Inference {
                    item: item.clone(),
                    included: false,
                }),
        );
    }
    inferences.extend(activated.into_iter().map(|item| Inference {
        item: item.to_owned(),
        included: true,
    }));
    inferences
}

/// Find edges from each source whose target package name is shared by another outgoing edge.
fn find_ambiguous_edges(
    graph: &Graph<ResolutionGraphNode, UniversalMarker>,
    sources: impl Iterator<Item = NodeIndex>,
) -> FxHashSet<EdgeIndex> {
    let mut ambiguous = FxHashSet::default();
    for source in sources {
        let mut edges = graph.edges_directed(source, Direction::Outgoing);
        let (Some(first), Some(second)) = (edges.next(), edges.next()) else {
            continue;
        };
        let mut seen = FxHashMap::default();
        seen.insert(graph[first.target()].package_name(), first.id());
        for edge in std::iter::once(second).chain(edges) {
            if let Some(previous) = seen.insert(graph[edge.target()].package_name(), edge.id()) {
                ambiguous.insert(previous);
                ambiguous.insert(edge.id());
            }
        }
    }
    ambiguous
}

pub trait Reachable<T> {
    /// The marker representing the "true" value.
    fn true_marker() -> T;

    /// The marker attached to the edge.
    fn marker(&self) -> T;
}

impl Reachable<Self> for MarkerTree {
    fn true_marker() -> Self {
        Self::TRUE
    }

    fn marker(&self) -> Self {
        *self
    }
}

impl Reachable<Self> for UniversalMarker {
    fn true_marker() -> Self {
        Self::TRUE
    }

    fn marker(&self) -> Self {
        *self
    }
}

/// A trait for types that can be used as markers in the dependency graph.
pub trait Boolean {
    /// Perform a logical AND operation with another marker.
    fn and(&mut self, other: Self);

    /// Perform a logical OR operation with another marker.
    fn or(&mut self, other: Self);
}

impl Boolean for UniversalMarker {
    fn and(&mut self, other: Self) {
        self.and(other);
    }

    fn or(&mut self, other: Self) {
        self.or(other);
    }
}

impl Boolean for MarkerTree {
    fn and(&mut self, other: Self) {
        *self = Self::and(*self, other);
    }

    fn or(&mut self, other: Self) {
        *self = Self::or(*self, other);
    }
}
