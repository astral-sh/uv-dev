use std::hint::black_box;
use std::str::FromStr;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_distribution_types::{Index, IndexLocations, IndexMetadata, IndexName, IndexUrl};

fn synthetic_indexes(index_count: usize) -> IndexLocations {
    let indexes = (0..index_count)
        .map(|position| {
            let mut index = Index::from(
                IndexUrl::from_str(&format!("https://index-{position:05}.example/simple"))
                    .expect("valid index URL"),
            );
            index.name = Some(
                IndexName::from_str(&format!("index-{position:05}")).expect("valid index name"),
            );
            index
        })
        .collect();
    IndexLocations::new(indexes, Vec::new(), false)
}

fn fetch_index_enumeration(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("fetch_index_enumeration");
    for index_count in [64, 256, 1_024] {
        let locations = synthetic_indexes(index_count);
        let fetch_indexes: Arc<[_]> = locations
            .fetch_indexes()
            .map(|index| IndexMetadata {
                url: index.url.clone(),
                format: index.format,
            })
            .collect();

        group.bench_function(BenchmarkId::new("rebuild", index_count), |benchmark| {
            benchmark.iter(|| {
                let mut total = 0;
                for _ in 0..index_count {
                    total += black_box(&locations)
                        .fetch_indexes()
                        .map(|index| black_box(index.raw_url().as_str().len()))
                        .sum::<usize>();
                }
                black_box(total)
            });
        });
        group.bench_function(BenchmarkId::new("cached", index_count), |benchmark| {
            benchmark.iter(|| {
                let mut total = 0;
                for _ in 0..index_count {
                    total += black_box(&fetch_indexes)
                        .iter()
                        .map(|index| black_box(index.url.url().as_str().len()))
                        .sum::<usize>();
                }
                black_box(total)
            });
        });
    }
    group.finish();
}

criterion_group!(fetch_indexes, fetch_index_enumeration);
criterion_main!(fetch_indexes);
