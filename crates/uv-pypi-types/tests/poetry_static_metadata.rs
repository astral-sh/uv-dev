use std::assert_matches;

use anyhow::Result;
use uv_pep440::Version;
use uv_pypi_types::{MetadataError, PyProjectToml, RequiresDist, ResolutionMetadata};

fn resolution_metadata(source: &str) -> Result<ResolutionMetadata, MetadataError> {
    ResolutionMetadata::parse_pyproject_toml(
        PyProjectToml::from_toml(source, "pyproject.toml")?,
        None,
    )
}

fn requires_dist(source: &str) -> Result<RequiresDist, MetadataError> {
    RequiresDist::from_pyproject_toml(PyProjectToml::from_toml(source, "pyproject.toml")?)
}

#[test]
fn poetry_without_project_dependencies_defers_static_metadata() {
    let source = r#"
        [project]
        name = "example"
        version = "1.0"

        [tool.poetry]
    "#;

    assert_matches!(
        resolution_metadata(source),
        Err(MetadataError::PoetrySyntax)
    );
    assert_matches!(requires_dist(source), Err(MetadataError::PoetrySyntax));
}

#[test]
fn explicit_project_dependencies_remain_authoritative() -> Result<()> {
    for (dependencies, expected) in [
        ("[]", vec![]),
        (r#"["dependency>=2"]"#, vec!["dependency>=2"]),
    ] {
        let source = format!(
            r#"
            [project]
            name = "example"
            version = "1.0"
            dependencies = {dependencies}

            [tool.poetry]
            "#
        );

        let metadata = resolution_metadata(&source)?;
        let requirements = requires_dist(&source)?;
        assert_eq!(
            metadata
                .requires_dist
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            expected,
        );
        assert_eq!(requirements.requires_dist, metadata.requires_dist);
    }
    Ok(())
}

#[test]
fn project_without_poetry_accepts_missing_dependencies() -> Result<()> {
    let source = r#"
        [project]
        name = "example"
        version = "1.0"
    "#;

    assert!(resolution_metadata(source)?.requires_dist.is_empty());
    assert!(requires_dist(source)?.requires_dist.is_empty());
    Ok(())
}

#[test]
fn dynamic_fields_precede_poetry_fallback() {
    for field in [
        "dependencies",
        "optional-dependencies",
        "version",
        "requires-python",
    ] {
        let source = format!(
            r#"
            [project]
            name = "example"
            dynamic = ["{field}"]

            [tool.poetry]
            "#
        );

        assert_matches!(
            resolution_metadata(&source),
            Err(MetadataError::DynamicField(actual)) if actual == field
        );
        if matches!(field, "dependencies" | "optional-dependencies") {
            assert_matches!(
                requires_dist(&source),
                Err(MetadataError::DynamicField(actual)) if actual == field
            );
        } else {
            assert_matches!(requires_dist(&source), Err(MetadataError::PoetrySyntax));
        }
    }
}

#[test]
fn requires_dist_does_not_require_resolution_fields() -> Result<()> {
    let source = r#"
        [project]
        name = "example"
        dependencies = []

        [tool.poetry]
    "#;
    assert_matches!(
        resolution_metadata(source),
        Err(MetadataError::FieldNotFound("version"))
    );
    assert!(!requires_dist(source)?.dynamic);

    let source = r#"
        [project]
        name = "example"
        dependencies = []
        dynamic = ["version", "requires-python"]

        [tool.poetry]
    "#;
    assert_matches!(
        resolution_metadata(source),
        Err(MetadataError::DynamicField("version"))
    );
    assert_matches!(
        ResolutionMetadata::parse_pyproject_toml(
            PyProjectToml::from_toml(source, "pyproject.toml")?,
            Some(&Version::new([1, 0])),
        ),
        Err(MetadataError::DynamicField("requires-python"))
    );
    assert!(requires_dist(source)?.dynamic);
    Ok(())
}
