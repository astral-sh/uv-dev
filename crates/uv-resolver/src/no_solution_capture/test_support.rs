use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use papaya::HashMap;
use pubgrub::{Derived, External, Map, Term};
use rustc_hash::FxHashMap;
use uv_distribution_types::{IndexCapabilities, IndexLocations, RequiresPython};
use uv_normalize::PackageName;
use uv_pep440::{MIN_VERSION, Version};
use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder, MarkerTree};

use crate::PythonRequirement;
use crate::error::ErrorTree;
use crate::fork_indexes::ForkIndexes;
use crate::fork_urls::ForkUrls;
use crate::pubgrub::{PubGrubPackage, PubGrubPackageInner, Range};
use crate::resolver::{
    InMemoryIndex, Indexes, MetadataUnavailable, ResolverEnvironment, UnavailablePackage, Urls,
};

use super::producer::CaptureContext;
use super::{CaptureMetadata, CaptureOperation, CaptureOptions, CaptureScope, CaptureToken};

static MARKER_ENV: LazyLock<MarkerEnvironment> = LazyLock::new(|| {
    MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
        implementation_name: "cpython",
        implementation_version: "3.12.5",
        os_name: "posix",
        platform_machine: "x86_64",
        platform_python_implementation: "CPython",
        platform_release: "6.8.0",
        platform_system: "Linux",
        platform_version: "capture-test",
        python_full_version: "3.12.5",
        python_version: "3.12",
        sys_platform: "linux",
    })
    .expect("valid marker environment")
});

pub(super) fn token() -> CaptureToken {
    CaptureToken::new("0123456789abcdef0123456789abcdef", 1234).expect("valid capture token")
}

pub(super) fn options() -> CaptureOptions {
    token().for_lock(
        CaptureScope::Workspace,
        CaptureOperation::Write,
        CaptureMetadata::Standard,
    )
}

pub(super) fn package_name(name: &str) -> PackageName {
    name.parse().expect("valid package name")
}

pub(super) fn version(version: &str) -> Version {
    version.parse().expect("valid version")
}

pub(super) fn marker(marker: &str) -> MarkerTree {
    marker.parse().expect("valid marker")
}

pub(super) fn marker_environment() -> MarkerEnvironment {
    MARKER_ENV.clone()
}

pub(super) fn package(name: &str) -> PubGrubPackage {
    PubGrubPackageInner::Package {
        name: package_name(name),
        extra: None,
        group: None,
        marker: MarkerTree::TRUE,
    }
    .into()
}

fn root() -> PubGrubPackage {
    PubGrubPackageInner::Root(Some(package_name("capture-project"))).into()
}

pub(super) fn basic_tree() -> ErrorTree {
    let package = package("a");
    ErrorTree::Derived(Derived {
        terms: Map::from_iter([
            (
                root(),
                Term::Negative(Range::singleton(MIN_VERSION.clone())),
            ),
            (package.clone(), Term::Positive(Range::full())),
        ]),
        shared_id: None,
        cause1: Arc::new(ErrorTree::External(External::FromDependencyOf(
            root(),
            Range::singleton(MIN_VERSION.clone()),
            package.clone(),
            Range::full(),
        ))),
        cause2: Arc::new(ErrorTree::External(External::NoVersions(
            package,
            Range::full(),
        ))),
    })
}

pub(super) struct Fixture {
    pub(super) project: PackageName,
    pub(super) workspace_members: BTreeSet<PackageName>,
    pub(super) environment: ResolverEnvironment,
    pub(super) original_python: PythonRequirement,
    pub(super) effective_python: PythonRequirement,
    pub(super) index: InMemoryIndex,
    pub(super) index_locations: IndexLocations,
    pub(super) index_capabilities: IndexCapabilities,
    pub(super) urls: Urls,
    pub(super) indexes: Indexes,
    pub(super) fork_urls: ForkUrls,
    pub(super) fork_indexes: ForkIndexes,
    pub(super) known_versions: FxHashMap<PackageName, Arc<[Version]>>,
    pub(super) unavailable_packages: HashMap<PackageName, UnavailablePackage>,
    pub(super) incomplete_packages: HashMap<PackageName, HashMap<Version, MetadataUnavailable>>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let python = PythonRequirement::from_marker_environment(
            &MARKER_ENV,
            RequiresPython::from_specifiers(
                ">=3.12,<3.15".parse().expect("valid Python specifiers"),
            ),
        );
        Self {
            project: package_name("capture-project"),
            workspace_members: BTreeSet::from([package_name("capture-project")]),
            environment: ResolverEnvironment::universal(Vec::new()),
            original_python: python.clone(),
            effective_python: python,
            index: InMemoryIndex::default(),
            index_locations: IndexLocations::default(),
            index_capabilities: IndexCapabilities::default(),
            urls: Urls::default(),
            indexes: Indexes::default(),
            fork_urls: ForkUrls::default(),
            fork_indexes: ForkIndexes::default(),
            known_versions: FxHashMap::default(),
            unavailable_packages: HashMap::new(),
            incomplete_packages: HashMap::new(),
        }
    }

    pub(super) fn context<'a>(&'a self, error: &'a ErrorTree) -> CaptureContext<'a> {
        CaptureContext {
            error,
            project: Some(&self.project),
            workspace_members: &self.workspace_members,
            environment: &self.environment,
            original_python: &self.original_python,
            effective_python: &self.effective_python,
            index: &self.index,
            index_locations: &self.index_locations,
            index_capabilities: &self.index_capabilities,
            urls: &self.urls,
            indexes: &self.indexes,
            fork_urls: &self.fork_urls,
            fork_indexes: &self.fork_indexes,
            known_versions: &self.known_versions,
            unavailable_packages: &self.unavailable_packages,
            incomplete_packages: &self.incomplete_packages,
        }
    }
}
