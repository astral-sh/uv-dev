use std::hint::black_box;
use std::str::FromStr;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_pep440::{
    Version, VersionSpecifiers, release_specifier_to_range, release_specifiers_to_ranges,
};
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

    let mut inputs = vec![
        ("ordinary", 0, ">=3.8,<4".to_string()),
        ("ordinary", 1, ">=3.8,<4,!=3.9".to_string()),
    ];
    for count in [10, 100, 1000, 2000] {
        for shape in ["distinct", "duplicate", "wildcard", "local"] {
            let specifiers = (0..count)
                .map(|version| match shape {
                    "duplicate" => format!("!={}.0", version / 2),
                    "wildcard" => format!("!={version}.*"),
                    "local" => format!("!={version}.0+local"),
                    _ => format!("!={version}.0"),
                })
                .collect::<Vec<_>>()
                .join(",");
            inputs.push((shape, count, specifiers));
        }
    }

    for (shape, count, specifiers) in inputs {
        let specifiers = specifiers
            .parse::<VersionSpecifiers>()
            .expect("benchmark input should be valid");

        for (name, conversion) in [
            (
                "pep440/baseline",
                sequential_pep440_ranges as fn(VersionSpecifiers) -> Ranges<Version>,
            ),
            ("pep440/batched", batched_pep440_ranges),
            ("release/baseline", sequential_release_ranges),
            ("release/batched", release_specifiers_to_ranges),
        ] {
            group.bench_with_input(
                BenchmarkId::new(format!("{name}/{shape}"), count),
                &specifiers,
                |benchmark, specifiers| {
                    benchmark.iter_batched(
                        || specifiers.clone(),
                        |specifiers| conversion(black_box(specifiers)),
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }

    group.finish();
}

fn sequential_pep440_ranges(specifiers: VersionSpecifiers) -> Ranges<Version> {
    specifiers
        .into_iter()
        .fold(Ranges::full(), |range, specifier| {
            range.intersection(&Ranges::from(specifier))
        })
}

fn batched_pep440_ranges(specifiers: VersionSpecifiers) -> Ranges<Version> {
    Ranges::from(specifiers)
}

fn sequential_release_ranges(specifiers: VersionSpecifiers) -> Ranges<Version> {
    specifiers
        .into_iter()
        .fold(Ranges::full(), |range, specifier| {
            range.intersection(&release_specifier_to_range(specifier, false))
        })
}

criterion_group!(
    uv_pep440,
    parse_version_specifiers,
    convert_version_specifiers
);
criterion_main!(uv_pep440);
