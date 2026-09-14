//! Whole-command native and PEP 517 builds of real Python projects.

mod common;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    NATIVE_SOURCE_FIXTURES, PreparedNativeSource, is_codspeed_simulation, run_command,
    source_fixture, uv_command,
};

fn build_frontend(c: &mut Criterion<WallTime>) {
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
    let mut group = c.benchmark_group("build_frontend");
    group.sampling_mode(SamplingMode::Flat);
    for fixture in NATIVE_SOURCE_FIXTURES {
        let source = PreparedNativeSource::new(
            fixture,
            &source_fixture(fixture.name),
            uv_version::version(),
        );
        for (mode, pep517) in [("direct", false), ("pep517", true)] {
            for (operation, argument) in [
                ("wheel", Some("--wheel")),
                ("sdist", Some("--sdist")),
                ("sdist_wheel", None),
            ] {
                group.bench_function(
                    BenchmarkId::new(format!("{mode}/{operation}"), fixture.name),
                    |b| {
                        b.iter_batched(
                            || {
                                let output = tempfile::tempdir()
                                    .expect("Failed to create build output directory");
                                let mut command = uv_command();
                                command
                                    .env("UV_PYTHON_INSTALL_DIR", &python_directory)
                                    .args(["--offline", "--no-progress", "build"])
                                    .arg(source.path())
                                    .args(["--no-index", "--find-links"])
                                    .arg(&backend)
                                    .args(["--managed-python", "--python", "3.11.13", "--out-dir"])
                                    .arg(output.path());
                                if let Some(argument) = argument {
                                    command.arg(argument);
                                }
                                if pep517 {
                                    command.arg("--force-pep517");
                                }
                                (output, command)
                            },
                            |(output, mut command)| {
                                run_command(&mut command);
                                output
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
    name = builds;
    config = common::walltime_criterion();
    targets = build_frontend
}
criterion_main!(builds);
