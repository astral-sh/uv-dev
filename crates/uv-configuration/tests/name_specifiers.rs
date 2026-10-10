use std::{fmt::Display, str::FromStr};

use anyhow::Result;
use serde::Deserialize;
use serde::de::value::{BorrowedStrDeserializer, Error as ValueError};
use serde_json::json;

use uv_configuration::{NoBinary, PackageNameSpecifier};
use uv_normalize::PackageName;

fn selection<E: Display>(result: Result<PackageNameSpecifier, E>) -> Result<NoBinary, String> {
    result
        .map(NoBinary::from_pip_arg)
        .map_err(|error| error.to_string())
}

fn deserialize(input: &str) -> Result<NoBinary, String> {
    selection(PackageNameSpecifier::deserialize(
        BorrowedStrDeserializer::<ValueError>::new(input),
    ))
}

#[test]
fn deserialize_matches_from_str() -> Result<()> {
    let normalized = NoBinary::Packages(vec![PackageName::from_str("friendly-bard")?]);
    for (input, expected) in [
        (":all:", NoBinary::All),
        (":none:", NoBinary::None),
        ("Friendly._--BARD", normalized.clone()),
    ] {
        let parsed = NoBinary::from_pip_arg(PackageNameSpecifier::from_str(input)?);
        assert_eq!(parsed, expected, "{input:?}");
        assert_eq!(deserialize(input), Ok(parsed), "{input:?}");
    }

    let invalid = ":if-available:";
    let expected = selection(PackageNameSpecifier::from_str(invalid));
    assert!(expected.is_err());
    assert_eq!(deserialize(invalid), expected);
    assert_eq!(
        selection(serde_json::from_str(r#""Friendly\u002eBard""#)),
        Ok(normalized),
    );
    let expected = "invalid type: integer `0`, expected a package name or `:all:` or `:none:`";
    assert_eq!(
        selection(serde_json::from_value(json!(0))),
        Err(expected.to_owned()),
    );

    Ok(())
}
