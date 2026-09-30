//! Extended Git fetch measurements for multi-gigabyte repository histories.

mod common;
mod git_repositories;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::large_git_fixtures;

fn git_fetch_large(criterion: &mut Criterion<WallTime>) {
    git_repositories::benchmark(criterion, "git_fetch_large", large_git_fixtures());
}

criterion_group! {
    name = repositories;
    config = common::walltime_criterion();
    targets = git_fetch_large
}
criterion_main!(repositories);
