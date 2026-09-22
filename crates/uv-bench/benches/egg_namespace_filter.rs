//! Benchmarks for filtering legacy egg-info top-level modules against namespace packages.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rustc_hash::FxHashSet;

fn linear(namespace_packages: &str, top_level: &str) -> usize {
    let namespace_packages = namespace_packages
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    top_level
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !namespace_packages.contains(line))
        .count()
}

fn adaptive(namespace_packages: &str, top_level: &str) -> usize {
    let namespace_packages = namespace_packages
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let top_level = top_level
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());

    if namespace_packages.len() <= 64 {
        top_level
            .filter(|line| !namespace_packages.contains(line))
            .count()
    } else {
        let namespace_packages = namespace_packages.into_iter().collect::<FxHashSet<_>>();
        top_level
            .filter(|line| !namespace_packages.contains(line))
            .count()
    }
}

fn inputs(entries: usize, case: &str) -> (String, String, usize) {
    let namespace_packages = (0..entries)
        .map(|index| format!(" namespace_{index:05} "))
        .collect::<Vec<_>>()
        .join("\n");
    let top_level = (0..entries)
        .map(|index| match case {
            "hit" => format!("namespace_{:05}", entries - 1 - index),
            "miss" => format!("module_{index:05}"),
            _ if index % 2 == 0 => format!("namespace_{:05}", entries - 1 - index),
            _ => format!("module_{index:05}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let retained = match case {
        "hit" => 0,
        "miss" => entries,
        _ => entries / 2,
    };
    (namespace_packages, top_level, retained)
}

fn egg_namespace_filter(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("egg namespace filter");

    for entries in [64, 128, 256, 512, 1024, 4096, 16384] {
        group.throughput(Throughput::Elements(entries as u64));
        for case in ["hit", "miss", "mixed"] {
            let (namespace_packages, top_level, retained) = inputs(entries, case);
            assert_eq!(linear(&namespace_packages, &top_level), retained);
            assert_eq!(adaptive(&namespace_packages, &top_level), retained);

            group.bench_with_input(
                BenchmarkId::new(format!("linear {case}"), entries),
                &(&namespace_packages, &top_level),
                |bencher, (namespace_packages, top_level)| {
                    bencher.iter(|| linear(black_box(namespace_packages), black_box(top_level)));
                },
            );
            group.bench_with_input(
                BenchmarkId::new(format!("adaptive {case}"), entries),
                &(&namespace_packages, &top_level),
                |bencher, (namespace_packages, top_level)| {
                    bencher.iter(|| adaptive(black_box(namespace_packages), black_box(top_level)));
                },
            );
        }
    }

    group.finish();
}

criterion_group!(benches, egg_namespace_filter);
criterion_main!(benches);
