//! Resolve static, dynamic, and legacy metadata from real local source trees.

mod common;

use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    copy_cache, is_codspeed_simulation, run_command, source_fixture, uv_command_with_cache,
};
use uv_redacted::DisplaySafeUrl;

fn command(cache: &Path, arguments: &[&str], project: &Path) -> Command {
    let mut command = uv_command_with_cache(cache);
    command
        .env(
            "UV_PYTHON_INSTALL_DIR",
            std::path::absolute("../../.cache/bench-python")
                .expect("Failed to locate benchmark Python directory"),
        )
        .args(["--offline", "--no-progress"])
        .args(arguments)
        .args(["--no-index", "--find-links"])
        .arg(
            std::path::absolute("../../.cache/bench-fixtures")
                .expect("Failed to locate backend wheel fixtures"),
        )
        .arg("--build-constraints")
        .arg(
            std::path::absolute("../../scripts/benchmark/source-build-constraints.txt")
                .expect("Failed to locate build constraints"),
        )
        .args(["--managed-python", "--python", "3.11.13"])
        .arg(project);
    command
}

fn compile(cache: &Path, requirements: &Path) -> Command {
    command(
        cache,
        &[
            "pip",
            "compile",
            "--no-deps",
            "--no-header",
            "--no-annotate",
        ],
        requirements,
    )
}

fn source_metadata(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("source_metadata");
    group.sampling_mode(SamplingMode::Flat);
    for name in ["sampleproject", "flask", "django", "pip-test-package"] {
        for state in ["cold", "warm", "no_build", "generated_metadata"] {
            // Django's version and the legacy package's metadata require a backend hook.
            if state == "no_build" && !matches!(name, "sampleproject" | "flask") {
                continue;
            }
            // Flit does not leave a setuptools egg-info directory in the checkout.
            if state == "generated_metadata" && name == "flask" {
                continue;
            }
            group.bench_function(BenchmarkId::new(state, name), |b| {
                b.iter_batched(
                    || {
                        let directory =
                            tempfile::tempdir().expect("Failed to create source directory");
                        let project = directory.path().join("project");
                        copy_cache(&source_fixture(name), &project)
                            .expect("Failed to copy source fixture");
                        let requirements = directory.path().join("requirements.in");
                        let url = DisplaySafeUrl::from_file_path(&project)
                            .expect("Invalid source fixture URL");
                        fs_err::write(&requirements, format!("{name} @ {url}\n"))
                            .expect("Failed to write source requirement");
                        let cache = directory.path().join("cache");
                        match state {
                            "warm" => run_command(&mut compile(&cache, &requirements)),
                            "generated_metadata" => {
                                let mut build = command(
                                    &directory.path().join("build-cache"),
                                    &["build", "--sdist", "--force-pep517"],
                                    &project,
                                );
                                build.arg("--out-dir").arg(directory.path().join("dist"));
                                run_command(&mut build);
                                let metadata = match name {
                                    "sampleproject" => "src/sampleproject.egg-info/PKG-INFO",
                                    "django" => "Django.egg-info/PKG-INFO",
                                    "pip-test-package" => "pip_test_package.egg-info/PKG-INFO",
                                    _ => unreachable!(),
                                };
                                assert!(
                                    project.join(metadata).is_file(),
                                    "Missing generated metadata"
                                );
                            }
                            _ => {}
                        }
                        let mut command = compile(&cache, &requirements);
                        if state == "no_build" {
                            command.arg("--no-build");
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
    name = metadata;
    config = common::walltime_criterion();
    targets = source_metadata
}
criterion_main!(metadata);
