extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::str::FromStr;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use uv_distribution_types::{DependencyMetadata, StaticMetadata};
use uv_normalize::PackageName;
use uv_pep440::Version;

fn exact_entries(package: &PackageName, count: u64) -> (Vec<StaticMetadata>, Vec<Version>) {
    let versions: Vec<_> = (0..count).map(|version| Version::new([version])).collect();
    let metadata = versions
        .iter()
        .cloned()
        .map(|version| StaticMetadata {
            name: package.clone(),
            version: Some(version),
            requires_dist: Box::default(),
            requires_python: None,
            provides_extra: Box::default(),
        })
        .collect();
    (metadata, versions)
}

fn global_entries(package: &PackageName, count: u64) -> (Vec<StaticMetadata>, Vec<Version>) {
    let versions: Vec<_> = (0..count)
        .map(|version| Version::new([count + version]))
        .collect();
    let metadata = (0..count)
        .map(|version| StaticMetadata {
            name: package.clone(),
            version: Some(Version::new([version])),
            requires_dist: Box::default(),
            requires_python: None,
            provides_extra: Box::default(),
        })
        .chain(std::iter::once(StaticMetadata {
            name: package.clone(),
            version: None,
            requires_dist: Box::default(),
            requires_python: None,
            provides_extra: Box::default(),
        }))
        .collect();
    (metadata, versions)
}

fn dependency_metadata(c: &mut Criterion) {
    let package = PackageName::from_str("dependency-metadata-package").unwrap();
    let mut group = c.benchmark_group("dependency_metadata");

    for count in [1, 2, 4, 8, 16, 32, 64, 256, 1_024, 4_096, 16_384] {
        group.throughput(Throughput::Elements(count));

        let (entries, versions) = exact_entries(&package, count);
        group.bench_with_input(
            BenchmarkId::new("construction", count),
            &entries,
            |b, entries| {
                b.iter_batched(
                    || entries.clone(),
                    |entries| black_box(DependencyMetadata::from_entries(entries)),
                    BatchSize::SmallInput,
                );
            },
        );

        let metadata = DependencyMetadata::from_entries(entries);
        group.bench_with_input(
            BenchmarkId::new("exact", count),
            &versions,
            |b, versions| {
                b.iter(|| {
                    for version in versions {
                        black_box(metadata.get(&package, Some(version)));
                    }
                });
            },
        );

        let (entries, versions) = global_entries(&package, count);
        let metadata = DependencyMetadata::from_entries(entries);
        group.bench_with_input(
            BenchmarkId::new("global", count),
            &versions,
            |b, versions| {
                b.iter(|| {
                    for version in versions {
                        black_box(metadata.get(&package, Some(version)));
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, dependency_metadata);
criterion_main!(benches);
