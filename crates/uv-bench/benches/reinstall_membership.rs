use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use rustc_hash::FxHashSet;
use uv_configuration::Reinstall;
use uv_normalize::PackageName;

fn package_names(prefix: &str, count: usize) -> Vec<PackageName> {
    (0..count)
        .map(|index| {
            PackageName::from_str(&format!("{prefix}-{index:05}")).expect("valid package name")
        })
        .collect()
}

fn reinstall_membership(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("reinstall_membership");
    for package_count in [1, 2, 8, 64, 256, 1_024, 4_096] {
        let packages = package_names("package", package_count);
        let missing = package_names("missing", package_count);
        let reinstall = Reinstall::Packages(packages.clone(), Vec::new());
        let indexed: FxHashSet<_> = packages.iter().cloned().collect();

        group.bench_function(
            BenchmarkId::new("resolver-hit", package_count),
            |benchmark| {
                benchmark.iter(|| {
                    black_box(
                        packages
                            .iter()
                            .filter(|package| indexed.contains(black_box(*package)))
                            .count(),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new("resolver-miss", package_count),
            |benchmark| {
                benchmark.iter(|| {
                    black_box(
                        missing
                            .iter()
                            .filter(|package| indexed.contains(black_box(*package)))
                            .count(),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new("planner-hit", package_count),
            |benchmark| {
                benchmark.iter(|| {
                    let indexed = packages.iter().collect::<FxHashSet<&PackageName>>();
                    black_box(
                        packages
                            .iter()
                            .filter(|package| indexed.contains(black_box(*package)))
                            .count(),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new("planner-miss", package_count),
            |benchmark| {
                benchmark.iter(|| {
                    let indexed = packages.iter().collect::<FxHashSet<&PackageName>>();
                    black_box(
                        missing
                            .iter()
                            .filter(|package| indexed.contains(black_box(*package)))
                            .count(),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new("build-borrowed", package_count),
            |benchmark| {
                benchmark.iter(|| {
                    black_box(packages.iter().collect::<FxHashSet<&PackageName>>());
                });
            },
        );
        group.bench_function(
            BenchmarkId::new("build-owned", package_count),
            |benchmark| {
                benchmark.iter(|| {
                    black_box(packages.iter().cloned().collect::<FxHashSet<PackageName>>());
                });
            },
        );

        // Keep the quadratic control at a size suitable for instrumented CodSpeed runs.
        if package_count <= 1_024 {
            group.bench_function(BenchmarkId::new("linear-hit", package_count), |benchmark| {
                benchmark.iter(|| {
                    black_box(
                        packages
                            .iter()
                            .filter(|package| reinstall.contains_package(black_box(package)))
                            .count(),
                    );
                });
            });
            group.bench_function(
                BenchmarkId::new("linear-miss", package_count),
                |benchmark| {
                    benchmark.iter(|| {
                        black_box(
                            missing
                                .iter()
                                .filter(|package| reinstall.contains_package(black_box(package)))
                                .count(),
                        );
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, reinstall_membership);
criterion_main!(benches);
