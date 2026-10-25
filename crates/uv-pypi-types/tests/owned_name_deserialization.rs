use std::error::Error as _;

use serde::Deserialize;
use serde::de::value::{Error, StringDeserializer};
use serde_json::Value;

use uv_pypi_types::{Identifier, ModuleName};

#[test]
fn identifier_owned_string_round_trip() {
    for text in ["alpha", "_", "a123", "férrîs", "안녕하세요"] {
        let identifier =
            Identifier::deserialize(StringDeserializer::<Error>::new(text.to_owned())).unwrap();
        assert_eq!(identifier.as_ref(), text);
        assert_eq!(identifier.to_string(), text);
        assert_eq!(identifier, text.parse::<Identifier>().unwrap());
        assert_eq!(serde_json::to_value(identifier).unwrap(), Value::from(text));
    }
}

#[test]
fn identifier_json_round_trip() {
    for (json, text) in [
        (r#""alpha""#, "alpha"),
        (r#""férrîs""#, "férrîs"),
        (r#""f\u00e9rr\u00ees""#, "férrîs"),
        (r#""안녕하세요""#, "안녕하세요"),
    ] {
        let identifier: Identifier = serde_json::from_str(json).unwrap();
        assert_eq!(identifier.as_ref(), text);
        assert_eq!(serde_json::to_value(identifier).unwrap(), Value::from(text));
    }
}

#[test]
fn identifier_deserialization_preserves_errors() {
    for (text, expected) in [
        ("", "An identifier must not be empty"),
        (
            "1alpha",
            "Invalid first character `1` for identifier `1alpha`, expected an underscore or an alphabetic character",
        ),
        (
            "a-1",
            "Invalid character `-` at position 2 for identifier `a-1`, expected an underscore or an alphanumeric character",
        ),
        (
            "fé-rrîs",
            "Invalid character `-` at position 3 for identifier `fé-rrîs`, expected an underscore or an alphanumeric character",
        ),
    ] {
        assert_eq!(
            Identifier::deserialize(StringDeserializer::<Error>::new(text.to_owned()))
                .unwrap_err()
                .to_string(),
            expected,
        );
        assert_eq!(
            serde_json::from_value::<Identifier>(Value::from(text))
                .unwrap_err()
                .to_string(),
            expected,
        );
        assert_eq!(
            text.parse::<Identifier>().unwrap_err().to_string(),
            expected
        );
    }
    assert_eq!(
        serde_json::from_value::<Identifier>(Value::Bool(true))
            .unwrap_err()
            .to_string(),
        "invalid type: boolean `true`, expected a string",
    );
}

#[test]
fn module_name_owned_string_round_trip() {
    for text in ["alpha", "alpha.beta", "_", "package.안녕하세요"] {
        let module =
            ModuleName::deserialize(StringDeserializer::<Error>::new(text.to_owned())).unwrap();
        assert_eq!(module.as_ref(), text);
        assert_eq!(module.to_string(), text);
        assert_eq!(module, text.parse::<ModuleName>().unwrap());
        assert_eq!(serde_json::to_value(module).unwrap(), Value::from(text));
    }
}

#[test]
fn module_name_json_round_trip() {
    for (json, text) in [
        (r#""alpha.beta""#, "alpha.beta"),
        (r#""package.férrîs""#, "package.férrîs"),
        (r#""package.f\u00e9rr\u00ees""#, "package.férrîs"),
        (r#""package.안녕하세요""#, "package.안녕하세요"),
    ] {
        let module: ModuleName = serde_json::from_str(json).unwrap();
        assert_eq!(module.as_ref(), text);
        assert_eq!(serde_json::to_value(module).unwrap(), Value::from(text));
    }
}

#[test]
fn module_name_deserialization_preserves_errors() {
    for (text, expected) in [
        ("", "A module name must not be empty"),
        (".alpha", "Invalid module name component `` in `.alpha`"),
        (
            "alpha..beta",
            "Invalid module name component `` in `alpha..beta`",
        ),
        (
            "alpha.beta-gamma",
            "Invalid module name component `beta-gamma` in `alpha.beta-gamma`",
        ),
        (
            "alpha.beta-gamma.1last",
            "Invalid module name component `beta-gamma` in `alpha.beta-gamma.1last`",
        ),
    ] {
        assert_eq!(
            ModuleName::deserialize(StringDeserializer::<Error>::new(text.to_owned()))
                .unwrap_err()
                .to_string(),
            expected,
        );
        assert_eq!(
            serde_json::from_value::<ModuleName>(Value::from(text))
                .unwrap_err()
                .to_string(),
            expected,
        );
        assert_eq!(
            text.parse::<ModuleName>().unwrap_err().to_string(),
            expected
        );
    }
    let error = "alpha.beta-gamma.1last".parse::<ModuleName>().unwrap_err();
    assert_eq!(
        error.source().unwrap().to_string(),
        "Invalid character `-` at position 5 for identifier `beta-gamma`, expected an underscore or an alphanumeric character",
    );
    assert_eq!(
        serde_json::from_value::<ModuleName>(Value::Bool(true))
            .unwrap_err()
            .to_string(),
        "invalid type: boolean `true`, expected a string",
    );
}
