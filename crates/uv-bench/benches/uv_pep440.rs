use std::hint::black_box;
use std::str::FromStr;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_pep440::{Version, VersionSpecifiers, release_specifiers_to_ranges};
use version_ranges::Ranges;

fn parse_version_specifiers(c: &mut Criterion<WallTime>) {
    for specifiers in [">=3.8", ">=3.8,<4", ">=2.5, !=3.0.*, !=3.1.*, !=3.2.*, <4"] {
        let name = format!("parse_version_specifiers {specifiers}");
        c.bench_function(&name, |benchmark| {
            benchmark.iter(|| {
                VersionSpecifiers::from_str(black_box(specifiers))
                    .expect("benchmark input should be valid")
            });
        });
    }
}

fn convert_version_specifiers(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("convert_version_specifiers");

    for count in [10, 100, 1000] {
        let specifiers = (0..count)
            .map(|version| format!("!={version}.0"))
            .collect::<Vec<_>>()
            .join(",")
            .parse::<VersionSpecifiers>()
            .expect("benchmark input should be valid");

        group.bench_with_input(
            BenchmarkId::new("pep440_exclusions", count),
            &specifiers,
            |b, specifiers| {
                b.iter_batched(
                    || specifiers.clone(),
                    |specifiers| Ranges::<Version>::from(black_box(specifiers)),
                    BatchSize::SmallInput,
                );
            },
        );
        group.bench_with_input(
            BenchmarkId::new("release_only_exclusions", count),
            &specifiers,
            |b, specifiers| {
                b.iter_batched(
                    || specifiers.clone(),
                    |specifiers| release_specifiers_to_ranges(black_box(specifiers)),
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    uv_pep440,
    parse_version_specifiers,
    convert_version_specifiers
);
criterion_main!(uv_pep440);
