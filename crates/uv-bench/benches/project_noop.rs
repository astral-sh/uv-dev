//! Whole-process commands against synchronized projects of different sizes.

mod common;

use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{PreparedEnvironment, environment_fixtures, is_codspeed_simulation, run_command};

fn project_noop(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("project_noop");
    for fixture in environment_fixtures() {
        let environment = PreparedEnvironment::from_fixture(&fixture);
        for name in ["sync_frozen", "run_no_sync"] {
            let command = || {
                if name == "sync_frozen" {
                    environment.sync_command()
                } else {
                    let mut command = environment.command();
                    command.args(["run", "--no-sync", "python", "-c", "pass"]);
                    command
                }
            };
            run_command(&mut command());
            group.bench_function(BenchmarkId::new(name, &fixture.name), |b| {
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
    targets = project_noop
}
criterion_main!(projects);
