use std::sync::Arc;

use tokio::sync::Semaphore;
use url::Url;

use uv_cache::Cache;
use uv_client::{
    AuthIntegration, BaseClientBuilder, Connectivity, MetadataFormat, RegistryClient,
    RegistryClientBuilder,
};
use uv_configuration::{BuildOptions, KeyringProviderType};
use uv_distribution_types::Requirement;
use uv_pep508::{MarkerEnvironmentBuilder, Pep508Url, VerbatimUrl};
use uv_pypi_types::VerbatimParsedUrl;
use uv_types::HashStrategy;

use crate::Manifest;
use crate::resolver::UnsatisfiableRequirement;
use crate::version_map::VersionMap;
use crate::yanks::AllowedYanks;

use super::*;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct LocalIndex {
    cache: Cache,
    client: RegistryClient,
}

impl LocalIndex {
    fn new() -> TestResult<Self> {
        let cache = Cache::temp()?;
        let client = RegistryClientBuilder::new(
            BaseClientBuilder::default()
                .connectivity(Connectivity::Offline)
                .keyring(KeyringProviderType::Disabled)
                .auth_integration(AuthIntegration::NoAuthMiddleware),
            cache.clone(),
        )
        .build()?;
        Ok(Self { cache, client })
    }

    async fn version_map(
        &self,
        name: &PackageName,
        slot: &str,
        status: Option<&str>,
        origin: Option<&IndexUrl>,
    ) -> TestResult<(IndexUrl, VersionMap)> {
        let root = self.cache.root().join("local-index").join(slot);
        let directory = root.join(name.as_ref());
        fs_err::create_dir_all(&directory)?;
        let status = status
            .map(|status| format!(r#"<meta name="pypi:project-status" content="{status}">"#))
            .unwrap_or_default();
        // This is literal metadata, not an executable package or a downloadable artifact.
        let html = format!(
            r#"<!doctype html><html><head>{status}<meta name="pypi:project-status-reason" content="DO-NOT-DISPLAY https://user:secret@example.invalid/?token=private#fragment"></head><body></body></html>"#
        );
        fs_err::write(directory.join("index.html"), html)?;
        let local_index = IndexUrl::from(VerbatimUrl::from_url(
            Url::from_directory_path(&root)
                .expect("the owned cache path is absolute")
                .into(),
        ));
        assert!(matches!(&local_index, IndexUrl::Path(_)));
        let mut responses = self
            .client
            .simple_detail(
                name,
                Some((&local_index).into()),
                &IndexCapabilities::default(),
                &Semaphore::new(1),
            )
            .await?;
        assert_eq!(responses.len(), 1);
        let (observed_index, metadata) = responses.pop().expect("one local metadata response");
        assert_eq!(observed_index, &local_index);
        let MetadataFormat::Simple(metadata) = metadata else {
            return Err("expected Simple API metadata".into());
        };
        // A separately authored origin can exercise credential-safe formatting without making
        // a request to that URL. The archive itself always comes from the real local reader.
        let index = origin.cloned().unwrap_or_else(|| observed_index.clone());
        let version_map = VersionMap::from_simple_metadata(
            metadata,
            name,
            index.clone(),
            None,
            RequiresPython::greater_than_equal_version(&Version::new([3_u64, 12])),
            AllowedYanks::default(),
            HashStrategy::default(),
            None,
            None,
            None,
            &BuildOptions::default(),
        );
        assert_eq!(version_map.versions().count(), 0);
        assert_eq!(version_map.index(), Some(&index));
        Ok((index, version_map))
    }
}

struct HintFixture {
    current_environment: MarkerEnvironment,
    environment: ResolverEnvironment,
    python_requirement: PythonRequirement,
    versions: FxHashMap<PackageName, BTreeSet<Version>>,
    workspace_members: BTreeSet<PackageName>,
    index: InMemoryIndex,
    available_indexes: FxHashMap<PackageName, BTreeSet<IndexUrl>>,
    unavailable_packages: FxHashMap<PackageName, UnavailablePackage>,
    fork_urls: ForkUrls,
    fork_indexes: ForkIndexes,
}

impl HintFixture {
    fn new() -> Self {
        let current_environment = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: "3.12.0",
            os_name: "posix",
            platform_machine: "x86_64",
            platform_python_implementation: "CPython",
            platform_release: "",
            platform_system: "Linux",
            platform_version: "",
            python_full_version: "3.12.0",
            python_version: "3.12",
            sys_platform: "linux",
        })
        .expect("valid authored marker environment");
        let python_requirement = PythonRequirement::from_marker_environment(
            &current_environment,
            RequiresPython::greater_than_equal_version(&Version::new([3_u64, 12])),
        );
        Self {
            current_environment,
            environment: ResolverEnvironment::universal(Vec::new()),
            python_requirement,
            versions: FxHashMap::default(),
            workspace_members: BTreeSet::new(),
            index: InMemoryIndex::default(),
            available_indexes: FxHashMap::default(),
            unavailable_packages: FxHashMap::default(),
            fork_urls: ForkUrls::default(),
            fork_indexes: ForkIndexes::default(),
        }
    }

    fn formatter(&self) -> PubGrubReportFormatter<'_> {
        PubGrubReportFormatter {
            included_versions: &self.versions,
            available_versions: &self.versions,
            python_requirement: &self.python_requirement,
            workspace_members: &self.workspace_members,
            tags: None,
        }
    }

    fn visit(&mut self, name: &PackageName, index: &IndexUrl) {
        self.available_indexes
            .entry(name.clone())
            .or_default()
            .insert(index.clone());
    }

    fn implicit(&self, name: &PackageName, response: VersionsResponse) {
        self.index.implicit().done(name.clone(), Arc::new(response));
    }

    fn hints(&self, tree: &ErrorTree) -> Vec<String> {
        self.hints_with(tree, IndexSet::new())
    }

    fn hints_with(&self, tree: &ErrorTree, mut hints: IndexSet<PubGrubHint>) -> Vec<String> {
        let options = Options::default();
        let selector = CandidateSelector::for_resolution(
            &options,
            &Manifest::simple(Vec::new()),
            &self.environment,
        );
        self.formatter().generate_hints(
            tree,
            &self.index,
            &selector,
            &IndexLocations::default(),
            &IndexCapabilities::default(),
            &self.available_indexes,
            &self.unavailable_packages,
            &FxHashMap::default(),
            &self.fork_urls,
            &self.fork_indexes,
            &self.environment,
            &self.current_environment,
            None,
            &self.workspace_members,
            &options,
            &FxHashMap::default(),
            &mut hints,
        );
        hints.iter().map(ToString::to_string).collect()
    }
}

fn package(name: &PackageName) -> PubGrubPackage {
    PubGrubPackageInner::Package {
        name: name.clone(),
        extra: None,
        group: None,
        marker: MarkerTree::TRUE,
    }
    .into()
}

fn no_versions(name: &PackageName) -> ErrorTree {
    DerivationTree::External(External::NoVersions(package(name), Range::full()))
}

fn not_found(name: &PackageName) -> ErrorTree {
    DerivationTree::External(External::Custom(
        package(name),
        Range::full(),
        UnavailableReason::Package(UnavailablePackage::NotFound),
    ))
}

fn derived(cause1: ErrorTree, cause2: ErrorTree) -> ErrorTree {
    DerivationTree::Derived(Derived {
        terms: Map::default(),
        shared_id: None,
        cause1: cause1.into(),
        cause2: cause2.into(),
    })
}

fn quarantine_message(name: &PackageName) -> String {
    format!(
        "A package index used during resolution marked `{}` as quarantined",
        name.cyan(),
    )
}

#[tokio::test]
async fn cached_quarantine_adds_only_the_package_hint() -> TestResult {
    let local = LocalIndex::new()?;
    let name = "Demo".parse::<PackageName>()?;
    let (index, version_map) = local
        .version_map(&name, "quarantined", Some("quarantined"), None)
        .await?;
    let mut fixture = HintFixture::new();
    fixture.implicit(&name, VersionsResponse::Found(vec![version_map]));
    fixture.visit(&name, &index);

    for (tree, report_text) in [
        (no_versions(&name), "there are no versions of demo"),
        (
            not_found(&name),
            "demo was not found in the package registry",
        ),
    ] {
        assert_eq!(report(&tree, &fixture.formatter()), report_text);
        assert_eq!(fixture.hints(&tree), vec![quarantine_message(&name)]);
    }
    Ok(())
}

#[tokio::test]
async fn other_project_statuses_leave_hints_unchanged() -> TestResult {
    let local = LocalIndex::new()?;
    let name = "demo".parse::<PackageName>()?;
    for (slot, status) in [
        ("active", Some("active")),
        ("archived", Some("archived")),
        ("deprecated", Some("deprecated")),
        ("omitted", None),
        ("unknown", Some("not-a-project-status")),
    ] {
        let (index, version_map) = local.version_map(&name, slot, status, None).await?;
        let mut fixture = HintFixture::new();
        fixture.implicit(&name, VersionsResponse::Found(vec![version_map]));
        fixture.visit(&name, &index);
        assert_eq!(
            fixture.hints(&no_versions(&name)),
            Vec::<String>::new(),
            "{slot}",
        );
        assert_eq!(
            fixture.hints(&not_found(&name)),
            Vec::<String>::new(),
            "{slot}",
        );
    }
    Ok(())
}

#[tokio::test]
async fn quarantine_requires_a_completed_visited_response() -> TestResult {
    let local = LocalIndex::new()?;
    let name = "demo".parse::<PackageName>()?;
    let unrelated = IndexUrl::parse("https://example.invalid/unvisited", None)?;
    let (index, version_map) = local
        .version_map(&name, "visited", Some("quarantined"), None)
        .await?;
    let mut fixture = HintFixture::new();
    fixture.implicit(&name, VersionsResponse::Found(vec![version_map]));
    assert_eq!(fixture.hints(&no_versions(&name)), Vec::<String>::new());
    fixture
        .available_indexes
        .insert(name.clone(), BTreeSet::new());
    assert_eq!(fixture.hints(&no_versions(&name)), Vec::<String>::new());
    fixture.visit(&name, &unrelated);
    assert_eq!(fixture.hints(&no_versions(&name)), Vec::<String>::new());
    fixture.visit(&name, &index);
    let visited_hints = fixture.hints(&no_versions(&name));

    for response in [
        VersionsResponse::NotFound,
        VersionsResponse::NoIndex,
        VersionsResponse::Offline,
        VersionsResponse::Found(Vec::new()),
        VersionsResponse::Found(vec![VersionMap::from_flat_metadata(
            Vec::new(),
            None,
            &HashStrategy::default(),
            &BuildOptions::default(),
        )]),
    ] {
        let mut fixture = HintFixture::new();
        fixture.implicit(&name, response);
        fixture.visit(&name, &index);
        assert_eq!(fixture.hints(&no_versions(&name)), Vec::<String>::new());
    }
    let mut pending = HintFixture::new();
    pending.visit(&name, &index);
    assert_eq!(pending.hints(&no_versions(&name)), Vec::<String>::new());
    assert!(pending.index.implicit().register(name.clone()));
    assert_eq!(pending.hints(&no_versions(&name)), Vec::<String>::new());
    for (response, reason, hint) in [
        (
            VersionsResponse::NoIndex,
            UnavailablePackage::NoIndex,
            PubGrubHint::NoIndex,
        ),
        (
            VersionsResponse::Offline,
            UnavailablePackage::Offline,
            PubGrubHint::Offline,
        ),
    ] {
        let mut fixture = HintFixture::new();
        fixture.implicit(&name, response);
        fixture.visit(&name, &index);
        fixture.unavailable_packages.insert(name.clone(), reason);
        assert_eq!(fixture.hints(&no_versions(&name)), vec![hint.to_string()]);
    }
    assert_eq!(visited_hints, vec![quarantine_message(&name)]);
    Ok(())
}

#[tokio::test]
async fn quarantine_respects_explicit_indexes_and_direct_urls() -> TestResult {
    let local = LocalIndex::new()?;
    let name = "demo".parse::<PackageName>()?;
    let (implicit_index, implicit_map) = local
        .version_map(&name, "implicit", Some("quarantined"), None)
        .await?;
    let (explicit_index, explicit_map) = local
        .version_map(&name, "explicit-active", Some("active"), None)
        .await?;
    let mut fixture = HintFixture::new();
    fixture.implicit(&name, VersionsResponse::Found(vec![implicit_map]));
    fixture.visit(&name, &implicit_index);
    fixture.visit(&name, &explicit_index);
    fixture.fork_indexes.insert(
        &name,
        &IndexMetadata::from(explicit_index.clone()),
        &fixture.environment,
    )?;
    assert_eq!(fixture.hints(&no_versions(&name)), Vec::<String>::new());
    fixture.index.explicit().done(
        (name.clone(), explicit_index.clone()),
        Arc::new(VersionsResponse::NotFound),
    );
    assert_eq!(fixture.hints(&no_versions(&name)), Vec::<String>::new());
    fixture.index.explicit().done(
        (name.clone(), explicit_index.clone()),
        Arc::new(VersionsResponse::Found(vec![explicit_map])),
    );
    assert_eq!(fixture.hints(&no_versions(&name)), Vec::<String>::new());

    let (_, explicit_map) = local
        .version_map(
            &name,
            "explicit-quarantine",
            Some("quarantined"),
            Some(&explicit_index),
        )
        .await?;
    fixture.index.explicit().done(
        (name.clone(), explicit_index.clone()),
        Arc::new(VersionsResponse::Found(vec![explicit_map])),
    );
    let explicit_hints = fixture.hints(&no_versions(&name));

    let direct = VerbatimParsedUrl::parse_url("https://example.invalid/demo-1.0.tar.gz", None)?;
    fixture
        .fork_urls
        .insert(&name, &direct, &fixture.environment)?;
    assert_eq!(fixture.hints(&no_versions(&name)), Vec::<String>::new());
    assert_eq!(fixture.hints(&not_found(&name)), Vec::<String>::new());
    assert_eq!(explicit_hints, vec![quarantine_message(&name)]);
    Ok(())
}

#[tokio::test]
async fn quarantine_deduplicates_without_changing_other_hints() -> TestResult {
    let local = LocalIndex::new()?;
    let name = "demo".parse::<PackageName>()?;
    let other = "other".parse::<PackageName>()?;
    let secret_index = IndexUrl::parse(
        "https://user:secret@example.invalid/simple?token=private#fragment",
        None,
    )?;
    let (first_index, first_map) = local
        .version_map(&name, "first", Some("quarantined"), Some(&secret_index))
        .await?;
    let (second_index, second_map) = local
        .version_map(&name, "second", Some("quarantined"), None)
        .await?;
    let (other_index, other_map) = local
        .version_map(&other, "other", Some("quarantined"), None)
        .await?;
    let mut fixture = HintFixture::new();
    fixture.implicit(&name, VersionsResponse::Found(vec![first_map, second_map]));
    fixture.implicit(&other, VersionsResponse::Found(vec![other_map]));
    fixture.visit(&name, &first_index);
    fixture.visit(&name, &second_index);
    fixture.visit(&other, &other_index);
    let tree = derived(
        no_versions(&name),
        derived(not_found(&name), no_versions(&other)),
    );
    let mut existing = IndexSet::new();
    existing.insert(PubGrubHint::Offline);
    let hints = fixture.hints_with(&tree, existing);

    let root = PubGrubPackage::from(PubGrubPackageInner::Root(None));
    let requirement =
        Requirement::from("demo>=2,<1".parse::<uv_pep508::Requirement<VerbatimParsedUrl>>()?);
    let unsatisfiable = UnsatisfiableRequirement::from_requirement(&requirement)
        .expect("the authored requirement has an empty range");
    for tree in [
        DerivationTree::External(External::NotRoot(root.clone(), Version::new([1_u64]))),
        DerivationTree::External(External::FromDependencyOf(
            root,
            Range::full(),
            package(&name),
            Range::full(),
        )),
        DerivationTree::External(External::Custom(
            package(&name),
            Range::full(),
            UnavailableReason::Version(UnavailableVersion::UnsatisfiableDependency(unsatisfiable)),
        )),
    ] {
        assert_eq!(fixture.hints(&tree), Vec::<String>::new());
    }
    assert_eq!(
        hints,
        vec![
            PubGrubHint::Offline.to_string(),
            quarantine_message(&name),
            quarantine_message(&other),
        ],
    );
    Ok(())
}
