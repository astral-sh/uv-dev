//! Fetch and reuse real, pinned Git source trees through the local Git transport.

mod common;

use std::hint::black_box;
use std::path::Path;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{git_fixtures, is_codspeed_simulation};
use uv_git::{Fetch, GitHttpSettings, GitResolver};
use uv_git_types::{GitLfs, GitOid, GitReference, GitUrl};
use uv_redacted::DisplaySafeUrl;

fn fetch(runtime: &tokio::runtime::Runtime, git: &GitUrl, cache: &Path) -> Fetch {
    runtime
        .block_on(GitResolver::default().fetch(
            git,
            GitHttpSettings::default().with_offline(true),
            cache.to_path_buf(),
            None,
        ))
        .expect("Failed to fetch pinned Git repository")
}

fn git_fetch(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let mut group = c.benchmark_group("git_fetch");
    group.sampling_mode(SamplingMode::Flat);
    for fixture in git_fixtures()
        .into_iter()
        .filter(|fixture| fixture.name != "pip-test-package")
    {
        let commit: GitOid = fixture.commit.parse().expect("Invalid Git commit");
        let unresolved = GitUrl::from_fields(
            DisplaySafeUrl::from_file_path(fixture.path()).expect("Invalid Git fixture URL"),
            GitReference::from_rev(fixture.commit),
            None,
            GitLfs::Disabled,
        )
        .expect("Invalid Git URL");
        let precise = unresolved
            .clone()
            .with_precise(commit)
            .expect("Invalid precise Git URL");
        group.bench_function(BenchmarkId::new("cold_precise", &fixture.name), |b| {
            b.iter_batched(
                || tempfile::tempdir().expect("Failed to create Git cache"),
                |cache| {
                    let fetched = fetch(&runtime, &precise, cache.path());
                    assert_eq!(fetched.git().precise(), Some(commit));
                    black_box((cache, fetched))
                },
                BatchSize::PerIteration,
            );
        });

        let cache = tempfile::tempdir().expect("Failed to create Git cache");
        assert_eq!(
            fetch(&runtime, &precise, cache.path()).git().precise(),
            Some(commit)
        );
        let reference = precise.clone().with_reference(GitReference::from_rev(
            fixture
                .reference
                .trim_start_matches("refs/heads/")
                .trim_start_matches("refs/tags/")
                .to_owned(),
        ));
        for (name, git) in [
            ("warm_precise", &precise),
            ("warm_commit", &unresolved),
            ("warm_reference", &reference),
        ] {
            group.bench_function(BenchmarkId::new(name, &fixture.name), |b| {
                b.iter(|| black_box(fetch(&runtime, git, cache.path())));
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = repositories;
    config = common::walltime_criterion();
    targets = git_fetch
}
criterion_main!(repositories);
