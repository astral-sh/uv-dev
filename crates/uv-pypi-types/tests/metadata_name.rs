use std::assert_matches;

use anyhow::Result;

use uv_pypi_types::{Metadata10, Metadata23, MetadataError, ResolutionMetadata};

fn parse_names(content: &str) -> [(&'static str, Result<String, MetadataError>); 4] {
    let content = content.as_bytes();
    [
        (
            "Metadata10",
            Metadata10::parse_pkg_info(content).map(|metadata| metadata.name.to_string()),
        ),
        (
            "Metadata23",
            Metadata23::parse(content).map(|metadata| metadata.name),
        ),
        (
            "ResolutionMetadata::parse_metadata",
            ResolutionMetadata::parse_metadata(content).map(|metadata| metadata.name.to_string()),
        ),
        (
            "ResolutionMetadata::parse_pkg_info",
            ResolutionMetadata::parse_pkg_info(content).map(|metadata| metadata.name.to_string()),
        ),
    ]
}

#[test]
fn unknown_metadata_name() {
    for headers in [
        "Name: UNKNOWN",
        "nAmE: UNKNOWN",
        "Name: =?utf-8?q?UNKNOWN?=",
        "Name: UNKNOWN\nName: valid",
    ] {
        let content = format!("Metadata-Version: 2.3\n{headers}\nVersion: 1.0.0\n");
        for (parser, result) in parse_names(&content) {
            let error = result.expect_err(parser);
            assert_matches!(error, MetadataError::UnknownName);
            assert_eq!(
                error.to_string(),
                "Metadata field `Name` is set to the placeholder `UNKNOWN`",
                "{parser}: {headers}",
            );
        }
    }
}

#[test]
fn missing_metadata_name() {
    for (_, result) in parse_names("Metadata-Version: 2.3\nVersion: 1.0.0\n") {
        assert_matches!(result, Err(MetadataError::FieldNotFound("Name")));
    }
}

#[test]
fn valid_metadata_names() -> Result<()> {
    for name in ["unknown", "Unknown", "valid", "Foo_Bar"] {
        let content = format!("Metadata-Version: 2.3\nName: {name}\nVersion: 1.0.0\n");
        for (parser, result) in parse_names(&content) {
            let expected = if parser == "Metadata23" {
                name.to_string()
            } else {
                name.to_ascii_lowercase().replace('_', "-")
            };
            assert_eq!(result?, expected, "{parser}: {name}");
        }
    }

    let content = "Metadata-Version: 2.3\nName: valid\nName: UNKNOWN\nVersion: 1.0.0\n";
    for (parser, result) in parse_names(content) {
        assert_eq!(result?, "valid", "{parser}");
    }
    Ok(())
}

#[test]
fn other_unknown_metadata_fields() -> Result<()> {
    let content = "Metadata-Version: 2.3\nName: valid\nVersion: UNKNOWN\n";
    for (_, result) in parse_names(content) {
        assert_matches!(result, Err(MetadataError::FieldNotFound("Version")));
    }

    let content = b"Metadata-Version: 2.3\nName: valid\nVersion: 1.0.0\nSummary: UNKNOWN\nClassifier: UNKNOWN\nRequires-Dist: UNKNOWN\nRequires-Dist: Unknown\nRequires-Python: UNKNOWN\nProvides-Extra: UNKNOWN\nDynamic: UNKNOWN\n";
    let full = Metadata23::parse(content)?;
    assert!(full.summary.is_none());
    assert!(full.classifiers.is_empty());
    assert_eq!(full.requires_dist, ["Unknown"]);
    assert!(full.requires_python.is_none());
    assert!(full.provides_extra.is_empty());
    assert!(full.dynamic.is_empty());
    for metadata in [
        ResolutionMetadata::parse_metadata(content)?,
        ResolutionMetadata::parse_pkg_info(content)?,
    ] {
        assert_eq!(metadata.requires_dist.len(), 1);
        assert_eq!(metadata.requires_dist[0].name.as_ref(), "unknown");
        assert!(metadata.requires_python.is_none());
        assert!(metadata.provides_extra.is_empty());
        assert!(!metadata.dynamic);
    }
    Ok(())
}

#[test]
fn metadata_name_error_order() {
    let content = b"Name: UNKNOWN\nVersion: 1.0.0\n";
    assert_matches!(
        Metadata23::parse(content),
        Err(MetadataError::FieldNotFound("Metadata-Version"))
    );
    assert_matches!(
        ResolutionMetadata::parse_pkg_info(content),
        Err(MetadataError::FieldNotFound("Metadata-Version"))
    );
    assert_matches!(
        ResolutionMetadata::parse_pkg_info(
            b"Metadata-Version: 2.1\nName: UNKNOWN\nVersion: 1.0.0\n"
        ),
        Err(MetadataError::UnsupportedMetadataVersion(_))
    );
    assert_matches!(
        ResolutionMetadata::parse_pkg_info(
            b"Metadata-Version: 2.3\nName: UNKNOWN\nVersion: 1.0.0\nDynamic: Requires-Dist\n"
        ),
        Err(MetadataError::DynamicField("Requires-Dist"))
    );
}
