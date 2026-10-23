use std::error::Error;

use anyhow::{Context, Result};

use uv_distribution_types::{InstalledDist, InstalledDistError};
use uv_fs::Simplified;
use uv_pypi_types::MetadataError;

const MISSING_NAME_METADATA: &str = "Metadata-Version: 2.3\nVersion: 1.0.0\n\n";

fn assert_missing_name_source(
    error: &InstalledDistError,
    parse_error: &MetadataError,
) -> Result<()> {
    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<Box<MetadataError>>())
        .context("missing boxed metadata error source")?
        .as_ref();
    assert!(std::ptr::eq(source, parse_error));
    let MetadataError::FieldNotFound(field) = source else {
        anyhow::bail!("unexpected metadata error: {source:?}");
    };
    assert_eq!(*field, "Name");
    assert_eq!(source.to_string(), "Metadata field Name not found");
    Ok(())
}

#[test]
fn installed_dist_info_metadata_error_preserves_source() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let distribution = temp_dir.path().join("broken-1.0.0.dist-info");
    fs_err::create_dir_all(&distribution)?;
    let metadata = distribution.join("METADATA");
    fs_err::write(&metadata, MISSING_NAME_METADATA)?;

    let installed =
        InstalledDist::try_from_path(&distribution)?.context("missing installed distribution")?;
    let error = installed
        .read_metadata()
        .expect_err("the metadata does not contain a name");
    let InstalledDistError::MetadataParse { path, err } = &error else {
        anyhow::bail!("unexpected installed-distribution error: {error:?}");
    };
    assert_eq!(path, &metadata);
    assert_eq!(
        error.to_string(),
        format!(
            "Failed to parse METADATA file: `{}`",
            metadata.user_display()
        )
    );
    assert_missing_name_source(&error, err)?;
    Ok(())
}

#[test]
fn installed_egg_info_metadata_error_preserves_source() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let directory = temp_dir.path().join("directory/broken-1.0.0.egg-info");
    let file = temp_dir.path().join("file/broken-1.0.0.egg-info");

    for (distribution, metadata) in [
        (directory.clone(), directory.join("PKG-INFO")),
        (file.clone(), file),
    ] {
        fs_err::create_dir_all(metadata.parent().context("missing metadata parent")?)?;
        fs_err::write(&metadata, MISSING_NAME_METADATA)?;

        let installed = InstalledDist::try_from_path(&distribution)?
            .context("missing installed egg distribution")?;
        let error = installed
            .read_metadata()
            .expect_err("the metadata does not contain a name");
        let InstalledDistError::PkgInfoParse { path, err } = &error else {
            anyhow::bail!("unexpected installed-distribution error: {error:?}");
        };
        assert_eq!(path, &metadata);
        assert_eq!(
            error.to_string(),
            format!(
                "Failed to parse `PKG-INFO` file: `{}`",
                metadata.user_display()
            )
        );
        assert_missing_name_source(&error, err)?;
    }
    Ok(())
}
