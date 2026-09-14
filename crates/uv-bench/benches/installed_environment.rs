//! Inspect and modify a warm environment containing Prefect's real runtime dependencies.

mod common;

use std::process::Command;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{PreparedEnvironment, fixture_path, is_codspeed_simulation, run_command};

fn installed_environment(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let environment = PreparedEnvironment::prefect();
    let click_wheel = std::path::absolute(fixture_path("click-8.4.2-py3-none-any.whl"))
        .expect("Failed to locate Click wheel");
    let mut group = c.benchmark_group("installed_environment");
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
        (
            "install_no_deps",
            &["install", "--no-deps", "pydocket==0.23.1"][..],
        ),
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
            if name.starts_with("reinstall") {
                command.arg(&click_wheel);
            }
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
    name = environments;
    config = common::walltime_criterion();
    targets = installed_environment
}
criterion_main!(environments);
