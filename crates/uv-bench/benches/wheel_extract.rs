//! Extraction of published wheels through the local-file and download paths.

extern crate uv_performance_memory_allocator;

use std::hint::black_box;

use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{WHEEL_FIXTURES, fixture_path, is_codspeed_simulation};

fn wheel_extract(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let mut group = c.benchmark_group("wheel_extract");
    for &(project, filename) in WHEEL_FIXTURES {
        let archive = fixture_path(filename);
        let bytes = fs_err::read(&archive).expect("Failed to read wheel fixture");
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_function(BenchmarkId::new("seek", project), |b| {
            b.iter_batched(
                || {
                    (
                        fs_err::File::open(&archive).expect("Failed to open wheel fixture"),
                        tempfile::tempdir().expect("Failed to create extraction directory"),
                    )
                },
                |(reader, target)| {
                    let files =
                        uv_extract::unzip(reader, target.path()).expect("Failed to extract wheel");
                    black_box((files, target))
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("seek_dirhash", project), |b| {
            b.iter_batched(
                || {
                    (
                        fs_err::File::open(&archive).expect("Failed to open wheel fixture"),
                        tempfile::tempdir().expect("Failed to create extraction directory"),
                    )
                },
                |(reader, target)| {
                    let output = uv_extract::unzip_and_hash(reader, target.path())
                        .expect("Failed to extract and hash wheel");
                    black_box((output, target))
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("stream_dirhash", project), |b| {
            b.iter_batched(
                || tempfile::tempdir().expect("Failed to create extraction directory"),
                |target| {
                    black_box(
                        runtime
                            .block_on(uv_extract::stream::unzip_and_hash(
                                black_box(bytes.as_slice()),
                                target,
                            ))
                            .expect("Failed to stream and hash wheel"),
                    )
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group!(wheels, wheel_extract);
criterion_main!(wheels);
