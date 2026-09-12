use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_cache::{Cache, Refresh};
use uv_normalize::PackageName;

fn package_names(count: usize) -> Vec<PackageName> {
    (0..count)
        .map(|index| {
            PackageName::from_str(&format!("package-{index:05}")).expect("valid package name")
        })
        .collect()
}

fn missing_package_names(count: usize) -> Vec<PackageName> {
    (0..count)
        .map(|index| {
            PackageName::from_str(&format!("missing-{index:05}")).expect("valid package name")
        })
        .collect()
}

fn cache_refresh_packages(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("cache_refresh_packages");
    for package_count in [1, 100, 1_000, 4_000] {
        let packages = package_names(package_count);
        let missing = missing_package_names(package_count);
        let cache = Cache::from_path("unused-cache")
            .with_refresh(Refresh::from_args(None, packages.clone()));

        group.bench_function(BenchmarkId::new("hit", package_count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    packages
                        .iter()
                        .filter(|package| cache.must_revalidate_package(black_box(package)))
                        .count(),
                );
            });
        });
        group.bench_function(BenchmarkId::new("miss", package_count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    missing
                        .iter()
                        .filter(|package| cache.must_revalidate_package(black_box(package)))
                        .count(),
                );
            });
        });
    }
    group.finish();
}

criterion_group!(cache_refresh, cache_refresh_packages);
criterion_main!(cache_refresh);
