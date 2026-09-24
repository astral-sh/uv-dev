extern crate uv_performance_memory_allocator;

use std::alloc::{Allocator, Global};
use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime};
use uv_allocator::with_arena;

fn allocate_vectors<A: Allocator + Copy>(allocator: A) {
    let mut vectors = Vec::with_capacity_in(32, allocator);
    for length in 1..=32 {
        let mut values = Vec::with_capacity_in(length, allocator);
        values.extend(0..length);
        vectors.push(values);
    }
    black_box(vectors);
}

fn temporary_vectors(criterion: &mut Criterion<WallTime>) {
    let mut group = criterion.benchmark_group("temporary_vectors");
    group.throughput(Throughput::Elements(32));
    group.bench_function("global", |benchmark| {
        benchmark.iter(|| allocate_vectors(Global));
    });
    group.bench_function("arena", |benchmark| {
        benchmark.iter(|| with_arena(|allocator| allocate_vectors(allocator)));
    });
    group.finish();
}

criterion_group!(uv_allocator, temporary_vectors);
criterion_main!(uv_allocator);
