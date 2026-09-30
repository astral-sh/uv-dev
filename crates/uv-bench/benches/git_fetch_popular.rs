//! Fetch and upgrade popular Python Git dependencies with independent preview settings.

mod common;
mod git_repositories;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::popular_git_fixtures;

fn git_fetch_popular(criterion: &mut Criterion<WallTime>) {
    git_repositories::benchmark(
        criterion,
        "git_fetch_popular",
        popular_git_fixtures()
            .into_iter()
            .filter(|fixture| !fixture.revisions.is_empty())
            .collect(),
    );
}

criterion_group! {
    name = repositories;
    config = common::walltime_criterion();
    targets = git_fetch_popular
}
criterion_main!(repositories);
