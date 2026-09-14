//! Reinstall complete real environments using their warm wheel caches.

mod common;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{PreparedEnvironment, environment_fixtures, is_codspeed_simulation, run_command};

fn environment_reinstall(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("environment_reinstall");
    for fixture in environment_fixtures() {
        let environment = PreparedEnvironment::from_fixture(&fixture);
        let command = || {
            let mut command = environment.sync_command();
            command.arg("--reinstall");
            command
        };
        // Reinstallation restores the same files and versions on every iteration.
        run_command(&mut command());
        group.bench_function(BenchmarkId::new("cached", &fixture.name), |b| {
            b.iter_batched(
                command,
                |mut command| run_command(&mut command),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = environments;
    config = common::walltime_criterion();
    targets = environment_reinstall
}
criterion_main!(environments);
