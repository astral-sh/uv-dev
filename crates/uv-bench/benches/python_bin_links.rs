//! Rechecking executable links for already-installed managed Python versions.

mod common;
#[path = "common/python.rs"]
mod python;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{is_codspeed_simulation, run_command};

fn python_bin_links(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let archives = python::archive_directory();
    let mut group = c.benchmark_group("python_bin_links");
    for count in [1, 2, 4] {
        let directory = tempfile::tempdir().expect("Failed to create install directory");
        let mut command = python::command(directory.path(), &archives);
        command
            .args(["--offline", "python", "install", "--no-registry"])
            .args(&python::VERSIONS[..count]);
        run_command(&mut command);
        group.bench_function(BenchmarkId::new("already-installed", count), |b| {
            b.iter(|| run_command(&mut command));
        });
    }
    group.finish();
}

criterion_group! {
    name = links;
    config = common::walltime_criterion();
    targets = python_bin_links
}
criterion_main!(links);
