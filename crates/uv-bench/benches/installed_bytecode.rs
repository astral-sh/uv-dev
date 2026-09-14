//! Compile bytecode for a real wheel installed into a populated environment.

mod common;

use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{PreparedEnvironment, fixture_path, is_codspeed_simulation, run_command};

fn installed_bytecode(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let environment = PreparedEnvironment::prefect();
    let mut group = c.benchmark_group("installed_bytecode");
    for (name, filename) in [
        ("flask", "flask-3.1.2-py3-none-any.whl"),
        ("sympy", "sympy-1.14.0-py3-none-any.whl"),
    ] {
        let wheel =
            std::path::absolute(fixture_path(filename)).expect("Failed to locate wheel fixture");
        let command = || {
            let mut command = environment.command();
            command
                .args([
                    "pip",
                    "install",
                    "--no-deps",
                    "--reinstall",
                    "--compile-bytecode",
                ])
                .arg(&wheel)
                .arg("--python")
                .arg(environment.python());
            command
        };
        run_command(&mut command());
        group.bench_function(BenchmarkId::new("reinstall", name), |b| {
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
    name = bytecode;
    config = common::walltime_criterion();
    targets = installed_bytecode
}
criterion_main!(bytecode);
