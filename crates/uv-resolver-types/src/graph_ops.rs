use std::collections::BTreeSet;
use std::ops::{Index, IndexMut};

use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::{Direction, Graph};
use rustc_hash::{FxHashMap, FxHashSet};

use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictItemRef, Conflicts, Inference};

use crate::ResolutionGraphNode;
use crate::universal_marker::UniversalMarker;

/// Optional state indexed by the dense node indices of a graph with fixed structure.
///
/// Node removal or insertion can invalidate the association with the original graph. A stored
/// false marker remains distinct from a node that has no state.
#[derive(Debug)]
pub struct NodeMap<T>(Vec<Option<T>>);

impl<T> NodeMap<T> {
    /// Allocate an empty slot for every node in the graph.
    pub fn new(node_count: usize) -> Self {
        Self((0..node_count).map(|_| None).collect())
    }
}

impl<T> Index<NodeIndex> for NodeMap<T> {
    type Output = Option<T>;

    fn index(&self, index: NodeIndex) -> &Self::Output {
        &self.0[index.index()]
    }
}

impl<T> IndexMut<NodeIndex> for NodeMap<T> {
    fn index_mut(&mut self, index: NodeIndex) -> &mut Self::Output {
        &mut self.0[index.index()]
    }
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
) -> NodeMap<Marker> {
    // Note that we build including the virtual packages due to how we propagate markers through
    // the graph, even though we then only read the markers for base packages.
    let mut reachability = NodeMap::new(graph.node_count());

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
        reachability[*root_index] = Some(root_markers);
    }

    // Propagate all markers through the graph, so that the eventual marker for each node is the
    // union of the markers of each path we can reach the node by.
    while let Some(parent_index) = queue.pop() {
        let Some(marker) = reachability[parent_index] else {
            continue;
        };
        for child_edge in graph.edges_directed(parent_index, Direction::Outgoing) {
            // The marker for all paths to the child through the parent.
            let mut child_marker = child_edge.weight().marker();
            child_marker.and(marker);
            match &mut reachability[child_edge.target()] {
                Some(existing) => {
                    // If the marker is a subset of the existing marker (A ⊆ B exactly if
                    // A ∪ B = A), updating the child wouldn't change child's marker.
                    child_marker.or(*existing);
                    if child_marker != *existing {
                        *existing = child_marker;
                        queue.push(child_edge.target());
                    }
                }
                slot @ None => {
                    *slot = Some(child_marker);
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

    while let Some(parent_index) = queue.pop() {
        let extra = graph[parent_index]
            .package_extra_names()
            .map(ConflictItemRef::from);
        let group = graph[parent_index]
            .package_group_names()
            .map(ConflictItemRef::from);
        for item in extra
            .into_iter()
            .chain(group)
            .filter(|item| relevant.contains(item))
        {
            for set in activated
                .entry(parent_index)
                .or_insert_with(|| vec![FxHashSet::default()])
            {
                set.insert(item);
            }
        }
        let sets = activated
            .get(&parent_index)
            .cloned()
            .unwrap_or_else(|| vec![FxHashSet::default()]);
        for child_edge in graph.edges_directed(parent_index, Direction::Outgoing) {
            let mut change = false;
            let existing = activated.entry(child_edge.target()).or_default();
            for set in &sets {
                if !existing.contains(set) {
                    existing.push(set.clone());
                    change = true;
                }
            }
            if change {
                queue.push(child_edge.target());
            }
        }
    }

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

pub trait Reachable<T> {
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use petgraph::Graph;
    use uv_pep508::{MarkerTree, Pep508Error};

    use super::marker_reachability;

    #[test]
    fn reachability_unions_diamonds_and_handles_cycles() -> Result<(), Pep508Error> {
        let linux = MarkerTree::from_str("sys_platform == 'linux'")?;
        let windows = MarkerTree::from_str("sys_platform == 'win32'")?;
        let mut graph = Graph::new();
        let root = graph.add_node(());
        let left = graph.add_node(());
        let right = graph.add_node(());
        let join = graph.add_node(());
        graph.add_edge(root, left, linux);
        graph.add_edge(root, right, windows);
        graph.add_edge(left, join, MarkerTree::TRUE);
        graph.add_edge(right, join, MarkerTree::TRUE);
        graph.add_edge(join, left, linux);
        let markers = marker_reachability(&graph, &[]);
        assert_eq!(markers[root], Some(MarkerTree::TRUE));
        assert_eq!(markers[left], Some(linux));
        assert_eq!(markers[right], Some(windows));
        assert_eq!(markers[join], Some(linux.or(windows)));

        let markers = marker_reachability(&graph, &[linux]);
        assert_eq!(markers[root], Some(linux));
        assert_eq!(markers[left], Some(linux));
        assert_eq!(markers[right], Some(MarkerTree::FALSE));
        assert_eq!(markers[join], Some(linux));
        Ok(())
    }

    #[test]
    fn reachability_distinguishes_false_from_unvisited() {
        let mut graph = Graph::new();
        let root = graph.add_node(());
        let false_child = graph.add_node(());
        let descendant = graph.add_node(());
        let isolated = graph.add_node(());
        let unreachable_left = graph.add_node(());
        let unreachable_right = graph.add_node(());
        graph.add_edge(root, false_child, MarkerTree::FALSE);
        graph.add_edge(false_child, descendant, MarkerTree::TRUE);
        graph.add_edge(unreachable_left, unreachable_right, MarkerTree::TRUE);
        graph.add_edge(unreachable_right, unreachable_left, MarkerTree::TRUE);
        let markers = marker_reachability(&graph, &[]);
        assert_eq!(markers[root], Some(MarkerTree::TRUE));
        assert_eq!(markers[false_child], Some(MarkerTree::FALSE));
        assert_eq!(markers[descendant], Some(MarkerTree::FALSE));
        assert_eq!(markers[isolated], Some(MarkerTree::TRUE));
        assert_eq!(markers[unreachable_left], None);
        assert_eq!(markers[unreachable_right], None);
    }
}
