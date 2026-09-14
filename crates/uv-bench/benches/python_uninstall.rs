//! Managed Python cleanup with one, two, and four genuine installed interpreters.

mod common;
#[path = "common/python.rs"]
mod python;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{is_codspeed_simulation, run_command};

fn python_uninstall(c: &mut Criterion<WallTime>) {
    // Windows uninstall also cleans user-level registry entries outside the install directory.
    if cfg!(windows) || is_codspeed_simulation() {
        return;
    }
    let archives = python::archive_directory();
    let mut group = c.benchmark_group("python_uninstall");
    for count in [1, 2, 4] {
        for (mode, all) in [("one", false), ("all", true)] {
            group.bench_function(BenchmarkId::new(mode, count), |b| {
                b.iter_batched(
                    || {
                        let directory =
                            tempfile::tempdir().expect("Failed to create install directory");
                        run_command(
                            python::command(directory.path(), &archives)
                                .args(["--offline", "python", "install", "--no-registry"])
                                .args(&python::VERSIONS[..count]),
                        );
                        let mut command = python::command(directory.path(), &archives);
                        command.args(["--offline", "python", "uninstall"]);
                        if all {
                            command.arg("--all");
                        } else {
                            command.arg(python::VERSIONS[0]);
                        }
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
    group.finish();
}

criterion_group! {
    name = uninstall;
    config = common::walltime_criterion();
    targets = python_uninstall
}
criterion_main!(uninstall);
