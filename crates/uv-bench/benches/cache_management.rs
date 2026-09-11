//! Inspect and clean a cache populated by real wheel installations.

mod common;

use std::process::Command;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{
    WHEEL_FIXTURES, fixture_path, is_codspeed_simulation, run_command, uv_command_with_cache,
};

struct PackageCache {
    directory: tempfile::TempDir,
}

impl PackageCache {
    fn prepare() -> Self {
        let cache = Self {
            directory: tempfile::tempdir().expect("Failed to create package cache"),
        };
        let mut command = cache.command();
        command
            .env(
                "UV_PYTHON_INSTALL_DIR",
                std::path::absolute("../../.cache/bench-python")
                    .expect("Failed to locate benchmark Python directory"),
            )
            .args([
                "pip",
                "install",
                "--managed-python",
                "--python",
                "3.11.13",
                "--python-platform",
                "aarch64-manylinux2014",
                "--no-deps",
                "--link-mode",
                "hardlink",
                "--target",
            ])
            .arg(cache.directory.path().join("site-packages"));
        for (_, filename) in WHEEL_FIXTURES {
            command.arg(std::path::absolute(fixture_path(filename)).expect("Missing wheel"));
        }
        run_command(&mut command);
        cache
    }

    fn command(&self) -> Command {
        let mut command = uv_command_with_cache(&self.directory.path().join("cache"));
        command.arg("--offline");
        command
    }
}

fn cache_management(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("cache_management");
    for (name, arguments) in [
        ("size", &["size", "--output-format", "machine"][..]),
        ("prune", &["prune"][..]),
        ("clean_flask", &["clean", "flask"][..]),
        (
            "clean_packages",
            &["clean", "flask", "jupyterlab", "numpy", "sympy"][..],
        ),
    ] {
        group.bench_function(name, |b| {
            b.iter_batched(
                || {
                    let cache = PackageCache::prepare();
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
    group.finish();
}

criterion_group! {
    name = caches;
    config = common::walltime_criterion();
    targets = cache_management
}
criterion_main!(caches);
