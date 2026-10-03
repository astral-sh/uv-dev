use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_configuration::EditableMode;
use uv_pep508::{Requirement, VerbatimUrl};

fn editable_lookup(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("editable package lookup");

    for package_count in [1, 16, 64, 256, 1024, 4096] {
        let packages = (0..package_count)
            .map(|index| {
                Requirement::<VerbatimUrl>::from_str(&format!("package-{index}"))
                    .expect("valid requirement")
                    .name
            })
            .collect::<Vec<_>>();
        let queries = (0..package_count)
            .map(|index| {
                let requirement = match index % 3 {
                    0 => "package-0".to_string(),
                    1 => format!("package-{}", package_count - 1),
                    _ => format!("other-{index}"),
                };
                Requirement::<VerbatimUrl>::from_str(&requirement)
                    .expect("valid requirement")
                    .name
            })
            .collect::<Vec<_>>();
        let editable = EditableMode::NonEditablePackages(packages);

        group.bench_with_input(
            BenchmarkId::new("repeated scan", package_count),
            &queries,
            |benchmark, queries| {
                benchmark.iter(|| {
                    queries
                        .iter()
                        .filter(|package| editable.for_package(black_box(package)).is_some())
                        .count()
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("indexed build and lookup", package_count),
            &queries,
            |benchmark, queries| {
                benchmark.iter(|| {
                    let lookup = editable.lookup();
                    queries
                        .iter()
                        .filter(|package| lookup(black_box(package)).is_some())
                        .count()
                });
            },
        );
    }

    group.finish();
}

criterion_group!(editable, editable_lookup);
criterion_main!(editable);
