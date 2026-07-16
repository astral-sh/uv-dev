use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_distribution::IndexLookup;
use uv_distribution_types::{Index, IndexLocations, IndexName, Origin};

fn scan<'data>(
    locations: &'data IndexLocations,
    project: &'data [Index],
    workspace: &'data [Index],
    name: &IndexName,
) -> Option<&'data Index> {
    locations
        .indexes()
        .filter(|index| matches!(index.origin, Some(Origin::Cli)))
        .chain(project.iter())
        .chain(workspace.iter())
        .find(|index| {
            index
                .name
                .as_ref()
                .is_some_and(|candidate| candidate == name)
        })
}

fn registry_source_index_lookup(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("registry_source_index_lookup");
    for index_count in [1, 8, 64, 256, 1_024, 4_096] {
        let project = (0..index_count)
            .map(|position| {
                Index::from_str(&format!(
                    "index-{position}=https://index-{position}.example.com/simple"
                ))
                .expect("valid named index")
            })
            .collect::<Vec<_>>();
        let locations = IndexLocations::default();
        let first = IndexName::from_str("index-0").expect("valid index name");
        let last =
            IndexName::from_str(&format!("index-{}", index_count - 1)).expect("valid index name");
        let missing = IndexName::from_str("missing").expect("valid index name");

        for (case, name) in [("first", &first), ("last", &last), ("missing", &missing)] {
            let lookup = IndexLookup::new(&locations, &project, &[]);
            black_box(lookup.get(name));
            assert_eq!(
                scan(&locations, &project, &[], name).map(Index::raw_url),
                lookup.get(name).map(Index::raw_url)
            );
            group.bench_function(
                BenchmarkId::new(format!("{case}/scan"), index_count),
                |benchmark| {
                    benchmark.iter(|| {
                        for _ in 0..index_count {
                            black_box(scan(&locations, &project, &[], black_box(name)));
                        }
                    });
                },
            );
            group.bench_function(
                BenchmarkId::new(format!("{case}/lookup"), index_count),
                |benchmark| {
                    benchmark.iter(|| {
                        for _ in 0..index_count {
                            black_box(lookup.get(black_box(name)));
                        }
                    });
                },
            );
        }
        group.bench_function(BenchmarkId::new("build/lookup", index_count), |benchmark| {
            benchmark.iter(|| {
                let lookup = IndexLookup::new(&locations, &project, &[]);
                black_box(lookup.get(black_box(&last)));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, registry_source_index_lookup);
criterion_main!(benches);
