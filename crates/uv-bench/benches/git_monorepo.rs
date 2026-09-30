//! Resolve real Airflow provider packages from one Git monorepo checkout.

mod common;

use std::fmt::Write;
use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    GIT_FETCH_MODES, is_codspeed_simulation, popular_git_fixtures, run_command,
    uv_command_with_cache,
};
use uv_redacted::DisplaySafeUrl;

fn command(cache: &Path, requirements: &Path, partial: bool, worktrees: bool) -> Command {
    let mut command = uv_command_with_cache(cache);
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", cache.join("missing-git-config"))
        .env("GIT_ALLOW_PROTOCOL", "file")
        .env(
            "UV_PYTHON_INSTALL_DIR",
            std::path::absolute("../../.cache/bench-python")
                .expect("Failed to locate benchmark Python"),
        )
        .args([
            "--offline",
            "pip",
            "compile",
            "--managed-python",
            "--python",
            "3.12.11",
            "--no-deps",
            "--no-build",
            "--no-header",
            "--no-annotate",
        ]);
    if partial {
        command.args(["--preview-features", "git-partial-fetches"]);
    }
    if worktrees {
        command.args(["--preview-features", "git-worktrees"]);
    }
    command.arg(requirements);
    command
}

fn compile(command: &mut Command, cache: &Path, worktrees: bool) {
    run_command(command);
    assert_eq!(cache.join("git-v0/worktrees/checkouts").is_dir(), worktrees);
}

fn git_monorepo(criterion: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let fixture = popular_git_fixtures()
        .into_iter()
        .find(|fixture| fixture.name == "airflow")
        .expect("Missing Airflow Git fixture");
    let url = DisplaySafeUrl::from_file_path(fixture.path()).expect("Invalid Git fixture URL");
    let mut group = criterion.benchmark_group("git_monorepo");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    for count in [1, 4] {
        let packages = fixture
            .packages
            .get(..count)
            .expect("Missing Airflow provider packages");
        let project = tempfile::tempdir().expect("Failed to create requirements directory");
        let requirements = project.path().join("requirements.in");
        let mut contents = String::new();
        for package in packages {
            writeln!(
                contents,
                "{} @ git+{url}@{}#subdirectory={}",
                package.name, fixture.commit, package.subdirectory
            )
            .expect("Failed to format monorepo requirement");
        }
        fs_err::write(&requirements, contents).expect("Failed to write monorepo requirements");
        for &(mode, partial, worktrees) in GIT_FETCH_MODES {
            group.bench_function(
                BenchmarkId::new("cold", format!("airflow/{mode}/{count}")),
                |bencher| {
                    bencher.iter_batched(
                        || tempfile::tempdir().expect("Failed to create source cache"),
                        |cache| {
                            compile(
                                &mut command(cache.path(), &requirements, partial, worktrees),
                                cache.path(),
                                worktrees,
                            );
                            cache
                        },
                        BatchSize::PerIteration,
                    );
                },
            );
            group.bench_function(
                BenchmarkId::new("warm", format!("airflow/{mode}/{count}")),
                |bencher| {
                    let cache = tempfile::tempdir().expect("Failed to create source cache");
                    let mut command = command(cache.path(), &requirements, partial, worktrees);
                    compile(&mut command, cache.path(), worktrees);
                    bencher.iter(|| compile(&mut command, cache.path(), worktrees));
                },
            );
        }
    }
    group.finish();
}

criterion_group! {
    name = monorepos;
    config = common::walltime_criterion();
    targets = git_monorepo
}
criterion_main!(monorepos);
