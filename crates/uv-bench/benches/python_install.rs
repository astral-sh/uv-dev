//! Managed Python installation from real cached and locally served interpreter archives.

mod common;
#[path = "common/python.rs"]
mod python;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{FixtureServer, is_codspeed_simulation, run_command};

fn python_install(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let archives = python::archive_directory();
    let server = FixtureServer::start_python();
    let mut group = c.benchmark_group("python_install");
    for count in [1, 2, 4] {
        for (mode, cached) in [("cached", true), ("download", false)] {
            group.bench_function(BenchmarkId::new(mode, count), |b| {
                b.iter_batched(
                    || {
                        let directory =
                            tempfile::tempdir().expect("Failed to create install directory");
                        let cache = if cached {
                            archives.clone()
                        } else {
                            directory.path().join("archives")
                        };
                        let mut command = python::command(directory.path(), &cache);
                        command
                            .env("UV_PYTHON_INSTALL_MIRROR", server.url("/python"))
                            .args(["python", "install", "--no-bin", "--no-registry"])
                            .args(&python::VERSIONS[..count]);
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
    name = install;
    config = common::walltime_criterion();
    targets = python_install
}
criterion_main!(install);
