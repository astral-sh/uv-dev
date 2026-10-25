// Don't optimize the alloc crate away due to it being otherwise unused.
// https://github.com/rust-lang/rust/issues/64402
extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use petgraph::graph::DiGraph;
use uv_distribution_filename::SourceDistExtension;
use uv_distribution_types::{
    DerivationChain, Dist, Edge, Node, PathSourceDist, Resolution, ResolvedDist, SourceDist,
};
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::Version;
use uv_pep508::VerbatimUrl;
use uv_pypi_types::HashDigests;

fn package(name: &str) -> (Node, Arc<Dist>) {
    let path = format!("/benchmark/{name}-1.0.0.tar.gz");
    let dist = Arc::new(Dist::Source(SourceDist::Path(PathSourceDist {
        name: PackageName::from_str(name).expect("valid benchmark package name"),
        version: Some(Version::new([1, 0, 0])),
        install_path: PathBuf::from(&path).into_boxed_path(),
        ext: SourceDistExtension::TarGz,
        url: VerbatimUrl::parse_url(format!("file://{path}"))
            .expect("valid benchmark distribution URL"),
    })));
    (
        Node::Dist {
            dist: ResolvedDist::Installable {
                dist: dist.clone(),
                version: Some(Version::new([1, 0, 0])),
            },
            hashes: HashDigests::empty(),
            install: true,
        },
        dist,
    )
}

fn edge(index: usize) -> Edge {
    match index % 3 {
        0 => Edge::Prod,
        1 => Edge::Optional(ExtraName::from_str("optional").expect("valid benchmark extra")),
        _ => Edge::Dev(GroupName::from_str("dev").expect("valid benchmark group")),
    }
}

fn linear_resolution(depth: usize) -> (Resolution, Arc<Dist>) {
    let mut graph = DiGraph::new();
    let mut parent = graph.add_node(Node::Root);
    let mut target = None;
    for index in 0..depth {
        let (node, dist) = package(&format!("linear-{index:04}"));
        let node = graph.add_node(node);
        graph.add_edge(parent, node, edge(index));
        parent = node;
        target = Some(dist);
    }
    (
        Resolution::new(graph),
        target.expect("benchmark resolution is non-empty"),
    )
}

fn layered_resolution(depth: usize, width: usize) -> (Resolution, Arc<Dist>) {
    let mut graph = DiGraph::new();
    let root = graph.add_node(Node::Root);
    let mut previous = vec![root];
    let mut target = None;
    for layer in 0..depth {
        let mut current = Vec::with_capacity(width);
        for index in 0..width {
            let (node, dist) = package(&format!("layered-{layer:04}-{index:02}"));
            let node = graph.add_node(node);
            for (parent_index, parent) in previous.iter().enumerate() {
                graph.add_edge(*parent, node, edge(layer + index + parent_index));
            }
            current.push(node);
            target = Some(dist);
        }
        previous = current;
    }
    (
        Resolution::new(graph),
        target.expect("benchmark resolution is non-empty"),
    )
}

fn shallow_resolution(packages: usize) -> (Resolution, Arc<Dist>) {
    let mut graph = DiGraph::new();
    let root = graph.add_node(Node::Root);
    for index in 0..packages {
        let (node, _) = package(&format!("unrelated-{index:04}"));
        graph.add_node(node);
    }
    let (target, target_dist) = package("target");
    let target = graph.add_node(target);
    graph.add_edge(root, target, Edge::Prod);
    (Resolution::new(graph), target_dist)
}

fn derivation_chain_linear(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("derivation_chain_linear");
    for depth in [64, 256, 1024] {
        let (resolution, target) = linear_resolution(depth);
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, _| {
            b.iter(|| {
                DerivationChain::from_resolution(black_box(&resolution), target.as_ref().into())
            });
        });
    }
    group.finish();
}

fn derivation_chain_layered(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("derivation_chain_layered");
    for depth in [32, 64, 128] {
        let (resolution, target) = layered_resolution(depth, 8);
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, _| {
            b.iter(|| {
                DerivationChain::from_resolution(black_box(&resolution), target.as_ref().into())
            });
        });
    }
    group.finish();
}

fn derivation_chain_shallow(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("derivation_chain_shallow");
    for packages in [64, 256, 1024] {
        let (resolution, target) = shallow_resolution(packages);
        group.bench_with_input(BenchmarkId::from_parameter(packages), &packages, |b, _| {
            b.iter(|| {
                DerivationChain::from_resolution(black_box(&resolution), target.as_ref().into())
            });
        });
    }
    group.finish();
}

criterion_group!(
    derivation_chain,
    derivation_chain_linear,
    derivation_chain_layered,
    derivation_chain_shallow
);
criterion_main!(derivation_chain);
