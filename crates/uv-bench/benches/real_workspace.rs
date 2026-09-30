//! Discover and inspect a real Python monorepo with 127 workspace members.

mod common;

extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{is_codspeed_simulation, run_command, source_fixture, uv_command_with_cache};
use uv_cache::Cache;
use uv_workspace::{DiscoveryOptions, ProjectWorkspace, Workspace, WorkspaceCache};

fn real_workspace(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create workspace runtime");
    let cache_directory = tempfile::tempdir().expect("Failed to create cache directory");
    let cache = Cache::from_path(cache_directory.path());
    let options = DiscoveryOptions::default();
    let mut group = c.benchmark_group("real_workspace");

    for (project, expected_members) in [("flask", 1), ("airflow", 127)] {
        let root = source_fixture(project);
        let cached_workspace = WorkspaceCache::default();
        let workspace = runtime
            .block_on(Workspace::discover(
                &root,
                &options,
                &cache,
                &cached_workspace,
            ))
            .expect("Failed to discover source fixture");
        assert_eq!(workspace.packages().len(), expected_members);
        let members = workspace
            .packages()
            .values()
            .map(|member| member.root().clone())
            .collect::<Vec<_>>();

        group.bench_function(BenchmarkId::new("discover", project), |b| {
            b.iter(|| {
                black_box(
                    runtime
                        .block_on(Workspace::discover(
                            &root,
                            &options,
                            &cache,
                            &WorkspaceCache::default(),
                        ))
                        .expect("Failed to discover workspace"),
                )
            });
        });
        group.bench_function(BenchmarkId::new("discover_all_members", project), |b| {
            b.iter(|| {
                let workspace_cache = WorkspaceCache::default();
                for member in &members {
                    black_box(
                        runtime
                            .block_on(ProjectWorkspace::discover(
                                member,
                                &options,
                                &cache,
                                &workspace_cache,
                            ))
                            .expect("Failed to discover workspace member"),
                    );
                }
            });
        });
        group.bench_function(BenchmarkId::new("cached_members", project), |b| {
            b.iter(|| {
                for member in &members {
                    black_box(
                        runtime
                            .block_on(ProjectWorkspace::discover(
                                member,
                                &options,
                                &cache,
                                &cached_workspace,
                            ))
                            .expect("Failed to discover cached workspace member"),
                    );
                }
            });
        });
    }
    group.finish();
}

fn monorepo_inspection(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let root = source_fixture("airflow");
    let cache = tempfile::tempdir().expect("Failed to create inspection cache");
    let mut group = c.benchmark_group("monorepo_inspection");
    for (name, arguments) in [
        ("export", &["export", "--frozen", "--all-groups"][..]),
        ("tree", &["tree", "--frozen", "--all-groups"][..]),
        (
            "workspace_metadata",
            &["workspace", "metadata", "--frozen"][..],
        ),
    ] {
        let command = || {
            let mut command = uv_command_with_cache(cache.path());
            command.args(["--offline", "--directory"]).arg(&root);
            command.args(arguments);
            command
        };
        run_command(&mut command());
        group.bench_function(BenchmarkId::new(name, "airflow"), |b| {
            b.iter_batched(
                command,
                |mut command: Command| run_command(&mut command),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = workspaces;
    config = common::walltime_criterion();
    targets = real_workspace, monorepo_inspection
}
criterion_main!(workspaces);
