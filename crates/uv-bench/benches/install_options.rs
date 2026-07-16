use std::collections::BTreeSet;
use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_configuration::{InstallOptions, InstallTarget};
use uv_normalize::PackageName;

fn package_names(count: usize) -> Vec<PackageName> {
    (0..count)
        .map(|index| {
            PackageName::from_str(&format!("package-{index:05}")).expect("valid package name")
        })
        .collect()
}

fn install_package_filters(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("install_package_filters");
    for package_count in [100, 1_000, 4_000] {
        let packages = package_names(package_count);
        let members = BTreeSet::default();
        let no_install = InstallOptions::new(
            false,
            false,
            false,
            false,
            false,
            false,
            packages.clone(),
            Vec::new(),
        );
        let only_install = InstallOptions::new(
            false,
            false,
            false,
            false,
            false,
            false,
            Vec::new(),
            packages.clone(),
        );

        group.bench_function(BenchmarkId::new("linear", package_count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    packages
                        .iter()
                        .filter(|package| packages.contains(black_box(package)))
                        .count(),
                );
            });
        });
        group.bench_function(
            BenchmarkId::new("no_install_package", package_count),
            |benchmark| {
                benchmark.iter(|| {
                    black_box(
                        packages
                            .iter()
                            .filter(|package| {
                                no_install.include_package(
                                    InstallTarget {
                                        name: black_box(package),
                                        is_local: false,
                                    },
                                    None,
                                    &members,
                                )
                            })
                            .count(),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new("only_install_package", package_count),
            |benchmark| {
                benchmark.iter(|| {
                    black_box(
                        packages
                            .iter()
                            .filter(|package| {
                                only_install.include_package(
                                    InstallTarget {
                                        name: black_box(package),
                                        is_local: false,
                                    },
                                    None,
                                    &members,
                                )
                            })
                            .count(),
                    );
                });
            },
        );
    }
    group.finish();
}

criterion_group!(install_options, install_package_filters);
criterion_main!(install_options);
