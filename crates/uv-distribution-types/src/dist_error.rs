use std::collections::VecDeque;
use std::fmt::{Debug, Display, Formatter};

use petgraph::Direction;
use petgraph::graph::NodeIndex;
use petgraph::prelude::EdgeRef;
use version_ranges::Ranges;

use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::Version;

use crate::{
    BuiltDist, Dist, DistRef, Edge, Name, Node, RequestedDist, Resolution, ResolvedDist, SourceDist,
};

/// Inspect whether an error type is a build error.
pub trait IsBuildBackendError: uv_errors::Hint + std::error::Error + Send + Sync + 'static {
    /// Returns whether the build backend failed to build the package, so it's not a uv error.
    fn is_build_backend_error(&self) -> bool;
}

/// The operation(s) that failed when reporting an error with a distribution.
#[derive(Debug)]
pub enum DistErrorKind {
    Download,
    DownloadAndBuild,
    Build,
    BuildBackend,
    Read,
}

impl DistErrorKind {
    pub fn from_requested_dist(dist: &RequestedDist, err: &impl IsBuildBackendError) -> Self {
        match dist {
            RequestedDist::Installed(_) => Self::Read,
            RequestedDist::Installable(dist) => Self::from_dist(dist, err),
        }
    }

    pub fn from_dist(dist: &Dist, err: &impl IsBuildBackendError) -> Self {
        if err.is_build_backend_error() {
            Self::BuildBackend
        } else {
            match dist {
                Dist::Built(BuiltDist::Path(_)) => Self::Read,
                Dist::Source(SourceDist::Path(_) | SourceDist::Directory(_)) => Self::Build,
                Dist::Built(_) => Self::Download,
                Dist::Source(source_dist) => {
                    if source_dist.is_local() {
                        Self::Build
                    } else {
                        Self::DownloadAndBuild
                    }
                }
            }
        }
    }
}

impl Display for DistErrorKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Download => f.write_str("Failed to download"),
            Self::DownloadAndBuild => f.write_str("Failed to download and build"),
            Self::Build => f.write_str("Failed to build"),
            Self::BuildBackend => f.write_str("Failed to build"),
            Self::Read => f.write_str("Failed to read"),
        }
    }
}

/// A chain of derivation steps from the root package to the current package, to explain why a
/// package is included in the resolution.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct DerivationChain(Vec<DerivationStep>);

impl FromIterator<DerivationStep> for DerivationChain {
    fn from_iter<T: IntoIterator<Item = DerivationStep>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl DerivationChain {
    /// Compute a [`DerivationChain`] from a resolution graph.
    ///
    /// This is used to construct a derivation chain upon install failure in the `uv pip` context,
    /// where we don't have a lockfile describing the resolution.
    pub fn from_resolution(resolution: &Resolution, target: DistRef<'_>) -> Option<Self> {
        // Find the target distribution in the resolution graph.
        let target = resolution.graph().node_indices().find(|node| {
            let Node::Dist {
                dist: ResolvedDist::Installable { dist, .. },
                ..
            } = &resolution.graph()[*node]
            else {
                return false;
            };
            target == dist.as_ref().into()
        })?;

        // A direct dependency has no derivation steps and is always the shortest path.
        if resolution
            .graph()
            .edges_directed(target, Direction::Incoming)
            .any(|edge| matches!(&resolution.graph()[edge.source()], Node::Root))
        {
            return Some(Self::default());
        }

        // Perform a BFS to find the shortest path to the root.
        let mut queue = VecDeque::from([target]);
        let mut predecessors: Vec<Option<(NodeIndex, &Edge)>> =
            vec![None; resolution.graph().node_count()];

        // TODO(charlie): Consider respecting markers here.
        while let Some(node) = queue.pop_front() {
            match &resolution.graph()[node] {
                Node::Root => {
                    let mut path = Vec::new();
                    let mut node = node;
                    while let Some((next, edge)) = predecessors[node.index()] {
                        if let Node::Dist { dist, .. } = &resolution.graph()[node] {
                            let extra = match edge {
                                Edge::Optional(extra) => Some(extra.clone()),
                                _ => None,
                            };
                            let group = match edge {
                                Edge::Dev(group) => Some(group.clone()),
                                _ => None,
                            };
                            path.push(DerivationStep::new(
                                dist.name().clone(),
                                extra,
                                group,
                                dist.version().cloned(),
                                Ranges::empty(),
                            ));
                        }
                        node = next;
                    }
                    return Some(Self::from_iter(path));
                }
                Node::Dist { .. } => {
                    for edge in resolution.graph().edges_directed(node, Direction::Incoming) {
                        let predecessor = edge.source();
                        if predecessor != target && predecessors[predecessor.index()].is_none() {
                            predecessors[predecessor.index()] = Some((node, edge.weight()));
                            queue.push_back(predecessor);
                        }
                    }
                }
            }
        }

        None
    }

    /// Returns `true` if the derivation chain is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns an iterator over the steps in the derivation chain.
    pub fn iter(&self) -> std::slice::Iter<'_, DerivationStep> {
        self.0.iter()
    }
}

impl<'chain> IntoIterator for &'chain DerivationChain {
    type Item = &'chain DerivationStep;
    type IntoIter = std::slice::Iter<'chain, DerivationStep>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl IntoIterator for DerivationChain {
    type Item = DerivationStep;
    type IntoIter = std::vec::IntoIter<DerivationStep>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// A step in a derivation chain.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DerivationStep {
    /// The name of the package.
    pub name: PackageName,
    /// The enabled extra of the package, if any.
    pub extra: Option<ExtraName>,
    /// The enabled dependency group of the package, if any.
    pub group: Option<GroupName>,
    /// The version of the package.
    pub version: Option<Version>,
    /// The constraints applied to the subsequent package in the chain.
    pub range: Ranges<Version>,
}

impl DerivationStep {
    /// Create a [`DerivationStep`] from a package name and version.
    pub fn new(
        name: PackageName,
        extra: Option<ExtraName>,
        group: Option<GroupName>,
        version: Option<Version>,
        range: Ranges<Version>,
    ) -> Self {
        Self {
            name,
            extra,
            group,
            version,
            range,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;

    use petgraph::graph::DiGraph;
    use uv_distribution_filename::SourceDistExtension;
    use uv_pep508::VerbatimUrl;
    use uv_pypi_types::HashDigests;
    use uv_redacted::DisplaySafeUrl;

    use super::{DerivationChain, DerivationStep};
    use crate::{DirectUrlSourceDist, Dist, Edge, Node, Resolution, ResolvedDist, SourceDist};
    use uv_normalize::{ExtraName, GroupName, PackageName};
    use uv_pep440::Version;
    use version_ranges::Ranges;

    fn package(name: &str, install: bool) -> (Node, Arc<Dist>) {
        let location = DisplaySafeUrl::parse(&format!("https://example.com/{name}.tar.gz"))
            .expect("valid distribution URL");
        let dist = Arc::new(Dist::Source(SourceDist::DirectUrl(DirectUrlSourceDist {
            name: PackageName::from_str(name).expect("valid package name"),
            location: Box::new(location.clone()),
            subdirectory: None,
            ext: SourceDistExtension::TarGz,
            url: VerbatimUrl::from_url(location),
            size: None,
        })));
        (
            Node::Dist {
                dist: ResolvedDist::Installable {
                    dist: dist.clone(),
                    version: Some(Version::new([1, 0, 0])),
                },
                hashes: HashDigests::empty(),
                install,
            },
            dist,
        )
    }

    fn step(name: &str, extra: Option<&str>, group: Option<&str>) -> DerivationStep {
        DerivationStep::new(
            PackageName::from_str(name).expect("valid package name"),
            extra.map(|extra| ExtraName::from_str(extra).expect("valid extra name")),
            group.map(|group| GroupName::from_str(group).expect("valid group name")),
            Some(Version::new([1, 0, 0])),
            Ranges::empty(),
        )
    }

    #[test]
    fn derivation_chain_preserves_annotations_and_filtered_nodes() {
        let mut graph = DiGraph::new();
        let root = graph.add_node(Node::Root);
        let (project, _) = package("project", true);
        let project = graph.add_node(project);
        let (filtered, _) = package("filtered", false);
        let filtered = graph.add_node(filtered);
        let (target, target_dist) = package("target", true);
        let target = graph.add_node(target);

        graph.add_edge(root, project, Edge::Prod);
        graph.add_edge(
            project,
            filtered,
            Edge::Optional(ExtraName::from_str("visual").expect("valid extra name")),
        );
        graph.add_edge(
            filtered,
            target,
            Edge::Dev(GroupName::from_str("test").expect("valid group name")),
        );

        let resolution = Resolution::new(graph);
        let chain = DerivationChain::from_resolution(&resolution, target_dist.as_ref().into())
            .expect("target is reachable");

        assert_eq!(
            chain.into_iter().collect::<Vec<_>>(),
            vec![
                step("project", Some("visual"), None),
                step("filtered", None, Some("test")),
            ]
        );
    }

    #[test]
    fn derivation_chain_preserves_shortest_path_and_incoming_edge_order() {
        let mut graph = DiGraph::new();
        let root = graph.add_node(Node::Root);
        let (first, _) = package("first", true);
        let first = graph.add_node(first);
        let (second, _) = package("second", true);
        let second = graph.add_node(second);
        let (long, _) = package("long", true);
        let long = graph.add_node(long);
        let (target, target_dist) = package("target", true);
        let target = graph.add_node(target);

        graph.add_edge(root, first, Edge::Prod);
        graph.add_edge(root, second, Edge::Prod);
        graph.add_edge(root, long, Edge::Prod);
        graph.add_edge(first, target, Edge::Prod);
        graph.add_edge(
            second,
            target,
            Edge::Optional(ExtraName::from_str("preferred").expect("valid extra name")),
        );
        graph.add_edge(long, first, Edge::Prod);
        graph.add_edge(target, long, Edge::Prod);

        let resolution = Resolution::new(graph);
        let chain = DerivationChain::from_resolution(&resolution, target_dist.as_ref().into())
            .expect("target is reachable");

        assert_eq!(
            chain.into_iter().collect::<Vec<_>>(),
            vec![step("second", Some("preferred"), None)]
        );
    }

    #[test]
    fn derivation_chain_preserves_parallel_edge_annotation() {
        let mut graph = DiGraph::new();
        let root = graph.add_node(Node::Root);
        let (project, _) = package("project", true);
        let project = graph.add_node(project);
        let (target, target_dist) = package("target", true);
        let target = graph.add_node(target);

        graph.add_edge(root, project, Edge::Prod);
        graph.add_edge(project, target, Edge::Prod);
        graph.add_edge(
            project,
            target,
            Edge::Dev(GroupName::from_str("preferred").expect("valid group name")),
        );

        let resolution = Resolution::new(graph);
        let chain = DerivationChain::from_resolution(&resolution, target_dist.as_ref().into())
            .expect("target is reachable");

        assert_eq!(
            chain.into_iter().collect::<Vec<_>>(),
            vec![step("project", None, Some("preferred"))]
        );
    }

    #[test]
    fn derivation_chain_handles_direct_missing_and_cyclic_targets() {
        let mut direct = DiGraph::new();
        let root = direct.add_node(Node::Root);
        let (target, target_dist) = package("target", true);
        let target = direct.add_node(target);
        direct.add_edge(root, target, Edge::Prod);
        let direct = Resolution::new(direct);

        assert!(
            DerivationChain::from_resolution(&direct, target_dist.as_ref().into())
                .expect("target is reachable")
                .is_empty()
        );

        let (_, missing) = package("missing", true);
        assert!(DerivationChain::from_resolution(&direct, missing.as_ref().into()).is_none());

        let mut cyclic = DiGraph::new();
        let (parent, _) = package("parent", true);
        let parent = cyclic.add_node(parent);
        let (target, target_dist) = package("target", true);
        let target = cyclic.add_node(target);
        cyclic.add_edge(parent, target, Edge::Prod);
        cyclic.add_edge(target, parent, Edge::Prod);
        let cyclic = Resolution::new(cyclic);

        assert!(DerivationChain::from_resolution(&cyclic, target_dist.as_ref().into()).is_none());
    }
}
