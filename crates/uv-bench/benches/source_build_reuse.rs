//! Resolve and install real dynamic-metadata projects through isolated PEP 517 builds.

mod common;

use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    copy_cache, is_codspeed_simulation, run_command, source_fixture, uv_command_with_cache,
};

fn install(cache: &Path, project: &Path, target: &Path, editable: bool) -> Command {
    let mut command = uv_command_with_cache(cache);
    command
        .env(
            "UV_PYTHON_INSTALL_DIR",
            std::path::absolute("../../.cache/bench-python")
                .expect("Failed to locate benchmark Python"),
        )
        .args([
            "--offline",
            "--no-progress",
            "pip",
            "install",
            "--no-deps",
            "--no-index",
            "--find-links",
        ])
        .arg(
            std::path::absolute("../../.cache/bench-fixtures")
                .expect("Failed to locate backend wheels"),
        )
        .arg("--build-constraints")
        .arg(
            std::path::absolute("../../scripts/benchmark/source-build-constraints.txt")
                .expect("Failed to locate build constraints"),
        )
        .args([
            "--managed-python",
            "--python",
            "3.11.13",
            "--link-mode",
            "hardlink",
            "--target",
        ])
        .arg(target);
    if editable {
        command.arg("--editable");
    }
    command.arg(project);
    command
}

fn source_build_reuse(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("source_build_reuse");
    group.sampling_mode(SamplingMode::Flat);
    for name in ["pip-test-package", "packaging", "django"] {
        for (mode, editable) in [("wheel", false), ("editable", true)] {
            for state in ["cold", "warm"] {
                group.bench_function(BenchmarkId::new(format!("{mode}/{state}"), name), |b| {
                    b.iter_batched(
                        || {
                            let directory =
                                tempfile::tempdir().expect("Failed to create source directory");
                            let project = directory.path().join("project");
                            copy_cache(&source_fixture(name), &project)
                                .expect("Failed to copy source fixture");
                            let cache = directory.path().join("cache");
                            if state == "warm" {
                                run_command(&mut install(
                                    &cache,
                                    &project,
                                    &directory.path().join("seed"),
                                    editable,
                                ));
                            }
                            let command = install(
                                &cache,
                                &project,
                                &directory.path().join("target"),
                                editable,
                            );
                            (directory, command)
                        },
                        |(directory, mut command)| {
                            run_command(&mut command);
                            directory
                        },
                        BatchSize::PerIteration,
                    );
                });
            }
        }
    }
    group.finish();
}

criterion_group! {
    name = builds;
    config = common::walltime_criterion();
    targets = source_build_reuse
}
criterion_main!(builds);
