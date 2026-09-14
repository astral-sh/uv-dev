use std::sync::LazyLock;

use uv_cache::Cache;
use uv_client::RegistryClient;
use uv_configuration::{BuildOptions, Concurrency, Constraints, IndexStrategy, NoSources};
use uv_dispatch::{BuildDispatch, SharedState};
use uv_distribution::DistributionDatabase;
use uv_distribution_types::{
    ConfigSettings, DependencyMetadata, ExtraBuildRequires, ExtraBuildVariables, IndexLocations,
    PackageConfigSettings, RequiresPython,
};
use uv_install_wheel::LinkMode;
use uv_pep440::Version;
use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};
use uv_platform_tags::{Arch, Os, Platform, Tags, TagsOptions};
use uv_preview::Preview;
use uv_pypi_types::{Conflicts, ResolverMarkerEnvironment};
use uv_python::{Interpreter, PythonVersion};
use uv_resolver::{
    ExcludeNewer, FlatIndex, InMemoryIndex, Manifest, OptionsBuilder, PythonRequirement,
    ResolveError, Resolver, ResolverEnvironment, ResolverOutput,
};
use uv_types::{BuildIsolation, EmptyInstalledPackages, HashStrategy, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

static MARKERS: LazyLock<MarkerEnvironment> = LazyLock::new(|| {
    MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
        implementation_name: "cpython",
        implementation_version: "3.11.5",
        os_name: "posix",
        platform_machine: "arm64",
        platform_python_implementation: "CPython",
        platform_release: "21.6.0",
        platform_system: "Darwin",
        platform_version: "Darwin Kernel Version 21.6.0: Mon Aug 22 20:19:52 PDT 2022; root:xnu-8020.140.49~2/RELEASE_ARM64_T6000",
        python_full_version: "3.11.5",
        python_version: "3.11",
        sys_platform: "darwin",
    }).unwrap()
});

static PLATFORM: Platform = Platform::new(
    Os::Macos {
        major: 21,
        minor: 6,
    },
    Arch::Aarch64,
);

static TAGS: LazyLock<Tags> = LazyLock::new(|| {
    Tags::from_env(
        PLATFORM.clone(),
        (3, 11),
        "cpython",
        (3, 11),
        TagsOptions::default(),
    )
    .unwrap()
});

pub(crate) struct Settings {
    pub(crate) universal: bool,
    pub(crate) python_version: Option<PythonVersion>,
    pub(crate) exclude_newer: ExcludeNewer,
    pub(crate) index_locations: IndexLocations,
}

pub(crate) async fn resolve(
    manifest: Manifest,
    cache: Cache,
    client: &RegistryClient,
    interpreter: &Interpreter,
    settings: &Settings,
) -> Result<ResolverOutput, ResolveError> {
    let build_isolation = BuildIsolation::default();
    let extra_build_requires = ExtraBuildRequires::default();
    let extra_build_variables = ExtraBuildVariables::default();
    let build_options = BuildOptions::default();
    let concurrency = Concurrency::default();
    let config_settings = ConfigSettings::default();
    let config_settings_package = PackageConfigSettings::default();
    let exclude_newer = settings.exclude_newer.clone();
    let build_constraints = Constraints::default();
    let flat_index = FlatIndex::default();
    let hashes = HashStrategy::default();
    let state = SharedState::default();
    let index = InMemoryIndex::default();
    let installed_packages = EmptyInstalledPackages;
    let options = OptionsBuilder::new()
        .exclude_newer(exclude_newer.clone())
        .build();
    let sources = NoSources::default();
    let dependency_metadata = DependencyMetadata::default();
    let conflicts = Conflicts::empty();
    let workspace_cache = WorkspaceCache::default();

    let python_requirement = if settings.universal {
        PythonRequirement::from_requires_python(
            interpreter,
            RequiresPython::greater_than_equal_version(&Version::new([3, 11])),
        )
    } else if let Some(version) = &settings.python_version {
        PythonRequirement::from_python_version(interpreter, version)
    } else {
        PythonRequirement::from_interpreter(interpreter)
    };

    let build_context = BuildDispatch::new(
        client,
        &cache,
        &build_constraints,
        interpreter,
        &settings.index_locations,
        &flat_index,
        &dependency_metadata,
        state,
        IndexStrategy::default(),
        &config_settings,
        &config_settings_package,
        build_isolation,
        &extra_build_requires,
        &extra_build_variables,
        LinkMode::default(),
        &build_options,
        &hashes,
        exclude_newer,
        sources,
        SourceTreeEditablePolicy::Project,
        workspace_cache,
        concurrency.clone(),
        Preview::default(),
    );

    let markers = if settings.universal {
        ResolverEnvironment::universal(vec![])
    } else {
        let markers = settings.python_version.as_ref().map_or_else(
            || MARKERS.clone(),
            |version| version.markers(MARKERS.clone()),
        );
        ResolverEnvironment::specific(ResolverMarkerEnvironment::from(markers))
    };
    let tags = settings.python_version.as_ref().map(|version| {
        let version = (version.major(), version.minor());
        Tags::from_env(
            PLATFORM.clone(),
            version,
            "cpython",
            version,
            TagsOptions::default(),
        )
        .expect("Invalid benchmark Python tags")
    });

    let resolver = Resolver::new(
        manifest,
        options,
        &python_requirement,
        markers,
        interpreter.markers(),
        conflicts,
        Some(tags.as_ref().unwrap_or(&TAGS)),
        &flat_index,
        &index,
        &hashes,
        &build_context,
        installed_packages,
        DistributionDatabase::new(
            client,
            &build_context,
            concurrency.downloads_semaphore.clone(),
        ),
    )?;

    resolver.resolve().await
}
