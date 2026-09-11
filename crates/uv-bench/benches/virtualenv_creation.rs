//! Create virtual environments using a pinned, installed CPython interpreter.

mod common;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{is_codspeed_simulation, run_command, uv_command};

fn virtualenv_creation(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let python = std::path::absolute("../../.cache/bench-python")
        .expect("Failed to locate benchmark Python directory");
    let mut group = c.benchmark_group("virtualenv_creation");
    for (name, path, relocatable) in [
        ("default", ".venv", false),
        ("spaces", "project with spaces/.venv", false),
        ("relocatable", ".venv", true),
    ] {
        group.bench_function(name, |b| {
            b.iter_batched(
                || {
                    let directory =
                        tempfile::tempdir().expect("Failed to create project directory");
                    let mut command = uv_command();
                    command.env("UV_PYTHON_INSTALL_DIR", &python).args([
                        "--offline",
                        "venv",
                        "--no-project",
                        "--managed-python",
                        "--python",
                        "3.11.13",
                    ]);
                    if relocatable {
                        command.arg("--relocatable");
                    }
                    command.arg(directory.path().join(path));
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
    group.finish();
}

criterion_group! {
    name = environments;
    config = common::walltime_criterion();
    targets = virtualenv_creation
}
criterion_main!(environments);
