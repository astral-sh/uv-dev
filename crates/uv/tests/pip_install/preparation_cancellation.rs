use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::poll;
use tokio::sync::Semaphore;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_configuration::{BuildOptions, Concurrency, Constraints, IndexStrategy, NoSources};
use uv_dispatch::{BuildDispatch, SharedState};
use uv_distribution::DistributionDatabase;
use uv_distribution_types::{
    BuiltDist, ConfigSettings, DependencyMetadata, DirectUrlBuiltDist, Dist, ExtraBuildRequires,
    ExtraBuildVariables, Identifier, IndexLocations, PackageConfigSettings, Resolution,
};
use uv_installer::Preparer;
use uv_pep508::VerbatimUrl;
use uv_preview::Preview;
use uv_python_interpreter::Interpreter;
use uv_redacted::DisplaySafeUrl;
use uv_resolver::{ExcludeNewer, FlatIndex};
use uv_types::{BuildIsolation, HashStrategy, InFlight, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

#[tokio::test]
async fn cancelled_preparation_can_reuse_in_flight_state() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let cache = Cache::temp()?.init().await?;
    let executable = context.interpreter();
    let _features = uv_preview::test::with_features(&[]);
    let interpreter = Interpreter::query(executable, &cache)?;
    let server = MockServer::start().await;
    let wheel = fs_err::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/links/ok-1.0.0-py3-none-any.whl"),
    )?;
    Mock::given(method("GET"))
        .and(path("/ok-1.0.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .expect(1)
        .mount(&server)
        .await;
    let url = format!("{}/ok-1.0.0-py3-none-any.whl", server.uri());
    let distribution = Arc::new(Dist::Built(BuiltDist::DirectUrl(DirectUrlBuiltDist {
        filename: "ok-1.0.0-py3-none-any.whl".parse()?,
        location: Box::new(DisplaySafeUrl::parse(&url)?),
        url: VerbatimUrl::from_str(&url)?,
        size: None,
    })));
    let id = distribution.distribution_id();
    let client = RegistryClientBuilder::new(BaseClientBuilder::default(), cache.clone()).build()?;
    let constraints = Constraints::default();
    let locations = IndexLocations::default();
    let flat_index = FlatIndex::default();
    let metadata = DependencyMetadata::default();
    let config_settings = ConfigSettings::default();
    let package_settings = PackageConfigSettings::default();
    let extra_requires = ExtraBuildRequires::default();
    let extra_variables = ExtraBuildVariables::default();
    let build_options = BuildOptions::default();
    let hashes = HashStrategy::default();
    let dispatch = BuildDispatch::new(
        &client,
        &cache,
        &constraints,
        &interpreter,
        &locations,
        &flat_index,
        &metadata,
        SharedState::default(),
        IndexStrategy::default(),
        &config_settings,
        &package_settings,
        BuildIsolation::Isolated,
        &extra_requires,
        &extra_variables,
        uv_install_wheel::LinkMode::default(),
        &build_options,
        &hashes,
        ExcludeNewer::default(),
        NoSources::None,
        SourceTreeEditablePolicy::Project,
        WorkspaceCache::default(),
        Concurrency::default(),
        Preview::default(),
    );
    let downloads = Arc::new(Semaphore::new(0));
    let database = DistributionDatabase::new(&client, &dispatch, downloads.clone());
    let preparer = Preparer::new(
        &cache,
        interpreter.tags()?,
        &hashes,
        &build_options,
        database,
    );
    let in_flight = InFlight::default();
    let resolution = Resolution::default();
    let mut first = Box::pin(preparer.prepare(vec![distribution.clone()], &in_flight, &resolution));
    assert!(poll!(first.as_mut()).is_pending());
    let mut waiting = Box::pin(in_flight.downloads.register_or_wait(&id));
    assert!(poll!(waiting.as_mut()).is_pending());
    drop(waiting);
    drop(first);
    downloads.add_permits(1);
    let wheels = tokio::time::timeout(
        Duration::from_secs(10),
        preparer.prepare(vec![distribution.clone()], &in_flight, &resolution),
    )
    .await??;
    assert_eq!(wheels.len(), 1);
    assert!(
        in_flight
            .downloads
            .get(&id)
            .is_some_and(|result| result.is_ok())
    );
    let cached = preparer
        .prepare(vec![distribution], &in_flight, &resolution)
        .await?;
    assert_eq!(cached.len(), 1);
    Ok(())
}
