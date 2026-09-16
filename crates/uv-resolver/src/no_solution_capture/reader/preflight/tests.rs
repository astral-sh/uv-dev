use std::cell::Cell;

use serde_json::json;

use super::*;
use crate::no_solution_capture::test_support::{Fixture, basic_tree, options, token};

fn complete_bytes() -> Vec<u8> {
    let fixture = Fixture::new();
    let tree = basic_tree();
    options()
        .capture(fixture.context(&tree))
        .to_json()
        .expect("serializable complete fixture")
}

fn complete_value() -> serde_json::Value {
    serde_json::from_slice(&complete_bytes()).expect("valid fixture JSON")
}

fn check_value(value: &serde_json::Value) -> Result<Budget, CaptureReadError> {
    check(
        &serde_json::to_vec(value).expect("serializable fixture JSON"),
        &token(),
        CaptureLimits::V1,
    )
}

fn rejection(bytes: &[u8]) -> ReadErrorKind {
    check(bytes, &token(), CaptureLimits::V1)
        .err()
        .expect("capture must be rejected")
        .0
}

#[test]
fn schema_grammar_rejects_duplicate_unknown_and_trailing_content() {
    let valid = String::from_utf8(complete_bytes()).expect("fixture is UTF-8");
    assert!(
        check(
            format!("{valid} \n\t").as_bytes(),
            &token(),
            CaptureLimits::V1
        )
        .is_ok()
    );
    for prefix in [r#"{"schema":1,"#, r#"{"\u0073chema":1,"#] {
        let duplicate = valid.replacen('{', prefix, 1);
        assert_eq!(
            rejection(duplicate.as_bytes()),
            ReadErrorKind::InvalidSchema
        );
    }
    let unknown = valid.replacen('{', r#"{"unknown":null,"#, 1);
    assert_eq!(rejection(unknown.as_bytes()), ReadErrorKind::InvalidSchema);
    assert_eq!(
        rejection(format!("{valid}{{}}").as_bytes()),
        ReadErrorKind::InvalidJson
    );
    for end in [0, 1, valid.len() / 2, valid.len() - 1] {
        assert!(check(&valid.as_bytes()[..end], &token(), CaptureLimits::V1).is_err());
    }

    for (field, value) in [
        ("schema", json!(2)),
        ("schema", json!(-1)),
        ("schema", json!(1.0)),
        ("scope", json!("unknown")),
        ("status", json!("partial")),
        ("terminal", json!("build")),
        ("reason", json!("invented")),
    ] {
        let mut invalid = complete_value();
        invalid[field] = value;
        assert_eq!(
            check_value(&invalid).err().expect("invalid schema").0,
            ReadErrorKind::InvalidSchema
        );
    }
}

#[test]
fn request_identity_and_declared_json_budget_are_admission_checks() {
    let mut invalid = complete_value();
    invalid["producer_pid"] = json!(1235);
    assert_eq!(
        check_value(&invalid).err().expect("wrong PID").0,
        ReadErrorKind::MismatchedRequest
    );
    invalid = complete_value();
    invalid["request"] = json!("ffffffffffffffffffffffffffffffff");
    assert_eq!(
        check_value(&invalid).err().expect("wrong nonce").0,
        ReadErrorKind::MismatchedRequest
    );

    invalid = complete_value();
    invalid["limits"]["json_bytes"] = json!(0);
    assert_eq!(
        check_value(&invalid).err().expect("forged byte cap").0,
        ReadErrorKind::Limit(CaptureReason::JsonBytes)
    );
    invalid = complete_value();
    invalid["limits"]["json_bytes"] = json!(CaptureLimits::V1.json_bytes + 1);
    assert_eq!(
        check_value(&invalid).err().expect("unsupported byte cap").0,
        ReadErrorKind::InvalidLimits
    );

    let bytes = complete_bytes();
    assert_eq!(
        check(
            &bytes,
            &token(),
            CaptureLimits {
                json_bytes: bytes.len() - 1,
                ..CaptureLimits::V1
            }
        )
        .err()
        .expect("caller byte cap")
        .0,
        ReadErrorKind::Limit(CaptureReason::JsonBytes)
    );
}

#[test]
fn escaped_string_scratch_is_bounded_by_decoded_utf8_bytes() {
    let limit = CaptureLimits::V1.atom_bytes;
    let ascii = format!("\"{}\"", "\\u0061".repeat(limit));
    assert!(scan_strings(ascii.as_bytes()).is_ok());
    let too_many = format!("\"{}\"", "\\u0061".repeat(limit + 1));
    assert_eq!(
        scan_strings(too_many.as_bytes())
            .err()
            .expect("decoded atom limit")
            .0,
        ReadErrorKind::Limit(CaptureReason::AtomBytes)
    );

    let pairs = format!("\"{}\"", "\\ud83d\\ude00".repeat(limit / 4));
    assert!(scan_strings(pairs.as_bytes()).is_ok());
    let too_many = format!("\"{}\\u0061\"", "\\ud83d\\ude00".repeat(limit / 4));
    assert_eq!(
        scan_strings(too_many.as_bytes())
            .err()
            .expect("surrogate-pair byte limit")
            .0,
        ReadErrorKind::Limit(CaptureReason::AtomBytes)
    );

    for invalid in [
        b"\"\\ud800\"".as_slice(),
        b"\"\\udc00\"".as_slice(),
        b"\"\\ud800\\u0061\"".as_slice(),
        b"\"\\u0g00\"".as_slice(),
        b"\"\\x00\"".as_slice(),
        b"\"\n\"".as_slice(),
        b"\"unterminated".as_slice(),
        &[b'"', 0xff, b'"'],
    ] {
        assert_eq!(
            scan_strings(invalid).err().expect("invalid JSON string").0,
            ReadErrorKind::InvalidJson
        );
    }
}

#[test]
fn component_grammar_accepts_full_width_strings_and_rejects_lossy_forms() {
    let mut valid = complete_value();
    valid["usage"]["text_bytes"] = json!(CaptureLimits::V1.text_bytes);
    valid["usage"]["max_atom_bytes"] = json!(CaptureLimits::V1.atom_bytes);
    valid["graph"]["root_version"]["epoch"] = json!(u64::MAX.to_string());
    valid["graph"]["root_version"]["release"][0] = json!(u64::MAX.to_string());
    valid["graph"]["root_version"]["local"] = json!({
        "segments": [{"value": "18446744073709551616", "kind": "string"}],
        "kind": "segments"
    });
    assert!(check_value(&valid).is_ok());

    for invalid in [
        json!(0),
        json!(-1),
        json!(1.5),
        json!(""),
        json!("00"),
        json!("+1"),
        json!("18446744073709551616"),
    ] {
        let mut value = complete_value();
        value["graph"]["root_version"]["epoch"] = invalid;
        assert_eq!(
            check_value(&value).err().expect("invalid decimal").0,
            ReadErrorKind::InvalidSchema
        );
    }
    for (field, invalid) in [
        ("min", json!("0")),
        ("max", json!("1")),
        ("unknown", json!("0")),
    ] {
        let mut value = complete_value();
        value["graph"]["root_version"][field] = invalid;
        assert_eq!(
            check_value(&value).err().expect("invalid version field").0,
            ReadErrorKind::InvalidSchema
        );
    }
    for local in [
        json!({"kind": "max", "segments": []}),
        json!({"kind": "segments"}),
        json!({"kind": "unknown"}),
        json!({"kind": "segments", "segments": [{"kind": "string", "value": "1"}]}),
        json!({"kind": "segments", "segments": [{"kind": "string", "value": "Mixed"}]}),
        json!({"kind": "segments", "segments": [{"kind": "number", "value": "18446744073709551616"}]}),
    ] {
        let mut value = complete_value();
        value["graph"]["root_version"]["local"] = local;
        assert_eq!(
            check_value(&value).err().expect("invalid local version").0,
            ReadErrorKind::InvalidSchema
        );
    }
}

#[test]
fn adjacent_tag_payloads_are_checked_in_either_order() {
    let valid = String::from_utf8(complete_bytes()).expect("fixture is UTF-8");
    let local = r#""local":{"kind":"segments","segments":[]}"#;
    assert!(valid.contains(local));
    let reordered = valid.replace(local, r#""local":{"segments":[],"kind":"segments"}"#);
    assert!(check(reordered.as_bytes(), &token(), CaptureLimits::V1).is_ok());
    let invalid = valid.replace(local, r#""local":{"segments":[],"kind":"max"}"#);
    assert_eq!(rejection(invalid.as_bytes()), ReadErrorKind::InvalidSchema);
}

#[test]
fn aggregate_resources_are_charged_before_private_reconstruction() {
    let bytes = complete_bytes();
    let usage = check(&bytes, &token(), CaptureLimits::V1)
        .expect("valid preflight")
        .usage;
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
    for (limits, expected) in cases {
        assert_eq!(
            check(&bytes, &token(), limits)
                .err()
                .expect("lowered reader limit")
                .0,
            ReadErrorKind::Limit(expected)
        );
    }
}

struct ProbeDeserializer<'a>(&'a Cell<bool>);

impl<'de> Deserializer<'de> for ProbeDeserializer<'_> {
    type Error = de::value::Error;

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        self.0.set(true);
        Err(de::Error::custom("probe was deserialized"))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier
        ignored_any
    }
}

#[test]
fn excess_items_and_excess_depth_are_rejected_before_deserializing_the_payload() {
    let token = token();
    for (kind, limits, index, reason) in [
        (
            ArrayKind::Nodes,
            CaptureLimits {
                derivation_nodes: 0,
                ..CaptureLimits::V1
            },
            0,
            CaptureReason::DerivationNodes,
        ),
        (
            ArrayKind::Packages,
            CaptureLimits {
                packages: 0,
                ..CaptureLimits::V1
            },
            0,
            CaptureReason::Packages,
        ),
        (
            ArrayKind::Markers,
            CaptureLimits {
                marker_nodes: 0,
                ..CaptureLimits::V1
            },
            0,
            CaptureReason::MarkerNodes,
        ),
        (
            ArrayKind::Terms,
            CaptureLimits {
                terms: 0,
                ..CaptureLimits::V1
            },
            0,
            CaptureReason::Terms,
        ),
        (
            ArrayKind::EncodedIntervals,
            CaptureLimits {
                intervals: 0,
                ..CaptureLimits::V1
            },
            0,
            CaptureReason::Intervals,
        ),
        (
            ArrayKind::VersionEdges,
            CaptureLimits {
                marker_edges: 0,
                ..CaptureLimits::V1
            },
            0,
            CaptureReason::MarkerEdges,
        ),
        (
            ArrayKind::AvailableVersions,
            CaptureLimits {
                availability_entries: 0,
                ..CaptureLimits::V1
            },
            0,
            CaptureReason::AvailabilityEntries,
        ),
        (
            ArrayKind::Release,
            CaptureLimits::V1,
            CaptureLimits::V1.version_components,
            CaptureReason::VersionComponents,
        ),
        (
            ArrayKind::LocalSegments,
            CaptureLimits::V1,
            CaptureLimits::V1.version_components,
            CaptureReason::VersionComponents,
        ),
    ] {
        let visited = Cell::new(false);
        let mut state = State {
            budget: Budget::new(limits),
            token: &token,
            json_bytes: 0,
            rejection: None,
        };
        assert!(
            ItemSeed {
                state: &mut state,
                kind,
                index,
                depth: 1
            }
            .deserialize(ProbeDeserializer(&visited))
            .is_err()
        );
        assert!(!visited.get());
        assert_eq!(state.rejection, Some(ReadErrorKind::Limit(reason)));
    }
    let visited = Cell::new(false);
    let mut state = State {
        budget: Budget::new(CaptureLimits::V1),
        token: &token,
        json_bytes: 0,
        rejection: None,
    };
    assert!(
        Seed {
            state: &mut state,
            shape: Shape::new(Kind::Atom),
            depth: MAX_DEPTH + 1
        }
        .deserialize(ProbeDeserializer(&visited))
        .is_err()
    );
    assert!(!visited.get());
    assert_eq!(state.rejection, Some(ReadErrorKind::InvalidSchema));
}
