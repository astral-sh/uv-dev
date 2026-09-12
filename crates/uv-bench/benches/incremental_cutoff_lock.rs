//! Re-resolve real application graphs after widening their release-date cutoff.

mod common;

use std::path::Path;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{is_codspeed_simulation, run_command, uv_command_with_cache};

#[derive(serde::Deserialize)]
struct Workload {
    name: String,
}

fn incremental_cutoff_lock(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let workloads: Vec<Workload> = serde_json::from_str(include_str!(
        "../../../scripts/benchmark/incremental-locks.json"
    ))
    .expect("Invalid incremental-lock workloads");
    let mut group = c.benchmark_group("incremental_cutoff_lock");
    for workload in workloads {
        let prepared = std::path::absolute(
            Path::new("../../.cache/bench-incremental-locks").join(&workload.name),
        )
        .expect("Failed to locate prepared lock");
        assert!(
            prepared.join("cache").is_dir(),
            "Run `python3 scripts/benchmark/prepare-incremental-locks.py`"
        );
        let initial = fs_err::read(
            Path::new("../../scripts/benchmark/incremental-locks")
                .join(format!("{}.lock", workload.name)),
        )
        .expect("Missing initial lock");
        let directory = tempfile::tempdir().expect("Failed to create project");
        fs_err::copy(
            prepared.join("project/pyproject.toml"),
            directory.path().join("pyproject.toml"),
        )
        .expect("Failed to copy project requirements");
        let mut command = uv_command_with_cache(&prepared.join("cache"));
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
                "2026-01-01T00:00:00Z",
            ]);
        group.bench_function(BenchmarkId::new("widen", &workload.name), |b| {
            b.iter_batched(
                || {
                    fs_err::write(directory.path().join("uv.lock"), &initial)
                        .expect("Failed to restore initial lock");
                },
                |()| run_command(&mut command),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = incremental;
    config = common::walltime_criterion();
    targets = incremental_cutoff_lock
}
criterion_main!(incremental);
