//! Benchmarks for separating reinstalls between isolated and shared source-build phases.

use std::hint::black_box;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_distribution_filename::{DistExtension, SourceDistExtension};
use uv_distribution_types::{Dist, InstalledDist, InstalledDistKind, InstalledRegistryDist, Name};
use uv_installer::Plan;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pep508::VerbatimUrl;

#[derive(Clone, Copy)]
enum Pattern {
    First,
    Last,
    Miss,
    Mixed,
}

fn remote_dist(name: &str) -> Arc<Dist> {
    let url = VerbatimUrl::parse_url(format!("https://example.org/{name}-1.0.0.tar.gz"))
        .expect("valid source URL");
    Arc::new(
        Dist::from_http_url(
            PackageName::from_str(name).expect("valid package name"),
            url.clone(),
            url.to_url(),
            None,
            DistExtension::Source(SourceDistExtension::TarGz),
        )
        .expect("valid source distribution"),
    )
}

fn installed_dist(name: &str, index: usize) -> InstalledDist {
    InstalledDist::from(InstalledDistKind::Registry(InstalledRegistryDist {
        name: PackageName::from_str(name).expect("valid package name"),
        version: Version::from_str("1.0.0").expect("valid version"),
        path: PathBuf::from(format!("{name}-{index}.dist-info")).into_boxed_path(),
        cache_info: None,
        build_info: None,
    }))
}

fn fixture(
    right_count: usize,
    reinstall_count: usize,
    pattern: Pattern,
) -> (Vec<Arc<Dist>>, Vec<InstalledDist>) {
    let remote = (0..right_count)
        .map(|index| remote_dist(&format!("source-build-{index:05}")))
        .collect::<Vec<_>>();
    let reinstalls = (0..reinstall_count)
        .map(|index| match pattern {
            Pattern::First if right_count > 0 => installed_dist("source-build-00000", index),
            Pattern::Last if right_count > 0 => {
                installed_dist(&format!("source-build-{:05}", right_count - 1), index)
            }
            Pattern::Mixed if right_count > 0 && index % 2 == 0 => {
                installed_dist(&format!("source-build-{:05}", index % right_count), index)
            }
            Pattern::First | Pattern::Last | Pattern::Miss | Pattern::Mixed => {
                installed_dist(&format!("unrelated-{index:05}"), index)
            }
        })
        .collect();
    (remote, reinstalls)
}

fn plan(remote: &[Arc<Dist>], reinstalls: &[InstalledDist]) -> Plan {
    Plan {
        remote: remote.to_vec(),
        reinstalls: reinstalls.to_vec(),
        ..Plan::default()
    }
}

fn partition_linear(plan: Plan) -> (Plan, Plan) {
    let Plan {
        cached,
        remote,
        reinstalls,
        extraneous,
    } = plan;
    let right_remote = remote;
    let (left_reinstalls, right_reinstalls) = reinstalls
        .into_iter()
        .partition::<Vec<_>, _>(|dist| !right_remote.iter().any(|d| d.name() == dist.name()));
    (
        Plan {
            cached,
            reinstalls: left_reinstalls,
            ..Plan::default()
        },
        Plan {
            remote: right_remote,
            reinstalls: right_reinstalls,
            extraneous,
            ..Plan::default()
        },
    )
}

fn summary((left, right): &(Plan, Plan)) -> (Vec<String>, Vec<String>, Vec<String>) {
    (
        right
            .remote
            .iter()
            .map(|dist| dist.name().to_string())
            .collect(),
        left.reinstalls
            .iter()
            .map(|dist| format!("{}:{}", dist.name(), dist.install_path().display()))
            .collect(),
        right
            .reinstalls
            .iter()
            .map(|dist| format!("{}:{}", dist.name(), dist.install_path().display()))
            .collect(),
    )
}

fn build_reinstall_partition(criterion: &mut Criterion<WallTime>) {
    let mut group = criterion.benchmark_group("build_reinstall_partition");
    for (label, right_count, reinstall_count, pattern) in [
        ("mixed", 8, 8, Pattern::Mixed),
        ("mixed", 16, 16, Pattern::Mixed),
        ("mixed", 32, 32, Pattern::Mixed),
        ("mixed", 64, 64, Pattern::Mixed),
        ("mixed", 128, 128, Pattern::Mixed),
        ("mixed", 512, 512, Pattern::Mixed),
        ("mixed", 1_024, 1_024, Pattern::Mixed),
        ("mixed", 4_096, 4_096, Pattern::Mixed),
        ("asymmetric", 8, 4_096, Pattern::Mixed),
        ("asymmetric", 16, 4_096, Pattern::Mixed),
        ("asymmetric", 4_096, 8, Pattern::Mixed),
        ("asymmetric", 4_096, 16, Pattern::Mixed),
        ("asymmetric", 4_096, 32, Pattern::Mixed),
        ("hit-first", 4_096, 4_096, Pattern::First),
        ("hit-last", 4_096, 4_096, Pattern::Last),
        ("miss", 4_096, 4_096, Pattern::Miss),
    ] {
        let (remote, reinstalls) = fixture(right_count, reinstall_count, pattern);
        let input = format!("{label}-{right_count}x{reinstall_count}");
        let linear = partition_linear(plan(&remote, &reinstalls));
        let indexed = plan(&remote, &reinstalls).partition(|_| false);
        assert_eq!(summary(&linear), summary(&indexed));

        group.bench_function(BenchmarkId::new("linear", &input), |benchmark| {
            benchmark.iter(|| partition_linear(black_box(plan(&remote, &reinstalls))));
        });
        group.bench_function(BenchmarkId::new("indexed", &input), |benchmark| {
            benchmark.iter(|| black_box(plan(&remote, &reinstalls)).partition(|_| false));
        });
    }
    group.finish();
}

criterion_group!(build_reinstall_partition_group, build_reinstall_partition);
criterion_main!(build_reinstall_partition_group);
