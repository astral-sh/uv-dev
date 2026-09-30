//! Parses ordinary markers and long Boolean chains from conflict-heavy lockfiles.

extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use uv_normalize::{ExtraName, PackageName};
use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictSet, Conflicts};
use uv_resolver_types::{ConflictMarker, UniversalMarker};

fn conflict_marker(count: usize) -> (Vec<String>, MarkerTree) {
    let package = PackageName::from_str("repro").expect("valid benchmark package");
    let items = (0..count)
        .map(|index| {
            let extra =
                ExtraName::from_str(&format!("e{index:03}")).expect("valid benchmark extra");
            ConflictItem::from((package.clone(), extra))
        })
        .collect::<Vec<_>>();
    let mut conflicts = Conflicts::empty();
    conflicts.push(ConflictSet::try_from(items).expect("valid benchmark conflicts"));
    let world = UniversalMarker::new(MarkerTree::TRUE, ConflictMarker::from_conflicts(&conflicts))
        .combined();
    let environment =
        MarkerTree::from_str("python_full_version >= '3.13'").expect("valid benchmark marker");
    let mut clauses = vec!["python_full_version >= '3.13'".to_string()];
    for left in 0..count {
        for right in left + 1..count {
            clauses.push(format!(
                "(extra == 'extra-5-repro-e{left:03}' and extra == 'extra-5-repro-e{right:03}')"
            ));
        }
    }
    (clauses, world.implies(environment))
}

fn marker_parse(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("marker_parse");
    for (name, source) in [
        ("python-version", "python_version >= '3.10'"),
        (
            "platform",
            "sys_platform == 'win32' or sys_platform == 'darwin'",
        ),
        (
            "nested-environment",
            "python_version < '3.12' and (sys_platform == 'win32' or platform_python_implementation != 'CPython')",
        ),
        ("extra", "extra == 'test' and python_version >= '3.10'"),
        (
            "quoted-operators",
            "os_name not in 'a or b' and sys_platform != 'linux and darwin)'",
        ),
    ] {
        MarkerTree::from_str(source).expect("valid benchmark marker");
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("ordinary", name),
            source,
            |bench, source| {
                bench.iter(|| {
                    MarkerTree::from_str(black_box(source)).expect("valid benchmark marker")
                });
            },
        );
    }
    for count in [4, 16, 32, 64] {
        let (mut clauses, expected) = conflict_marker(count);
        for order in ["forward", "reverse"] {
            if order == "reverse" {
                clauses.reverse();
            }
            let source = clauses.join(" or ");
            assert_eq!(
                MarkerTree::from_str(&source).expect("valid benchmark marker"),
                expected,
                "{count} extras in {order} order"
            );
            group.throughput(Throughput::Bytes(source.len() as u64));
            group.bench_with_input(BenchmarkId::new(order, count), &source, |bench, source| {
                bench.iter(|| {
                    MarkerTree::from_str(black_box(source)).expect("valid benchmark marker")
                });
            });
        }
    }
    for count in [4, 32, 256] {
        for (name, distinct) in [("atoms_distinct", count), ("atoms_repeated", 4)] {
            let terms = (0..count)
                .map(|index| format!("extra == 'item-{:03}'", index % distinct))
                .collect::<Vec<_>>();
            for (operator, initial, combine) in [
                ("or", MarkerTree::FALSE, MarkerTree::or as fn(_, _) -> _),
                ("and", MarkerTree::TRUE, MarkerTree::and as fn(_, _) -> _),
            ] {
                let expected = terms.iter().fold(initial, |marker, term| {
                    combine(
                        marker,
                        MarkerTree::from_str(term).expect("valid benchmark marker"),
                    )
                });
                let source = terms.join(&format!(" {operator} "));
                assert_eq!(
                    MarkerTree::from_str(&source).expect("valid benchmark marker"),
                    expected,
                );
                group.throughput(Throughput::Bytes(source.len() as u64));
                group.bench_with_input(
                    BenchmarkId::new(format!("{name}_{operator}"), count),
                    &source,
                    |bench, source| {
                        bench.iter(|| {
                            MarkerTree::from_str(black_box(source)).expect("valid benchmark marker")
                        });
                    },
                );
            }
        }
    }
    group.finish();
}

criterion_group!(benches, marker_parse);
criterion_main!(benches);
