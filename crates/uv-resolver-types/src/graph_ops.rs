use std::collections::BTreeSet;
use std::collections::hash_map::Entry;

use indexmap::IndexSet;
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::{Direction, Graph};
use rustc_hash::{FxBuildHasher, FxHashMap};

use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictItemRef, Conflicts, Inference};

use crate::ResolutionGraphNode;
use crate::universal_marker::UniversalMarker;

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

    // Unrelated extras and groups cannot simplify a conflict marker. Keep the pool local to
    // this traversal so identical activation paths and their inferences can be shared by nodes.
    let mut worlds = ConflictWorlds::new(
        conflicts
            .iter()
            .flat_map(|set| set.iter().map(ConflictItem::as_ref)),
    );
    let activated = propagate_conflict_activations(graph, &mut worlds, |node| {
        [
            node.package_extra_names().map(ConflictItemRef::from),
            node.package_group_names().map(ConflictItemRef::from),
        ]
    });

    let inferences: Vec<BTreeSet<Inference>> = worlds
        .worlds
        .iter()
        .map(|world| {
            let mut inferences = BTreeSet::new();
            for item in world.iter().map(|item| worlds.items[item.0]) {
                for conflict_set in conflicts.iter() {
                    if !conflict_set.contains(item.package(), item.kind()) {
                        continue;
                    }
                    for conflict_item in conflict_set.iter() {
                        if conflict_item.as_ref() == item {
                            continue;
                        }
                        inferences.insert(Inference {
                            item: conflict_item.clone(),
                            included: false,
                        });
                    }
                }
                inferences.insert(Inference {
                    item: item.to_owned(),
                    included: true,
                });
            }
            inferences
        })
        .collect();

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
        let Some(node_worlds) = activated.get(&from_index) else {
            continue;
        };
        // If not all possible paths (represented by our inferences)
        // satisfy the conflict marker on this edge, then we can't make any
        // simplifications. Namely, because it follows that out inferences
        // aren't always true. Some of them may sometimes be false.
        let all_paths_satisfied = node_worlds.iter().all(|world| {
            let set = &inferences[world.0];
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
            for world in node_worlds {
                for inf in &inferences[world.0] {
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
            graph[edge_index]
                .unify_inference_sets(node_worlds.iter().map(|world| &inferences[world.0]));
        }
    }
}

/// An item and a world are scoped to one conflict propagation, never to the resolver as a whole.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct ConflictItemId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WorldId(usize);

struct ConflictWorlds<'a> {
    items: IndexSet<ConflictItemRef<'a>, FxBuildHasher>,
    worlds: IndexSet<Box<[ConflictItemId]>, FxBuildHasher>,
}

impl<'a> ConflictWorlds<'a> {
    const EMPTY: WorldId = WorldId(0);

    fn new(items: impl IntoIterator<Item = ConflictItemRef<'a>>) -> Self {
        Self {
            items: items.into_iter().collect(),
            worlds: IndexSet::from_iter([Box::default()]),
        }
    }

    /// Return the canonical world containing the item without mutating an interned key.
    fn activate(&mut self, world: WorldId, item: ConflictItemId) -> WorldId {
        match self.worlds[world.0].binary_search(&item) {
            Ok(_) => world,
            Err(index) => {
                let mut items = self.worlds[world.0].to_vec();
                items.insert(index, item);
                WorldId(self.worlds.insert_full(items.into_boxed_slice()).0)
            }
        }
    }
}

/// Propagate every distinct activation path. A union of paths would lose the all-paths condition
/// required to simplify a conflict marker.
fn propagate_conflict_activations<'a, Node, Edge>(
    graph: &'a Graph<Node, Edge>,
    worlds: &mut ConflictWorlds<'a>,
    conflict_items: impl Fn(&'a Node) -> [Option<ConflictItemRef<'a>>; 2],
) -> FxHashMap<NodeIndex, Vec<WorldId>> {
    let mut activated: FxHashMap<NodeIndex, Vec<WorldId>> = FxHashMap::default();

    // Besides the virtual root, virtual dev-dependency packages can also be roots.
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
            let Some(item) = worlds.items.get_index_of(&item).map(ConflictItemId) else {
                continue;
            };
            let existing = activated
                .entry(parent_index)
                .or_insert_with(|| vec![ConflictWorlds::EMPTY]);
            // Activation can make distinct paths equivalent. Retain their first-seen order.
            let mut next = Vec::with_capacity(existing.len());
            for world in existing.iter() {
                let world = worlds.activate(*world, item);
                if !next.contains(&world) {
                    next.push(world);
                }
            }
            *existing = next;
        }
        let node_worlds = activated
            .get(&parent_index)
            .cloned()
            .unwrap_or_else(|| vec![ConflictWorlds::EMPTY]);
        for child_edge in graph.edges_directed(parent_index, Direction::Outgoing) {
            let mut change = false;
            let existing = activated.entry(child_edge.target()).or_default();
            for world in &node_worlds {
                if !existing.contains(world) {
                    existing.push(*world);
                    change = true;
                }
            }
            if change {
                queue.push(child_edge.target());
            }
        }
    }
    activated
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
    use std::collections::BTreeSet;

    use petgraph::graph::NodeIndex;
    use petgraph::visit::EdgeRef;
    use petgraph::{Direction, Graph};
    use rustc_hash::{FxHashMap, FxHashSet};
    use uv_normalize::{ExtraName, GroupName, PackageName};
    use uv_pypi_types::{ConflictItem, ConflictItemRef};

    use super::{ConflictWorlds, propagate_conflict_activations};

    // A direct set representation provides an independent oracle for the interned traversal.
    fn propagate_sets<'a>(
        graph: &Graph<[Option<ConflictItemRef<'a>>; 2], ()>,
        relevant: &FxHashSet<ConflictItemRef<'a>>,
    ) -> FxHashMap<NodeIndex, Vec<FxHashSet<ConflictItemRef<'a>>>> {
        let mut activated: FxHashMap<NodeIndex, Vec<FxHashSet<ConflictItemRef<'a>>>> =
            FxHashMap::default();
        let mut queue: Vec<_> = graph
            .node_indices()
            .filter(|index| {
                graph
                    .edges_directed(*index, Direction::Incoming)
                    .next()
                    .is_none()
            })
            .collect();
        while let Some(parent) = queue.pop() {
            for item in graph[parent]
                .into_iter()
                .flatten()
                .filter(|item| relevant.contains(item))
            {
                for set in activated
                    .entry(parent)
                    .or_insert_with(|| vec![FxHashSet::default()])
                {
                    set.insert(item);
                }
            }
            let sets = activated
                .get(&parent)
                .cloned()
                .unwrap_or_else(|| vec![FxHashSet::default()]);
            for edge in graph.edges_directed(parent, Direction::Outgoing) {
                let existing = activated.entry(edge.target()).or_default();
                let mut change = false;
                for set in &sets {
                    if !existing.contains(set) {
                        existing.push(set.clone());
                        change = true;
                    }
                }
                if change {
                    queue.push(edge.target());
                }
            }
        }
        activated
    }

    #[test]
    fn interned_activation_matches_set_propagation() -> Result<(), Box<dyn std::error::Error>> {
        let package: PackageName = "package".parse()?;
        let other_package: PackageName = "other-package".parse()?;
        let extra: ExtraName = "shared".parse()?;
        let other_extra: ExtraName = "other".parse()?;
        let group: GroupName = "shared".parse()?;
        let items = [
            ConflictItem::from((package.clone(), extra.clone())),
            ConflictItem::from((other_package, extra)),
            ConflictItem::from((package.clone(), group)),
            ConflictItem::from((package.clone(), other_extra)),
            ConflictItem::from((package, "irrelevant".parse::<ExtraName>()?)),
        ];
        let relevant: FxHashSet<_> = items[..4].iter().map(ConflictItem::as_ref).collect();
        // Include cycles, fan-in, repeated activation, namespaced extras, groups, irrelevant
        // extras and nodes that cannot be reached from a root.
        for seed in 0..64u64 {
            let mut graph = Graph::new();
            let nodes: Vec<_> = (0..7)
                .map(|index| {
                    graph.add_node(if index == 0 || index >= 5 {
                        [None, None]
                    } else {
                        [
                            Some(items[index % 5].as_ref()),
                            (index % 2 == 0).then(|| items[0].as_ref()),
                        ]
                    })
                })
                .collect();
            let mut state = seed + 1;
            for (index, source) in nodes.iter().enumerate() {
                for target in &nodes[index + 1..] {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1);
                    if state >> 61 == 0 {
                        graph.add_edge(*source, *target, ());
                    }
                }
            }
            graph.add_edge(nodes[5], nodes[6], ());
            graph.add_edge(nodes[6], nodes[5], ());
            let expected = propagate_sets(&graph, &relevant);
            let mut worlds = ConflictWorlds::new(relevant.iter().copied());
            let actual = propagate_conflict_activations(&graph, &mut worlds, |node| *node);
            assert_eq!(actual.len(), expected.len(), "seed {seed}");
            for (node, expected) in expected {
                let expected: BTreeSet<BTreeSet<_>> = expected
                    .into_iter()
                    .map(|set| set.into_iter().collect())
                    .collect();
                let actual: BTreeSet<BTreeSet<_>> = actual[&node]
                    .iter()
                    .map(|world| {
                        worlds.worlds[world.0]
                            .iter()
                            .map(|item| worlds.items[item.0])
                            .collect()
                    })
                    .collect();
                assert_eq!(actual, expected, "seed {seed}, node {node:?}");
            }
        }
        Ok(())
    }
}
