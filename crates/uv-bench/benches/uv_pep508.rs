// Don't optimize the alloc crate away due to it being otherwise unused.
// https://github.com/rust-lang/rust/issues/64402
extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::ops::Bound;

use criterion::{Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime};
use uv_pep440::Version;
use uv_pep508::{MarkerTree, MarkerTreeContents};

fn collect_markers(value: &toml::Value, markers: &mut Vec<MarkerTreeContents>) {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                match (key.as_str(), value) {
                    ("marker", toml::Value::String(marker)) => {
                        markers.extend(marker.parse::<MarkerTree>().unwrap().contents());
                    }
                    ("resolution-markers", toml::Value::Array(values)) => {
                        for value in values {
                            markers.extend(
                                value
                                    .as_str()
                                    .unwrap()
                                    .parse::<MarkerTree>()
                                    .unwrap()
                                    .contents(),
                            );
                        }
                    }
                    _ => collect_markers(value, markers),
                }
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                collect_markers(value, markers);
            }
        }
        _ => {}
    }
}

fn format_markers(criterion: &mut Criterion<WallTime>) {
    let mut markers = Vec::new();
    for lockfile in [
        include_str!("../../../uv.lock"),
        include_str!("../../../scripts/benchmark/uv.lock"),
    ] {
        collect_markers(
            &toml::from_str::<toml::Value>(lockfile).unwrap(),
            &mut markers,
        );
    }

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

    let lower = Version::new([3, 9]);
    let upper = Version::new([3, 14]);
    let mut group = criterion.benchmark_group("marker_simplify_python_versions");
    group.throughput(Throughput::Elements(markers.len() as u64));
    group.bench_function("lockfiles", |benchmark| {
        benchmark.iter(|| {
            for marker in &markers {
                black_box(
                    marker
                        .as_ref()
                        .simplify_python_versions(Bound::Included(&lower), Bound::Excluded(&upper)),
                );
            }
        });
    });
    group.finish();
}

criterion_group!(uv_pep508, format_markers);
criterion_main!(uv_pep508);
