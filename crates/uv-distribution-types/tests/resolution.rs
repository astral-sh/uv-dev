use std::cell::RefCell;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

use petgraph::graph::{DiGraph, NodeIndex};

use uv_distribution_types::{
    Edge, InstalledDistKind, InstalledRegistryDist, Name, Node, Resolution, ResolutionDiagnostic,
    ResolvedDist,
};
use uv_pypi_types::{HashDigest, HashDigests};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn installed(name: &str, version: &str) -> TestResult<ResolvedDist> {
    Ok(ResolvedDist::Installed {
        dist: Arc::new(
            InstalledDistKind::Registry(InstalledRegistryDist {
                name: name.parse()?,
                version: version.parse()?,
                path: PathBuf::from("fixture")
                    .join(format!("{name}-{version}.dist-info"))
                    .into_boxed_path(),
                cache_info: None,
                build_info: None,
            })
            .into(),
        ),
    })
}

fn hashes(byte: u8) -> TestResult<HashDigests> {
    Ok(format!("sha256:{}", format!("{byte:02x}").repeat(32))
        .parse::<HashDigest>()?
        .into())
}

fn fixture() -> TestResult<(Resolution, [NodeIndex; 3])> {
    let mut graph = DiGraph::new();
    let root = graph.add_node(Node::Root);
    let alpha = graph.add_node(Node::Dist {
        dist: installed("alpha", "1.0")?,
        hashes: hashes(0xaa)?,
        install: true,
    });
    let beta = graph.add_node(Node::Dist {
        dist: installed("beta", "1.0")?,
        hashes: hashes(0xbb)?,
        install: true,
    });
    let gamma = graph.add_node(Node::Dist {
        dist: installed("gamma", "1.0")?,
        hashes: hashes(0xcc)?,
        install: false,
    });
    graph.add_edge(root, alpha, Edge::Prod);
    graph.add_edge(alpha, beta, Edge::Optional("feature".parse()?));
    graph.add_edge(beta, gamma, Edge::Dev("dev".parse()?));

    let resolution = Resolution::new(graph)
        .with_diagnostics(vec![ResolutionDiagnostic::MissingLowerBound {
            package_name: "alpha".parse()?,
        }])
        .with_diagnostics(vec![ResolutionDiagnostic::MissingGroup {
            dist: installed("gamma", "1.0")?,
            group: "missing".parse()?,
        }]);
    Ok((resolution, [alpha, beta, gamma]))
}

fn same_distribution(actual: &ResolvedDist, expected: &ResolvedDist) -> bool {
    match (actual, expected) {
        (ResolvedDist::Installed { dist: actual }, ResolvedDist::Installed { dist: expected }) => {
            Arc::ptr_eq(actual, expected)
        }
        (
            ResolvedDist::Installable {
                dist: actual,
                version: actual_version,
            },
            ResolvedDist::Installable {
                dist: expected,
                version: expected_version,
            },
        ) => Arc::ptr_eq(actual, expected) && actual_version == expected_version,
        (ResolvedDist::Installed { .. }, ResolvedDist::Installable { .. })
        | (ResolvedDist::Installable { .. }, ResolvedDist::Installed { .. }) => false,
    }
}

fn edge_kind(edge: &Edge) -> (&str, Option<&str>) {
    match edge {
        Edge::Prod => ("production", None),
        Edge::Optional(extra) => ("optional", Some(extra.as_ref())),
        Edge::Dev(group) => ("group", Some(group.as_ref())),
    }
}

fn assert_graph(actual: &DiGraph<Node, Edge>, expected: &DiGraph<Node, Edge>) -> TestResult {
    assert_eq!(actual.node_count(), expected.node_count());
    assert_eq!(actual.edge_count(), expected.edge_count());
    for index in expected.node_indices() {
        let actual = actual.node_weight(index).ok_or("missing node")?;
        match (actual, &expected[index]) {
            (Node::Root, Node::Root) => {}
            (
                Node::Dist {
                    dist: actual,
                    hashes: actual_hashes,
                    install: actual_install,
                },
                Node::Dist {
                    dist: expected,
                    hashes: expected_hashes,
                    install: expected_install,
                },
            ) => {
                assert!(same_distribution(actual, expected), "node {index:?}");
                assert_eq!(actual_hashes, expected_hashes, "node {index:?}");
                assert_eq!(actual_install, expected_install, "node {index:?}");
            }
            (Node::Root, Node::Dist { .. }) | (Node::Dist { .. }, Node::Root) => {
                return Err(format!("node {index:?} changed kind").into());
            }
        }
    }
    for index in expected.edge_indices() {
        assert_eq!(actual.edge_endpoints(index), expected.edge_endpoints(index));
        let actual = actual.edge_weight(index).ok_or("missing edge")?;
        assert_eq!(edge_kind(actual), edge_kind(&expected[index]));
    }
    Ok(())
}

#[test]
fn empty_and_root_only_resolutions() {
    let mut graph = DiGraph::new();
    graph.add_node(Node::Root);
    for resolution in [Resolution::default(), Resolution::new(graph)] {
        assert!(resolution.is_empty());
        assert_eq!(resolution.len(), 0);
        assert_eq!(resolution.distributions().count(), 0);
        assert_eq!(resolution.hashes().count(), 0);
    }
}

#[test]
fn filtering_retains_the_dependency_graph() -> TestResult {
    let (resolution, [alpha, _, _]) = fixture()?;
    let mut expected = resolution.graph().clone();
    let diagnostics = format!("{:?}", resolution.diagnostics());
    let resolution = resolution.filter(|dist| dist.name().as_str() != "alpha");
    if let Node::Dist { install, .. } = &mut expected[alpha] {
        *install = false;
    }
    assert_graph(resolution.graph(), &expected)?;
    assert_eq!(format!("{:?}", resolution.diagnostics()), diagnostics);
    assert_eq!(
        resolution
            .distributions()
            .map(|dist| dist.name().as_str())
            .collect::<Vec<_>>(),
        ["beta"]
    );
    assert_eq!(
        resolution
            .hashes()
            .map(|(dist, hashes)| (dist.name().as_str(), hashes))
            .collect::<Vec<_>>(),
        [("beta", hashes(0xbb)?.as_slice())]
    );
    assert_eq!(resolution.len(), 1);
    assert!(!resolution.is_empty());

    // An additional filter cannot re-enable either a previously excluded package or one
    // that was already non-installable in the original graph.
    let resolution = resolution.filter(|_| true);
    assert_graph(resolution.graph(), &expected)?;
    let resolution = resolution.filter(|_| false);
    for node in expected.node_weights_mut() {
        if let Node::Dist { install, .. } = node {
            *install = false;
        }
    }
    assert_graph(resolution.graph(), &expected)?;
    assert_eq!(format!("{:?}", resolution.diagnostics()), diagnostics);
    assert!(resolution.is_empty());
    assert_eq!(resolution.len(), 0);
    assert_eq!(resolution.distributions().count(), 0);
    assert_eq!(resolution.hashes().count(), 0);
    Ok(())
}

#[test]
fn mapping_preserves_installation_state() -> TestResult {
    let (resolution, [alpha, beta, _]) = fixture()?;
    let resolution = resolution.filter(|dist| dist.name().as_str() != "alpha");
    let mut expected = resolution.graph().clone();
    let diagnostics = format!("{:?}", resolution.diagnostics());
    let replacements = [installed("alpha", "2.0")?, installed("beta", "2.0")?];
    for (index, replacement) in [alpha, beta].into_iter().zip(&replacements) {
        if let Node::Dist { dist, .. } = &mut expected[index] {
            *dist = replacement.clone();
        }
    }
    let visited = RefCell::new(Vec::new());
    let resolution = resolution.map(|dist| {
        visited.borrow_mut().push(dist.name().to_string());
        replacements
            .iter()
            .find(|replacement| replacement.name() == dist.name())
            .cloned()
    });
    let mut visited = visited.into_inner();
    visited.sort();
    assert_eq!(visited, ["alpha", "beta", "gamma"]);
    assert_graph(resolution.graph(), &expected)?;
    assert_eq!(format!("{:?}", resolution.diagnostics()), diagnostics);
    assert_eq!(
        resolution
            .distributions()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["beta==2.0"]
    );
    assert_eq!(
        resolution
            .hashes()
            .map(|(dist, hashes)| (dist.to_string(), hashes))
            .collect::<Vec<_>>(),
        [("beta==2.0".to_string(), hashes(0xbb)?.as_slice())]
    );
    assert_eq!(resolution.len(), 1);
    assert!(!resolution.is_empty());
    Ok(())
}
