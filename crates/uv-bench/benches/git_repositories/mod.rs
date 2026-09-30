use std::hint::black_box;
use std::path::Path;

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, measurement::WallTime};
use uv_bench::{GIT_FETCH_MODES, GitFixture, is_codspeed_simulation};
use uv_git::{Fetch, GitFetchSettings, GitResolver};
use uv_git_types::{GitLfs, GitReference, GitUrl};
use uv_redacted::DisplaySafeUrl;

fn fetch(
    runtime: &tokio::runtime::Runtime,
    git: &GitUrl,
    cache: &Path,
    settings: GitFetchSettings,
    worktrees: bool,
) -> Fetch {
    let fetched = runtime
        .block_on(GitResolver::default().fetch(git, settings, cache.to_path_buf(), None))
        .expect("Failed to fetch pinned Git repository");
    assert_eq!(fetched.git().precise(), git.precise());
    assert_eq!(fetched.path().join(".git").is_file(), worktrees);
    fetched
}

pub(crate) fn benchmark(
    criterion: &mut Criterion<WallTime>,
    name: &str,
    fixtures: Vec<GitFixture>,
) {
    if is_codspeed_simulation() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let mut group = criterion.benchmark_group(name);
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    for fixture in fixtures {
        let url = DisplaySafeUrl::from_file_path(fixture.path()).expect("Invalid Git fixture URL");
        let revisions: Vec<_> = fixture
            .revisions
            .iter()
            .map(|revision| {
                GitUrl::from_fields(
                    url.clone(),
                    GitReference::from_rev(revision.commit.clone()),
                    Some(revision.commit.parse().expect("Invalid Git commit")),
                    GitLfs::Disabled,
                )
                .expect("Invalid Git URL")
            })
            .collect();
        let precise = revisions.last().expect("Missing Git revision history");
        for &(mode, partial, worktrees) in GIT_FETCH_MODES {
            let settings = GitFetchSettings::default()
                .with_offline(true)
                .with_partial_fetches(partial)
                .with_worktrees(worktrees);
            // One revision is a fresh install; four revisions model successive upgrades
            // sharing the same developer or CI cache.
            for selected in [std::slice::from_ref(precise), revisions.as_slice()] {
                group.bench_function(
                    BenchmarkId::new(
                        "revision_history",
                        format!("{}/{mode}/{}", fixture.name, selected.len()),
                    ),
                    |bencher| {
                        bencher.iter_batched(
                            || tempfile::tempdir().expect("Failed to create Git cache"),
                            |cache| {
                                for git in selected {
                                    black_box(fetch(
                                        &runtime,
                                        git,
                                        cache.path(),
                                        settings,
                                        worktrees,
                                    ));
                                }
                                black_box(cache)
                            },
                            BatchSize::PerIteration,
                        );
                    },
                );
            }
            group.bench_function(
                BenchmarkId::new("warm_precise", format!("{}/{mode}", fixture.name)),
                |bencher| {
                    let cache = tempfile::tempdir().expect("Failed to create Git cache");
                    fetch(&runtime, precise, cache.path(), settings, worktrees);
                    bencher.iter(|| {
                        black_box(fetch(&runtime, precise, cache.path(), settings, worktrees))
                    });
                },
            );
        }
    }
    group.finish();
}
