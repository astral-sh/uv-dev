//! Parse the real PyPI Simple API response recorded by the HTML parser's upstream benchmarks.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::fixture_path;

fn simple_api_html(c: &mut Criterion<WallTime>) {
    let source = fs_err::read_to_string(fixture_path("astral-tl-benchmarks.rs"))
        .expect("Failed to read upstream HTML fixture");
    let (_, response) = source
        .split_once("const PYPI_SIMPLE: &str = r#\"")
        .expect("Missing upstream PyPI response");
    let (response, _) = response
        .split_once("\"#;")
        .expect("Unterminated upstream PyPI response");
    tl::parse(response, tl::ParserOptions::default()).expect("Invalid PyPI response");

    let mut group = c.benchmark_group("simple_api_html");
    group.throughput(Throughput::Bytes(response.len() as u64));
    group.bench_function("iniconfig", |b| {
        b.iter(|| {
            tl::parse(black_box(response), tl::ParserOptions::default())
                .expect("Failed to parse PyPI response")
        });
    });
    group.finish();
}

criterion_group!(html, simple_api_html);
criterion_main!(html);
