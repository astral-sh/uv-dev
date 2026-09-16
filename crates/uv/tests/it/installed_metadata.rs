use std::error::Error;

use anyhow::{Context, Result};

use uv_distribution_types::{InstalledDist, InstalledDistError, InstalledDistKind};
use uv_fs::Simplified;
use uv_pypi_types::MetadataError;

const MISSING_NAME_METADATA: &str = "Metadata-Version: 2.3\nVersion: 1.0.0\n\n";
const VALID_METADATA: &str = "Name: fixture\nVersion: 1.0.0\n\n";

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

#[test]
fn installed_metadata_caches_only_successful_reads() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let registry = temp_dir.path().join("registry/fixture-1.0.0.dist-info");
    let url = temp_dir.path().join("url/fixture-1.0.0.dist-info");
    let file = temp_dir.path().join("file/fixture-1.0.0.egg-info");
    let directory = temp_dir.path().join("directory/fixture-1.0.0.egg-info");
    let egg_link = temp_dir.path().join("legacy/fixture.egg-link");
    let legacy_metadata = temp_dir
        .path()
        .join("legacy")
        .join("source")
        .join("fixture.egg-info")
        .join("PKG-INFO");

    fs_err::create_dir_all(&url)?;
    fs_err::write(
        url.join("direct_url.json"),
        r#"{"url":"https://example.invalid/fixture-1.0.0.whl","archive_info":{}}"#,
    )?;
    fs_err::create_dir_all(egg_link.parent().context("missing egg-link parent")?)?;
    fs_err::write(&egg_link, "source\n")?;

    for (expected_kind, distribution, metadata) in [
        ("registry", registry.clone(), registry.join("METADATA")),
        ("url", url.clone(), url.join("METADATA")),
        ("egg-info file", file.clone(), file),
        (
            "egg-info directory",
            directory.clone(),
            directory.join("PKG-INFO"),
        ),
        ("legacy editable", egg_link, legacy_metadata),
    ] {
        fs_err::create_dir_all(metadata.parent().context("missing metadata parent")?)?;
        fs_err::write(&metadata, VALID_METADATA)?;
        let installed = InstalledDist::try_from_path(&distribution)?
            .context("missing installed distribution")?;
        assert_eq!(
            match &installed.kind {
                InstalledDistKind::Registry(_) => "registry",
                InstalledDistKind::Url(_) => "url",
                InstalledDistKind::EggInfoFile(_) => "egg-info file",
                InstalledDistKind::EggInfoDirectory(_) => "egg-info directory",
                InstalledDistKind::LegacyEditable(_) => "legacy editable",
            },
            expected_kind
        );

        fs_err::remove_file(&metadata)?;
        let expected = fs_err::read(&metadata).expect_err("the metadata file is absent");
        let error = installed
            .read_metadata()
            .expect_err("the metadata file is absent");
        let InstalledDistError::Io(io_error) = &error else {
            anyhow::bail!("unexpected installed-distribution error: {error:?}");
        };
        assert_eq!(io_error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(io_error.raw_os_error(), expected.raw_os_error());
        assert_eq!(error.to_string(), expected.to_string());

        fs_err::write(&metadata, MISSING_NAME_METADATA)?;
        let error = installed
            .read_metadata()
            .expect_err("the metadata does not contain a name");
        let parse_error = match (&installed.kind, &error) {
            (
                InstalledDistKind::Registry(_) | InstalledDistKind::Url(_),
                InstalledDistError::MetadataParse { path, err },
            )
            | (
                InstalledDistKind::EggInfoFile(_)
                | InstalledDistKind::EggInfoDirectory(_)
                | InstalledDistKind::LegacyEditable(_),
                InstalledDistError::PkgInfoParse { path, err },
            ) => {
                assert_eq!(path, &metadata);
                err
            }
            _ => anyhow::bail!("unexpected installed-distribution error: {error:?}"),
        };
        assert_missing_name_source(&error, parse_error)?;

        fs_err::write(&metadata, VALID_METADATA)?;
        let cached = installed.read_metadata()?;
        assert_eq!(cached.name.as_ref(), "fixture");
        assert_eq!(cached.version.to_string(), "1.0.0");
        fs_err::write(&metadata, MISSING_NAME_METADATA)?;
        assert!(std::ptr::eq(cached, installed.read_metadata()?));
        fs_err::remove_file(&metadata)?;
        assert!(std::ptr::eq(cached, installed.read_metadata()?));
    }
    Ok(())
}

#[test]
fn installed_metadata_keeps_resolution_parser_semantics() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let directory = temp_dir.path().join("fixture-1.0.0.dist-info");
    let file = temp_dir.path().join("fixture-1.0.0.egg-info");
    for (distribution, metadata) in [
        (directory.clone(), directory.join("METADATA")),
        (file.clone(), file),
    ] {
        fs_err::create_dir_all(metadata.parent().context("missing metadata parent")?)?;
        // Installed resolution metadata does not require Metadata-Version, validate import-name
        // overlap, or decode the description body.
        fs_err::write(
            &metadata,
            b"Name: fixture\nVersion: 1.0.0\nImport-Name: fixture\nImport-Namespace: fixture\n\n\xff",
        )?;
        let installed = InstalledDist::try_from_path(&distribution)?
            .context("missing installed distribution")?;
        let parsed = installed.read_metadata()?;
        assert_eq!(parsed.name.as_ref(), "fixture");
        assert_eq!(parsed.version.to_string(), "1.0.0");
    }
    Ok(())
}
