//! Inspect and modify small, medium, and large frozen Python environments.

mod common;

use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{
    PreparedEnvironment, environment_fixtures, fixture_path, is_codspeed_simulation, run_command,
};

fn installed_environment(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let packaging_wheel = std::path::absolute(fixture_path("packaging-26.3-py3-none-any.whl"))
        .expect("Failed to locate packaging wheel");
    let mut group = c.benchmark_group("installed_environment");
    for fixture in environment_fixtures() {
        let environment = PreparedEnvironment::from_fixture(&fixture);
        for (name, arguments) in [
            ("list", &["list", "--format", "json"][..]),
            (
                "list_exclude",
                &[
                    "list",
                    "--format",
                    "json",
                    "--exclude",
                    "click",
                    "--exclude",
                    "rich",
                    "--exclude",
                    "anyio",
                    "--exclude",
                    "httpx",
                    "--exclude",
                    "pydantic",
                ][..],
            ),
            ("freeze", &["freeze"][..]),
            ("check", &["check"][..]),
            ("install_no_deps", &["install", "--no-deps"][..]),
            (
                "reinstall_dry_run",
                &["install", "--no-deps", "--reinstall", "--dry-run"][..],
            ),
            ("reinstall", &["install", "--no-deps", "--reinstall"][..]),
        ] {
            let command = || {
                let mut command = environment.command();
                command
                    .arg("pip")
                    .args(arguments)
                    .arg("--python")
                    .arg(environment.python());
                if name == "install_no_deps" {
                    command.arg(&fixture.installed_requirement);
                } else if name.starts_with("reinstall") {
                    command.arg(&packaging_wheel);
                }
                command
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
    name = environments;
    config = common::walltime_criterion();
    targets = installed_environment
}
criterion_main!(environments);
