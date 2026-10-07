use std::cell::LazyCell;
use std::env;
use std::hint::black_box;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, measurement::WallTime};
use uv_git::GitFetchSettings;
use uv_git_types::GitUrl;
use uv_redacted::DisplaySafeUrl;

use super::git_fixture::{AIRFLOW, GitFixture, LARGE_GIT_FIXTURES, POPULAR_GIT_FIXTURES};
use super::{fetch, git_url, is_codspeed_simulation};

const GIT_FETCH_MODES: &[(&str, bool, bool)] = &[
    ("full_clone", false, false),
    ("partial_clone", true, false),
    ("full_worktree", false, true),
    ("partial_worktree", true, true),
];

pub(super) fn popular(criterion: &mut Criterion<WallTime>) {
    benchmark(criterion, "git_fetch_popular", POPULAR_GIT_FIXTURES);
}

pub(super) fn large(criterion: &mut Criterion<WallTime>) {
    if env::var("UV_BENCH_LARGE_GIT").as_deref() != Ok("1") {
        return;
    }
    benchmark(criterion, "git_fetch_large", LARGE_GIT_FIXTURES);
}

fn revision_url(repository: &DisplaySafeUrl, commit: &str) -> GitUrl {
    git_url(
        repository,
        commit,
        Some(commit.parse().expect("Invalid Git commit")),
    )
}

fn benchmark(criterion: &mut Criterion<WallTime>, name: &str, fixtures: &[GitFixture]) {
    if is_codspeed_simulation() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let mut group = criterion.benchmark_group(name);
    group.sampling_mode(SamplingMode::Flat);
    for fixture in fixtures {
        let repository =
            LazyCell::new(|| fixture.prepare().expect("Failed to prepare Git repository"));
        let newest = fixture
            .revisions
            .last()
            .expect("Missing Git revision history");
        for &(mode, partial, worktrees) in GIT_FETCH_MODES {
            let settings = GitFetchSettings::default()
                .with_offline(true)
                .with_partial_fetches(partial)
                .with_worktrees(worktrees);
            // One revision is a fresh install; four revisions model successive upgrades.
            for selected in [std::slice::from_ref(newest), fixture.revisions] {
                group.bench_function(
                    BenchmarkId::new(
                        "git_fetch_revision_history",
                        format!("{}/{mode}/{}", fixture.name, selected.len()),
                    ),
                    |bencher| {
                        let revisions: Vec<_> = selected
                            .iter()
                            .map(|revision| revision_url(&repository, revision))
                            .collect();
                        bencher.iter_batched(
                            || tempfile::tempdir().expect("Failed to create Git cache"),
                            |cache| {
                                for git in &revisions {
                                    let fetched = fetch(&runtime, git, cache.path(), settings);
                                    assert_eq!(fetched.path().join(".git").is_file(), worktrees);
                                    black_box(fetched);
                                }
                                black_box(cache)
                            },
                            BatchSize::PerIteration,
                        );
                    },
                );
            }
            group.bench_function(
                BenchmarkId::new("git_fetch_warm_precise", format!("{}/{mode}", fixture.name)),
                |bencher| {
                    let precise = revision_url(&repository, newest);
                    let cache = tempfile::tempdir().expect("Failed to create Git cache");
                    let fetched = fetch(&runtime, &precise, cache.path(), settings);
                    assert_eq!(fetched.path().join(".git").is_file(), worktrees);
                    bencher.iter(|| black_box(fetch(&runtime, &precise, cache.path(), settings)));
                },
            );
        }
    }
    group.finish();
}

fn compile_command(cache: &Path, requirements: &Path, partial: bool, worktrees: bool) -> Command {
    let binary = env::var_os("UV_BENCH_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::path::absolute("../../target/profiling")
                .expect("Failed to locate benchmark binary")
                .join(format!("uv{}", env::consts::EXE_SUFFIX))
        });
    assert!(
        binary.is_file(),
        "Run `cargo build --locked --profile profiling --bin uv` first"
    );
    let mut command = Command::new(binary);
    for (name, _) in env::vars_os() {
        if name.to_string_lossy().starts_with("UV_") {
            command.env_remove(name);
        }
    }
    command
        .env_remove("VIRTUAL_ENV")
        .env_remove("CONDA_PREFIX")
        .env("UV_PYTHON_DOWNLOADS", "never")
        .args(["--no-config", "--cache-dir"])
        .arg(cache)
        .stdout(Stdio::null())
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
    let output = command
        .output()
        .expect("Failed to execute benchmark command");
    assert!(
        output.status.success(),
        "Benchmark command {command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(cache.join("git-v1/worktrees/checkouts").is_dir(), worktrees);
}

fn airflow_requirements(repository: &DisplaySafeUrl, count: usize) -> tempfile::NamedTempFile {
    const PROVIDERS: &[&str] = &["standard", "http", "amazon", "postgres"];

    let mut requirements = tempfile::NamedTempFile::new().expect("Failed to create requirements");
    for provider in &PROVIDERS[..count] {
        writeln!(
            requirements,
            "apache-airflow-providers-{provider} @ git+{repository}@{}#subdirectory=providers/{provider}",
            AIRFLOW.commit
        )
        .expect("Failed to write monorepo requirement");
    }
    requirements
}

pub(super) fn monorepo(criterion: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let repository = LazyCell::new(|| {
        AIRFLOW
            .prepare()
            .expect("Failed to prepare Airflow repository")
    });
    let mut group = criterion.benchmark_group("git_monorepo");
    group.sampling_mode(SamplingMode::Flat);
    for count in [1, 4] {
        for &(mode, partial, worktrees) in GIT_FETCH_MODES {
            group.bench_function(
                BenchmarkId::new("git_monorepo_cold", format!("airflow/{mode}/{count}")),
                |bencher| {
                    let requirements = airflow_requirements(&repository, count);
                    bencher.iter_batched(
                        || tempfile::tempdir().expect("Failed to create source cache"),
                        |cache| {
                            compile(
                                &mut compile_command(
                                    cache.path(),
                                    requirements.path(),
                                    partial,
                                    worktrees,
                                ),
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
                BenchmarkId::new("git_monorepo_warm", format!("airflow/{mode}/{count}")),
                |bencher| {
                    let requirements = airflow_requirements(&repository, count);
                    let cache = tempfile::tempdir().expect("Failed to create source cache");
                    let mut command =
                        compile_command(cache.path(), requirements.path(), partial, worktrees);
                    compile(&mut command, cache.path(), worktrees);
                    bencher.iter(|| compile(&mut command, cache.path(), worktrees));
                },
            );
        }
    }
    group.finish();
}
