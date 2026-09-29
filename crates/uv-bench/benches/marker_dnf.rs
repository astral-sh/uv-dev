// Don't optimize the alloc crate away due to it being otherwise unused.
// https://github.com/rust-lang/rust/issues/64402
extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_pep508::{MarkerTree, MarkerTreeContents};

/// An environment marker under pairwise exclusive extras, as in `uv lock` conflict resolution.
fn conflicting_extras(count: usize) -> MarkerTreeContents {
    let extras: Vec<_> = (0..count)
        .map(|index| {
            MarkerTree::from_str(&format!("extra == 'extra-5-repro-e{index:03}'"))
                .expect("benchmark marker should be valid")
        })
        .collect();
    let mut conflicts = MarkerTree::TRUE;
    for (index, &left) in extras.iter().enumerate() {
        for &right in &extras[index + 1..] {
            conflicts = conflicts.and(left.and(right).negate());
        }
    }
    let environment = MarkerTree::from_str("python_full_version >= '3.12'")
        .expect("benchmark marker should be valid");
    conflicts
        .implies(environment)
        .contents()
        .expect("benchmark marker should not be constant")
}

fn marker_dnf(criterion: &mut Criterion<WallTime>) {
    let mut group = criterion.benchmark_group("marker_dnf_conflicts");
    for count in [4, 16, 32] {
        let marker = conflicting_extras(count);
        group.bench_with_input(
            BenchmarkId::new("format", count),
            &marker,
            |benchmark, marker| {
                benchmark.iter(|| black_box(black_box(marker).to_string()));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("owned", count),
            &marker,
            |benchmark, marker| {
                benchmark.iter(|| black_box(black_box(marker.as_ref()).to_dnf()));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, marker_dnf);
criterion_main!(benches);
