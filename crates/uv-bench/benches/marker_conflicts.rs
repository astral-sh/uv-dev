// Don't optimize the alloc crate away due to it being otherwise unused.
// https://github.com/rust-lang/rust/issues/64402
extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use rustc_hash::FxHashMap;

use uv_normalize::{ExtraName, PackageName};
use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictSet, Conflicts};
use uv_resolver_types::universal_marker::resolve_activated_extras;
use uv_resolver_types::{ConflictMarker, UniversalMarker};

fn conflicting_extras(count: usize) -> (ConflictMarker, Vec<ConflictItem>) {
    let package = PackageName::from_str("repro").expect("valid benchmark package");
    let items: Vec<_> = (0..count)
        .map(|index| {
            let extra =
                ExtraName::from_str(&format!("e{index:03}")).expect("valid benchmark extra");
            ConflictItem::from((package.clone(), extra))
        })
        .collect();
    let mut conflicts = Conflicts::empty();
    conflicts.push(ConflictSet::try_from(items.clone()).expect("valid benchmark conflicts"));
    (ConflictMarker::from_conflicts(&conflicts), items)
}

fn marker_conflicts(criterion: &mut Criterion<WallTime>) {
    let environment =
        MarkerTree::from_str("python_full_version >= '3.12'").expect("valid benchmark marker");
    let darwin = MarkerTree::from_str("sys_platform == 'darwin'").expect("valid benchmark marker");
    let mut group = criterion.benchmark_group("marker_conflicts");
    for count in [4, 16, 32] {
        let (conflicts, items) = conflicting_extras(count);
        let marker = UniversalMarker::new(environment, conflicts);
        group.bench_with_input(
            BenchmarkId::new("imbibe_and_format", count),
            &(marker, conflicts),
            |benchmark, &(marker, conflicts)| {
                benchmark.iter(|| {
                    let mut marker = black_box(marker);
                    marker.imbibe(black_box(conflicts));
                    black_box(marker.combined().try_to_string())
                });
            },
        );

        // Existing lockfiles can contain the implication of the conflict world. Construct it
        // directly so changes to `imbibe` do not change the input to this benchmark.
        let world = UniversalMarker::new(MarkerTree::TRUE, conflicts).combined();
        let expanded = world.implies(environment);
        let known_conflicts = FxHashMap::from_iter([
            (items[0].clone(), darwin),
            (items[1].clone(), darwin.negate()),
        ]);
        group.bench_with_input(
            BenchmarkId::new("resolve_activated_extras", count),
            &(expanded, known_conflicts),
            |benchmark, (marker, known_conflicts)| {
                benchmark.iter(|| {
                    black_box(resolve_activated_extras(
                        black_box(*marker),
                        None,
                        black_box(known_conflicts),
                    ))
                });
            },
        );
    }
    let package = PackageName::from_str("repro").expect("valid benchmark package");
    for pairs in [4, 12, 20] {
        let mut marker = MarkerTree::TRUE;
        let mut known_conflicts = FxHashMap::default();
        for index in 0..pairs {
            let left =
                ExtraName::from_str(&format!("pair-{index:03}-a")).expect("valid benchmark extra");
            let right =
                ExtraName::from_str(&format!("pair-{index:03}-b")).expect("valid benchmark extra");
            marker = marker.and(
                MarkerTree::from_str(&format!("extra == '{left}' or extra == '{right}'"))
                    .expect("valid benchmark marker"),
            );
            known_conflicts.insert(ConflictItem::from((package.clone(), left)), darwin);
            known_conflicts.insert(
                ConflictItem::from((package.clone(), right)),
                darwin.negate(),
            );
        }
        group.bench_with_input(
            BenchmarkId::new("resolve_shared_extra_pairs", pairs),
            &(marker, known_conflicts),
            |benchmark, (marker, known_conflicts)| {
                benchmark.iter(|| {
                    black_box(resolve_activated_extras(
                        black_box(*marker),
                        Some(black_box(&package)),
                        black_box(known_conflicts),
                    ))
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, marker_conflicts);
criterion_main!(benches);
