// Don't optimize the alloc crate away due to it being otherwise unused.
// https://github.com/rust-lang/rust/issues/64402
extern crate uv_performance_memory_allocator;

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime};
use uv_pep508::{MarkerTree, MarkerTreeContents};

fn format_markers(criterion: &mut Criterion<WallTime>) {
    // Retain repeated markers from the lockfile corpus so their frequency remains part of the workload.
    let markers: Vec<MarkerTreeContents> =
        serde_json::from_str::<Vec<String>>(include_str!("../fixtures/markers.json"))
            .expect("valid benchmark marker corpus")
            .into_iter()
            .filter_map(|marker| {
                marker
                    .parse::<MarkerTree>()
                    .expect("valid benchmark marker")
                    .contents()
            })
            .collect();

    let mut group = criterion.benchmark_group("marker_format");
    group.throughput(Throughput::Elements(markers.len() as u64));
    group.bench_function("lockfiles", |benchmark| {
        benchmark.iter(|| {
            for marker in &markers {
                black_box(black_box(marker).to_string());
            }
        });
    });

    let complex = "(sys_platform == 'win32' and python_version < '3.11') or (sys_platform == 'linux' and python_version >= '3.9') or (sys_platform == 'darwin' and platform_machine == 'arm64')"
        .parse::<MarkerTree>()
        .unwrap()
        .contents()
        .unwrap();
    group.throughput(Throughput::Elements(1));
    group.bench_function("complex", |benchmark| {
        benchmark.iter(|| black_box(black_box(&complex).to_string()));
    });
    group.finish();

    let mut group = criterion.benchmark_group("marker_dnf");
    group.throughput(Throughput::Elements(markers.len() as u64));
    group.bench_function("lockfiles", |benchmark| {
        benchmark.iter(|| {
            for marker in &markers {
                black_box(black_box(marker.as_ref()).to_dnf());
            }
        });
    });
    group.finish();
}

criterion_group!(uv_pep508, format_markers);
criterion_main!(uv_pep508);
