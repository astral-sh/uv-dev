use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_configuration::HashCheckingMode;
use uv_distribution_types::{HashPolicy, Requirement, RequirementSource, UnresolvedRequirement};
use uv_types::HashStrategy;

fn requirements(count: usize) -> Vec<Requirement> {
    (0..count)
        .map(|index| {
            Requirement::from(
                uv_pep508::Requirement::from_str(&format!(
                    "package-{index} @ https://example.invalid/package-{index}-1.0.0-py3-none-any.whl#sha256={index:064x}"
                ))
                .expect("valid archive URL requirement"),
            )
        })
        .collect()
}

fn augment_with_repeated_clones(requirements: &[Requirement]) -> HashStrategy {
    let mut hasher = empty_hash_strategy();
    for requirement in requirements {
        let in_flight = hasher.clone();
        hasher = hasher
            .augment_with_requirements(std::iter::once(requirement))
            .expect("non-conflicting archive URL hashes");
        black_box(in_flight);
    }
    hasher
}

fn augment_after_first_clone(requirements: &[Requirement]) -> HashStrategy {
    let mut hasher = empty_hash_strategy();
    let in_flight = hasher.clone();
    for requirement in requirements {
        hasher = hasher
            .augment_with_requirements(std::iter::once(requirement))
            .expect("non-conflicting archive URL hashes");
    }
    black_box(in_flight);
    hasher
}

fn empty_hash_strategy() -> HashStrategy {
    HashStrategy::from_requirements(
        std::iter::empty::<(&UnresolvedRequirement, &[String])>(),
        std::iter::empty::<(&Requirement, &[String])>(),
        None,
        HashCheckingMode::Require,
    )
    .expect("empty requirements produce an empty required hash strategy")
}

fn assert_equivalent(requirements: &[Requirement]) {
    let repeated = augment_with_repeated_clones(requirements);
    let reused = augment_after_first_clone(requirements);
    for requirement in requirements {
        assert!(matches!(requirement.source, RequirementSource::Url { .. }));
        let RequirementSource::Url { url, .. } = &requirement.source else {
            continue;
        };
        assert!(matches!(repeated.get_url(url), HashPolicy::All(_)));
        assert_eq!(repeated.get_url(url), reused.get_url(url));
    }
}

fn lookahead_hash_augmentation(criterion: &mut Criterion<WallTime>) {
    let mut group = criterion.benchmark_group("lookahead_hash_augmentation");
    for count in [8, 64, 256, 1_024, 2_048] {
        let requirements = requirements(count);
        assert_equivalent(&requirements);

        if count <= 512 {
            group.bench_function(BenchmarkId::new("repeated-clone", count), |benchmark| {
                benchmark
                    .iter(|| black_box(augment_with_repeated_clones(black_box(&requirements))));
            });
        }
        group.bench_function(BenchmarkId::new("reuse-after-first", count), |benchmark| {
            benchmark.iter(|| black_box(augment_after_first_clone(black_box(&requirements))));
        });
    }
    group.finish();
}

criterion_group!(benches, lookahead_hash_augmentation);
criterion_main!(benches);
