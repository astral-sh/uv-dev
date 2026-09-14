//! Parse the index options and includes in vLLM's real requirements files.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use futures::executor::block_on;
use uv_bench::fixture_path;
use uv_client::{BaseClientBuilder, Connectivity};
use uv_requirements_txt::{RequirementsTxt, SourceCache};

fn requirements_directives(c: &mut Criterion<WallTime>) {
    let root = std::path::absolute("../../.cache/bench-vllm-requirements")
        .expect("Failed to locate requirements root");
    let client = BaseClientBuilder::default().connectivity(Connectivity::Offline);
    let sources = [
        ("rubin-prerelease.txt", "vllm-rubin-prerelease.txt"),
        ("cuda.txt", "vllm-cuda.txt"),
        ("common.txt", "vllm-common.txt"),
        ("dev.txt", "vllm-dev.txt"),
        ("lint.txt", "vllm-lint.txt"),
        ("test/cuda.txt", "vllm-test-cuda.txt"),
    ]
    .into_iter()
    .map(|(path, filename)| {
        (
            root.join(path),
            fs_err::read_to_string(fixture_path(filename))
                .expect("Failed to read requirements fixture"),
        )
    })
    .collect::<SourceCache>();

    let mut group = c.benchmark_group("requirements_directives");
    for (name, path) in [
        ("prerelease", "rubin-prerelease.txt"),
        ("cuda", "cuda.txt"),
        ("development", "dev.txt"),
    ] {
        let path = root.join(path);
        let content = sources.get(&path).expect("Missing root requirements");
        let mut cache = sources.clone();
        let mut parse = |content: &str| {
            block_on(RequirementsTxt::parse_str(
                content, &path, &root, &client, &mut cache,
            ))
            .expect("Invalid requirements fixture")
        };
        let expected = parse(content);
        assert!(!expected.requirements.is_empty());
        if name != "development" {
            assert!(!expected.extra_index_urls.is_empty());
        }
        group.bench_with_input(BenchmarkId::new("parse", name), content, |b, content| {
            b.iter(|| black_box(parse(black_box(content))));
        });
        assert_eq!(cache.len(), sources.len());
    }
    group.finish();
}

criterion_group!(requirements, requirements_directives);
criterion_main!(requirements);
