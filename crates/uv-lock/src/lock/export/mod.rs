use std::collections::VecDeque;

use either::Either;
use petgraph::prelude::EdgeRef;
use petgraph::visit::IntoNodeReferences;
use petgraph::{Direction, Graph};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};

use uv_configuration::{
    DependencyGroupsWithDefaults, ExtrasSpecificationWithDefaults, InstallOptions,
};
use uv_normalize::{ExtraName, PackageName};
use uv_pep508::MarkerTree;
use uv_pypi_types::ConflictItem;
use uv_resolver_types::UniversalMarker;

pub use crate::lock::export::metadata::{Metadata, PythonReport};
pub(crate) use crate::lock::export::metadata::{
    MetadataNode, MetadataNodeId, MetadataNodeKind, MetadataScript, MetadataWorkspace,
    MetadataWorkspaceMember,
};
pub use crate::lock::export::pylock_toml::{PylockToml, PylockTomlError, PylockTomlErrorKind};
pub use crate::lock::export::requirements_txt::RequirementsTxtExport;
use crate::lock::installable::InstallableRootKind;
use crate::lock::reachability::{Edge, Node, conflict_marker_reachability};
use crate::lock::{LockErrorKind, PackageIndex};
use crate::{Installable, LockError, Package};

pub mod cyclonedx_json;
mod metadata;
mod pylock_toml;
mod requirements_txt;

/// A flat requirement, with its associated marker.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportableRequirement<'lock> {
    /// The [`Package`] associated with the requirement.
    package: &'lock Package,
    /// The marker that must be satisfied to install the package.
    marker: MarkerTree,
    /// The list of packages that depend on this package.
    dependents: Vec<&'lock Package>,
}

/// A set of flattened, exportable requirements, generated from a lockfile.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportableRequirements<'lock>(Vec<ExportableRequirement<'lock>>);

impl<'lock> ExportableRequirements<'lock> {
    /// Generate the set of exportable [`ExportableRequirement`] entries from the given lockfile.
    fn from_lock(
        target: &impl Installable<'lock>,
        prune: &[PackageName],
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
        annotate: bool,
        install_options: &'lock InstallOptions,
    ) -> Result<Self, LockError> {
        let size_guess = target.lock().packages.len();
        let mut graph = Graph::<Node<'lock>, Edge<'lock>>::with_capacity(size_guess, size_guess);
        let mut inverse = vec![None; size_guess];

        let mut queue: VecDeque<(PackageIndex, Option<&ExtraName>)> = VecDeque::new();
        let mut seen = FxHashSet::default();
        let mut activated_items = FxHashMap::default();

        let root = graph.add_node(Node::Root);

        // Add the workspace packages and any additional dependency-group roots to the queue.
        for (root_name, root_kind) in target
            .roots()
            .map(|root| (root, InstallableRootKind::Production))
            .chain(
                target
                    .group_root(groups)
                    .map(|root| (root, InstallableRootKind::DependencyGroups)),
            )
        {
            if prune.contains(root_name) {
                continue;
            }

            let dist = target
                .lock()
                .find_by_name(root_name)
                .map_err(|_| LockErrorKind::MultipleRootPackages {
                    name: root_name.clone(),
                })?
                .ok_or_else(|| LockErrorKind::MissingRootPackage {
                    name: root_name.clone(),
                })?;

            if root_kind == InstallableRootKind::Production {
                // Track the activated package in the list of known conflicts.
                activated_items.insert(ConflictItem::from(dist.id.name.clone()), MarkerTree::TRUE);
            }

            if root_kind == InstallableRootKind::Production && groups.prod() {
                let package_index = target.lock().by_id[&dist.id];

                // Add the workspace package to the graph.
                let index = *inverse[package_index.0]
                    .get_or_insert_with(|| graph.add_node(Node::Package(dist, None)));
                graph.add_edge(
                    root,
                    index,
                    Edge::Prod {
                        marker: MarkerTree::TRUE,
                        dep_extras: Vec::new(),
                    },
                );

                // Push its dependencies on the queue.
                queue.push_back((package_index, None));
                for extra in extras
                    .extra_names(dist.optional_dependencies.keys())
                    .filter(|extra| !dist.is_known_missing_extra(extra))
                {
                    queue.push_back((package_index, Some(extra)));
                    activated_items.insert(
                        ConflictItem::from((dist.id.name.clone(), extra.clone())),
                        MarkerTree::TRUE,
                    );
                }
            }

            // Add any development dependencies.
            for (group, dep) in dist
                .dependency_groups
                .iter()
                .filter_map(|(group, deps)| {
                    if target.includes_group(Some(&dist.id.name), group, groups) {
                        Some(deps.iter().map(move |dep| (group, dep)))
                    } else {
                        None
                    }
                })
                .flatten()
            {
                // Track the activated group in the list of known conflicts.
                activated_items.insert(
                    ConflictItem::from((dist.id.name.clone(), group.clone())),
                    MarkerTree::TRUE,
                );

                if prune.contains(&dep.package_id.name) {
                    continue;
                }

                let dep_dist = target.lock().package(dep.index);

                // Add the dependency to the graph.
                let dep_index = *inverse[dep.index.0]
                    .get_or_insert_with(|| graph.add_node(Node::Package(dep_dist, None)));

                // Add an edge from the root. Development dependencies may be installed without
                // installing the workspace package itself (which can never have markers on it
                // anyway), so they're directly connected to the root.
                graph.add_edge(
                    root,
                    dep_index,
                    Edge::Dev {
                        group,
                        marker: target
                            .lock()
                            .constrain_conflicts(UniversalMarker::from_combined(
                                dep.simplified_marker.as_simplified_marker_tree(),
                            ))
                            .combined(),
                        dep_extras: target.lock().dependency_extras(dep).collect(),
                    },
                );

                // Push its dependencies on the queue.
                if seen.insert((dep.index, None)) {
                    queue.push_back((dep.index, None));
                }
                for extra in target.lock().dependency_extras(dep) {
                    if seen.insert((dep.index, Some(extra))) {
                        queue.push_back((dep.index, Some(extra)));
                    }
                }
            }
        }

        // Add requirements that are exclusive to the workspace root (e.g., dependency groups in
        // non-project workspace roots).
        let root_requirements = target
            .lock()
            .requirements()
            .iter()
            .chain(
                target
                    .lock()
                    .dependency_groups()
                    .iter()
                    .filter_map(|(group, deps)| {
                        if target.includes_group(None, group, groups) {
                            Some(deps)
                        } else {
                            None
                        }
                    })
                    .flatten(),
            )
            .filter(|dep| !prune.contains(&dep.name))
            .collect::<Vec<_>>();

        // Index the lockfile by package name, to avoid making multiple passes over the lockfile.
        if !root_requirements.is_empty() {
            let by_name: FxHashMap<_, Vec<_>> = {
                let names = root_requirements
                    .iter()
                    .map(|dep| &dep.name)
                    .collect::<FxHashSet<_>>();
                target.lock().packages().iter().fold(
                    FxHashMap::with_capacity_and_hasher(size_guess, FxBuildHasher),
                    |mut map, package| {
                        if names.contains(&package.id.name) {
                            map.entry(&package.id.name).or_default().push(package);
                        }
                        map
                    },
                )
            };

            for requirement in root_requirements {
                for dist in by_name.get(&requirement.name).into_iter().flatten() {
                    // Determine whether this entry is relevant for the requirement by
                    // intersecting and simplifying the markers.
                    let Some(marker) = target.lock().root_requirement_marker(requirement, dist)
                    else {
                        continue;
                    };
                    let package_index = target.lock().by_id[&dist.id];

                    // Add the dependency to the graph and get its index.
                    let dep_index = *inverse[package_index.0]
                        .get_or_insert_with(|| graph.add_node(Node::Package(dist, None)));

                    // Add an edge from the root.
                    graph.add_edge(
                        root,
                        dep_index,
                        Edge::Prod {
                            marker,
                            dep_extras: requirement
                                .extras
                                .iter()
                                .filter(|extra| !dist.is_known_missing_extra(extra))
                                .collect(),
                        },
                    );

                    // Push its dependencies on the queue.
                    if seen.insert((package_index, None)) {
                        queue.push_back((package_index, None));
                    }
                    for extra in requirement
                        .extras
                        .iter()
                        .filter(|extra| !dist.is_known_missing_extra(extra))
                    {
                        if seen.insert((package_index, Some(extra))) {
                            queue.push_back((package_index, Some(extra)));
                        }
                    }
                }
            }
        }

        // Create all the relevant nodes.
        while let Some((package_index, extra)) = queue.pop_front() {
            let index = inverse[package_index.0].expect("queued package has a graph node");
            let package = target.lock().package(package_index);

            let deps = if let Some(extra) = extra {
                Either::Left(
                    package
                        .optional_dependencies
                        .get(extra)
                        .into_iter()
                        .flatten(),
                )
            } else {
                Either::Right(package.dependencies.iter())
            };

            for dep in deps {
                if prune.contains(&dep.package_id.name) {
                    continue;
                }

                // Evaluate the conflict marker.
                let dep_dist = target.lock().package(dep.index);

                // Add the dependency to the graph.
                let dep_index = *inverse[dep.index.0]
                    .get_or_insert_with(|| graph.add_node(Node::Package(dep_dist, None)));

                let dep_extras = target.lock().dependency_extras(dep).collect::<Vec<_>>();
                let marker = target
                    .lock()
                    .constrain_conflicts(UniversalMarker::from_combined(
                        dep.simplified_marker.as_simplified_marker_tree(),
                    ))
                    .combined();
                graph.add_edge(
                    index,
                    dep_index,
                    if let Some(extra) = extra {
                        Edge::Optional {
                            extra,
                            marker,
                            dep_extras,
                        }
                    } else {
                        Edge::Prod { marker, dep_extras }
                    },
                );

                // Push its dependencies on the queue.
                if seen.insert((dep.index, None)) {
                    queue.push_back((dep.index, None));
                }
                for extra in target.lock().dependency_extras(dep) {
                    if seen.insert((dep.index, Some(extra))) {
                        queue.push_back((dep.index, Some(extra)));
                    }
                }
            }
        }

        // Determine the reachability of each node in the graph.
        let mut reachability = conflict_marker_reachability(&graph, &[], &activated_items);

        // Collect all packages.
        let nodes = graph
            .node_references()
            .filter_map(|(index, node)| match node {
                Node::Root => None,
                Node::Package(package, _) => Some((index, package)),
            })
            .filter(|(_index, package)| {
                install_options.include_package(
                    package.as_install_target(),
                    target.project_name(),
                    target.lock().members(),
                )
            })
            .map(|(index, package)| ExportableRequirement {
                package,
                marker: reachability.remove(&index).unwrap_or_default(),
                dependents: if annotate {
                    let mut dependents = graph
                        .edges_directed(index, Direction::Incoming)
                        .map(|edge| &graph[edge.source()])
                        .filter_map(|node| match node {
                            Node::Package(package, _) => Some(*package),
                            Node::Root => None,
                        })
                        .collect::<Vec<_>>();
                    dependents.sort_unstable_by_key(|package| package.name());
                    dependents.dedup_by_key(|package| package.name());
                    dependents
                } else {
                    Vec::new()
                },
            })
            .filter(|requirement| !requirement.marker.is_false())
            .collect::<Vec<_>>();

        Ok(Self(nodes))
    }
}
