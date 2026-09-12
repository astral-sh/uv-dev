//! Discover real managed interpreters with warm and cold interpreter-query caches.

mod common;

use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{is_codspeed_simulation, run_command, uv_command, uv_command_with_cache};

fn configure(mut command: Command) -> Command {
    command
        .env(
            "UV_PYTHON_INSTALL_DIR",
            std::path::absolute("../../.cache/bench-python")
                .expect("Failed to locate benchmark Python directory"),
        )
        .args(["--offline", "--managed-python"]);
    command
}

fn python_discovery(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    for version in ["3.10.18", "3.11.13", "3.12.11", "3.13.4"] {
        run_command(configure(uv_command()).args(["python", "find", "--system", version]));
    }
    let mut group = c.benchmark_group("python_discovery");
    for (name, arguments) in [
        ("find", &["python", "find", "--system", "3.11.13"][..]),
        (
            "list",
            &[
                "python",
                "list",
                "--only-installed",
                "--output-format",
                "json",
            ][..],
        ),
    ] {
        let command = || {
            let mut command = configure(uv_command());
            command.args(arguments);
            command
        };
        run_command(&mut command());
        group.bench_function(BenchmarkId::new(name, "warm"), |b| {
            b.iter_batched(
                command,
                |mut command| run_command(&mut command),
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new(name, "cold"), |b| {
            b.iter_batched(
                || {
                    let cache = tempfile::tempdir().expect("Failed to create interpreter cache");
                    let mut command = configure(uv_command_with_cache(cache.path()));
                    command.args(arguments);
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
    name = interpreters;
    config = common::walltime_criterion();
    targets = python_discovery
}
criterion_main!(interpreters);
