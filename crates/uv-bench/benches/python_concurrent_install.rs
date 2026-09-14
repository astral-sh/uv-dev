//! Independent Python installations contending for one initially empty archive cache.

mod common;
#[path = "common/python.rs"]
mod python;

use std::process::Stdio;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{FixtureServer, is_codspeed_simulation};

fn python_concurrent_install(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let _archives = python::archive_directory();
    let server = FixtureServer::start_python();
    let mut group = c.benchmark_group("python_concurrent_install");
    for count in [1, 2, 4] {
        for (mode, distinct) in [("shared-version", false), ("distinct-versions", true)] {
            group.bench_function(BenchmarkId::new(mode, count), |b| {
                b.iter_batched(
                    || {
                        let directory =
                            tempfile::tempdir().expect("Failed to create install directory");
                        let archive_cache = directory.path().join("archives");
                        let commands = (0..count)
                            .map(|index| {
                                let mut command = python::command(
                                    &directory.path().join(index.to_string()),
                                    &archive_cache,
                                );
                                command
                                    .env("UV_PYTHON_INSTALL_MIRROR", server.url("/python"))
                                    .args(["python", "install", "--no-bin", "--no-registry"])
                                    .arg(python::VERSIONS[if distinct { index } else { 0 }])
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
                                command.spawn().expect("Failed to start installation")
                            })
                            .collect::<Vec<_>>();
                        let outputs = children
                            .into_iter()
                            .map(|child| {
                                child
                                    .wait_with_output()
                                    .expect("Failed to wait for installation")
                            })
                            .collect::<Vec<_>>();
                        for output in outputs {
                            assert!(
                                output.status.success(),
                                "Concurrent installation failed: {}",
                                String::from_utf8_lossy(&output.stderr)
                            );
                        }
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
    name = concurrent_install;
    config = common::walltime_criterion();
    targets = python_concurrent_install
}
criterion_main!(concurrent_install);
