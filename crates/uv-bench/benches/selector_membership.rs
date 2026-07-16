use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_configuration::{DependencyGroups, ExtrasSpecification};
use uv_normalize::{DefaultExtras, DefaultGroups, ExtraName, GroupName};

fn extra_names(prefix: &str, count: usize) -> Vec<ExtraName> {
    (0..count)
        .map(|index| ExtraName::from_str(&format!("{prefix}-{index:05}")).expect("valid extra"))
        .collect()
}

fn group_names(prefix: &str, count: usize) -> Vec<GroupName> {
    (0..count)
        .map(|index| GroupName::from_str(&format!("{prefix}-{index:05}")).expect("valid group"))
        .collect()
}

fn selector_membership(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("selector_membership");
    for count in [0, 1, 2, 4, 8, 9, 16, 100, 1_000, 4_000] {
        let included_extras = extra_names("feature", count);
        let excluded_extras = extra_names("feature", count / 2);
        let included_groups = group_names("feature", count);
        let excluded_groups = group_names("feature", count / 2);
        let extra_queries = included_extras
            .iter()
            .cloned()
            .chain(extra_names("missing", count))
            .collect::<Vec<_>>();
        let default_extra = ExtraName::from_str("default-extra").expect("valid extra");
        let dev_group = GroupName::from_str("dev").expect("valid group");
        let group_queries = included_groups
            .iter()
            .cloned()
            .chain(group_names("missing", count))
            .collect::<Vec<_>>();
        let extras = ExtrasSpecification::from_args(
            included_extras.clone(),
            excluded_extras.clone(),
            false,
            Vec::new(),
            false,
        );
        let groups = DependencyGroups::from_args(
            None,
            included_groups.clone(),
            excluded_groups.clone(),
            false,
            Vec::new(),
            false,
        );

        group.bench_function(BenchmarkId::new("linear_extras", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    extra_queries
                        .iter()
                        .filter(|extra| {
                            !excluded_extras.contains(black_box(extra))
                                && included_extras.contains(black_box(extra))
                        })
                        .count(),
                );
            });
        });
        group.bench_function(BenchmarkId::new("indexed_extras", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    extra_queries
                        .iter()
                        .filter(|extra| extras.contains(black_box(extra)))
                        .count(),
                );
            });
        });
        group.bench_function(BenchmarkId::new("construct_extras", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    ExtrasSpecification::from_args(
                        included_extras.clone(),
                        excluded_extras.clone(),
                        false,
                        Vec::new(),
                        false,
                    )
                    .with_defaults(DefaultExtras::default()),
                );
            });
        });
        group.bench_function(
            BenchmarkId::new("construct_extras_with_default", count),
            |benchmark| {
                benchmark.iter(|| {
                    black_box(
                        ExtrasSpecification::from_args(
                            included_extras.clone(),
                            excluded_extras.clone(),
                            false,
                            Vec::new(),
                            false,
                        )
                        .with_defaults(DefaultExtras::List(vec![default_extra.clone()])),
                    );
                });
            },
        );

        group.bench_function(BenchmarkId::new("linear_groups", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    group_queries
                        .iter()
                        .filter(|name| {
                            !excluded_groups.contains(black_box(name))
                                && included_groups.contains(black_box(name))
                        })
                        .count(),
                );
            });
        });
        group.bench_function(BenchmarkId::new("indexed_groups", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    group_queries
                        .iter()
                        .filter(|name| groups.contains(black_box(name)))
                        .count(),
                );
            });
        });
        group.bench_function(BenchmarkId::new("construct_groups", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    DependencyGroups::from_args(
                        None,
                        included_groups.clone(),
                        excluded_groups.clone(),
                        false,
                        Vec::new(),
                        false,
                    )
                    .with_defaults(DefaultGroups::default()),
                );
            });
        });
        group.bench_function(
            BenchmarkId::new("construct_groups_with_dev", count),
            |benchmark| {
                benchmark.iter(|| {
                    black_box(
                        DependencyGroups::from_args(
                            None,
                            included_groups.clone(),
                            excluded_groups.clone(),
                            false,
                            Vec::new(),
                            false,
                        )
                        .with_defaults(DefaultGroups::List(vec![dev_group.clone()])),
                    );
                });
            },
        );
    }
    group.finish();
}

criterion_group!(selectors, selector_membership);
criterion_main!(selectors);
