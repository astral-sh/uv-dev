use serde_json::json;

use super::*;
use crate::no_solution_capture::test_support::{Fixture, basic_tree, options, token, version};

fn complete_value() -> serde_json::Value {
    let fixture = Fixture::new();
    let tree = basic_tree();
    let evidence = options().capture(fixture.context(&tree));
    assert_eq!(evidence.status(), CaptureStatus::Complete);
    let mut value = serde_json::to_value(&evidence.0).expect("serializable fixture");
    // Reference mutations may change atom lengths without changing structural counts.
    value["usage"]["text_bytes"] = json!(CaptureLimits::V1.text_bytes);
    value["usage"]["max_atom_bytes"] = json!(CaptureLimits::V1.atom_bytes);
    value
}

fn rejection(value: &serde_json::Value) -> ReadErrorKind {
    NoSolutionEvidence::from_json(
        &serde_json::to_vec(value).expect("serializable fixture"),
        &token(),
    )
    .err()
    .expect("invalid capture must be rejected")
    .0
}

fn encoded(version_text: &str) -> serde_json::Value {
    serde_json::to_value(
        EncodedVersion::try_from(version(version_text)).expect("supported version"),
    )
    .expect("serializable version")
}

fn version_marker_mut(value: &mut serde_json::Value) -> (usize, &mut serde_json::Value) {
    let markers = value["graph"]["markers"]
        .as_array_mut()
        .expect("marker table");
    let index = markers
        .iter()
        .position(|marker| marker.get("version").is_some())
        .expect("version marker");
    (index, &mut markers[index]["version"])
}

#[test]
fn derivation_reference_and_identity_checks_fail_closed() {
    type Mutation = fn(&mut serde_json::Value);
    let cases: &[(&str, Mutation, &str)] = &[
        (
            "root",
            |value| value["graph"]["root"] = json!(0),
            "root is not the final derivation node",
        ),
        (
            "root package",
            |value| value["graph"]["root_package"] = json!(1),
            "root package is not a root identity",
        ),
        (
            "root version",
            |value| value["graph"]["root_version"]["epoch"] = json!("1"),
            "unexpected root version",
        ),
        (
            "cycle",
            |value| value["graph"]["nodes"][2]["derived"]["cause1"] = json!(2),
            "derivation is not in postorder",
        ),
        (
            "unreachable",
            |value| {
                value["graph"]["nodes"][2]["derived"]["cause1"] = json!(1);
                value["graph"]["nodes"][2]["derived"]["cause2"] = json!(1);
            },
            "unreachable derivation node",
        ),
        (
            "package reference",
            |value| value["graph"]["nodes"][0]["from_dependency_of"]["dependency"] = json!(99),
            "invalid package reference",
        ),
        (
            "duplicate term",
            |value| {
                let first = value["graph"]["nodes"][2]["derived"]["terms"][0]["package"].clone();
                value["graph"]["nodes"][2]["derived"]["terms"][1]["package"] = first;
            },
            "duplicate signed term",
        ),
        (
            "duplicate package",
            |value| value["graph"]["packages"][1] = value["graph"]["packages"][0].clone(),
            "duplicate package identity",
        ),
        (
            "package spelling",
            |value| value["graph"]["packages"][1]["package"]["name"] = json!("A"),
            "non-normalized package name",
        ),
        (
            "duplicate observation",
            |value| value["graph"]["observations"][1] = value["graph"]["observations"][0].clone(),
            "unexpected or duplicate package observation",
        ),
        (
            "listing source",
            |value| {
                value["graph"]["observations"][0]["source"] = json!("archive");
                value["graph"]["observations"][0]["listing"] = json!("found");
            },
            "listing state does not match its source kind",
        ),
    ];
    for (name, mutate, expected) in cases {
        let mut value = complete_value();
        mutate(&mut value);
        assert_eq!(
            rejection(&value),
            ReadErrorKind::InvalidGraph(expected),
            "{name}"
        );
    }
}

#[test]
fn marker_partitions_references_and_reachability_are_checked() {
    let mut value = complete_value();
    let (index, marker) = version_marker_mut(&mut value);
    marker["edges"][0]["child"] = json!(index);
    assert_eq!(
        rejection(&value),
        ReadErrorKind::InvalidGraph("marker is not in postorder")
    );

    let mut value = complete_value();
    let (_, marker) = version_marker_mut(&mut value);
    marker["edges"][0]["intervals"][0]["upper"] =
        json!({"kind": "excluded", "version": encoded("3.0")});
    assert_eq!(
        rejection(&value),
        ReadErrorKind::InvalidGraph("marker edges do not partition the domain")
    );

    let mut value = complete_value();
    let (_, marker) = version_marker_mut(&mut value);
    marker["edges"][0]["intervals"][0]["lower"] =
        json!({"kind": "included", "version": encoded("3.0")});
    assert_eq!(
        rejection(&value),
        ReadErrorKind::InvalidGraph("marker partition has no lower tail")
    );

    let mut value = complete_value();
    let (_, marker) = version_marker_mut(&mut value);
    marker["edges"][0]["intervals"]
        .as_array_mut()
        .expect("intervals")
        .push(json!({
            "lower": {"kind": "included", "version": encoded("3.15")},
            "upper": {"kind": "unbounded"}
        }));
    let intervals = value["usage"]["intervals"]
        .as_u64()
        .expect("interval count");
    value["usage"]["intervals"] = json!(intervals + 1);
    assert_eq!(
        rejection(&value),
        ReadErrorKind::InvalidGraph("marker edge has multiple intervals")
    );

    let mut value = complete_value();
    value["graph"]["markers"]
        .as_array_mut()
        .expect("marker table")
        .push(json!("true"));
    let markers = value["usage"]["marker_nodes"]
        .as_u64()
        .expect("marker count");
    value["usage"]["marker_nodes"] = json!(markers + 1);
    assert_eq!(
        rejection(&value),
        ReadErrorKind::InvalidGraph("unreachable marker node")
    );
}

#[test]
fn malformed_and_noncanonical_ranges_are_not_normalized_into_claims() {
    let mut value = complete_value();
    value["graph"]["nodes"][0]["from_dependency_of"]["dependency_range"]["encoded"] = json!([{
        "lower": {"kind": "included", "version": encoded("2")},
        "upper": {"kind": "excluded", "version": encoded("1")}
    }]);
    assert_eq!(rejection(&value), ReadErrorKind::InvalidSchema);

    let mut value = complete_value();
    value["graph"]["nodes"][0]["from_dependency_of"]["dependency_range"]["encoded"] = json!([
        {"lower": {"kind": "unbounded"}, "upper": {"kind": "excluded", "version": encoded("1")}},
        {"lower": {"kind": "included", "version": encoded("1")}, "upper": {"kind": "unbounded"}}
    ]);
    let intervals = value["usage"]["intervals"]
        .as_u64()
        .expect("interval count");
    value["usage"]["intervals"] = json!(intervals + 1);
    assert_eq!(rejection(&value), ReadErrorKind::InvalidSchema);
}

#[test]
fn status_headers_cannot_carry_a_partial_graph() -> Result<(), Box<dyn Error>> {
    for (status, reason) in [
        ("truncated", "derivation_nodes"),
        ("unsupported", "unsupported_version"),
    ] {
        let mut value = complete_value();
        value["status"] = json!(status);
        value["reason"] = json!(reason);
        assert_eq!(rejection(&value), ReadErrorKind::InvalidSchema);
        value["graph"] = serde_json::Value::Null;
        let checked = NoSolutionEvidence::from_json(&serde_json::to_vec(&value)?, &token())?;
        assert_ne!(checked.status(), CaptureStatus::Complete);
        assert!(checked.0.graph.is_none());
    }
    let mut value = complete_value();
    value["usage"]["derivation_nodes"] = json!(0);
    assert_eq!(rejection(&value), ReadErrorKind::InvalidSchema);
    Ok(())
}

#[test]
fn bounded_writer_does_not_extend_the_buffer_after_limit_failure() -> Result<(), Box<dyn Error>> {
    let mut buffer = BoundedBuffer::new(3);
    buffer.write_all(b"ab")?;
    assert!(buffer.write_all(b"cd").is_err());
    assert!(buffer.exceeded);
    assert_eq!(buffer.bytes, b"ab");
    Ok(())
}
use std::error::Error;
