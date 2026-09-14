//! Reuse of remote wheels when a later installation requires their recorded hash.

mod common;

use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use serde::Deserialize;
use uv_bench::{FixtureServer, WHEEL_FIXTURES, is_codspeed_simulation, run_command};

#[derive(Deserialize)]
struct Artifact {
    filename: String,
    sha256: String,
}

fn install_command(
    server: &FixtureServer,
    cache: &Path,
    target: &Path,
    requirements: &Path,
    require_hashes: bool,
) -> Command {
    let mut command = server.command(cache);
    command
        .args([
            "pip",
            "install",
            "--no-index",
            "--no-deps",
            "--python-platform",
            "aarch64-manylinux2014",
            "--link-mode",
            "hardlink",
            "--target",
        ])
        .arg(target)
        .arg("--requirements")
        .arg(requirements);
    if require_hashes {
        command.arg("--require-hashes");
    }
    command
}

fn download_hashing(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let artifacts: Vec<Artifact> =
        serde_json::from_str(include_str!("../../../scripts/benchmark/fixtures.json"))
            .expect("Invalid artifact manifest");
    let server = FixtureServer::start(&[]);
    let mut group = c.benchmark_group("download_hashing");
    group.sampling_mode(SamplingMode::Flat);
    for (name, filename) in WHEEL_FIXTURES {
        let artifact = artifacts
            .iter()
            .find(|artifact| artifact.filename == *filename)
            .expect("Missing wheel digest");
        let requirement = format!("{name} @ {}", server.url(&format!("/files/{filename}")));
        for (state, seed, require_hashes) in [
            ("cold_unhashed", false, false),
            ("cold_required", false, true),
            ("required_after_unhashed", true, true),
        ] {
            group.bench_function(BenchmarkId::new(state, name), |b| {
                b.iter_batched(
                    || {
                        let directory = tempfile::tempdir().expect("Failed to create cache");
                        let cache = directory.path().join("cache");
                        let unhashed = directory.path().join("unhashed.txt");
                        let hashed = directory.path().join("hashed.txt");
                        fs_err::write(&unhashed, format!("{requirement}\n"))
                            .expect("Failed to write requirements");
                        fs_err::write(
                            &hashed,
                            format!("{requirement} --hash=sha256:{}\n", artifact.sha256),
                        )
                        .expect("Failed to write hashed requirements");
                        if seed {
                            run_command(&mut install_command(
                                &server,
                                &cache,
                                &directory.path().join("seed"),
                                &unhashed,
                                false,
                            ));
                        }
                        let command = install_command(
                            &server,
                            &cache,
                            &directory.path().join("environment"),
                            if require_hashes { &hashed } else { &unhashed },
                            require_hashes,
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
    group.finish();
}

criterion_group! {
    name = downloads;
    config = common::walltime_criterion();
    targets = download_hashing
}
criterion_main!(downloads);
