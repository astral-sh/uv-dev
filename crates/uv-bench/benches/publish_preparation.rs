//! Hash and validate real release artifacts without uploading them.

mod common;

use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{WHEEL_FIXTURES, fixture_path, is_codspeed_simulation, run_command, uv_command};

fn publish_preparation(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("publish_preparation");
    for &(name, filename) in WHEEL_FIXTURES
        .iter()
        .chain([("django_sdist", "django-5.2.6.tar.gz")].iter())
    {
        let artifact =
            std::path::absolute(fixture_path(filename)).expect("Failed to locate publish artifact");
        let command = || {
            let mut command = uv_command();
            command
                .args([
                    "publish",
                    "--dry-run",
                    "--no-attestations",
                    "--trusted-publishing",
                    "never",
                    "--publish-url",
                    "http://127.0.0.1:0/",
                    "--username",
                    "benchmark",
                    "--password",
                    "benchmark",
                ])
                .arg(&artifact);
            command
        };
        run_command(&mut command());
        group.bench_function(BenchmarkId::new("dry_run", name), |b| {
            b.iter_batched(
                command,
                |mut command: Command| run_command(&mut command),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = publishing;
    config = common::walltime_criterion();
    targets = publish_preparation
}
criterion_main!(publishing);
