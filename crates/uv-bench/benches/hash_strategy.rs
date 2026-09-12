use std::hint::black_box;
use std::str::FromStr;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use uv_configuration::HashCheckingMode;
use uv_distribution_types::{HashGeneration, Requirement, UnresolvedRequirement};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_redacted::DisplaySafeUrl;
use uv_types::HashStrategy;

fn hash_strategy(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_strategy");

    for size in [1_000_u64, 8_000] {
        let names = (0..size)
            .map(|index| PackageName::from_str(&format!("example-package-{index}")).unwrap())
            .collect::<Vec<_>>();
        let versions = (0..size)
            .map(|index| Version::from_str(&format!("1.2.{index}+2026.07.15.abcdef")).unwrap())
            .collect::<Vec<_>>();
        let urls = (0..size)
            .map(|index| {
                DisplaySafeUrl::parse(&format!(
                    "https://files.example.com/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/example_package_{index}-1.2.{index}-py3-none-any.whl?download=1&source=large-lockfile"
                ))
                .unwrap()
            })
            .collect::<Vec<_>>();
        let require = HashStrategy::from_requirements(
            std::iter::empty::<(&UnresolvedRequirement, &[String])>(),
            std::iter::empty::<(&Requirement, &[String])>(),
            None,
            HashCheckingMode::Require,
        )
        .expect("an empty set of requirements should produce a hash strategy");
        let strategies = [
            ("none", HashStrategy::default()),
            ("generate", HashStrategy::generate(HashGeneration::All)),
            ("verify", HashStrategy::verify(Arc::default())),
            ("require", require),
        ];

        group.throughput(Throughput::Elements(size));

        for (name, strategy) in strategies {
            group.bench_with_input(
                BenchmarkId::new(format!("package/{name}"), size),
                &strategy,
                |b, strategy| {
                    b.iter(|| {
                        for (name, version) in std::iter::zip(&names, &versions) {
                            black_box(
                                black_box(strategy)
                                    .get_package(black_box(name), black_box(version)),
                            );
                        }
                    });
                },
            );
            group.bench_with_input(
                BenchmarkId::new(format!("url/{name}"), size),
                &strategy,
                |b, strategy| {
                    b.iter(|| {
                        for url in &urls {
                            black_box(black_box(strategy).get_url(black_box(url)));
                        }
                    });
                },
            );
        }
    }

    group.finish();
}

criterion_group!(benches, hash_strategy);
criterion_main!(benches);
