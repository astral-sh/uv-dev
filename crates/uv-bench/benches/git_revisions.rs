//! Format ordinary Git dependency revisions and their source-cache paths.

extern crate uv_performance_memory_allocator;

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::git_fixtures;
use uv_cache::WheelCache;
use uv_git_types::{GitLfs, GitOid, GitReference, GitUrl};
use uv_redacted::DisplaySafeUrl;

fn git_revisions(c: &mut Criterion<WallTime>) {
    let fixtures = git_fixtures();
    let sample = fixtures
        .iter()
        .find(|fixture| fixture.name == "sampleproject")
        .expect("Missing sampleproject Git fixture");
    let flask = fixtures
        .iter()
        .find(|fixture| fixture.name == "flask")
        .expect("Missing Flask Git fixture");
    let mut group = c.benchmark_group("git_revisions");
    for (name, repository, reference) in [
        (
            "full_commit",
            &sample.repository,
            GitReference::from_rev(sample.commit.clone()),
        ),
        (
            "short_commit",
            &sample.repository,
            GitReference::from_rev(sample.commit[..7].to_owned()),
        ),
        (
            "branch",
            &sample.repository,
            GitReference::Branch(
                sample
                    .reference
                    .trim_start_matches("refs/heads/")
                    .to_owned(),
            ),
        ),
        (
            "release_tag",
            &flask.repository,
            GitReference::Tag(flask.reference.trim_start_matches("refs/tags/").to_owned()),
        ),
        (
            "pull_request",
            &sample.repository,
            GitReference::NamedRef("refs/pull/219/head".to_owned()),
        ),
    ] {
        let url = GitUrl::from_fields(
            DisplaySafeUrl::parse(repository).expect("Invalid Git repository URL"),
            reference.clone(),
            None,
            GitLfs::Disabled,
        )
        .expect("Invalid Git URL");
        group.bench_function(format!("encode/{name}"), |b| {
            b.iter(|| black_box(&reference).as_url_rev());
        });
        group.bench_function(format!("url/{name}"), |b| {
            b.iter_batched(
                || url.clone(),
                |url| black_box(DisplaySafeUrl::from(url)),
                BatchSize::SmallInput,
            );
        });
    }
    for fixture in fixtures
        .iter()
        .filter(|fixture| fixture.name != "pip-test-package")
    {
        let repository =
            DisplaySafeUrl::parse(&fixture.repository).expect("Invalid Git repository URL");
        let commit: GitOid = fixture.commit.parse().expect("Invalid Git commit");
        group.bench_function(format!("cache_key/{}", fixture.name), |b| {
            b.iter(|| {
                WheelCache::Git(black_box(&repository), black_box(&commit).as_short_str()).root()
            });
        });
    }
    group.finish();
}

criterion_group!(revisions, git_revisions);
criterion_main!(revisions);
