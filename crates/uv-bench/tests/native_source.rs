use std::path::Path;
use uv_bench::{NATIVE_SOURCE_FIXTURES, PreparedNativeSource};

#[test]
fn adapts_packaging_without_changing_source_files() {
    let fixture = &NATIVE_SOURCE_FIXTURES[0];
    assert_eq!(fixture.name, "sampleproject");
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/packages/setuptools_editable");
    let prepared = PreparedNativeSource::new(fixture, &source, "0.12.13");
    let original: toml::Table =
        toml::from_str(&fs_err::read_to_string(source.join("pyproject.toml")).unwrap()).unwrap();
    let metadata: toml::Table =
        toml::from_str(&fs_err::read_to_string(prepared.path().join("pyproject.toml")).unwrap())
            .unwrap();
    assert_eq!(
        metadata["project"]["dependencies"],
        original["project"]["dependencies"]
    );
    assert_eq!(metadata["project"]["version"].as_str(), Some("4.0.0"));
    assert_eq!(
        metadata["build-system"]["build-backend"].as_str(),
        Some("uv_build")
    );
    assert_eq!(
        metadata["build-system"]["requires"][0].as_str(),
        Some("uv_build==0.12.13")
    );
    assert_eq!(
        metadata["tool"]["uv"]["build-backend"]["module-name"].as_str(),
        Some("sample")
    );
    assert_eq!(
        fs_err::read(source.join("setuptools_editable/__init__.py")).unwrap(),
        fs_err::read(prepared.path().join("setuptools_editable/__init__.py")).unwrap()
    );
}
