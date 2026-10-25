use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_distribution_types::{Index, IndexLocations, IndexLocationsLookup, IndexUrl};

fn index_lookup(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("index lookup");

    for count in [1, 2, 4, 8, 16, 32, 64, 128, 256] {
        let urls = (0..count)
            .map(|index| {
                IndexUrl::from_str(&format!("https://index-{index}.example.com/simple"))
                    .expect("benchmark index should be valid")
            })
            .collect::<Vec<_>>();
        let indexes = urls.iter().cloned().map(Index::from).collect();
        let locations = IndexLocations::new(indexes, Vec::new(), false);
        let lookup = IndexLocationsLookup::from(&locations);

        group.bench_with_input(BenchmarkId::new("linear", count), &urls, |b, urls| {
            b.iter(|| {
                for url in urls {
                    black_box(locations.exclude_newer_for(black_box(url)));
                }
            });
        });
        group.bench_with_input(BenchmarkId::new("indexed", count), &urls, |b, urls| {
            b.iter(|| {
                for url in urls {
                    black_box(lookup.exclude_newer_for(black_box(url)));
                }
            });
        });
    }

    group.finish();
}

criterion_group!(benches, index_lookup);
criterion_main!(benches);
