//! Reachability with package-scoped conflict predicates.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use itertools::Itertools;
use petgraph::graph::NodeIndex;
use petgraph::prelude::EdgeRef;
use petgraph::{Direction, Graph};
use rustc_hash::{FxBuildHasher, FxHashMap};
use uv_configuration::{
    DependencyGroupsWithDefaults, DependencyModifierScope, ExtrasSpecificationWithDefaults,
};
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep508::MarkerTree;
use uv_pypi_types::{
    ConflictItem, ConflictKind, ConflictKindRef, ConflictSet, ResolverMarkerEnvironment,
};
use uv_resolver_types::graph_ops::{Reachable, marker_reachability};
use uv_resolver_types::universal_marker::resolve_activated_extras;
use uv_resolver_types::{ConflictMarker, UniversalMarker};

use crate::lock::{
    DependencyContext, DependencySelectionContext, LockErrorKind, PackageIndex,
    normalize_requirement,
};
use crate::{
    Installable, InstallableRootKind, Lock, LockError, Package, implicit_constraints_marker,
};

/// A request graph retains conflict guards until package and extra activation is known.
struct ConflictRequests<'lock> {
    lock: &'lock Lock,
    marker_env: Option<ResolverMarkerEnvironment>,
    graph: Graph<Node<'lock>, Edge<'lock>>,
    nodes: FxHashMap<(PackageIndex, Option<ExtraName>), NodeIndex>,
    root: NodeIndex,
    queue: VecDeque<(PackageIndex, Option<ExtraName>, NodeIndex)>,
    /// Separate nodes retain the reachability of each ambiguous incoming request.
    unrecorded_requests: Vec<(NodeIndex, PackageIndex)>,
}

impl<'lock> ConflictRequests<'lock> {
    fn new(lock: &'lock Lock, marker_env: Option<&ResolverMarkerEnvironment>) -> Self {
        let mut graph = Graph::new();
        let root = graph.add_node(Node::Root);
        Self {
            lock,
            marker_env: marker_env.cloned(),
            graph,
            root,
            nodes: FxHashMap::default(),
            queue: VecDeque::new(),
            unrecorded_requests: Vec::new(),
        }
    }

    /// Resolve platform predicates before discovering which conflict selections can activate.
    fn marker_for_environment(&self, marker: MarkerTree) -> MarkerTree {
        match self.marker_env.as_ref() {
            Some(environment) => UniversalMarker::new(
                MarkerTree::TRUE,
                UniversalMarker::from_combined(marker).conflict_for_environment(environment),
            )
            .combined(),
            None => marker,
        }
    }

    fn node(&mut self, index: PackageIndex, extra: Option<ExtraName>) -> NodeIndex {
        let key = (index, extra.clone());
        if let Some(node) = self.nodes.get(&key) {
            return *node;
        }
        let node = self
            .graph
            .add_node(Node::Package(self.lock.package(index), extra.clone()));
        self.nodes.insert(key, node);
        let is_extra = extra.is_some();
        self.queue.push_back((index, extra, node));
        if is_extra {
            // An extra also activates the package's production dependencies under the same guard.
            let base = self.node(index, None);
            self.graph.add_edge(
                node,
                base,
                Edge::Prod {
                    marker: MarkerTree::TRUE,
                    dep_extras: Vec::new(),
                },
            );
        }
        node
    }

    fn push(
        &mut self,
        parent: NodeIndex,
        index: PackageIndex,
        extra: Option<ExtraName>,
        marker: MarkerTree,
        requests_recorded: bool,
    ) {
        let package = self.lock.package(index);
        let marker = if package.fork_markers.is_empty() {
            marker
        } else {
            marker.and(
                package
                    .fork_markers
                    .iter()
                    .fold(MarkerTree::FALSE, |combined, fork| {
                        combined.or(fork.pep508())
                    }),
            )
        };
        let marker = self.marker_for_environment(marker);
        if marker.is_false() {
            return;
        }
        if let Some(extra) = &extra {
            if package.is_known_missing_extra(extra) {
                return;
            }
            if !package.id.source.is_immutable()
                && !package.optional_dependencies.contains_key(extra)
                && !package.metadata.provides_extra.contains(extra)
            {
                return;
            }
        }
        if !requests_recorded
            && package.has_unrecorded_conflict_extra_requests(self.lock.conflicts())
        {
            let node = self.graph.add_node(Node::Package(package, None));
            self.graph.add_edge(
                parent,
                node,
                Edge::Prod {
                    marker,
                    dep_extras: Vec::new(),
                },
            );
            self.unrecorded_requests.push((node, index));
        }
        let node = self.node(index, extra);
        self.graph.add_edge(
            parent,
            node,
            Edge::Prod {
                marker,
                dep_extras: Vec::new(),
            },
        );
    }

    /// Return the project or extra requested by a graph node.
    fn requested_item(&self, node: &Node<'_>) -> Option<ConflictItem> {
        match node {
            Node::Root => None,
            Node::Package(package, Some(extra)) => {
                Some(ConflictItem::from((package.name().clone(), extra.clone())))
            }
            Node::Package(package, None) => self
                .lock
                .is_workspace_package(package)
                .then(|| ConflictItem::from(package.name().clone())),
        }
    }

    /// Resolve globally certain project and extra requests before evaluating sibling guards.
    /// Symbolic cycles stay local; every successful pass removes at least one pending item.
    fn global_conflicts(
        &self,
        known_conflicts: &FxHashMap<ConflictItem, MarkerTree>,
    ) -> FxHashMap<ConflictItem, MarkerTree> {
        let graph = self.graph.map(
            |_, _| (),
            |index, edge| {
                let mut marker = UniversalMarker::from_combined(*edge.marker());
                // A request activates its own selection on this edge. Ancestor guards still
                // determine which resolved package version can make the request.
                if let Some((_, target)) = self.graph.edge_endpoints(index)
                    && let Some(item) = self.requested_item(&self.graph[target])
                {
                    marker.assume_conflict_item(&item);
                }
                marker.combined()
            },
        );
        let reachability = marker_reachability(&graph, &[]);
        let mut pending = FxHashMap::<ConflictItem, MarkerTree>::default();
        for node in self.nodes.values() {
            let Some(item) = self.requested_item(&self.graph[*node]) else {
                continue;
            };
            if !self
                .lock
                .conflicts()
                .contains(item.package(), item.kind().as_ref())
            {
                continue;
            }
            let marker = reachability.get(node).copied().unwrap_or(MarkerTree::FALSE);
            pending
                .entry(item)
                .and_modify(|current| *current = current.or(marker))
                .or_insert(marker);
        }
        let mut resolved = known_conflicts.clone();
        for item in self.lock.conflicts().iter().flat_map(ConflictSet::iter) {
            if !pending.contains_key(item) {
                resolved.entry(item.clone()).or_insert(MarkerTree::FALSE);
            }
        }
        while !pending.is_empty() {
            let mut substitutions = resolved.clone();
            for item in pending.keys() {
                substitutions.entry(item.clone()).or_insert_with(|| {
                    UniversalMarker::new(MarkerTree::TRUE, ConflictMarker::from_conflict_item(item))
                        .combined()
                });
            }
            let remaining = pending.len();
            pending.retain(|item, marker| {
                let marker =
                    resolve_activated_extras(*marker, Some(item.package()), &substitutions);
                if UniversalMarker::from_combined(marker).has_conflict_marker() {
                    return true;
                }
                resolved
                    .entry(item.clone())
                    .and_modify(|current| *current = current.or(marker))
                    .or_insert(marker);
                false
            });
            if pending.len() == remaining {
                break;
            }
        }
        resolved
    }

    /// Require legacy declaration evidence only after resolving request reachability.
    fn finish(
        self,
        known_conflicts: &FxHashMap<ConflictItem, MarkerTree>,
    ) -> impl Iterator<Item = Result<(PackageIndex, Option<ExtraName>, MarkerTree), LockError>>
    {
        let known_conflicts = self.global_conflicts(known_conflicts);
        let reachability =
            conflict_marker_reachability(self.lock, &self.graph, &[], &known_conflicts);
        let lock = self.lock;
        let unrecorded_requests = self
            .unrecorded_requests
            .into_iter()
            .filter_map(|(node, index)| {
                let marker = reachability
                    .get(&node)
                    .copied()
                    .unwrap_or(MarkerTree::FALSE);
                (!marker.is_false()).then(|| {
                    Err(LockErrorKind::MissingExtraRequests {
                        package: lock.package(index).name().clone(),
                    }
                    .into())
                })
            })
            .collect::<Vec<_>>();
        unrecorded_requests
            .into_iter()
            .chain(
                self.nodes
                    .into_iter()
                    .filter_map(move |((index, extra), node)| {
                        let marker = reachability
                            .get(&node)
                            .copied()
                            .unwrap_or(MarkerTree::FALSE);
                        if marker.is_false() {
                            return None;
                        }
                        let package = lock.package(index);
                        // Legacy serialization can erase an empty extra and its incoming request, so
                        // reachable production nodes also need declaration evidence.
                        if let Err(err) = package.validate_conflict_extra_metadata(lock.conflicts())
                        {
                            return Some(Err(err));
                        }
                        Some(Ok((index, extra, marker)))
                    }),
            )
    }
}

/// The roots whose requests must be validated before materializing conflict forks.
#[derive(Clone, Copy)]
enum ConflictRoots<'a, 'lock> {
    Installable,
    Concrete {
        packages: &'a [&'lock Package],
        selection_context: DependencySelectionContext<'lock>,
    },
}

/// Validate requested selections before conflict guards can remove their resolved edges.
pub(super) fn validate_requested_conflicts<'lock>(
    target: &(impl Installable<'lock> + ?Sized),
    prune: &[PackageName],
    extras: &ExtrasSpecificationWithDefaults,
    groups: &DependencyGroupsWithDefaults,
    marker_env: Option<&ResolverMarkerEnvironment>,
) -> Result<(), LockError> {
    validate_conflict_roots(
        target,
        ConflictRoots::Installable,
        prune,
        extras,
        groups,
        marker_env,
    )
}

/// Validate concrete package roots without activating unrelated manifest requirements.
pub(super) fn validate_concrete_requested_conflicts<'lock>(
    target: &(impl Installable<'lock> + ?Sized),
    roots: &[&'lock Package],
    selection_context: DependencySelectionContext<'lock>,
    extras: &ExtrasSpecificationWithDefaults,
    groups: &DependencyGroupsWithDefaults,
    marker_env: &ResolverMarkerEnvironment,
) -> Result<(), LockError> {
    validate_conflict_roots(
        target,
        ConflictRoots::Concrete {
            packages: roots,
            selection_context,
        },
        &[],
        extras,
        groups,
        Some(marker_env),
    )
}

fn validate_conflict_roots<'lock>(
    target: &(impl Installable<'lock> + ?Sized),
    roots: ConflictRoots<'_, 'lock>,
    prune: &[PackageName],
    extras: &ExtrasSpecificationWithDefaults,
    groups: &DependencyGroupsWithDefaults,
    marker_env: Option<&ResolverMarkerEnvironment>,
) -> Result<(), LockError> {
    let lock = target.lock();
    if lock.conflicts().is_empty() {
        return Ok(());
    }
    let modifiers = lock.dependency_modifiers()?;
    let mut requests = ConflictRequests::new(lock, marker_env);
    let root_marker = requests.marker_for_environment(implicit_constraints_marker(
        lock.requires_python.to_marker_tree(),
        lock.supported_environments(),
    ));
    let (roots, include_manifest, selection_context) = match roots {
        ConflictRoots::Installable => {
            let mut roots = Vec::new();
            for (name, kind) in target
                .roots()
                .map(|name| (name, InstallableRootKind::Production))
                .chain(
                    target
                        .group_root(groups)
                        .map(|name| (name, InstallableRootKind::DependencyGroups)),
                )
            {
                if prune.contains(name) {
                    continue;
                }
                let package = lock
                    .find_by_name(name)
                    .map_err(|_| LockErrorKind::MultipleRootPackages { name: name.clone() })?
                    .ok_or_else(|| LockErrorKind::MissingRootPackage { name: name.clone() })?;
                roots.push((package, kind));
            }
            (roots, true, DependencySelectionContext::None)
        }
        ConflictRoots::Concrete {
            packages,
            selection_context,
        } => (
            packages
                .iter()
                .copied()
                .map(|package| (package, InstallableRootKind::Production))
                .collect(),
            false,
            selection_context,
        ),
    };
    let mut known_conflicts = FxHashMap::default();
    match selection_context {
        DependencySelectionContext::None => {}
        DependencySelectionContext::Production(project) => {
            known_conflicts.insert(ConflictItem::from(project.clone()), root_marker);
        }
        DependencySelectionContext::Group(project, group) => {
            known_conflicts.insert(
                ConflictItem::from((project.clone(), group.clone())),
                root_marker,
            );
        }
    }
    for (package, kind) in roots {
        let name = package.name();
        let index = lock.by_id[&package.id];
        if kind == InstallableRootKind::Production && groups.prod() {
            known_conflicts.insert(ConflictItem::from(name.clone()), root_marker);
            requests.push(requests.root, index, None, root_marker, true);
            for extra in extras
                .extra_names(
                    package
                        .optional_dependencies
                        .keys()
                        .chain(package.metadata.provides_extra.iter()),
                )
                .filter(|extra| !package.is_known_missing_extra(extra))
                .collect::<BTreeSet<_>>()
            {
                known_conflicts.insert(
                    ConflictItem::from((name.clone(), extra.clone())),
                    root_marker,
                );
                requests.push(requests.root, index, Some(extra.clone()), root_marker, true);
            }
        }
        for (group, dependencies) in &package.dependency_groups {
            if !target.includes_group(Some(name), group, groups) {
                continue;
            }
            known_conflicts.insert(
                ConflictItem::from((name.clone(), group.clone())),
                root_marker,
            );
            let requirements = package.dependency_requirements(
                DependencyContext::Group(group),
                &modifiers,
                target.install_path(),
                lock.requires_python(),
            )?;
            for dependency in dependencies {
                if prune.contains(dependency.package_name()) {
                    continue;
                }
                let activation =
                    dependency.activation(lock, requirements.as_deref(), target.install_path())?;
                requests.push(
                    requests.root,
                    dependency.index,
                    None,
                    root_marker.and(activation.marker),
                    activation.requests_recorded,
                );
                for (extra, marker) in activation.extras {
                    requests.push(
                        requests.root,
                        dependency.index,
                        Some(extra),
                        root_marker.and(marker),
                        true,
                    );
                }
            }
        }
        for group in package.metadata.dependency_groups.keys() {
            if target.includes_group(Some(name), group, groups) {
                known_conflicts.insert(
                    ConflictItem::from((name.clone(), group.clone())),
                    root_marker,
                );
            }
        }
    }
    let requirements = lock
        .requirements()
        .iter()
        .filter(|_| include_manifest && groups.prod())
        .chain(
            lock.dependency_groups()
                .iter()
                .filter(|(group, _)| include_manifest && target.includes_group(None, group, groups))
                .flat_map(|(_, requirements)| requirements),
        );
    for requirement in modifiers.apply(DependencyModifierScope::Global, requirements) {
        let requirement = normalize_requirement(
            requirement.into_owned(),
            target.install_path(),
            lock.requires_python(),
        )?;
        if prune.contains(&requirement.name) {
            continue;
        }
        for package in lock.packages_for_name(&requirement.name) {
            if !package
                .id
                .satisfies_requirement(&requirement, target.install_path())?
            {
                continue;
            }
            let Some(marker) = lock.root_requirement_marker(&requirement, package) else {
                continue;
            };
            let index = lock.by_id[&package.id];
            requests.push(requests.root, index, None, root_marker.and(marker), true);
            for extra in &requirement.extras {
                requests.push(
                    requests.root,
                    index,
                    Some(extra.clone()),
                    root_marker.and(marker),
                    true,
                );
            }
        }
    }
    while let Some((index, extra, parent)) = requests.queue.pop_front() {
        let package = lock.package(index);
        let context = extra
            .as_ref()
            .map_or(DependencyContext::Production, DependencyContext::Extra);
        let requirements = package.dependency_requirements(
            context,
            &modifiers,
            target.install_path(),
            lock.requires_python(),
        )?;
        for dependency in context.dependencies(package) {
            if prune.contains(dependency.package_name()) {
                continue;
            }
            let activation =
                dependency.activation(lock, requirements.as_deref(), target.install_path())?;
            requests.push(
                parent,
                dependency.index,
                None,
                activation.marker,
                activation.requests_recorded,
            );
            for (extra, marker) in activation.extras {
                requests.push(parent, dependency.index, Some(extra), marker, true);
            }
        }
    }
    let mut activated = known_conflicts
        .iter()
        .map(|(item, marker)| (item.clone(), *marker))
        .collect::<BTreeMap<_, _>>();
    for request in requests.finish(&known_conflicts) {
        let (index, extra, marker) = request?;
        let package = lock.package(index);
        if extra.is_none() && lock.is_workspace_package(package) {
            activated
                .entry(ConflictItem::from(package.name().clone()))
                .and_modify(|current| *current = current.or(marker))
                .or_insert(marker);
        }
        if let Some(extra) = extra {
            activated
                .entry(ConflictItem::from((package.name().clone(), extra)))
                .and_modify(|current| *current = current.or(marker))
                .or_insert(marker);
        }
    }
    for set in lock.conflicts().iter() {
        let items = set
            .iter()
            .filter_map(|item| activated.get(item).map(|marker| (item, marker)));
        for ((item1, marker1), (item2, marker2)) in items.tuple_combinations() {
            if marker1.is_disjoint(*marker2) {
                continue;
            }
            if let (ConflictKind::Extra(extra1), ConflictKind::Extra(extra2)) =
                (item1.kind(), item2.kind())
            {
                return Err(LockErrorKind::ConflictingExtra {
                    package1: item1.package().clone(),
                    extra1: extra1.clone(),
                    package2: item2.package().clone(),
                    extra2: extra2.clone(),
                }
                .into());
            }
            return Err(LockErrorKind::ConflictingSelections(item1.clone(), item2.clone()).into());
        }
    }
    Ok(())
}

/// A node in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Node<'lock> {
    Root,
    Package(&'lock Package, Option<ExtraName>),
}

/// An edge in the resolution graph, along with the marker that must be satisfied to traverse it.
#[derive(Debug, Clone)]
pub(super) enum Edge<'lock> {
    Prod {
        marker: MarkerTree,
        dep_extras: Vec<&'lock ExtraName>,
    },
    Optional {
        extra: &'lock ExtraName,
        marker: MarkerTree,
        dep_extras: Vec<&'lock ExtraName>,
    },
    Dev {
        group: &'lock GroupName,
        marker: MarkerTree,
        dep_extras: Vec<&'lock ExtraName>,
    },
}

impl Edge<'_> {
    /// Return the [`MarkerTree`] for this edge.
    fn marker(&self) -> &MarkerTree {
        match self {
            Self::Prod { marker, .. } => marker,
            Self::Optional { marker, .. } => marker,
            Self::Dev { marker, .. } => marker,
        }
    }

    /// Return the dependency extras activated by traversing this edge.
    fn dep_extras(&self) -> &[&ExtraName] {
        match self {
            Self::Prod { dep_extras, .. } => dep_extras,
            Self::Optional { dep_extras, .. } => dep_extras,
            Self::Dev { dep_extras, .. } => dep_extras,
        }
    }
}

impl Reachable<MarkerTree> for Edge<'_> {
    fn true_marker() -> MarkerTree {
        MarkerTree::TRUE
    }

    fn false_marker() -> MarkerTree {
        MarkerTree::FALSE
    }

    fn marker(&self) -> MarkerTree {
        *self.marker()
    }
}

/// Determine the markers under which a package is reachable in the dependency tree, taking into
/// account conflicts.
///
/// This method is structurally similar to [`marker_reachability`], but it _also_ attempts to resolve
/// conflict markers. Specifically, in addition to tracking the reachability marker for each node,
/// we also track (for each node) the conditions under which each conflict item is `true`. Then,
/// when evaluating the marker for the node, we inline the conflict marker conditions, thus removing
/// all conflict items from the marker expression.
pub(super) fn conflict_marker_reachability<'lock>(
    lock: &Lock,
    graph: &Graph<Node<'lock>, Edge<'lock>>,
    fork_markers: &[Edge<'lock>],
    known_conflicts: &FxHashMap<ConflictItem, MarkerTree>,
) -> FxHashMap<NodeIndex, MarkerTree> {
    // For each node, track the conditions under which each conflict item is enabled.
    let mut conflict_maps =
        FxHashMap::<NodeIndex, FxHashMap<ConflictItem, MarkerTree>>::with_capacity_and_hasher(
            graph.node_count(),
            FxBuildHasher,
        );

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
        MarkerTree::TRUE
    } else {
        fork_markers
            .iter()
            .fold(MarkerTree::FALSE, |mut acc, edge| {
                acc = acc.or(*edge.marker());
                acc
            })
    };
    for root_index in &queue {
        reachability.insert(*root_index, root_markers);
    }

    // Propagate all markers through the graph, so that the eventual marker for each node is the
    // union of the markers of each path we can reach the node by.
    while let Some(parent_index) = queue.pop() {
        // Resolve any conflicts in the parent marker.
        reachability.entry(parent_index).and_modify(|marker| {
            let conflict_map = conflict_maps.get(&parent_index).unwrap_or(known_conflicts);
            let scope_package = match &graph[parent_index] {
                Node::Package(package, _) => Some(package.name()),
                Node::Root => None,
            };
            *marker = resolve_activated_extras(*marker, scope_package, conflict_map);
        });

        // When we see an edge like `parent [dotenv]> flask`, we should take the reachability
        // on `parent`, combine it with the marker on the edge, then add `flask[dotenv]` to
        // the inference map on the `flask` node.
        for child_edge in graph.edges_directed(parent_index, Direction::Outgoing) {
            let mut parent_marker = reachability[&parent_index];

            // The marker for all paths to the child through the parent.
            let mut parent_map = conflict_maps
                .get(&parent_index)
                .cloned()
                .unwrap_or_else(|| known_conflicts.clone());

            if let Node::Package(child, requested_extra) = &graph[child_edge.target()] {
                // A dependency can request a workspace project's production context without
                // selecting it as an installation root, including through a dependency group.
                if requested_extra.is_none()
                    && lock
                        .conflicts()
                        .contains(child.name(), ConflictKindRef::Project)
                    && lock.is_workspace_package(child)
                {
                    parent_map.insert(ConflictItem::from(child.name().clone()), parent_marker);
                }
                for extra in child_edge
                    .weight()
                    .dep_extras()
                    .iter()
                    .copied()
                    .chain(requested_extra.iter())
                {
                    let item = ConflictItem::from((child.name().clone(), extra.clone()));
                    parent_map.insert(item, parent_marker);
                }
            }

            let scope_package = match &graph[parent_index] {
                Node::Package(package, _) => Some(package.name()),
                Node::Root => None,
            };

            let marker = match child_edge.weight() {
                Edge::Prod { marker, .. } => {
                    // Resolve any active extras on the edge.
                    resolve_activated_extras(*marker, scope_package, &parent_map)
                }
                Edge::Optional { extra, marker, .. } => {
                    // The optional edge is only active when its extra is active. Preserve the
                    // extra's reachability marker, since matching constraints can be omitted from
                    // the dependency marker as redundant when the lockfile is written.
                    let active_marker = if let Node::Package(parent, _) = &graph[parent_index] {
                        let item = ConflictItem::from((parent.name().clone(), (*extra).clone()));
                        *parent_map.entry(item).or_insert(MarkerTree::FALSE)
                    } else {
                        parent_marker
                    };

                    // Resolve any active extras on the edge.
                    let marker = resolve_activated_extras(*marker, scope_package, &parent_map);
                    marker.and(active_marker)
                }
                Edge::Dev { group, marker, .. } => {
                    // The dependency group is active for this edge itself, so add it before
                    // resolving any active extras on the edge.
                    if let Node::Package(parent, _) = &graph[parent_index] {
                        let item = ConflictItem::from((parent.name().clone(), (*group).clone()));
                        parent_map.insert(item, parent_marker);
                    }

                    // Resolve any active extras on the edge.
                    resolve_activated_extras(*marker, scope_package, &parent_map)
                }
            };

            // Propagate the edge to the known conflicts.
            for value in parent_map.values_mut() {
                *value = value.and(marker);
            }

            // Propagate the edge to the node itself.
            parent_marker = parent_marker.and(marker);

            // Combine the inferred conflicts with the existing conflicts on the node.
            let mut conflicts_changed = false;
            match conflict_maps.entry(child_edge.target()) {
                Entry::Occupied(mut existing) => {
                    let child_map = existing.get_mut();
                    for (key, value) in parent_map {
                        let child_marker = child_map.entry(key).or_insert(MarkerTree::FALSE);
                        let combined = child_marker.or(value);
                        conflicts_changed |= combined != *child_marker;
                        *child_marker = combined;
                    }
                }
                Entry::Vacant(vacant) => {
                    vacant.insert(parent_map);
                }
            }

            // Combine the inferred marker with the existing marker on the node.
            match reachability.entry(child_edge.target()) {
                Entry::Occupied(mut existing) => {
                    // If the marker is a subset of the existing marker (A ⊆ B exactly if
                    // A ∪ B = A), updating the child wouldn't change child's marker.
                    parent_marker = parent_marker.or(*existing.get());
                    // Extra activation can change even when package reachability does not.
                    if parent_marker != *existing.get() || conflicts_changed {
                        existing.insert(parent_marker);
                        queue.push(child_edge.target());
                    }
                }
                Entry::Vacant(vacant) => {
                    vacant.insert(parent_marker);
                    queue.push(child_edge.target());
                }
            }
        }
    }

    reachability
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashMap;
    use uv_pep508::MarkerTree;

    use super::ConflictRequests;
    use crate::Lock;

    #[test]
    fn impossible_legacy_extra_request_needs_no_metadata() -> Result<(), Box<dyn std::error::Error>>
    {
        let lock: Lock = toml::from_str(
            r#"
            version = 1
            revision = 5
            requires-python = ">=3.12"
            conflicts = [[
                { package = "child", extra = "feature" },
                { package = "project", extra = "feature" },
            ]]

            [[package]]
            name = "child"
            version = "1"
            source = { registry = "https://pypi.org/simple" }
            resolution-markers = ["sys_platform == 'win32'"]
            "#,
        )?;
        let package = lock
            .find_by_name(&"child".parse()?)?
            .ok_or("missing child")?;
        let index = lock.by_id[&package.id];
        let mut requests = ConflictRequests::new(&lock, None);
        requests.push(
            requests.root,
            index,
            Some("feature".parse()?),
            MarkerTree::FALSE,
            true,
        );
        requests.push(
            requests.root,
            index,
            Some("feature".parse()?),
            "sys_platform == 'linux'".parse()?,
            true,
        );
        assert!(requests.queue.is_empty());
        let requests = requests
            .finish(&FxHashMap::default())
            .collect::<Result<Vec<_>, _>>()?;
        assert!(requests.is_empty());
        Ok(())
    }
}
