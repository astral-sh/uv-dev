//! Parse real compiled Python requirements and constraints files.

use std::hint::black_box;
use std::path::Path;

use criterion::{
    BenchmarkId, Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime,
};
use futures::executor::block_on;
use uv_client::{BaseClientBuilder, Connectivity};
use uv_requirements_txt::{RequirementsTxt, SourceCache};

fn requirements_files(c: &mut Criterion<WallTime>) {
    let client = BaseClientBuilder::default().connectivity(Connectivity::Offline);
    let root = std::path::absolute("../..").expect("Failed to locate repository root");
    let mut group = c.benchmark_group("requirements_files");
    for (name, input) in [
        (
            "jupyter",
            include_str!("../../../test/requirements/compiled/jupyter.txt"),
        ),
        (
            "airflow_constraints",
            include_str!("../../../test/requirements/airflow2-constraints.txt"),
        ),
    ] {
        let parse = |input| {
            block_on(RequirementsTxt::parse_str(
                input,
                Path::new("requirements.txt"),
                &root,
                &client,
                &mut SourceCache::default(),
            ))
            .expect("Invalid requirements fixture")
        };
        assert!(!parse(input).requirements.is_empty());
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::new("parse", name), &input, |b, input| {
            b.iter(|| parse(black_box(input)));
        });
    }
    group.finish();
}

criterion_group!(requirements, requirements_files);
criterion_main!(requirements);
