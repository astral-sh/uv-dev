//! Native lockfile I/O over pinned ecosystem projects.

mod common;

extern crate uv_performance_memory_allocator;

use std::hint::black_box;

use criterion::{
    BenchmarkId, Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{fixture_path, is_codspeed_simulation};
use uv_resolver::Lock;

fn lockfile(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("lockfile");
    for project in ["packse", "uv", "prefect", "pufferlib"] {
        let input = fs_err::read_to_string(fixture_path(&format!("{project}.lock")))
            .expect("Failed to read lockfile fixture");
        let lock = Lock::from_toml(&input).expect("Invalid lockfile fixture");
        // Serializing the full conflict-heavy graph is too expensive under CPU instrumentation.
        // Retain the real graph in walltime instead of reducing its marker complexity.
        let measure_write = project != "pufferlib" || !is_codspeed_simulation();
        if measure_write {
            let serialized = lock
                .to_toml()
                .expect("Failed to serialize lockfile fixture");
            Lock::from_toml(&serialized).expect("Serialized fixture should be readable");
        }

        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::new("read", project), &input, |b, input| {
            b.iter(|| Lock::from_toml(black_box(input)).expect("Failed to parse lockfile"));
        });
        if measure_write {
            group.bench_with_input(BenchmarkId::new("write", project), &lock, |b, lock| {
                b.iter(|| {
                    black_box(lock)
                        .to_toml()
                        .expect("Failed to serialize lockfile")
                });
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = lockfiles;
    config = common::walltime_criterion();
    targets = lockfile
}
criterion_main!(lockfiles);
