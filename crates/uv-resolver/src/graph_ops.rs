use std::collections::BTreeSet;
use std::collections::hash_map::Entry;

use indexmap::IndexSet;
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::{Direction, Graph};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};

use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictItemRef, Conflicts, Inference};

use crate::resolution::ResolutionGraphNode;
use crate::universal_marker::UniversalMarker;

/// Determine the markers under which a package is reachable in the dependency tree.
///
/// The algorithm is a variant of Dijkstra's algorithm for not totally ordered distances:
/// Whenever we find a shorter distance to a node (a marker that is not a subset of the existing
/// marker), we re-queue the node and update all its children. This implicitly handles cycles,
/// whenever we re-reach a node through a cycle the marker we have is a more
/// specific marker/longer path, so we don't update the node and don't re-queue it.
pub(crate) fn marker_reachability<
    Marker: Boolean + Copy + PartialEq,
    Node,
    Edge: Reachable<Marker>,
>(
    graph: &Graph<Node, Edge>,
    fork_markers: &[Edge],
) -> FxHashMap<NodeIndex, Marker> {
    // Note that we build including the virtual packages due to how we propagate markers through
    // the graph, even though we then only read the markers for base packages.
    let mut reachability = FxHashMap::with_capacity_and_hasher(graph.node_count(), FxBuildHasher);

    // Collect the root nodes.
    //
    // Besides the actual virtual root node, virtual dev dependencies packages are also root
    // nodes since the edges don't cover dev dependencies.
    let mut queue: Vec<_> = graph
        .node_indices()
        .filter(|node_index| {
            graph
                .edges_directed(*node_index, Direction::Incoming)
                .next()
                .is_none()
        })
        .collect();

    // The root nodes are always applicable, unless the user has restricted resolver
    // environments with `tool.uv.environments`.
    let root_markers = if fork_markers.is_empty() {
        Edge::true_marker()
    } else {
        fork_markers
            .iter()
            .fold(Edge::false_marker(), |mut acc, edge| {
                acc.or(edge.marker());
                acc
            })
    };
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
            match reachability.entry(child_edge.target()) {
                Entry::Occupied(mut existing) => {
                    // If the marker is a subset of the existing marker (A ⊆ B exactly if
                    // A ∪ B = A), updating the child wouldn't change child's marker.
                    child_marker.or(*existing.get());
                    if &child_marker != existing.get() {
                        existing.insert(child_marker);
                        queue.push(child_edge.target());
                    }
                }
                Entry::Vacant(vacant) => {
                    vacant.insert(child_marker);
                    queue.push(child_edge.target());
                }
            }
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
pub(crate) fn simplify_conflict_markers(
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

    let activated = propagate_conflict_activations(graph, |node| {
        [
            node.package_extra_names()
                .map(ConflictItemRef::from)
                .filter(|item| relevant.contains(item)),
            node.package_group_names()
                .map(ConflictItemRef::from)
                .filter(|item| relevant.contains(item)),
        ]
    });

    let mut inferences: FxHashMap<NodeIndex, Vec<BTreeSet<Inference>>> = FxHashMap::default();
    for (node_id, sets) in activated {
        let mut new_sets = Vec::with_capacity(sets.len());
        for set in sets {
            let mut new_set = BTreeSet::default();
            for item in set {
                for conflict_set in conflicts.iter() {
                    if !conflict_set.contains(item.package(), item.kind()) {
                        continue;
                    }
                    for conflict_item in conflict_set.iter() {
                        if conflict_item.as_ref() == item {
                            continue;
                        }
                        new_set.insert(Inference {
                            item: conflict_item.clone(),
                            included: false,
                        });
                    }
                }
                new_set.insert(Inference {
                    item: item.to_owned(),
                    included: true,
                });
            }
            new_sets.push(new_set);
        }
        inferences.insert(node_id, new_sets);
    }

    for edge_index in (0..graph.edge_count()).map(EdgeIndex::new) {
        let (from_index, to_index) = graph.edge_endpoints(edge_index).unwrap();
        // If there are ambiguous edges (i.e., two or more edges
        // with the same package name), then we specifically skip
        // conflict marker simplification. It seems that in some
        // cases, the logic encoded in `inferences` isn't quite enough
        // to perfectly disambiguate between them. It's plausible we
        // could do better here, but it requires smarter simplification
        // logic. ---AG
        let ambiguous_edges = graph
            .edges_directed(from_index, Direction::Outgoing)
            .filter(|edge| graph[to_index].package_name() == graph[edge.target()].package_name())
            .count();
        if ambiguous_edges > 1 {
            continue;
        }
        let Some(inference_sets) = inferences.get(&from_index) else {
            continue;
        };
        // If not all possible paths (represented by our inferences)
        // satisfy the conflict marker on this edge, then we can't make any
        // simplifications. Namely, because it follows that out inferences
        // aren't always true. Some of them may sometimes be false.
        let all_paths_satisfied = inference_sets.iter().all(|set| {
            let extras = set
                .iter()
                .filter_map(|inf| {
                    if !inf.included {
                        return None;
                    }
                    Some((inf.item.package(), inf.item.extra()?))
                })
                .collect::<Vec<_>>();
            let groups = set
                .iter()
                .filter_map(|inf| {
                    if !inf.included {
                        return None;
                    }
                    Some((inf.item.package(), inf.item.group()?))
                })
                .collect::<Vec<_>>();
            // Notably, the marker must be possible to satisfy with the extras and groups alone.
            // For example, when `a` and `b` conflict, this marker does not simplify:
            // ```
            // (platform_machine == 'x86_64' and extra == 'extra-5-foo-b') or extra == 'extra-5-foo-a'
            // ````
            graph[edge_index].evaluate_only_extras(&extras, &groups)
        });
        if all_paths_satisfied {
            for set in inference_sets {
                for inf in set {
                    // TODO(konsti): Now that `Inference` is public, move more `included` handling
                    // to `UniversalMarker`.
                    if inf.included {
                        graph[edge_index].assume_conflict_item(&inf.item);
                    } else {
                        graph[edge_index].assume_not_conflict_item(&inf.item);
                    }
                }
            }
        } else {
            graph[edge_index].unify_inference_sets(inference_sets);
        }
    }
}

/// Propagate activated extras and groups through every path in the dependency graph.
fn propagate_conflict_activations<'graph, Node, Edge>(
    graph: &'graph Graph<Node, Edge>,
    conflict_items: impl Fn(&'graph Node) -> [Option<ConflictItemRef<'graph>>; 2],
) -> FxHashMap<NodeIndex, IndexSet<Vec<ConflictItemRef<'graph>>, FxBuildHasher>> {
    let mut activated: FxHashMap<NodeIndex, IndexSet<Vec<ConflictItemRef<'graph>>, FxBuildHasher>> =
        FxHashMap::default();

    // Besides the actual virtual root node, virtual dev dependencies packages are also root
    // nodes since the edges don't cover dev dependencies.
    let mut queue: Vec<_> = graph
        .node_indices()
        .filter(|node_index| {
            graph
                .edges_directed(*node_index, Direction::Incoming)
                .next()
                .is_none()
        })
        .collect();

    while let Some(parent_index) = queue.pop() {
        for item in conflict_items(&graph[parent_index]).into_iter().flatten() {
            activate_conflict_item(activated.entry(parent_index).or_default(), item);
        }
        let sets = activated
            .get(&parent_index)
            .cloned()
            .unwrap_or_else(|| IndexSet::from_iter([Vec::default()]));
        for child_edge in graph.edges_directed(parent_index, Direction::Outgoing) {
            let existing = activated.entry(child_edge.target()).or_default();
            let previous_len = existing.len();
            existing.extend(sets.iter().cloned());
            if existing.len() != previous_len {
                queue.push(child_edge.target());
            }
        }
    }
    activated
}

/// Add an activated conflict item to every path while preserving path order and uniqueness.
///
/// Each path is kept sorted so it can be used as a stable hash key. Rebuild the set after an
/// activation since mutating a key in place would invalidate its hash.
fn activate_conflict_item<'item>(
    sets: &mut IndexSet<Vec<ConflictItemRef<'item>>, FxBuildHasher>,
    item: ConflictItemRef<'item>,
) {
    if sets.is_empty() {
        sets.insert(vec![item]);
        return;
    }

    if sets.iter().all(|set| set.binary_search(&item).is_ok()) {
        return;
    }

    let previous = std::mem::take(sets);
    sets.reserve(previous.len());
    for mut set in previous {
        if let Err(index) = set.binary_search(&item) {
            set.insert(index, item);
        }
        sets.insert(set);
    }
}

pub(crate) trait Reachable<T> {
    /// The marker representing the "true" value.
    fn true_marker() -> T;

    /// The marker representing the "false" value.
    fn false_marker() -> T;

    /// The marker attached to the edge.
    fn marker(&self) -> T;
}

impl Reachable<Self> for MarkerTree {
    fn true_marker() -> Self {
        Self::TRUE
    }

    fn false_marker() -> Self {
        Self::FALSE
    }

    fn marker(&self) -> Self {
        *self
    }
}

impl Reachable<Self> for UniversalMarker {
    fn true_marker() -> Self {
        Self::TRUE
    }

    fn false_marker() -> Self {
        Self::FALSE
    }

    fn marker(&self) -> Self {
        *self
    }
}

/// A trait for types that can be used as markers in the dependency graph.
pub(crate) trait Boolean {
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use indexmap::IndexSet;
    use petgraph::Graph;
    use rustc_hash::FxBuildHasher;

    use uv_normalize::{ExtraName, GroupName, PackageName};
    use uv_pypi_types::{ConflictItem, ConflictItemRef};

    use super::{activate_conflict_item, propagate_conflict_activations};

    fn conflict_item(package: &str, extra: &str) -> ConflictItem {
        ConflictItem::from((
            PackageName::from_str(package).expect("valid package name"),
            ExtraName::from_str(extra).expect("valid extra name"),
        ))
    }

    fn group_item(package: &str, group: &str) -> ConflictItem {
        ConflictItem::from((
            PackageName::from_str(package).expect("valid package name"),
            GroupName::from_str(group).expect("valid group name"),
        ))
    }

    fn activation_path(mut items: Vec<ConflictItem>) -> Vec<ConflictItem> {
        items.sort_unstable();
        items
    }

    #[test]
    fn activation_paths_remain_sorted_and_distinct() {
        let left = conflict_item("left", "first");
        let right = conflict_item("right", "first");
        let shared = conflict_item("shared", "first");
        let mut sets: IndexSet<Vec<ConflictItemRef<'_>>, FxBuildHasher> =
            IndexSet::from_iter([vec![right.as_ref()], vec![left.as_ref()]]);

        activate_conflict_item(&mut sets, shared.as_ref());
        activate_conflict_item(&mut sets, shared.as_ref());

        assert_eq!(
            sets.into_iter().collect::<Vec<_>>(),
            vec![
                vec![right.as_ref(), shared.as_ref()],
                vec![left.as_ref(), shared.as_ref()]
            ],
        );
    }

    #[test]
    fn activation_reindexes_paths_that_become_equivalent() {
        let first = conflict_item("package", "first");
        let second = conflict_item("package", "second");
        let mut sets: IndexSet<Vec<ConflictItemRef<'_>>, FxBuildHasher> =
            IndexSet::from_iter([vec![first.as_ref()], vec![first.as_ref(), second.as_ref()]]);

        activate_conflict_item(&mut sets, second.as_ref());

        assert_eq!(
            sets.into_iter().collect::<Vec<_>>(),
            vec![vec![first.as_ref(), second.as_ref()]]
        );
    }

    #[test]
    fn activation_paths_propagate_through_convergence_and_cycles() {
        let left = conflict_item("left", "extra");
        let right = group_item("right", "group");
        let shared = conflict_item("shared", "extra");

        let mut graph = Graph::new();
        let left_root = graph.add_node([None, None]);
        let right_root = graph.add_node([None, None]);
        let left_node = graph.add_node([Some(left.clone()), None]);
        let right_node = graph.add_node([None, Some(right.clone())]);
        let shared_node = graph.add_node([Some(shared.clone()), None]);
        let tail = graph.add_node([None, None]);
        graph.add_edge(left_root, left_node, ());
        graph.add_edge(right_root, right_node, ());
        graph.add_edge(left_node, shared_node, ());
        graph.add_edge(right_node, shared_node, ());
        graph.add_edge(shared_node, tail, ());
        graph.add_edge(tail, shared_node, ());

        let activated = propagate_conflict_activations(&graph, |items| {
            [
                items[0].as_ref().map(ConflictItem::as_ref),
                items[1].as_ref().map(ConflictItem::as_ref),
            ]
        });
        let expected = vec![
            activation_path(vec![right.clone(), shared.clone()]),
            activation_path(vec![left.clone(), shared.clone()]),
        ];

        assert_eq!(
            activated[&shared_node]
                .iter()
                .map(|path| path
                    .iter()
                    .map(ConflictItemRef::to_owned)
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            activated[&tail]
                .iter()
                .map(|path| path
                    .iter()
                    .map(ConflictItemRef::to_owned)
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            expected
        );
    }
}
