//! Inspect and clean caches populated by real project environments.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    EnvironmentFixture, PreparedEnvironment, copy_cache, environment_fixtures, fixture_path,
    is_codspeed_simulation, run_command, uv_command_with_cache,
};

struct PackageCache {
    // Keep an installed environment alive so cached wheel files have real installation links.
    _environment: PreparedEnvironment,
    directory: tempfile::TempDir,
}

impl PackageCache {
    fn prepare(fixture: &EnvironmentFixture) -> Self {
        let source = Path::new("../../.cache/bench-caches").join(&fixture.name);
        Self::from_source(fixture, &source)
    }

    fn from_source(fixture: &EnvironmentFixture, source: &Path) -> Self {
        let directory = tempfile::tempdir().expect("Failed to create package cache");
        assert!(
            source.is_dir(),
            "Missing project cache. Run `python3 scripts/benchmark/prepare-environments.py --project-caches`."
        );
        copy_cache(source, directory.path()).expect("Failed to copy project cache");
        let environment = PreparedEnvironment::from_fixture_with_cache(fixture, directory.path());
        Self {
            _environment: environment,
            directory,
        }
    }

    fn command(&self) -> Command {
        let mut command = uv_command_with_cache(self.directory.path());
        command.arg("--offline");
        command
    }
}

fn cache_management(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("cache_management");
    // Reconstructing a populated cache is expensive even though setup is not timed.
    group.sampling_mode(SamplingMode::Flat);
    for fixture in environment_fixtures() {
        for (name, arguments) in [
            ("size", &["size", "--output-format", "machine"][..]),
            ("prune", &["prune"][..]),
            ("clean_one", &["clean", "packaging"][..]),
            (
                "clean_packages",
                &["clean", "packaging", "pyyaml", "click"][..],
            ),
        ] {
            group.bench_function(BenchmarkId::new(name, &fixture.name), |b| {
                b.iter_batched(
                    || {
                        let cache = PackageCache::prepare(&fixture);
                        let mut command = cache.command();
                        command.arg("cache").args(arguments);
                        (cache, command)
                    },
                    |(cache, mut command)| {
                        run_command(&mut command);
                        cache
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();

    let package_sets: BTreeMap<String, Vec<String>> = serde_json::from_slice(
        &fs_err::read(fixture_path("environment-packages.json"))
            .expect("Missing installed package sets. Run `python3 scripts/benchmark/prepare-environments.py --project-caches`."),
    )
    .expect("Invalid installed package sets");
    let source = Path::new("../../.cache/bench-caches/shared");
    let mut group = c.benchmark_group("cache_clean_shared");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    for fixture in environment_fixtures() {
        let packages = package_sets
            .get(&fixture.name)
            .expect("Missing installed package set");
        assert!(!packages.is_empty(), "Installed package set is empty");
        for (name, count) in [
            ("one", 1),
            ("eight", 8),
            ("twenty", 20),
            ("environment", packages.len()),
        ] {
            if count > packages.len() {
                continue;
            }
            group.bench_function(BenchmarkId::new(name, &fixture.name), |b| {
                b.iter_batched(
                    || {
                        let cache = PackageCache::from_source(&fixture, source);
                        let mut command = cache.command();
                        command.args(["cache", "clean"]).args(&packages[..count]);
                        (cache, command)
                    },
                    |(cache, mut command)| {
                        run_command(&mut command);
                        cache
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = caches;
    config = common::walltime_criterion();
    targets = cache_management
}
criterion_main!(caches);
