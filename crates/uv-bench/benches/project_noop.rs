//! Whole-process commands against an already synchronized Prefect environment.

mod common;

use std::process::Command;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{PreparedEnvironment, is_codspeed_simulation, run_command};

fn project_noop(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let environment = PreparedEnvironment::prefect();
    let mut group = c.benchmark_group("project_noop");
    for (name, arguments) in [
        (
            "sync_frozen",
            &[
                "sync",
                "--frozen",
                "--no-default-groups",
                "--no-install-project",
            ][..],
        ),
        (
            "run_no_sync",
            &["run", "--no-sync", "python", "-c", "pass"][..],
        ),
    ] {
        let command = || {
            let mut command = environment.command();
            command.args(arguments);
            command
        };
        run_command(&mut command());
        group.bench_function(name, |b| {
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
    name = projects;
    config = common::walltime_criterion();
    targets = project_noop
}
criterion_main!(projects);
