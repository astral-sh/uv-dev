//! Parsing and formatting dependencies from published wheel metadata.

extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::str::FromStr;

use criterion::{
    BenchmarkId, Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::fixture_path;
use uv_pep508::Requirement;
use uv_pypi_types::{ResolutionMetadata, VerbatimParsedUrl};

fn core_metadata(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("core_metadata");
    for project in ["flask", "jupyterlab", "airflow"] {
        let input = fs_err::read(fixture_path(&format!("{project}.metadata")))
            .expect("Failed to read core metadata");
        let metadata =
            ResolutionMetadata::parse_metadata(&input).expect("Invalid core metadata fixture");
        let requirements: Vec<String> = metadata
            .requires_dist
            .iter()
            .map(ToString::to_string)
            .collect();

        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_function(BenchmarkId::new("read", project), |b| {
            b.iter(|| {
                ResolutionMetadata::parse_metadata(black_box(&input))
                    .expect("Failed to parse core metadata")
            });
        });
        group.throughput(Throughput::Elements(requirements.len() as u64));
        group.bench_function(BenchmarkId::new("parse_requirements", project), |b| {
            b.iter(|| {
                black_box(&requirements)
                    .iter()
                    .map(|requirement| Requirement::<VerbatimParsedUrl>::from_str(requirement))
                    .collect::<Result<Vec<_>, _>>()
                    .expect("Failed to parse a wheel requirement")
            });
        });
        group.bench_function(BenchmarkId::new("format_requirements", project), |b| {
            b.iter(|| {
                black_box(&metadata.requires_dist)
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            });
        });
    }
    group.finish();
}

criterion_group!(metadata, core_metadata);
criterion_main!(metadata);
