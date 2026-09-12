use std::collections::BTreeSet;
use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_normalize::PackageName;

#[derive(Clone, Eq, PartialEq)]
struct Dependency {
    package: PackageName,
    marker: usize,
    extras: BTreeSet<usize>,
}

fn merge_by_scan(dependencies: &[Dependency]) -> Vec<Dependency> {
    let mut merged: Vec<Dependency> = Vec::new();
    for dependency in dependencies {
        if let Some(existing) = merged.iter_mut().find(|existing| {
            existing.package == dependency.package && existing.marker == dependency.marker
        }) {
            existing.extras.extend(&dependency.extras);
        } else {
            merged.push(dependency.clone());
        }
    }
    merged
}

fn merge_after_sort(dependencies: &[Dependency]) -> Vec<Dependency> {
    let mut merged = dependencies.to_vec();
    merged.sort_unstable_by(|dependency1, dependency2| {
        dependency1
            .package
            .cmp(&dependency2.package)
            .then_with(|| dependency1.marker.cmp(&dependency2.marker))
    });
    merged.dedup_by(|dependency, previous| {
        if dependency.package == previous.package && dependency.marker == previous.marker {
            previous.extras.append(&mut dependency.extras);
            true
        } else {
            false
        }
    });
    merged
}

fn lock_dependency_merge(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("lock_dependency_merge");
    for edge_count in [8, 64, 512, 1_024, 2_048] {
        for (case, distinct_count) in [("distinct", edge_count), ("duplicates", edge_count / 4)] {
            let dependencies = (0..edge_count)
                .map(|ordinal| {
                    // Outgoing graph edges are not ordered by package identity. All edge counts
                    // are powers of two, so this odd multiplier gives a deterministic shuffle.
                    let index = (ordinal * 2_053 + 1) % edge_count;
                    Dependency {
                        package: PackageName::from_str(&format!(
                            "package-{:05}",
                            index % distinct_count
                        ))
                        .expect("valid package name"),
                        marker: (index % distinct_count) % 8,
                        extras: [index % 16].into(),
                    }
                })
                .collect::<Vec<_>>();
            let mut scanned = merge_by_scan(&dependencies);
            scanned.sort_unstable_by(|dependency1, dependency2| {
                dependency1
                    .package
                    .cmp(&dependency2.package)
                    .then_with(|| dependency1.marker.cmp(&dependency2.marker))
            });
            assert!(scanned == merge_after_sort(&dependencies));

            group.bench_function(
                BenchmarkId::new(format!("{case}/scan"), edge_count),
                |benchmark| {
                    benchmark.iter(|| black_box(merge_by_scan(black_box(&dependencies))));
                },
            );
            group.bench_function(
                BenchmarkId::new(format!("{case}/sort"), edge_count),
                |benchmark| {
                    benchmark.iter(|| black_box(merge_after_sort(black_box(&dependencies))));
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, lock_dependency_merge);
criterion_main!(benches);
