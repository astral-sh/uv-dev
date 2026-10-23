use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, Connectivity, RegistryClientBuilder};
use uv_configuration::{
    BuildKind, BuildOptions, BuildOutput, Concurrency, Constraints, ExcludeNewer, IndexStrategy,
    NoSources,
};
use uv_dispatch::{BuildDispatch, SharedState};
use uv_distribution_types::{
    ConfigSettings, DependencyMetadata, ExtraBuildRequires, ExtraBuildVariables, IndexLocations,
    PackageConfigSettings,
};
use uv_install_wheel::LinkMode;
use uv_preview::Preview;
use uv_python_interpreter::PythonEnvironment;
use uv_resolver::FlatIndex;
use uv_types::{
    BuildContext, BuildIsolation, BuildStack, HashStrategy, SourceBuildTrait,
    SourceTreeEditablePolicy,
};
use uv_workspace::WorkspaceCache;

#[tokio::test]
async fn repeated_metadata_without_hook() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let _features = uv_preview::test::with_features(&[]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "demo"
        version = "1.0.0"

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            raise AssertionError("metadata preparation must not build a wheel")
    "#})?;

    let cache = Cache::from_path(context.cache_dir.path()).init().await?;
    let environment = PythonEnvironment::from_root(&context.venv, &cache)?;
    let client = RegistryClientBuilder::new(
        BaseClientBuilder::default().connectivity(Connectivity::Offline),
        cache.clone(),
    )
    .build()?;
    let constraints = Constraints::default();
    let indexes = IndexLocations::default();
    let flat_index = FlatIndex::default();
    let metadata = DependencyMetadata::default();
    let config = ConfigSettings::default();
    let package_config = PackageConfigSettings::default();
    let extra_requires = ExtraBuildRequires::default();
    let extra_variables = ExtraBuildVariables::default();
    let options = BuildOptions::default();
    let hashes = HashStrategy::default();
    let sources = NoSources::default();
    let dispatch = BuildDispatch::new(
        &client,
        &cache,
        &constraints,
        environment.interpreter(),
        &indexes,
        &flat_index,
        &metadata,
        SharedState::default(),
        IndexStrategy::default(),
        &config,
        &package_config,
        BuildIsolation::Shared(&environment),
        &extra_requires,
        &extra_variables,
        LinkMode::default(),
        &options,
        &hashes,
        ExcludeNewer::default(),
        sources.clone(),
        SourceTreeEditablePolicy::Project,
        WorkspaceCache::default(),
        Concurrency::default(),
        Preview::default(),
    );
    let mut builder = dispatch
        .setup_build(
            context.temp_dir.path(),
            None,
            context.temp_dir.path(),
            None,
            None,
            None,
            &sources,
            BuildKind::Wheel,
            BuildOutput::Quiet,
            BuildStack::default(),
        )
        .await?;
    assert_eq!(builder.metadata().await?, None);
    assert_eq!(builder.metadata().await?, None);
    Ok(())
}
