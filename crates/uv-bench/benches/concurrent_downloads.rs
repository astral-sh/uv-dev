//! Concurrent processes installing the same remote wheel into separate environments.

mod common;

use std::process::Stdio;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{FixtureServer, WHEEL_FIXTURES, is_codspeed_simulation};

fn concurrent_downloads(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let server = FixtureServer::start(&[]);
    let mut group = c.benchmark_group("concurrent_downloads");
    for (name, filename) in WHEEL_FIXTURES {
        let url = server.url(&format!("/files/{filename}"));
        group.bench_function(BenchmarkId::new("four_processes", name), |b| {
            b.iter_batched(
                || {
                    let directory = tempfile::tempdir().expect("Failed to create package cache");
                    let commands = (0..4)
                        .map(|index| {
                            let mut command = server.command(&directory.path().join("cache"));
                            command
                                .args([
                                    "pip",
                                    "install",
                                    "--no-deps",
                                    "--python-platform",
                                    "aarch64-manylinux2014",
                                    "--link-mode",
                                    "hardlink",
                                    "--target",
                                ])
                                .arg(directory.path().join(format!("environment-{index}")))
                                .arg(&url)
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
        });
    }
    group.finish();
}

criterion_group! {
    name = downloads;
    config = common::walltime_criterion();
    targets = concurrent_downloads
}
criterion_main!(downloads);
