//! Native lockfile I/O over pinned ecosystem projects.

extern crate uv_performance_memory_allocator;

use std::hint::black_box;

use criterion::{
    BenchmarkId, Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::fixture_path;
use uv_resolver::Lock;

fn lockfile(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("lockfile");
    for project in ["packse", "uv", "prefect"] {
        let input = fs_err::read_to_string(fixture_path(&format!("{project}.lock")))
            .expect("Failed to read lockfile fixture");
        let lock = Lock::from_toml(&input).expect("Invalid lockfile fixture");
        let serialized = lock
            .to_toml()
            .expect("Failed to serialize lockfile fixture");
        Lock::from_toml(&serialized).expect("Serialized fixture should be readable");

        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::new("read", project), &input, |b, input| {
            b.iter(|| Lock::from_toml(black_box(input)).expect("Failed to parse lockfile"));
        });
        group.bench_with_input(BenchmarkId::new("write", project), &lock, |b, lock| {
            b.iter(|| {
                black_box(lock)
                    .to_toml()
                    .expect("Failed to serialize lockfile")
            });
        });
    }
    group.finish();
}

criterion_group!(lockfiles, lockfile);
criterion_main!(lockfiles);
