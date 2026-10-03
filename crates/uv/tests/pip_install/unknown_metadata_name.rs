//! Diagnostics for placeholder names in distribution metadata.

use anyhow::Result;
use assert_fs::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use fs_err::File;
use indoc::indoc;

use uv_test::archive::write_tar_gz;
use uv_test::uv_snapshot;

const WHEEL_FILENAME: &str = "unknown_metadata_name-1.0.0-py3-none-any.whl";
const DIST_INFO: &str = "unknown_metadata_name-1.0.0.dist-info";

fn metadata(name: &str) -> String {
    format!("Metadata-Version: 2.3\nName: {name}\nVersion: 1.0.0\n")
}

/// Generate a wheel containing only distribution metadata, without importable package code.
async fn wheel(name: &str) -> Result<Vec<u8>> {
    let metadata = metadata(name);
    let wheel = indoc! {"
        Wheel-Version: 1.0
        Generator: uv-test
        Root-Is-Purelib: true
        Tag: py3-none-any
    "};
    let record = format!("{DIST_INFO}/METADATA,,\n{DIST_INFO}/WHEEL,,\n{DIST_INFO}/RECORD,,\n");
    let mut writer = ZipFileWriter::new(Vec::new());
    for (filename, contents) in [
        ("METADATA", metadata.as_str()),
        ("WHEEL", wheel),
        ("RECORD", record.as_str()),
    ] {
        let entry = ZipEntryBuilder::new(
            format!("{DIST_INFO}/{filename}").into(),
            Compression::Stored,
        );
        writer.write_entry_whole(entry, contents.as_bytes()).await?;
    }
    Ok(writer.close().await?)
}

#[tokio::test]
async fn unknown_wheel_metadata_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let archive = context.temp_dir.child(WHEEL_FILENAME);
    archive.write_binary(&wheel("UNKNOWN").await?)?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--no-index", "--offline", "--no-python-downloads"])
        .arg(archive.path()), @"
    exit_code: 1 (failure)
    ----- stderr -----
      × Failed to read `unknown-metadata-name @ file://[TEMP_DIR]/unknown_metadata_name-1.0.0-py3-none-any.whl`
      ├─▶ Couldn't parse metadata of unknown_metadata_name-1.0.0-py3-none-any.whl from unknown-metadata-name @ file://[TEMP_DIR]/unknown_metadata_name-1.0.0-py3-none-any.whl
      ╰─▶ Metadata field `Name` is set to the placeholder `UNKNOWN`
    ");
    Ok(())
}

#[tokio::test]
async fn unknown_pkg_info_name_falls_back() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let valid_wheel = context.temp_dir.child(WHEEL_FILENAME);
    valid_wheel.write_binary(&wheel("unknown-metadata-name").await?)?;
    let backend_marker = context.temp_dir.child("backend-marker");
    let context = context
        .with_env("UV_TEST_UNKNOWN_NAME_WHEEL", valid_wheel.path())
        .with_env("UV_TEST_UNKNOWN_NAME_BACKEND_MARKER", backend_marker.path());
    let source = context.temp_dir.child("unknown_metadata_name-1.0.0.tar.gz");
    write_tar_gz(
        File::create(source.path())?,
        &[
            (
                "unknown_metadata_name-1.0.0/pyproject.toml",
                indoc! {r#"
                    [build-system]
                    requires = []
                    build-backend = "backend"
                    backend-path = ["."]
                "#},
            ),
            ("unknown_metadata_name-1.0.0/PKG-INFO", &metadata("UNKNOWN")),
            (
                "unknown_metadata_name-1.0.0/backend.py",
                indoc! {r#"
                    import os
                    import shutil
                    from pathlib import Path

                    def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
                        wheel = Path(os.environ["UV_TEST_UNKNOWN_NAME_WHEEL"])
                        shutil.copyfile(wheel, Path(wheel_directory) / wheel.name)
                        Path(os.environ["UV_TEST_UNKNOWN_NAME_BACKEND_MARKER"]).write_text("executed")
                        return wheel.name
                "#},
            ),
        ],
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--no-index", "--offline", "--no-python-downloads", "--no-build-isolation"])
        .arg(source.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + unknown-metadata-name==1.0.0 (from file://[TEMP_DIR]/unknown_metadata_name-1.0.0.tar.gz)
    ");
    assert_eq!(fs_err::read_to_string(backend_marker.path())?, "executed");
    assert_eq!(
        fs_err::read_to_string(context.site_packages().join(DIST_INFO).join("METADATA"))?,
        metadata("unknown-metadata-name"),
    );
    Ok(())
}
