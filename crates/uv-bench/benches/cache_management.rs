//! Inspect and clean caches populated by real project environments.

mod common;

use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    EnvironmentFixture, PreparedEnvironment, environment_fixtures, is_codspeed_simulation,
    run_command, uv_command_with_cache,
};

fn copy_cache(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs_err::create_dir_all(destination)?;
    for entry in fs_err::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            let link = fs_err::read_link(entry.path())?;
            assert!(link.is_relative(), "Cache links must be relocatable");
            #[cfg(unix)]
            fs_err::os::unix::fs::symlink(link, target)?;
            #[cfg(windows)]
            if fs_err::metadata(entry.path())?.is_dir() {
                fs_err::os::windows::fs::symlink_dir(link, target)?;
            } else {
                fs_err::os::windows::fs::symlink_file(link, target)?;
            }
        } else if file_type.is_dir() {
            copy_cache(&entry.path(), &target)?;
        } else {
            fs_err::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

struct PackageCache {
    // Keep an installed environment alive so cached wheel files have real installation links.
    _environment: PreparedEnvironment,
    directory: tempfile::TempDir,
}

impl PackageCache {
    fn prepare(fixture: &EnvironmentFixture) -> Self {
        let directory = tempfile::tempdir().expect("Failed to create package cache");
        let source = Path::new("../../.cache/bench-caches").join(&fixture.name);
        assert!(
            source.is_dir(),
            "Missing project cache. Run `python3 scripts/benchmark/prepare-environments.py --project-caches`."
        );
        copy_cache(&source, directory.path()).expect("Failed to copy project cache");
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
}

criterion_group! {
    name = caches;
    config = common::walltime_criterion();
    targets = cache_management
}
criterion_main!(caches);
