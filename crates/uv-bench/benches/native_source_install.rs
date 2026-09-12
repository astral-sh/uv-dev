//! Install native-backend projects through cold and warm source caches.

mod common;

use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    NATIVE_SOURCE_FIXTURES, PreparedNativeSource, is_codspeed_simulation, run_command,
    source_fixture, uv_command_with_cache,
};

fn install_command(
    source: &Path,
    cache: &Path,
    target: &Path,
    backend: &Path,
    python_directory: &Path,
    editable: bool,
) -> Command {
    let mut command = uv_command_with_cache(cache);
    command
        .env("UV_PYTHON_INSTALL_DIR", python_directory)
        .args([
            "--offline",
            "--no-progress",
            "pip",
            "install",
            "--no-deps",
            "--no-index",
            "--find-links",
        ])
        .arg(backend)
        .args([
            "--managed-python",
            "--python",
            "3.11.13",
            "--link-mode",
            "hardlink",
            "--target",
        ])
        .arg(target);
    if editable {
        command.arg("--editable");
    }
    command.arg(source);
    command
}

fn native_source_install(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let python_directory = std::path::absolute("../../.cache/bench-python")
        .expect("Failed to locate benchmark Python directory");
    let backend = std::path::absolute("../../.cache/bench-build-backend")
        .expect("Failed to locate current build-backend wheel");
    assert!(
        backend.is_dir(),
        "Run `python3 scripts/benchmark/prepare-build-backend.py` first."
    );
    let mut group = c.benchmark_group("native_source_install");
    group.sampling_mode(SamplingMode::Flat);
    for fixture in NATIVE_SOURCE_FIXTURES {
        let source = PreparedNativeSource::new(
            fixture,
            &source_fixture(fixture.name),
            uv_version::version(),
        );
        for (mode, editable) in [("wheel", false), ("editable", true)] {
            for (state, warm) in [("cold", false), ("warm", true)] {
                group.bench_function(
                    BenchmarkId::new(format!("{mode}/{state}"), fixture.name),
                    |b| {
                        b.iter_batched(
                            || {
                                let directory =
                                    tempfile::tempdir().expect("Failed to create package cache");
                                let cache = directory.path().join("cache");
                                if warm {
                                    run_command(&mut install_command(
                                        source.path(),
                                        &cache,
                                        &directory.path().join("seed-environment"),
                                        &backend,
                                        &python_directory,
                                        editable,
                                    ));
                                }
                                let command = install_command(
                                    source.path(),
                                    &cache,
                                    &directory.path().join("environment"),
                                    &backend,
                                    &python_directory,
                                    editable,
                                );
                                (directory, command)
                            },
                            |(directory, mut command)| {
                                run_command(&mut command);
                                directory
                            },
                            BatchSize::PerIteration,
                        );
                    },
                );
            }
        }
    }
    group.finish();
}

criterion_group! {
    name = installs;
    config = common::walltime_criterion();
    targets = native_source_install
}
criterion_main!(installs);
