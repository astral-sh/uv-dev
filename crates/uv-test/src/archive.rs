//! Helpers for constructing test archives.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use async_zip::ZipEntry;
use async_zip::base::write::ZipFileWriter;
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD as base64};
use flate2::write::GzEncoder;
use futures::executor::block_on;
use futures::io::AllowStdIo;
use indoc::{formatdoc, indoc};
use sha2::{Digest, Sha256};
use tar_codec::{ArchiveBuilder as _, EntryMetadata, TarEncoder};
use tokio_util::compat::FuturesAsyncWriteCompatExt;

use uv_fs::PythonExt;
use uv_normalize::PackageName;
use uv_pep440::Version;

use crate::packse::generate_wheel;

/// Whether generated wheel records include file hashes and sizes.
#[derive(Debug, Clone, Copy)]
pub enum RecordHashes {
    Include,
    Omit,
}

/// Assemble a wheel from exact ZIP entries and append its ordinary `RECORD` file.
///
/// Entry names, bytes, compression, and permissions are supplied by the caller. This does not add
/// package metadata or modules. Fixtures with intentionally malformed records use raw ZIP writers.
pub fn generate_wheel_from_entries<'a>(
    entries: impl IntoIterator<Item = (ZipEntry, &'a [u8])>,
    record_entry: ZipEntry,
    record_hashes: RecordHashes,
) -> Result<Vec<u8>> {
    let mut writer = ZipFileWriter::new(Vec::new());
    let mut record = String::new();
    for (entry, contents) in entries {
        let path = entry.filename().as_str()?;
        match record_hashes {
            RecordHashes::Include => {
                let hash = base64.encode(Sha256::digest(contents));
                writeln!(record, "{path},sha256={hash},{}", contents.len())?;
            }
            RecordHashes::Omit => writeln!(record, "{path},,")?,
        }
        block_on(writer.write_entry_whole(entry, contents))?;
    }
    writeln!(record, "{},,", record_entry.filename().as_str()?)?;
    block_on(writer.write_entry_whole(record_entry, record.as_bytes()))?;
    Ok(block_on(writer.close())?)
}

/// Write the given files to a gzip-compressed tar archive.
pub fn write_tar_gz(writer: impl Write, entries: &[(&str, impl AsRef<[u8]>)]) -> Result<()> {
    let mut encoder = GzEncoder::new(writer, flate2::Compression::default());
    let mut tar = TarEncoder::new(AllowStdIo::new(&mut encoder).compat_write()).builder();

    for (path, contents) in entries {
        block_on(tar.add_file(path, contents.as_ref(), EntryMetadata::default()))?;
    }

    block_on(tar.finish())?;
    encoder.finish()?;
    Ok(())
}

/// Create a source archive with a backend that provides metadata and builds wheels.
///
/// An empty `subdirectory` produces an sdist. Dynamic dependencies require running the backend.
/// If `marker_path` is set, importing the backend creates that file.
/// The path is stored in the archive, so changing it changes the archive's hash.
pub fn generate_source_archive(
    name: &PackageName,
    version: &Version,
    subdirectory: &str,
    marker_path: Option<&Path>,
) -> Result<Vec<u8>> {
    let (filename, wheel) = generate_wheel(
        name,
        version,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let pkg_info = formatdoc! {r"
        Metadata-Version: 2.2
        Name: {name}
        Version: {version}
        Dynamic: Requires-Dist
    "};
    let name = name.as_dist_info_name();
    let pyproject = indoc! {r#"
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#};
    let marker = if let Some(marker_path) = marker_path {
        format!("Path({}).touch()", marker_path.escape_for_python())
    } else {
        String::new()
    };
    let backend = formatdoc! {r#"
        import shutil
        from pathlib import Path
        from zipfile import ZipFile

        {marker}

        WHEEL = Path(__file__).with_name("{filename}")
        DIST_INFO = "{name}-{version}.dist-info"

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            shutil.copyfile(WHEEL, Path(wheel_directory) / WHEEL.name)
            return WHEEL.name

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            dist_info = Path(metadata_directory) / DIST_INFO
            dist_info.mkdir()
            with ZipFile(WHEEL) as wheel:
                (dist_info / "METADATA").write_bytes(wheel.read(f"{{DIST_INFO}}/METADATA"))
            return dist_info.name
    "#};
    let mut prefix = format!("{name}-{version}/");
    if !subdirectory.is_empty() {
        prefix.push_str(subdirectory.trim_end_matches('/'));
        prefix.push('/');
    }
    let mut archive = Vec::new();
    write_tar_gz(
        &mut archive,
        &[
            (&format!("{prefix}pyproject.toml"), pyproject.as_bytes()),
            (&format!("{prefix}PKG-INFO"), pkg_info.as_bytes()),
            (&format!("{prefix}backend.py"), backend.as_bytes()),
            (&format!("{prefix}{filename}"), wheel.as_slice()),
        ],
    )?;
    Ok(archive)
}
