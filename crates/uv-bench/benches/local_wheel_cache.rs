//! Cache publication and reuse across real wheel releases.

mod common;

use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    WHEEL_FIXTURES, fixture_path, is_codspeed_simulation, run_command, uv_command_with_cache,
};

fn install_command(
    cache: &Path,
    target: &Path,
    wheel: &Path,
    python_directory: &Path,
    content_addressed: bool,
) -> Command {
    let mut command = uv_command_with_cache(cache);
    command
        .env("UV_PYTHON_INSTALL_DIR", python_directory)
        .args(["--offline", "--no-progress"]);
    if content_addressed {
        command.args(["--preview-features", "content-addressed-cache"]);
    }
    command
        .args([
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
        .arg(target)
        .arg(wheel);
    command
}

fn local_wheel_cache(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let python_directory = std::path::absolute("../../.cache/bench-python")
        .expect("Failed to locate benchmark Python directory");
    let mut group = c.benchmark_group("local_wheel_cache");
    // Populating an earlier release is deliberately outside the timed operation.
    group.sampling_mode(SamplingMode::Flat);
    for (name, filename) in WHEEL_FIXTURES {
        let previous = match *name {
            "flask" => "flask-3.1.1-py3-none-any.whl",
            "jupyterlab" => "jupyterlab-4.4.6-py3-none-any.whl",
            "numpy" => "numpy-2.2.5-cp311-cp311-manylinux_2_17_aarch64.manylinux2014_aarch64.whl",
            "sympy" => "sympy-1.13.3-py3-none-any.whl",
            _ => unreachable!("Missing preceding wheel release"),
        };
        let wheel =
            std::path::absolute(fixture_path(filename)).expect("Failed to locate wheel fixture");
        let previous = std::path::absolute(fixture_path(previous))
            .expect("Failed to locate preceding wheel fixture");
        for (mode, content_addressed) in [("default", false), ("content_addressed", true)] {
            for (state, seed) in [
                ("cold", None),
                ("warm", Some(&wheel)),
                ("previous_release", Some(&previous)),
            ] {
                group.bench_function(BenchmarkId::new(format!("{mode}/{state}"), name), |b| {
                    b.iter_batched(
                        || {
                            let directory =
                                tempfile::tempdir().expect("Failed to create package cache");
                            let cache = directory.path().join("cache");
                            if let Some(seed) = seed {
                                run_command(&mut install_command(
                                    &cache,
                                    &directory.path().join("seed-environment"),
                                    seed,
                                    &python_directory,
                                    content_addressed,
                                ));
                            }
                            let command = install_command(
                                &cache,
                                &directory.path().join("environment"),
                                &wheel,
                                &python_directory,
                                content_addressed,
                            );
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
        }
    }
    group.finish();
}

criterion_group! {
    name = wheels;
    config = common::walltime_criterion();
    targets = local_wheel_cache
}
criterion_main!(wheels);
