//! Check the registry packages in Prefect's real lockfile for newer releases.

mod common;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{FixtureServer, is_codspeed_simulation, run_command};

fn outdated_tree(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let server = FixtureServer::start(&["prefect.lock"]);
    let project = server.project("prefect");
    c.bench_function("outdated_tree/prefect", |b| {
        b.iter_batched(
            || {
                let cache = tempfile::tempdir().expect("Failed to create HTTP cache");
                let mut command = server.command(cache.path());
                command
                    .arg("--project")
                    .arg(project.path())
                    .args([
                        "tree",
                        "--frozen",
                        "--all-groups",
                        "--universal",
                        "--outdated",
                        "--default-index",
                    ])
                    .arg(server.url("/simple/"));
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

criterion_group! {
    name = projects;
    config = common::walltime_criterion();
    targets = outdated_tree
}
criterion_main!(projects);
