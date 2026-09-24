use std::error::Error;

use serde_json::{Value, json};
use uv_pep440::{EncodedVersion, EncodedVersionRanges};

use super::*;
use crate::no_solution_capture::CaptureToken;
use crate::no_solution_capture::test_support::{package_name, version};
use crate::no_solution_capture::wire::CapturedReason;

const ORIGINAL: &[u8] = include_bytes!("fixtures/group-proxy-no-solution.json");

fn options() -> CaptureOptions {
    CaptureToken::new("e850e730d1404dd1b4fbd821ebc05c62", 80645)
        .expect("valid recorded token")
        .for_lock(
            CaptureScope::Workspace,
            CaptureOperation::Write,
            CaptureMetadata::Standard,
        )
}

fn inventory_with_node2(versions: &[Version]) -> Result<ClosedWorldInventory, Box<dyn Error>> {
    let mut inventory = ClosedWorldInventory::new(
        &package_name("uv-scenario-root"),
        &">=3.12,<3.15".parse()?,
        &version("3.12.13"),
    )?;
    inventory.add_project_group(&"shared".parse()?)?;
    inventory.add_registry_package(
        &package_name("node-1"),
        &[version("1.0.0"), version("2.0.0")],
    )?;
    inventory.add_registry_package(&package_name("node-2"), versions)?;
    inventory.add_registry_reference(&package_name("node-0"))?;
    for name in ["node-1", "node-2"] {
        inventory.add_registry_extra(&package_name(name), &"feature".parse()?)?;
    }
    Ok(inventory)
}

fn inventory() -> Result<ClosedWorldInventory, Box<dyn Error>> {
    inventory_with_node2(&[version("1.0.0"), version("2.0.0")])
}

fn value() -> Result<Value, Box<dyn Error>> {
    let mut value: Value = serde_json::from_slice(ORIGINAL)?;
    // The reader permits a conservative reservation for atom text. Keep structural counters exact
    // while allowing the small same-shape mutations below to use different spellings.
    let text = value["usage"]["text_bytes"].as_u64().ok_or("text count")?;
    value["usage"]["text_bytes"] = json!(text + 16_384);
    value["usage"]["max_atom_bytes"] = json!(16_384);
    Ok(value)
}

fn reject(value: &Value, inventory: &ClosedWorldInventory) -> Result<String, Box<dyn Error>> {
    Ok(
        classify_closed_world_no_solution(&serde_json::to_vec(value)?, &options(), inventory)
            .expect_err("unsupported evidence must not classify")
            .to_string(),
    )
}

fn encoded(version: &Version) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::to_value(EncodedVersion::try_from(version)?)?)
}

fn replace_node2_range(value: &mut Value, range: &Ranges<Version>) -> Result<(), Box<dyn Error>> {
    let old = &value["graph"]["nodes"][1]["no_versions"]["range"];
    let old_count = old["encoded"].as_array().ok_or("encoded range")?.len()
        + old["logical"].as_array().map_or(0, Vec::len);
    let new = serde_json::to_value(EncodedVersionRanges::try_from(range)?)?;
    let new_count = new.as_array().ok_or("encoded range")?.len();
    value["graph"]["nodes"][1]["no_versions"]["range"] = json!({"encoded": new});
    let intervals = value["usage"]["intervals"]
        .as_u64()
        .ok_or("interval count")?;
    value["usage"]["intervals"] = json!(intervals - old_count as u64 + new_count as u64);
    Ok(())
}

fn replace_node2_inventory(value: &mut Value, versions: &[Version]) -> Result<(), Box<dyn Error>> {
    let encoded = versions
        .iter()
        .map(encoded)
        .collect::<Result<Vec<_>, _>>()?;
    let observation = &mut value["graph"]["observations"][2];
    let old_count = observation["listed_versions"]
        .as_array()
        .ok_or("listed versions")?
        .len()
        + observation["known_versions"]
            .as_array()
            .ok_or("known versions")?
            .len();
    observation["listed_versions"] = json!(encoded);
    observation["known_versions"] = observation["listed_versions"].clone();
    let entries = value["usage"]["availability_entries"]
        .as_u64()
        .ok_or("entry count")?;
    value["usage"]["availability_entries"] =
        json!(entries - old_count as u64 + 2 * versions.len() as u64);
    Ok(())
}

#[test]
fn original_group_proxy_capture_uses_raw_native_absence_ranges() -> Result<(), Box<dyn Error>> {
    let classified = classify_closed_world_no_solution(ORIGINAL, &options(), &inventory()?)?;
    insta::assert_json_snapshot!(classified, @r#"
    {
      "grammar": "uv-closed-world-no-solution-v1",
      "project": "uv-scenario-root",
      "original_nodes": 19,
      "external_leaves": 10,
      "checked_no_versions": [
        0,
        1,
        2
      ]
    }
    "#);
    Ok(())
}

#[test]
fn supported_environments_bind_initial_and_failed_forks() -> Result<(), Box<dyn Error>> {
    let mut inventory = inventory()?;
    inventory.set_supported_environments(&[
        "sys_platform == 'darwin'".parse()?,
        "sys_platform == 'win32'".parse()?,
    ])?;
    let mut capture = value()?;
    // The recorded failed fork is Darwin with Python >=3.13. Both initial fork markers already
    // occur in the checked capture as package markers.
    capture["graph"]["environment"]["initial_forks"] = json!([2, 3]);
    classify_closed_world_no_solution(&serde_json::to_vec(&capture)?, &options(), &inventory)?;

    for forks in [json!([]), json!([2]), json!([3, 2]), json!([2, 0])] {
        let mut changed = capture.clone();
        changed["graph"]["environment"]["initial_forks"] = forks;
        assert!(reject(&changed, &inventory)?.contains("initial forks differ"));
    }
    for marker in [0, 1, 4] {
        let mut changed = capture.clone();
        // Keep the original failed marker reachable when changing the active environment.
        changed["graph"]["packages"][1]["group"]["marker"] = json!(5);
        changed["graph"]["environment"]["marker"] = json!(marker);
        assert!(reject(&changed, &inventory)?.contains("outside the certified domain"));
    }
    let mut changed = capture.clone();
    changed["graph"]["original_python"]["target_marker"] = json!(7);
    changed["graph"]["effective_python"]["target_marker"] = json!(6);
    assert!(reject(&changed, &inventory)?.contains("Python marker differs"));

    // This ordered wire DAG describes `os_name == 'nt' and sys_platform == 'darwin'`.
    // Native marker algebra knows the conjunction is impossible even though its two keys differ.
    let mut changed = capture.clone();
    changed["graph"]["markers"][5] = changed["graph"]["markers"][2].clone();
    changed["graph"]["markers"][5]["string"]["key"] = json!("os_name");
    let edges = changed["graph"]["markers"][5]["string"]["edges"]
        .as_array_mut()
        .ok_or("string marker edges")?;
    for edge in edges.iter_mut() {
        for bound in ["lower", "upper"] {
            if edge["intervals"][0][bound]["value"] == "darwin" {
                edge["intervals"][0][bound]["value"] = json!("nt");
            }
        }
    }
    edges[1]["child"] = json!(2);
    changed["graph"]["packages"][1]["group"]["marker"] = json!(4);
    assert!(reject(&changed, &inventory)?.contains("linked OS marker variables"));

    // A checked but non-ordered DAG must not let the implication walker forget a variable.
    let mut changed = capture;
    changed["graph"]["markers"][5] = changed["graph"]["markers"][4].clone();
    changed["graph"]["markers"][5]["version"]["edges"][1]["child"] = json!(2);
    changed["graph"]["packages"][1]["group"]["marker"] = json!(4);
    changed["usage"]["marker_edges"] = json!(16);
    changed["usage"]["intervals"] = json!(58);
    assert!(reject(&changed, &inventory)?.contains("marker variables are not ordered"));
    Ok(())
}

#[test]
fn supported_environment_construction_is_bounded_and_fail_closed() -> Result<(), Box<dyn Error>> {
    let os_name: MarkerTree = "os_name == 'nt'".parse()?;
    let sys_platform: MarkerTree = "sys_platform == 'linux'".parse()?;
    assert!(os_name.is_disjoint(sys_platform));
    assert!(
        inventory()?
            .set_supported_environments(&[os_name, sys_platform])
            .expect_err("linked native variables")
            .to_string()
            .contains("linked OS marker variables")
    );
    let cases: &[(&[&str], &str)] = &[
        (
            &["sys_platform == 'darwin'", "python_version >= '3.12'"],
            "not disjoint",
        ),
        (
            &["python_version < '3.12'"],
            "excludes the project Python domain",
        ),
        (&["extra == 'feature'"], "UnsupportedMarker"),
    ];
    for (markers, expected) in cases {
        let markers = markers
            .iter()
            .map(|marker| marker.parse())
            .collect::<Result<Vec<_>, _>>()?;
        let mut inventory = inventory()?;
        assert!(
            inventory
                .set_supported_environments(&markers)
                .expect_err("unsupported domain")
                .to_string()
                .contains(expected)
        );
        assert!(
            classify_closed_world_no_solution(ORIGINAL, &options(), &inventory)
                .expect_err("poisoned inventory")
                .to_string()
                .contains("construction failed")
        );
    }

    let mut bounded = inventory()?;
    bounded.budget.limits.marker_nodes = 0;
    assert!(
        bounded
            .set_supported_environments(&["sys_platform == 'darwin'".parse()?])
            .expect_err("marker budget")
            .to_string()
            .contains("MarkerNodes")
    );

    let mut unrestricted = inventory()?;
    unrestricted.set_supported_environments(&[])?;
    classify_closed_world_no_solution(ORIGINAL, &options(), &unrestricted)?;
    assert!(
        unrestricted
            .set_supported_environments(&[])
            .expect_err("one configuration")
            .to_string()
            .contains("already supplied")
    );
    Ok(())
}

#[test]
fn typed_authentication_and_availability_controls_fail_closed() -> Result<(), Box<dyn Error>> {
    type Mutation = fn(&mut Value);
    let mutations: &[(&str, Mutation, &str)] = &[
        (
            "401",
            |value| value["graph"]["index_authentication"]["unauthorized"] = json!(true),
            "authentication",
        ),
        (
            "403",
            |value| value["graph"]["index_authentication"]["forbidden"] = json!(true),
            "authentication",
        ),
        (
            "wrong missing reason",
            |value| {
                value["graph"]["observations"][0]["unavailable"]["kind"] = json!("package_offline");
            },
            "empty registry",
        ),
        (
            "missing listing",
            |value| value["graph"]["observations"][0]["listing"] = json!("unobserved"),
            "availability",
        ),
        (
            "no index",
            |value| value["graph"]["observations"][0]["listing"] = json!("no_index"),
            "availability",
        ),
        (
            "offline",
            |value| value["graph"]["observations"][0]["listing"] = json!("offline"),
            "availability",
        ),
        (
            "URL policy",
            |value| value["graph"]["observations"][0]["has_url_policy"] = json!(true),
            "source",
        ),
        (
            "index policy",
            |value| value["graph"]["observations"][1]["has_index_policy"] = json!(true),
            "source",
        ),
        (
            "explicit index",
            |value| value["graph"]["observations"][1]["source"] = json!("explicit_index"),
            "source",
        ),
        (
            "wrong project source",
            |value| value["graph"]["observations"][3]["source"] = json!("path"),
            "generated-project observation",
        ),
        (
            "project policy",
            |value| value["graph"]["observations"][3]["has_url_policy"] = json!(false),
            "generated-project observation",
        ),
        (
            "found but unavailable",
            |value| {
                value["graph"]["observations"][1]["unavailable"] =
                    json!({"kind": "metadata_invalid_metadata"});
            },
            "unavailable package",
        ),
        (
            "Python identity",
            |value| value["graph"]["packages"][2] = json!({"python": {"kind": "target"}}),
            "package identity",
        ),
        (
            "system identity",
            |value| value["graph"]["packages"][2] = json!({"system": {"name": "node-2"}}),
            "package identity",
        ),
        (
            "initial fork policy",
            |value| value["graph"]["environment"]["initial_forks"] = json!([0]),
            "conflict policy",
        ),
        (
            "included conflict",
            |value| {
                value["graph"]["environment"]["include"] =
                    json!([{"project": {"package": "node-1"}}]);
            },
            "conflict policy",
        ),
        (
            "excluded conflict",
            |value| {
                value["graph"]["environment"]["exclude"] =
                    json!([{"group": {"package": "uv-scenario-root", "group": "shared"}}]);
            },
            "conflict policy",
        ),
        (
            "custom missing package",
            |value| {
                let old = value["graph"]["nodes"][2]["no_versions"].clone();
                value["graph"]["nodes"][2] = json!({"custom": {
                    "package": old["package"], "range": old["range"],
                    "reason": {"kind": "package_not_found"}
                }});
            },
            "custom resolver leaves",
        ),
    ];
    for (name, mutate, expected) in mutations {
        let mut value = value()?;
        mutate(&mut value);
        let error = reject(&value, &inventory()?)?;
        assert!(error.contains(expected), "{name}: {error}");
    }
    let mut value = value()?;
    value["graph"]["observations"][0]["unavailable"]["http_status"] = json!(404);
    assert!(reject(&value, &inventory()?)?.contains("unsupported capture schema"));
    Ok(())
}

#[test]
fn every_custom_reason_remains_unsupported() -> Result<(), Box<dyn Error>> {
    for reason in [
        CapturedReasonKind::PackageNoIndex,
        CapturedReasonKind::PackageOffline,
        CapturedReasonKind::PackageNotFound,
        CapturedReasonKind::PackageInvalidMetadata,
        CapturedReasonKind::PackageInvalidStructure,
        CapturedReasonKind::PackageNetwork,
        CapturedReasonKind::VersionUnsatisfiableDependency,
        CapturedReasonKind::VersionIncompatibleSelfDependency,
        CapturedReasonKind::VersionIncompatibleDist,
        CapturedReasonKind::VersionInvalidMetadata,
        CapturedReasonKind::VersionInconsistentMetadata,
        CapturedReasonKind::VersionInvalidStructure,
        CapturedReasonKind::VersionOffline,
        CapturedReasonKind::VersionRequiresPython,
        CapturedReasonKind::VersionNetwork,
        CapturedReasonKind::MetadataOffline,
        CapturedReasonKind::MetadataInvalidMetadata,
        CapturedReasonKind::MetadataInconsistentMetadata,
        CapturedReasonKind::MetadataInvalidStructure,
        CapturedReasonKind::MetadataRequiresPython,
        CapturedReasonKind::MetadataNetwork,
    ] {
        let http_status = match reason {
            CapturedReasonKind::PackageNetwork
            | CapturedReasonKind::VersionNetwork
            | CapturedReasonKind::MetadataNetwork => Some(503),
            CapturedReasonKind::PackageNoIndex
            | CapturedReasonKind::PackageOffline
            | CapturedReasonKind::PackageNotFound
            | CapturedReasonKind::PackageInvalidMetadata
            | CapturedReasonKind::PackageInvalidStructure
            | CapturedReasonKind::VersionUnsatisfiableDependency
            | CapturedReasonKind::VersionIncompatibleSelfDependency
            | CapturedReasonKind::VersionIncompatibleDist
            | CapturedReasonKind::VersionInvalidMetadata
            | CapturedReasonKind::VersionInconsistentMetadata
            | CapturedReasonKind::VersionInvalidStructure
            | CapturedReasonKind::VersionOffline
            | CapturedReasonKind::VersionRequiresPython
            | CapturedReasonKind::MetadataOffline
            | CapturedReasonKind::MetadataInvalidMetadata
            | CapturedReasonKind::MetadataInconsistentMetadata
            | CapturedReasonKind::MetadataInvalidStructure
            | CapturedReasonKind::MetadataRequiresPython => None,
        };
        let reason = CapturedReason {
            kind: reason,
            http_status,
        };
        let mut value = value()?;
        let leaf = value["graph"]["nodes"][2]["no_versions"].clone();
        value["graph"]["nodes"][2] = json!({"custom": {
            "package": leaf["package"], "range": leaf["range"], "reason": reason
        }});
        let error = reject(&value, &inventory()?)?;
        assert!(
            error.contains("custom resolver leaves"),
            "{reason:?}: {error}"
        );
    }
    Ok(())
}

#[test]
fn independent_names_and_project_context_cannot_be_invented() -> Result<(), Box<dyn Error>> {
    let mut missing = inventory()?;
    missing.registry.remove("node-0");
    assert!(reject(&value()?, &missing)?.contains("independent inventory"));

    let mut nonempty = inventory()?;
    nonempty.add_registry_package(&package_name("node-0"), &[version("1")])?;
    assert!(reject(&value()?, &nonempty)?.contains("empty registry"));

    let mut wrong_group = inventory()?;
    wrong_group.project_groups.clear();
    assert!(reject(&value()?, &wrong_group)?.contains("project selection"));

    let mut value = value()?;
    value["graph"]["packages"][0]["root"]["name"] = json!("uv-scenario-root");
    assert!(reject(&value, &inventory()?)?.contains("package identity"));

    let mut value = self::value()?;
    value["graph"]["nodes"][17]["from_dependency_of"]["dependency"] = json!(7);
    assert!(reject(&value, &inventory()?)?.contains("unrooted"));

    let mut value = self::value()?;
    let wrong = encoded(&version("0.0.1"))?;
    value["graph"]["nodes"][0]["no_versions"]["range"]["encoded"][0]["upper"]["version"] =
        wrong.clone();
    value["graph"]["nodes"][0]["no_versions"]["range"]["encoded"][1]["lower"]["version"] = wrong;
    assert!(reject(&value, &inventory()?)?.contains("raw candidate"));

    let mut value = self::value()?;
    value["graph"]["workspace_members"][0] = json!("different-project");
    assert!(reject(&value, &inventory()?)?.contains("workspace"));
    Ok(())
}

#[test]
fn original_and_effective_python_policies_are_distinct() -> Result<(), Box<dyn Error>> {
    let mut value = value()?;
    value["graph"]["original_python"]["source"] = json!("python_version");
    assert!(reject(&value, &inventory()?)?.contains("Python policy"));

    let mut value = self::value()?;
    let outside = encoded(&version("3.11"))?;
    value["graph"]["effective_python"]["target"]["lower"]["version"] = outside.clone();
    value["graph"]["effective_python"]["target"]["specifiers"][0]["version"] = outside;
    assert!(reject(&value, &inventory()?)?.contains("outside the original domain"));
    Ok(())
}

#[test]
fn native_membership_keeps_local_post_and_sentinel_endpoints() -> Result<(), Box<dyn Error>> {
    let maximum_local: EncodedVersion = serde_json::from_value(json!({
        "epoch": "0", "release": ["2"], "local": {"kind": "max"}
    }))?;
    let maximum_local = maximum_local.into_version();
    let candidates = [version("2"), version("2+local"), version("2.post0")];
    let cases = [
        ("full", Ranges::full(), false),
        ("below", Ranges::strictly_lower_than(version("2")), true),
        (
            "disjoint",
            Ranges::strictly_lower_than(version("2"))
                .union(&Ranges::strictly_higher_than(version("2.post0"))),
            true,
        ),
        (
            "included equality",
            Ranges::singleton(version("2+local")),
            false,
        ),
        (
            "excluded equality",
            Ranges::strictly_higher_than(version("2.post0")),
            true,
        ),
        ("post release", Ranges::singleton(version("2.post0")), false),
        ("max-local", Ranges::higher_than(maximum_local), false),
    ];
    for (name, range, accepted) in cases {
        let mut value = value()?;
        replace_node2_inventory(&mut value, &candidates)?;
        replace_node2_range(&mut value, &range)?;
        let result = classify_closed_world_no_solution(
            &serde_json::to_vec(&value)?,
            &options(),
            &inventory_with_node2(&candidates)?,
        );
        assert_eq!(result.is_ok(), accepted, "{name}: {result:?}");
        if !accepted {
            assert!(
                result
                    .expect_err("intersecting range")
                    .to_string()
                    .contains("raw candidate"),
                "{name}"
            );
        }
    }
    Ok(())
}

#[test]
fn ordinary_inventory_versions_cannot_contain_internal_sentinels() -> Result<(), Box<dyn Error>> {
    for candidate in [
        version("1.0"),
        version("1.0a1"),
        version("1.0.dev1"),
        version("1.0.post1"),
        version("1.0+local"),
        version("1.0rc1.post2"),
        version("1.0rc1.dev2"),
        version("1.0.post1.dev2"),
        version("1!1.0rc1.post2.dev3+local"),
        Version::new([u64::MAX]),
    ] {
        let mut inventory = inventory()?;
        inventory.add_registry_package(&package_name("ordinary"), [&candidate])?;
        assert!(ordinary_version(&candidate), "{candidate:?}");
    }
    let maximum_local: EncodedVersion = serde_json::from_value(json!({
        "epoch": "0", "release": ["1"], "local": {"kind": "max"}
    }))?;
    for candidate in [
        Version::new([1]).with_min(Some(0)),
        Version::new([1]).with_max(Some(0)),
        version("1a1").with_max(Some(0)),
        maximum_local.into_version(),
        Version::new([1]).with_min(Some(1)),
        Version::new([1]).with_max(Some(1)),
        Version::new([1]).with_min(Some(0)).with_max(Some(0)),
        Version::new([1]).with_min(Some(0)).with_post(Some(1)),
        Version::new([1]).with_max(Some(0)).with_post(Some(1)),
    ] {
        assert!(!ordinary_version(&candidate), "{candidate:?}");
        let mut inventory = inventory()?;
        assert!(
            inventory
                .add_registry_package(&package_name("sentinel"), [&candidate])
                .expect_err("an internal sentinel is not a raw candidate")
                .to_string()
                .contains("internal sentinel")
        );
        assert!(reject(&value()?, &inventory)?.contains("construction failed"));
    }
    Ok(())
}

#[test]
fn request_modes_and_independent_budgets_are_enforced() -> Result<(), Box<dyn Error>> {
    let mut wrong = options();
    wrong.scope = CaptureScope::Script;
    assert!(classify_closed_world_no_solution(ORIGINAL, &wrong, &inventory()?).is_err());
    wrong = options();
    wrong.operation = CaptureOperation::Locked;
    assert!(classify_closed_world_no_solution(ORIGINAL, &wrong, &inventory()?).is_err());
    wrong = options();
    wrong.metadata = CaptureMetadata::WithoutMetadata;
    assert!(classify_closed_world_no_solution(ORIGINAL, &wrong, &inventory()?).is_err());
    let mut value = value()?;
    value["metadata"] = json!("without_metadata");
    assert!(
        classify_closed_world_no_solution(&serde_json::to_vec(&value)?, &wrong, &inventory()?)
            .is_ok()
    );

    let mut poisoned = inventory()?;
    assert!(
        poisoned
            .add_registry_reference(&package_name("uv-scenario-root"))
            .is_err()
    );
    assert!(reject(&self::value()?, &poisoned)?.contains("construction failed"));

    let mut exhausted = inventory()?;
    exhausted.budget.limits.work = exhausted.budget.usage.work + 1;
    assert!(reject(&self::value()?, &exhausted)?.contains("limit exceeded"));

    let mut duplicate = inventory()?;
    assert!(
        duplicate
            .add_registry_package(&package_name("NODE_1"), &[version("1")])
            .is_err()
    );
    assert!(reject(&self::value()?, &duplicate)?.contains("construction failed"));

    let mut exhausted = ClosedWorldInventory::with_limits(
        &package_name("uv-scenario-root"),
        &">=3.12,<3.15".parse()?,
        &version("3.12.13"),
        CaptureLimits {
            packages: 1,
            ..CaptureLimits::V1
        },
    )?;
    assert!(
        exhausted
            .add_registry_reference(&package_name("a"))
            .expect_err("the package budget is exhausted")
            .to_string()
            .contains("limit exceeded")
    );
    assert!(reject(&self::value()?, &exhausted)?.contains("construction failed"));
    Ok(())
}
