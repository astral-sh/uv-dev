//! Whole-process inspection of real, frozen project lockfiles.

mod common;

use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{fixture_path, is_codspeed_simulation, run_command, uv_command};

fn project_inspection(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("project_inspection");
    for project in ["uv", "prefect"] {
        let directory = tempfile::tempdir().expect("Failed to create project directory");
        fs_err::copy(
            fixture_path(&format!("{project}.pyproject.toml")),
            directory.path().join("pyproject.toml"),
        )
        .expect("Failed to copy project metadata");
        fs_err::copy(
            fixture_path(&format!("{project}.lock")),
            directory.path().join("uv.lock"),
        )
        .expect("Failed to copy project lockfile");

        for (name, arguments) in [
            ("version", &["version", "--frozen"][..]),
            ("export", &["export", "--frozen", "--all-groups"][..]),
            ("tree", &["tree", "--frozen", "--all-groups"][..]),
            (
                "workspace_metadata",
                &["workspace", "metadata", "--frozen"][..],
            ),
        ] {
            // Prefect records a dynamic version without a frozen version in its lockfile.
            if name == "version" && project == "prefect" {
                continue;
            }
            let command = || {
                let mut command = uv_command();
                command
                    .args(["--offline", "--project"])
                    .arg(directory.path())
                    .args(arguments);
                command
            };
            run_command(&mut command());
            group.bench_function(BenchmarkId::new(name, project), |b| {
                b.iter_batched(
                    command,
                    |mut command: Command| run_command(&mut command),
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = projects;
    config = common::walltime_criterion();
    targets = project_inspection
}
criterion_main!(projects);
