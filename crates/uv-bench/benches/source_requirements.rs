//! Read real project manifests and extract their published dependency requirements.

mod common;

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::fixture_path;
use uv_cache::Cache;
use uv_client::{BaseClientBuilder, Connectivity, RegistryClient, RegistryClientBuilder};
use uv_configuration::{
    BuildOptions, Concurrency, Constraints, ExtrasSpecification, IndexStrategy, NoSources,
};
use uv_dispatch::{BuildDispatch, SharedState};
use uv_distribution::DistributionDatabase;
use uv_distribution_types::{
    ConfigSettings, DependencyMetadata, ExtraBuildRequires, ExtraBuildVariables, IndexLocations,
    PackageConfigSettings,
};
use uv_install_wheel::LinkMode;
use uv_preview::Preview;
use uv_python::{Interpreter, PythonEnvironment};
use uv_requirements::{
    RequirementsSource, RequirementsSpecification, SourceTreeResolution, SourceTreeResolver,
};
use uv_resolver::{ExcludeNewer, FlatIndex, InMemoryIndex};
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

async fn requirements(
    source: &RequirementsSource,
    client_builder: &BaseClientBuilder<'_>,
    client: &RegistryClient,
    cache: &Cache,
    interpreter: &Interpreter,
) -> anyhow::Result<Vec<SourceTreeResolution>> {
    let specification = RequirementsSpecification::from_source(source, client_builder).await?;
    let build_constraints = Constraints::default();
    let locations = IndexLocations::default();
    let flat_index = FlatIndex::default();
    let metadata = DependencyMetadata::default();
    let config = ConfigSettings::default();
    let package_config = PackageConfigSettings::default();
    let build_requires = ExtraBuildRequires::default();
    let build_variables = ExtraBuildVariables::default();
    let build_options = BuildOptions::default();
    let hashes = HashStrategy::default();
    let concurrency = Concurrency::default();
    let extras = ExtrasSpecification::default();
    let index = InMemoryIndex::default();
    let context = BuildDispatch::new(
        client,
        cache,
        &build_constraints,
        interpreter,
        &locations,
        &flat_index,
        &metadata,
        SharedState::default(),
        IndexStrategy::default(),
        &config,
        &package_config,
        BuildIsolation::default(),
        &build_requires,
        &build_variables,
        LinkMode::default(),
        &build_options,
        &hashes,
        ExcludeNewer::default(),
        NoSources::All,
        SourceTreeEditablePolicy::Project,
        WorkspaceCache::default(),
        concurrency.clone(),
        Preview::default(),
    );
    SourceTreeResolver::new(
        &extras,
        &hashes,
        &index,
        DistributionDatabase::new(client, &context, concurrency.downloads_semaphore),
    )
    .resolve(specification.source_trees.iter())
    .await
}

fn source_requirements(c: &mut Criterion<WallTime>) {
    uv_preview::set(Preview::default()).expect("Failed to configure preview features");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let cache = Cache::from_path("../../.cache")
        .init_no_wait()
        .expect("Unexpected benchmark cache contention")
        .expect("Failed to initialize cache");
    let interpreter = PythonEnvironment::from_root("../../.venv", &cache)
        .expect("Missing benchmark Python environment")
        .into_interpreter();
    let client_builder = BaseClientBuilder::default().connectivity(Connectivity::Offline);
    let client = RegistryClientBuilder::new(client_builder.clone(), cache.clone())
        .build()
        .expect("Failed to create registry client");
    let mut group = c.benchmark_group("source_requirements");
    for name in ["packse", "uv", "prefect"] {
        let directory = tempfile::tempdir().expect("Failed to create project directory");
        let project = directory.path().join("pyproject.toml");
        fs_err::copy(fixture_path(&format!("{name}.pyproject.toml")), &project)
            .expect("Failed to copy project metadata");
        let source = RequirementsSource::PyprojectToml(project);
        let expected = runtime
            .block_on(requirements(
                &source,
                &client_builder,
                &client,
                &cache,
                &interpreter,
            ))
            .expect("Failed to extract project requirements");
        assert_eq!(expected.len(), 1);
        group.bench_function(BenchmarkId::new("no_sources", name), |b| {
            b.iter(|| {
                black_box(
                    runtime
                        .block_on(requirements(
                            black_box(&source),
                            &client_builder,
                            &client,
                            &cache,
                            &interpreter,
                        ))
                        .expect("Failed to extract project requirements"),
                )
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = sources;
    config = common::walltime_criterion();
    targets = source_requirements
}
criterion_main!(sources);
