use std::{fmt::Display, str::FromStr};

use anyhow::Result;
use serde::Deserialize;
use serde::de::value::{
    BorrowedStrDeserializer, Error as ValueError, StrDeserializer, StringDeserializer,
};
use serde_json::{Value, json};

use uv_configuration::{NoBinary, PackageNameSpecifier};

fn selection<E: Display>(result: Result<PackageNameSpecifier, E>) -> Result<NoBinary, String> {
    result
        .map(NoBinary::from_pip_arg)
        .map_err(|error| error.to_string())
}

fn deserialize_values(input: &str) -> [Result<NoBinary, String>; 4] {
    [
        selection(PackageNameSpecifier::deserialize(StrDeserializer::<
            ValueError,
        >::new(input))),
        selection(PackageNameSpecifier::deserialize(
            BorrowedStrDeserializer::<ValueError>::new(input),
        )),
        selection(PackageNameSpecifier::deserialize(StringDeserializer::<
            ValueError,
        >::new(
            input.to_owned()
        ))),
        selection(serde_json::from_value(Value::String(input.to_owned()))),
    ]
}

#[test]
fn deserialize_matches_from_str() -> Result<()> {
    for (input, expected) in [
        (":all:", r#""all""#),
        (":none:", r#""none""#),
        ("requests", r#"{"packages":["requests"]}"#),
        ("R", r#"{"packages":["r"]}"#),
        ("1", r#"{"packages":["1"]}"#),
        ("Friendly._--BARD", r#"{"packages":["friendly-bard"]}"#),
        ("a-1", r#"{"packages":["a-1"]}"#),
    ] {
        let parsed = NoBinary::from_pip_arg(PackageNameSpecifier::from_str(input)?);
        assert_eq!(serde_json::to_string(&parsed)?, expected, "{input:?}");
        for deserialized in deserialize_values(input) {
            assert_eq!(deserialized, Ok(parsed.clone()), "{input:?}");
        }
        let json = serde_json::to_string(input)?;
        assert_eq!(
            selection(serde_json::from_str(&json)),
            Ok(parsed),
            "{input:?}"
        );
    }

    for (json, expected) in [
        (
            r#""Friendly\u002eBard""#,
            r#"{"packages":["friendly-bard"]}"#,
        ),
        (r#"":\u0061ll:""#, r#""all""#),
    ] {
        let deserialized = NoBinary::from_pip_arg(serde_json::from_str(json)?);
        assert_eq!(serde_json::to_string(&deserialized)?, expected, "{json}");
    }

    Ok(())
}

#[test]
fn deserialize_preserves_invalid_name_errors() -> Result<()> {
    for input in [
        "",
        ":ALL:",
        ":if-available:",
        "_leading",
        "trailing.",
        "bad/name",
        "two words",
        "naïve",
        "友",
        "bad\0name",
    ] {
        let expected = format!(
            "Not a valid package or extra name: \"{input}\". Names must start and end with a \
             letter or digit and may only contain -, _, ., and alphanumeric characters."
        );
        assert_eq!(
            selection(PackageNameSpecifier::from_str(input)),
            Err(expected.clone()),
            "{input:?}"
        );
        for deserialized in deserialize_values(input) {
            assert_eq!(deserialized, Err(expected.clone()), "{input:?}");
        }
        let json = serde_json::to_string(input)?;
        assert_eq!(
            selection(serde_json::from_str(&json)),
            Err(format!("{expected} at line 1 column {}", json.len())),
            "{input:?}"
        );
    }

    Ok(())
}

#[test]
fn deserialize_preserves_non_string_errors() {
    for (value, unexpected) in [
        (json!(null), "null"),
        (json!(true), "boolean `true`"),
        (json!(0), "integer `0`"),
        (json!(-2), "integer `-2`"),
        (json!(1.5), "floating point `1.5`"),
        (json!([]), "sequence"),
        (json!({}), "map"),
    ] {
        assert_eq!(
            selection(serde_json::from_value(value)),
            Err(format!(
                "invalid type: {unexpected}, expected a package name or `:all:` or `:none:`"
            ))
        );
    }
}
