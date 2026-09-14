//! Cache-key glob scopes over the real uv source tree.

mod common;

use std::hint::black_box;
use std::path::Path;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{copy_cache, is_codspeed_simulation, source_fixture};
use uv_cache_info::CacheInfo;

fn configure_keys(project: &Path, patterns: &[&str]) {
    let path = project.join("pyproject.toml");
    let mut metadata: toml::Table =
        toml::from_str(&fs_err::read_to_string(&path).expect("Failed to read project metadata"))
            .expect("Invalid project metadata");
    let uv = metadata
        .get_mut("tool")
        .and_then(toml::Value::as_table_mut)
        .and_then(|tool| tool.get_mut("uv"))
        .and_then(toml::Value::as_table_mut)
        .expect("Missing uv settings");
    uv.insert(
        "cache-keys".to_string(),
        toml::Value::Array(
            patterns
                .iter()
                .map(|pattern| toml::Value::Table(toml::toml! { file = (*pattern) }))
                .collect(),
        ),
    );
    fs_err::write(
        path,
        toml::to_string_pretty(&metadata).expect("Failed to serialize cache-key configuration"),
    )
    .expect("Failed to configure source cache keys");
}

fn source_cache_keys(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("source_cache_keys");
    for (name, project_path, patterns) in [
        (
            "narrow_crates",
            "",
            Some(
                &[
                    "pyproject.toml",
                    "Cargo.lock",
                    "crates/uv-cache-info/src/*.rs",
                    "crates/uv-build-backend/src/*.rs",
                ][..],
            ),
        ),
        ("native_workspace", "", None),
        ("whole_tree", "", Some(&["**/*"][..])),
        (
            "parent_crates",
            "crates/uv-build",
            Some(
                &[
                    "pyproject.toml",
                    "../../Cargo.lock",
                    "../uv-build-backend/src/**/*.rs",
                    "../uv-build-backend/Cargo.toml",
                    "src/**/*.rs",
                    "python/**/*.py",
                ][..],
            ),
        ),
    ] {
        let directory = tempfile::tempdir().expect("Failed to create source directory");
        copy_cache(&source_fixture("uv"), directory.path()).expect("Failed to copy source fixture");
        let project = directory.path().join(project_path);
        if let Some(patterns) = patterns {
            configure_keys(&project, patterns);
        }
        let expected =
            CacheInfo::from_directory(&project).expect("Failed to read source cache keys");
        group.bench_function(BenchmarkId::new("warm", name), |b| {
            b.iter(|| {
                let info = CacheInfo::from_directory(black_box(&project))
                    .expect("Failed to read source cache keys");
                black_box(info)
            });
        });
        assert_eq!(
            CacheInfo::from_directory(&project).expect("Failed to re-read source cache keys"),
            expected,
            "Source fixture changed while measuring cache keys"
        );
    }
    group.finish();
}

criterion_group! {
    name = cache_keys;
    config = common::walltime_criterion();
    targets = source_cache_keys
}
criterion_main!(cache_keys);
