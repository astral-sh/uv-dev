//! Resolve real GitHub dependencies with and without a materialized source cache.

mod common;

use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{FixtureServer, GitFixture, git_fixtures, is_codspeed_simulation, run_command};
use uv_redacted::DisplaySafeUrl;

fn command(
    server: &FixtureServer,
    fixture: &GitFixture,
    cache: &Path,
    requirements: &Path,
) -> Command {
    let local = DisplaySafeUrl::from_file_path(fixture.path()).expect("Invalid Git fixture URL");
    let mut command = server.command(cache);
    command
        .env("UV_GITHUB_FAST_PATH_URL", server.url("/github/repos"))
        .env("UV_GITHUB_RAW_URL", server.url("/github/raw"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", cache.join("missing-git-config"))
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", format!("url.{local}.insteadOf"))
        .env("GIT_CONFIG_VALUE_0", &fixture.repository)
        .env("GIT_CONFIG_KEY_1", "protocol.file.allow")
        .env("GIT_CONFIG_VALUE_1", "always")
        .env("GIT_ALLOW_PROTOCOL", "file")
        .args([
            "pip",
            "compile",
            "--no-deps",
            "--no-header",
            "--no-annotate",
            "--default-index",
        ])
        .arg(server.url("/simple/"))
        .arg("--build-constraints")
        .arg(
            std::path::absolute("../../scripts/benchmark/source-build-constraints.txt")
                .expect("Failed to locate source-build constraints"),
        )
        .arg(requirements);
    command
}

fn github_metadata(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let server = FixtureServer::start_git();
    let mut group = c.benchmark_group("github_metadata");
    group.sampling_mode(SamplingMode::Flat);
    for fixture in git_fixtures() {
        let project = tempfile::tempdir().expect("Failed to create requirements directory");
        let requirements = project.path().join("requirements.in");
        fs_err::write(
            &requirements,
            format!(
                "{} @ git+{}@{}\n",
                fixture.name, fixture.repository, fixture.commit
            ),
        )
        .expect("Failed to write Git requirement");
        group.bench_function(BenchmarkId::new("cold", &fixture.name), |b| {
            b.iter_batched(
                || {
                    let cache = tempfile::tempdir().expect("Failed to create source cache");
                    let command = command(&server, &fixture, cache.path(), &requirements);
                    (cache, command)
                },
                |(cache, mut command)| {
                    run_command(&mut command);
                    cache
                },
                BatchSize::PerIteration,
            );
        });

        let cache = tempfile::tempdir().expect("Failed to create source cache");
        // Materialize the real checkout and source metadata through the ordinary Git path.
        // Subsequent invocations decide for themselves whether GitHub requests are needed.
        run_command(
            command(&server, &fixture, cache.path(), &requirements)
                .env("UV_NO_GITHUB_FAST_PATH", "1"),
        );
        let mut command = command(&server, &fixture, cache.path(), &requirements);
        group.bench_function(BenchmarkId::new("warm", &fixture.name), |b| {
            b.iter(|| run_command(&mut command));
        });
    }
    group.finish();
}

criterion_group! {
    name = metadata;
    config = common::walltime_criterion();
    targets = github_metadata
}
criterion_main!(metadata);
