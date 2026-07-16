use std::fmt::Write;
use std::hint::black_box;
use std::path::Path;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_configuration::{DependencyGroupsWithDefaults, ExtrasSpecification};
use uv_normalize::{DefaultExtras, PackageName};
use uv_resolver::{Lock, Metadata, Package};

fn synthetic_lock(package_count: usize) -> (Lock, Vec<PackageName>) {
    let names: Vec<PackageName> = (0..package_count)
        .map(|index| {
            PackageName::from_str(&format!("package-{index:05}")).expect("valid package name")
        })
        .collect();
    let mut lock =
        String::from("version = 1\nrequires-python = \">=3.12\"\n\n[manifest]\nrequirements = [\n");
    for name in &names {
        writeln!(lock, "    {{ name = \"{name}\" }},").expect("write to string");
    }
    lock.push_str("]\n");
    for name in &names {
        write!(
            lock,
            "\n[[package]]\nname = \"{name}\"\nversion = \"1.0.0\"\nsource = {{ registry = \"https://example.com/simple\" }}\n"
        )
        .expect("write to string");
    }
    let lock = toml::from_str(&lock).expect("valid lock");
    (lock, names)
}

fn find_by_name_linear<'lock>(lock: &'lock Lock, name: &PackageName) -> Option<&'lock Package> {
    let mut found = None;
    for package in lock.packages() {
        if package.name() == name {
            assert!(found.is_none(), "duplicate package in benchmark lock");
            found = Some(package);
        }
    }
    found
}

fn lookup_all_locked_packages(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("lock_package_lookup");
    for package_count in [1_000, 2_000, 4_000] {
        let (lock, names) = synthetic_lock(package_count);

        group.bench_function(BenchmarkId::new("linear", package_count), |benchmark| {
            benchmark.iter(|| {
                for name in &names {
                    black_box(find_by_name_linear(&lock, black_box(name)));
                }
            });
        });
        group.bench_function(BenchmarkId::new("partition", package_count), |benchmark| {
            benchmark.iter(|| {
                for name in &names {
                    black_box(
                        lock.find_by_name(black_box(name))
                            .expect("unique benchmark package"),
                    );
                }
            });
        });
    }
    group.finish();
}

fn audit_all_locked_packages(c: &mut Criterion<WallTime>) {
    let extras = ExtrasSpecification::default().with_defaults(DefaultExtras::default());
    let groups = DependencyGroupsWithDefaults::none();
    let mut group = c.benchmark_group("lock_auditable");
    for package_count in [1_000, 2_000, 4_000] {
        let (lock, _) = synthetic_lock(package_count);
        group.bench_function(BenchmarkId::from_parameter(package_count), |benchmark| {
            benchmark.iter(|| black_box(lock.auditable(&extras, &groups, |_| true)));
        });
    }
    group.finish();
}

fn metadata_all_locked_packages(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("lock_workspace_metadata");
    for package_count in [1_000, 2_000, 4_000] {
        let (lock, _) = synthetic_lock(package_count);
        group.bench_function(BenchmarkId::from_parameter(package_count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    Metadata::from_script(Path::new("/workspace/script.py"), black_box(&lock))
                        .expect("valid metadata"),
                );
            });
        });
    }
    group.finish();
}

criterion_group!(
    lock_lookup,
    lookup_all_locked_packages,
    audit_all_locked_packages,
    metadata_all_locked_packages
);
criterion_main!(lock_lookup);
