//! Project away the conflicting extras in `PufferLib`'s real universal resolution.

use std::hint::black_box;
use std::str::FromStr;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::fixture_path;
use uv_pep508::MarkerTree;

fn conflict_markers(c: &mut Criterion<WallTime>) {
    let input = fs_err::read_to_string(fixture_path("pufferlib.lock"))
        .expect("Failed to read PufferLib lockfile");
    let lock: toml::Value = toml::from_str(&input).expect("Invalid PufferLib lockfile");
    let marker = lock["resolution-markers"]
        .as_array()
        .expect("Missing resolution markers")
        .iter()
        .map(|marker| {
            MarkerTree::from_str(marker.as_str().expect("Invalid resolution marker"))
                .expect("Failed to parse resolution marker")
        })
        .fold(MarkerTree::FALSE, MarkerTree::or);
    assert!(!marker.is_false());

    c.bench_function("pufferlib_without_extras", |b| {
        b.iter(|| black_box(marker).without_extras());
    });
}

criterion_group!(conflicts, conflict_markers);
criterion_main!(conflicts);
