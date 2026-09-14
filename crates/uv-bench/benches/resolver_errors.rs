//! Render fresh resolution failures using real package releases and Python constraints.

mod common;
#[path = "common/resolver.rs"]
mod resolver;

use std::hint::black_box;
use std::str::FromStr;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use serde::Deserialize;
use uv_cache::Cache;
use uv_client::{BaseClientBuilder, Connectivity, RegistryClient, RegistryClientBuilder};
use uv_distribution_types::{IndexLocations, Requirement};
use uv_preview::Preview;
use uv_python::{Interpreter, PythonEnvironment, PythonVersion};
use uv_resolver::{ExcludeNewer, Manifest, NoSolutionError, ResolveError};

#[derive(Deserialize)]
struct Fixture {
    name: String,
    python: PythonVersion,
    requirements: Vec<String>,
}

async fn no_solution(
    manifest: &Manifest,
    cache: &Cache,
    client: &RegistryClient,
    interpreter: &Interpreter,
    settings: &resolver::Settings,
) -> anyhow::Result<Box<NoSolutionError>> {
    match resolver::resolve(
        manifest.clone(),
        cache.clone(),
        client,
        interpreter,
        settings,
    )
    .await
    {
        Err(ResolveError::NoSolution(error)) => Ok(error),
        Err(error) => Err(error.into()),
        Ok(_) => anyhow::bail!("The benchmark requirements unexpectedly resolved"),
    }
}

fn resolver_errors(c: &mut Criterion<WallTime>) {
    uv_preview::set(Preview::default()).expect("Failed to configure preview features");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(256)
        .enable_all()
        .build()
        .expect("Failed to create resolver runtime");
    let cache = Cache::from_path("../../.cache")
        .init_no_wait()
        .expect("Unexpected benchmark cache contention")
        .expect("Failed to initialize cache");
    let interpreter = PythonEnvironment::from_root("../../.venv", &cache)
        .expect("Missing benchmark Python environment")
        .into_interpreter();
    let client = RegistryClientBuilder::new(
        BaseClientBuilder::default().connectivity(Connectivity::Offline),
        cache.clone(),
    )
    .build()
    .expect("Failed to create registry client");
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!(
        "../../../scripts/benchmark/resolver-errors.json"
    ))
    .expect("Invalid resolver-error fixtures");
    let mut group = c.benchmark_group("resolver_errors");
    for fixture in fixtures {
        let manifest = Manifest::simple(
            fixture
                .requirements
                .iter()
                .map(|requirement| {
                    Requirement::from(
                        uv_pep508::Requirement::from_str(requirement)
                            .expect("Invalid fixture requirement"),
                    )
                })
                .collect(),
        );
        let settings = resolver::Settings {
            universal: false,
            python_version: Some(fixture.python),
            exclude_newer: ExcludeNewer::global(
                jiff::civil::date(2024, 12, 1)
                    .to_zoned(jiff::tz::TimeZone::UTC)
                    .expect("Invalid fixture cutoff")
                    .timestamp()
                    .into(),
            ),
            index_locations: IndexLocations::default(),
        };
        let resolve = || {
            runtime
                .block_on(no_solution(
                    &manifest,
                    &cache,
                    &client,
                    &interpreter,
                    &settings,
                ))
                .expect("Run prepare-resolver-errors.py before benchmarking")
        };
        assert!(resolve().to_string().contains("unsatisfiable"));
        group.bench_function(BenchmarkId::new("render", &fixture.name), |b| {
            // Reports are cached on the error. Each sample needs a new, unformatted error.
            b.iter_batched(
                &resolve,
                |error| black_box(error.to_string()),
                BatchSize::SmallInput,
            );
        });
        group.bench_function(BenchmarkId::new("resolve_and_render", &fixture.name), |b| {
            b.iter(|| black_box(resolve().to_string()));
        });
    }
    group.finish();
}

criterion_group! {
    name = errors;
    config = common::walltime_criterion();
    targets = resolver_errors
}
criterion_main!(errors);
