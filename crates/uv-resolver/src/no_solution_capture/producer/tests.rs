use std::error::Error;

use pubgrub::{Derived, Map};
use reqwest::StatusCode;
use serde_json::json;
use uv_distribution_types::{
    Index, IndexMetadata, IndexStatusCodeDecision, IndexStatusCodeStrategy,
};
use uv_pep440::VersionSpecifiers;
use uv_pep508::{MarkerExpression, MarkerOperator, MarkerValueString};
use uv_pypi_types::ResolverMarkerEnvironment;

use super::*;
use crate::no_solution_capture::reader;
use crate::no_solution_capture::test_support::{
    Fixture, basic_tree, marker, marker_environment, options, package, package_name, token, version,
};

#[test]
fn original_graph_round_trips_through_the_checked_reader() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new();
    fixture.effective_python = fixture
        .original_python
        .split(Bound::Included(version("3.13")))
        .expect("Python domain can be split")
        .1;
    let tree = basic_tree();
    let evidence = options().capture(&fixture.context(&tree));
    assert_eq!(evidence.status(), CaptureStatus::Complete);
    let graph = evidence.0.graph.as_ref().expect("complete graph");
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.packages.len(), 2);
    assert_eq!(graph.root, 2);
    assert_eq!(graph.root_package, 0);
    assert_eq!(graph.root_version, EncodedVersion::try_from(&*MIN_VERSION)?);
    assert_ne!(
        serde_json::to_value(&graph.original_python.target)?,
        serde_json::to_value(&graph.effective_python.target)?
    );
    assert!(matches!(
        &graph.nodes[2],
        CapturedNode::Derived { cause1: 0, cause2: 1, terms }
            if terms.len() == 2 && terms.iter().any(|term| term.positive)
                && terms.iter().any(|term| !term.positive)
    ));
    let bytes = evidence.to_json()?;
    let checked = NoSolutionEvidence::from_json(&bytes, &token())?;
    assert!(checked.matches(&token()));
    assert_eq!(checked.status(), CaptureStatus::Complete);
    assert_eq!(checked.to_json()?, bytes);
    Ok(())
}

fn leaf(name: &str) -> Arc<ErrorTree> {
    Arc::new(ErrorTree::External(External::NoVersions(
        package(name),
        Range::full(),
    )))
}

fn derived(
    cause1: Arc<ErrorTree>,
    cause2: Arc<ErrorTree>,
    shared_id: Option<usize>,
) -> Arc<ErrorTree> {
    Arc::new(ErrorTree::Derived(Derived {
        terms: Map::default(),
        shared_id,
        cause1,
        cause2,
    }))
}

fn drop_tree(root: Arc<ErrorTree>) {
    let mut pending = vec![root];
    while let Some(tree) = pending.pop() {
        if let Ok(ErrorTree::Derived(tree)) = Arc::try_unwrap(tree) {
            pending.push(tree.cause1);
            pending.push(tree.cause2);
        }
    }
}

#[test]
fn shared_causes_are_captured_once_without_display_ids() {
    let depth = 64;
    let mut tree = leaf("a");
    for _ in 0..depth {
        tree = derived(tree.clone(), tree, None);
    }
    let mut collector = Collector::new(CaptureLimits::V1);
    let (nodes, root) = collector.derivation(&tree).expect("supported shared DAG");
    assert_eq!(nodes.len(), depth + 1);
    assert_eq!(root as usize, depth);
    for (index, node) in nodes.iter().enumerate().skip(1) {
        assert!(matches!(node, CapturedNode::Derived { cause1, cause2, .. }
            if *cause1 as usize == index - 1 && cause1 == cause2));
    }
    drop_tree(tree);
}

#[test]
fn distinct_nodes_with_reused_display_ids_remain_distinct() {
    let common = leaf("a");
    let first = derived(common.clone(), common.clone(), Some(7));
    let second = derived(common, leaf("b"), Some(7));
    let tree = derived(first, second, None);
    let mut collector = Collector::new(CaptureLimits::V1);
    let (nodes, root) = collector.derivation(&tree).expect("supported DAG");
    assert_eq!(nodes.len(), 5);
    assert!(matches!(
        &nodes[root as usize],
        CapturedNode::Derived {
            cause1: 1,
            cause2: 3,
            ..
        }
    ));
    assert_eq!(collector.packages.len(), 2);
    drop_tree(tree);
}

#[test]
fn deep_derivation_capture_stops_at_its_node_budget() {
    let depth = 4_096;
    let common = leaf("a");
    let mut tree = common.clone();
    for _ in 0..depth {
        tree = derived(tree, common.clone(), None);
    }
    let mut collector = Collector::new(CaptureLimits {
        derivation_nodes: 32,
        ..CaptureLimits::V1
    });
    assert_eq!(
        collector
            .derivation(&tree)
            .expect_err("node budget must stop the walk"),
        Stop::truncated(CaptureReason::DerivationNodes)
    );
    assert_eq!(collector.budget.usage.derivation_nodes, 33);
    drop(common);
    drop_tree(tree);
}

#[test]
fn encoded_membership_and_logical_ranges_stay_distinct() {
    let native = Range::from(
        "==1.0"
            .parse::<VersionSpecifiers>()
            .expect("valid specifier"),
    );
    let mut collector = Collector::new(CaptureLimits::V1);
    let captured = collector.range(&native).expect("supported sentinel range");
    assert_eq!(captured.encoded.to_ranges(), *native.encoded_versions());
    assert_eq!(
        captured
            .logical
            .as_ref()
            .map(EncodedVersionRanges::to_ranges),
        native.canonical_versions().cloned()
    );
    assert!(captured.logical.is_some());
    for candidate in ["0.9", "1.0", "1.0+local", "1.0.post0", "1.1"] {
        let candidate = version(candidate);
        assert_eq!(
            captured.encoded.to_ranges().contains(&candidate),
            native.contains(&candidate)
        );
    }
}

#[test]
fn native_component_preflight_handles_full_width_fields() {
    let maximum = u64::MAX.to_string();
    let native = serde_json::from_value::<EncodedVersion>(json!({
        "epoch": maximum,
        "release": [maximum, "0"],
        "pre": {"kind": "rc", "number": maximum},
        "post": maximum,
        "dev": maximum,
        "local": {"kind": "segments", "segments": [
            {"kind": "number", "value": maximum},
            {"kind": "string", "value": "18446744073709551616"}
        ]}
    }))
    .expect("supported full-width component encoding")
    .into_version();
    let mut collector = Collector::new(CaptureLimits::V1);
    let encoded = collector
        .version(&native)
        .expect("supported full-width version");
    assert_eq!(
        EncodedVersion::try_from(encoded.to_version()).expect("checked reconstruction"),
        encoded
    );
    assert_eq!(collector.budget.usage.max_version_components, 2);
    assert_eq!(collector.budget.usage.max_atom_bytes, 20);
    assert_eq!(
        collector
            .version(&Version::new([1]).with_max(Some(1)))
            .expect_err("nonzero max is outside the codec"),
        Stop::unsupported(CaptureReason::UnsupportedVersion)
    );
}

fn version_marker_value(markers: &[CapturedMarker], mut id: u32, value: &Version) -> Option<bool> {
    loop {
        match &markers[id as usize] {
            CapturedMarker::True => return Some(true),
            CapturedMarker::False => return Some(false),
            CapturedMarker::Version {
                key: VersionMarkerKey::PythonFullVersion,
                edges,
            } => {
                id = edges
                    .iter()
                    .find(|edge| edge.intervals.to_ranges().contains(value))?
                    .child;
            }
            CapturedMarker::Version { .. } | CapturedMarker::String { .. } => return None,
        }
    }
}

#[test]
fn complemented_marker_edges_keep_their_signed_identity() {
    let native = marker("python_full_version >= '3.13' and python_full_version < '3.14'");
    let mut collector = Collector::new(CaptureLimits::V1);
    let positive = collector.marker(native).expect("supported version marker");
    let negative = collector
        .marker(native.negate())
        .expect("supported complemented marker");
    assert_ne!(positive, negative);
    for (candidate, expected) in [("3.12", false), ("3.13", true), ("3.14", false)] {
        let candidate = version(candidate);
        assert_eq!(
            version_marker_value(&collector.markers, positive, &candidate),
            Some(expected)
        );
        assert_eq!(
            version_marker_value(&collector.markers, negative, &candidate),
            Some(!expected)
        );
    }
}

#[test]
fn json_escaping_is_bounded_before_publication() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new();
    fixture.environment =
        ResolverEnvironment::universal(vec![MarkerTree::expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: ArcStr::from("\0\"\\".repeat(256)),
        })]);
    let tree = basic_tree();
    let complete = options().capture(&fixture.context(&tree));
    assert_eq!(complete.status(), CaptureStatus::Complete);
    let complete_bytes = complete.to_json()?;
    NoSolutionEvidence::from_json(&complete_bytes, &token())?;
    let evidence = options().capture_with_limits(
        &fixture.context(&tree),
        CaptureLimits {
            json_bytes: complete_bytes.len() / 2,
            ..CaptureLimits::V1
        },
    );
    assert_eq!(evidence.status(), CaptureStatus::Complete);
    let bytes = evidence.to_json()?;
    assert!(bytes.len() <= evidence.0.limits.json_bytes);
    let checked = NoSolutionEvidence::from_json(&bytes, &token())?;
    assert_eq!(checked.status(), CaptureStatus::Truncated);
    assert_eq!(checked.0.reason, Some(CaptureReason::JsonBytes));
    assert!(checked.0.graph.is_none());
    Ok(())
}

#[test]
fn unsupported_marker_kinds_do_not_emit_partial_graphs() {
    for (expression, reason) in [
        ("sys_platform in 'nux'", CaptureReason::UnsupportedMarkerIn),
        (
            "'nux' in sys_platform",
            CaptureReason::UnsupportedMarkerContains,
        ),
        ("'feature' in extras", CaptureReason::UnsupportedMarkerList),
        ("extra == 'feature'", CaptureReason::UnsupportedMarkerExtra),
    ] {
        let mut fixture = Fixture::new();
        fixture.environment = ResolverEnvironment::universal(vec![marker(expression)]);
        let tree = basic_tree();
        let evidence = options().capture(&fixture.context(&tree));
        assert_eq!(
            evidence.status(),
            CaptureStatus::Unsupported,
            "{expression}"
        );
        assert_eq!(evidence.0.reason, Some(reason), "{expression}");
        assert!(evidence.0.graph.is_none());
    }

    let mut fixture = Fixture::new();
    fixture.environment =
        ResolverEnvironment::specific(ResolverMarkerEnvironment::from(marker_environment()));
    let tree = basic_tree();
    let evidence = options().capture(&fixture.context(&tree));
    assert_eq!(evidence.0.reason, Some(CaptureReason::SpecificEnvironment));
    assert!(evidence.0.graph.is_none());
}

#[test]
fn observations_distinguish_unobserved_listing_and_metadata_failure() -> Result<(), Box<dyn Error>>
{
    let mut fixture = Fixture::new();
    let tree = basic_tree();
    let name = package_name("a");
    assert!(fixture.index.implicit().register(name.clone()));
    let unobserved = options().capture(&fixture.context(&tree));
    let observation = unobserved
        .0
        .graph
        .as_ref()
        .expect("complete graph")
        .observations
        .iter()
        .find(|observation| observation.name == "a")
        .expect("observed package name");
    assert_eq!(observation.listing, CapturedListing::Unobserved);
    assert!(observation.listed_versions.is_empty());

    fixture
        .index
        .implicit()
        .done(name.clone(), Arc::new(VersionsResponse::Found(Vec::new())));
    fixture.known_versions.insert(
        name.clone(),
        Arc::from([version("1.0"), version("1.0+local"), version("1.0.post0")]),
    );
    fixture.unavailable_packages.pin().insert(
        name.clone(),
        UnavailablePackage::Network(StatusCode::UNAUTHORIZED),
    );
    let incomplete = HashMap::new();
    incomplete
        .pin()
        .insert(version("1.0"), MetadataUnavailable::Offline);
    fixture.incomplete_packages.pin().insert(name, incomplete);
    let evidence = options().capture(&fixture.context(&tree));
    let observation = evidence
        .0
        .graph
        .as_ref()
        .expect("complete graph")
        .observations
        .iter()
        .find(|observation| observation.name == "a")
        .expect("observed package name");
    assert_eq!(observation.listing, CapturedListing::Found);
    assert_eq!(observation.known_versions.len(), 3);
    assert_eq!(observation.incomplete.len(), 1);
    assert_eq!(
        observation
            .unavailable
            .as_ref()
            .map(|reason| (reason.kind, reason.http_status)),
        Some((CapturedReasonKind::PackageNetwork, Some(401)))
    );
    assert_eq!(
        observation.incomplete[0].reason.kind,
        CapturedReasonKind::MetadataOffline
    );
    NoSolutionEvidence::from_json(&evidence.to_json()?, &token())?;
    Ok(())
}

#[test]
fn index_authentication_uses_existing_global_capability_state() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new();
    let first = IndexUrl::parse("https://first.example/simple", None)?;
    let overridden = IndexUrl::parse("https://overridden.example/simple", None)?;
    fixture.index_locations = IndexLocations::new(
        vec![
            Index::from_index_url(first.clone()),
            Index::from_index_url(overridden.clone()),
        ],
        Vec::new(),
        false,
    );
    let authentication = |fixture: &Fixture| {
        options()
            .capture(&fixture.context(&basic_tree()))
            .0
            .graph
            .expect("complete graph")
            .index_authentication
    };

    assert_eq!(
        IndexStatusCodeStrategy::Default.handle_status_code(
            StatusCode::NOT_FOUND,
            &first,
            &fixture.index_capabilities,
        ),
        IndexStatusCodeDecision::Ignore
    );
    assert_eq!(
        authentication(&fixture),
        CapturedIndexAuthentication::default()
    );
    assert_eq!(
        IndexStatusCodeStrategy::Default.handle_status_code(
            StatusCode::UNAUTHORIZED,
            &first,
            &fixture.index_capabilities,
        ),
        IndexStatusCodeDecision::Fail(StatusCode::UNAUTHORIZED)
    );
    assert_eq!(
        authentication(&fixture),
        CapturedIndexAuthentication {
            unauthorized: true,
            forbidden: false,
        }
    );
    assert_eq!(
        IndexStatusCodeStrategy::Default.handle_status_code(
            StatusCode::FORBIDDEN,
            &overridden,
            &fixture.index_capabilities,
        ),
        IndexStatusCodeDecision::Fail(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        authentication(&fixture),
        CapturedIndexAuthentication {
            unauthorized: true,
            forbidden: true,
        }
    );

    // A selected explicit index can be absent from the general configured-index inventory.
    let mut fixture = Fixture::new();
    let selected = IndexUrl::parse("https://selected.example/simple", None)?;
    fixture.fork_indexes.insert(
        &package_name("a"),
        &IndexMetadata::from(selected.clone()),
        &fixture.environment,
    )?;
    assert_eq!(
        IndexStatusCodeStrategy::Default.handle_status_code(
            StatusCode::FORBIDDEN,
            &selected,
            &fixture.index_capabilities,
        ),
        IndexStatusCodeDecision::Fail(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        authentication(&fixture),
        CapturedIndexAuthentication {
            unauthorized: false,
            forbidden: true,
        }
    );

    // Explicitly ignored authentication responses do not set the native capability flags.
    // A consumer must independently reject that status-code policy before accepting absence.
    let fixture = Fixture::new();
    assert_eq!(
        IndexStatusCodeStrategy::ignore_authentication_error_codes().handle_status_code(
            StatusCode::UNAUTHORIZED,
            &first,
            &fixture.index_capabilities,
        ),
        IndexStatusCodeDecision::Ignore
    );
    assert_eq!(
        authentication(&fixture),
        CapturedIndexAuthentication::default()
    );
    let tree = basic_tree();
    let evidence = options().capture(&fixture.context(&tree));
    NoSolutionEvidence::from_json(&evidence.to_json()?, &token())?;
    Ok(())
}

#[test]
fn index_authentication_scan_cannot_return_partial_false_flags() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new();
    fixture.index_locations = IndexLocations::new(
        vec![Index::from_index_url(IndexUrl::parse(
            "https://second.example/simple",
            None,
        )?)],
        Vec::new(),
        false,
    );
    let tree = basic_tree();
    let mut collector = Collector::new(CaptureLimits {
        work: 1,
        ..CaptureLimits::V1
    });
    assert_eq!(
        collector
            .index_authentication(&fixture.context(&tree))
            .expect_err("the second lookup exceeds the work budget"),
        Stop::truncated(CaptureReason::Work)
    );
    assert_eq!(collector.budget.usage.work, 2);

    let long_url = format!(
        "https://example.invalid/{}",
        "a".repeat(CaptureLimits::V1.atom_bytes)
    );
    fixture.index_locations = IndexLocations::new(
        vec![Index::from_index_url(IndexUrl::parse(&long_url, None)?)],
        Vec::new(),
        false,
    );
    let evidence = options().capture(&fixture.context(&tree));
    assert_eq!(evidence.status(), CaptureStatus::Truncated);
    assert_eq!(evidence.0.reason, Some(CaptureReason::AtomBytes));
    assert!(evidence.0.graph.is_none());
    NoSolutionEvidence::from_json(&evidence.to_json()?, &token())?;
    Ok(())
}

#[test]
fn each_capture_budget_discards_the_partial_graph() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new();
    let tree = basic_tree();
    let complete = options().capture(&fixture.context(&tree));
    assert_eq!(complete.status(), CaptureStatus::Complete);
    let usage = complete.0.usage;
    let cases = [
        (
            CaptureLimits {
                derivation_nodes: usage.derivation_nodes - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::DerivationNodes,
        ),
        (
            CaptureLimits {
                packages: usage.packages - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::Packages,
        ),
        (
            CaptureLimits {
                terms: usage.terms - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::Terms,
        ),
        (
            CaptureLimits {
                intervals: usage.intervals - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::Intervals,
        ),
        (
            CaptureLimits {
                marker_nodes: usage.marker_nodes - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::MarkerNodes,
        ),
        (
            CaptureLimits {
                marker_edges: usage.marker_edges - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::MarkerEdges,
        ),
        (
            CaptureLimits {
                availability_entries: usage.availability_entries - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::AvailabilityEntries,
        ),
        (
            CaptureLimits {
                version_components: usage.max_version_components - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::VersionComponents,
        ),
        (
            CaptureLimits {
                atom_bytes: usage.max_atom_bytes - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::AtomBytes,
        ),
        (
            CaptureLimits {
                text_bytes: usage.text_bytes - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::TextBytes,
        ),
        (
            CaptureLimits {
                work: usage.work - 1,
                ..CaptureLimits::V1
            },
            CaptureReason::Work,
        ),
    ];
    for (limits, reason) in cases {
        let evidence = options().capture_with_limits(&fixture.context(&tree), limits);
        assert_eq!(evidence.status(), CaptureStatus::Truncated, "{reason:?}");
        assert_eq!(evidence.0.reason, Some(reason));
        assert!(evidence.0.graph.is_none());
        assert_eq!(
            NoSolutionEvidence::from_json(&evidence.to_json()?, &token())?.status(),
            CaptureStatus::Truncated
        );
    }

    let complete_bytes = complete.to_json()?;
    let evidence = options().capture_with_limits(
        &fixture.context(&tree),
        CaptureLimits {
            json_bytes: complete_bytes.len() / 2,
            ..CaptureLimits::V1
        },
    );
    assert!(serde_json::to_vec(&evidence.0)?.len() > evidence.0.limits.json_bytes);
    let bytes = evidence.to_json()?;
    let checked = NoSolutionEvidence::from_json(&bytes, &token())?;
    assert_eq!(checked.status(), CaptureStatus::Truncated);
    assert_eq!(checked.0.reason, Some(CaptureReason::JsonBytes));
    assert!(checked.0.graph.is_none());
    assert!(bytes.len() < complete_bytes.len());
    assert!(matches!(
        reader::encode(&EvidenceWire {
            limits: CaptureLimits {
                json_bytes: 0,
                ..CaptureLimits::V1
            },
            ..complete.0
        }),
        Err(reader::CaptureWriteError::EnvelopeTooLarge)
    ));
    Ok(())
}
