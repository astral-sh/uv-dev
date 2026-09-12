//! Freshness checks for real application graphs with member-local explicit indexes.

mod common;

use std::path::Path;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{copy_cache, is_codspeed_simulation, run_command, uv_command_with_cache};

#[derive(serde::Deserialize)]
struct Workload {
    name: String,
}

fn member_index_lock(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let workloads: Vec<Workload> = serde_json::from_str(include_str!(
        "../../../scripts/benchmark/incremental-locks.json"
    ))
    .expect("Invalid application workloads");
    let prepared = std::path::absolute("../../.cache/bench-member-index-locks")
        .expect("Failed to locate member-index projects");
    let mut group = c.benchmark_group("member_index_lock");
    for workload in workloads {
        for layout in ["path", "workspace"] {
            let directory = tempfile::tempdir().expect("Failed to create project");
            copy_cache(
                &prepared.join(&workload.name).join(layout),
                directory.path(),
            )
            .expect("Run `python3 scripts/benchmark/prepare-member-index-locks.py`");
            fs_err::copy(
                Path::new("../../scripts/benchmark/member-index-locks")
                    .join(format!("{}-{layout}.lock", workload.name)),
                directory.path().join("uv.lock"),
            )
            .expect("Failed to copy frozen lock");
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
                    "--locked",
                    "--managed-python",
                    "--python",
                    "3.11.13",
                    "--exclude-newer",
                    "2025-10-01T00:00:00Z",
                ]);
            group.bench_function(BenchmarkId::new(layout, &workload.name), |b| {
                b.iter(|| run_command(&mut command));
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = indexes;
    config = common::walltime_criterion();
    targets = member_index_lock
}
criterion_main!(indexes);
