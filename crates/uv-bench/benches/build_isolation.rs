use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use rustc_hash::FxHashSet;
use uv_normalize::PackageName;

fn package_names(count: usize) -> Vec<PackageName> {
    (0..count)
        .map(|index| {
            PackageName::from_str(&format!("package-{index:05}")).expect("valid package name")
        })
        .collect()
}

fn build_isolation_membership(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("build_isolation_membership");
    for (package_count, remote_count) in [
        (0, 0),
        (0, 4_000),
        (1, 1),
        (1, 4_000),
        (2, 2),
        (2, 4_000),
        (4, 4),
        (4, 4_000),
        (8, 8),
        (8, 4_000),
        (16, 16),
        (16, 4_000),
        (32, 32),
        (32, 4_000),
        (33, 33),
        (64, 64),
        (100, 100),
        (1_000, 1_000),
        (4_000, 0),
        (4_000, 1),
        (4_000, 2),
        (4_000, 4),
        (4_000, 8),
        (4_000, 16),
        (4_000, 32),
        (4_000, 4_000),
    ] {
        let packages = package_names(package_count);
        let remote = package_names(remote_count);
        let input = format!("{package_count}x{remote_count}");

        group.bench_function(BenchmarkId::new("linear", &input), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    remote
                        .iter()
                        .filter(|package| !packages.contains(black_box(package)))
                        .count(),
                );
            });
        });
        group.bench_function(BenchmarkId::new("indexed", &input), |benchmark| {
            benchmark.iter(|| {
                if packages.len() <= 8 || remote.len() <= 32 {
                    return black_box(
                        remote
                            .iter()
                            .filter(|package| !packages.contains(black_box(package)))
                            .count(),
                    );
                }

                let packages = packages.iter().collect::<FxHashSet<_>>();
                black_box(
                    remote
                        .iter()
                        .filter(|package| !packages.contains(black_box(package)))
                        .count(),
                )
            });
        });
    }

    let duplicate = PackageName::from_str("package-00000").expect("valid package name");
    for (input, packages, remote) in [
        (
            "4000x32_late",
            package_names(4_000),
            package_names(4_000).split_off(3_968),
        ),
        (
            "4000x32_miss",
            package_names(4_000),
            package_names(4_032).split_off(4_000),
        ),
        (
            "9x4000_duplicates",
            vec![duplicate; 9],
            package_names(4_000),
        ),
    ] {
        group.bench_function(BenchmarkId::new("linear", input), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    remote
                        .iter()
                        .filter(|package| !packages.contains(black_box(package)))
                        .count(),
                );
            });
        });
        group.bench_function(BenchmarkId::new("indexed", input), |benchmark| {
            benchmark.iter(|| {
                if packages.len() <= 8 || remote.len() <= 32 {
                    return black_box(
                        remote
                            .iter()
                            .filter(|package| !packages.contains(black_box(package)))
                            .count(),
                    );
                }

                let packages = packages.iter().collect::<FxHashSet<_>>();
                black_box(
                    remote
                        .iter()
                        .filter(|package| !packages.contains(black_box(package)))
                        .count(),
                )
            });
        });
    }
    group.finish();
}

criterion_group!(build_isolation, build_isolation_membership);
criterion_main!(build_isolation);
