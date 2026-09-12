//! Application resolutions using paired production-optimized release binaries.

mod common;

use std::path::Path;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{is_codspeed_simulation, run_command, uv_command_with_binary};

#[derive(serde::Deserialize)]
struct Workload {
    name: String,
}

fn release_runtime(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let binaries = std::path::absolute("../../.cache/bench-release")
        .expect("Failed to locate release binaries");
    assert!(
        binaries.join("manifest.json").is_file(),
        "Run `python3 scripts/benchmark/prepare-release-binaries.py`"
    );
    let workloads: Vec<Workload> = serde_json::from_str(include_str!(
        "../../../scripts/benchmark/incremental-locks.json"
    ))
    .expect("Invalid release-runtime workloads");
    let mut group = c.benchmark_group("release_runtime");
    for workload in workloads {
        let prepared = std::path::absolute(
            Path::new("../../.cache/bench-incremental-locks").join(&workload.name),
        )
        .expect("Failed to locate prepared application");
        for mode in ["baseline", "pgo"] {
            let directory = tempfile::tempdir().expect("Failed to create project");
            fs_err::copy(
                prepared.join("project/pyproject.toml"),
                directory.path().join("pyproject.toml"),
            )
            .expect("Run `python3 scripts/benchmark/prepare-incremental-locks.py`");
            let binary = binaries
                .join(mode)
                .join(format!("uv{}", std::env::consts::EXE_SUFFIX));
            let mut command = uv_command_with_binary(&binary, &prepared.join("cache"));
            command
                .env(
                    "UV_PYTHON_INSTALL_DIR",
                    std::path::absolute("../../.cache/bench-python")
                        .expect("Failed to locate benchmark Python"),
                )
                .args(["--offline", "--no-progress", "--project"])
                .arg(directory.path())
                .args([
                    "lock",
                    "--managed-python",
                    "--python",
                    "3.11.13",
                    "--exclude-newer",
                    "2025-10-01T00:00:00Z",
                ]);
            group.bench_function(BenchmarkId::new(mode, &workload.name), |b| {
                b.iter_batched(
                    || {
                        let lock = directory.path().join("uv.lock");
                        if lock.exists() {
                            fs_err::remove_file(lock)
                                .expect("Failed to remove previous resolution");
                        }
                    },
                    |()| run_command(&mut command),
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = release;
    config = common::walltime_criterion();
    targets = release_runtime
}
criterion_main!(release);
