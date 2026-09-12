//! Concurrent local wheel installs sharing a cold cache.

mod common;

use std::process::Stdio;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{WHEEL_FIXTURES, fixture_path, is_codspeed_simulation, uv_command_with_cache};

fn local_wheel_concurrency(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let python_directory = std::path::absolute("../../.cache/bench-python")
        .expect("Failed to locate benchmark Python directory");
    let mut group = c.benchmark_group("local_wheel_concurrency");
    for (name, filename) in WHEEL_FIXTURES {
        let wheel =
            std::path::absolute(fixture_path(filename)).expect("Failed to locate wheel fixture");
        for processes in [1, 4] {
            group.bench_function(
                BenchmarkId::new(format!("{processes}_processes"), name),
                |b| {
                    b.iter_batched(
                        || {
                            let directory =
                                tempfile::tempdir().expect("Failed to create package cache");
                            let commands = (0..processes)
                                .map(|index| {
                                    let mut command =
                                        uv_command_with_cache(&directory.path().join("cache"));
                                    command
                                        .env("UV_PYTHON_INSTALL_DIR", &python_directory)
                                        .args([
                                            "--offline",
                                            "--no-progress",
                                            "pip",
                                            "install",
                                            "--no-deps",
                                            "--managed-python",
                                            "--python",
                                            "3.11.13",
                                            "--python-platform",
                                            "aarch64-manylinux2014",
                                            "--link-mode",
                                            "hardlink",
                                            "--target",
                                        ])
                                        .arg(directory.path().join(format!("environment-{index}")))
                                        .arg(&wheel)
                                        .stderr(Stdio::piped());
                                    command
                                })
                                .collect::<Vec<_>>();
                            (directory, commands)
                        },
                        |(directory, commands)| {
                            let children = commands
                                .into_iter()
                                .map(|mut command| {
                                    command.spawn().expect("Failed to start concurrent install")
                                })
                                .collect::<Vec<_>>();
                            let outputs = children
                                .into_iter()
                                .map(|child| {
                                    child
                                        .wait_with_output()
                                        .expect("Failed to wait for concurrent install")
                                })
                                .collect::<Vec<_>>();
                            for output in outputs {
                                assert!(
                                    output.status.success(),
                                    "Concurrent install failed: {}",
                                    String::from_utf8_lossy(&output.stderr)
                                );
                            }
                            directory
                        },
                        BatchSize::PerIteration,
                    );
                },
            );
        }
    }
    group.finish();
}

criterion_group! {
    name = wheels;
    config = common::walltime_criterion();
    targets = local_wheel_concurrency
}
criterion_main!(wheels);
