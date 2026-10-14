use std::fmt::Write;
use std::path::Path;

use anyhow::Result;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::executor::block_on;
use uv_normalize::PackageName;

/// Write a wheel containing exactly `source_files` numbered Python modules.
pub(super) fn write_many_files_wheel(path: &Path, source_files: usize) -> Result<()> {
    let files = (0..source_files)
        .map(|index| {
            (
                format!("large_wheel/module_{index:05}.py"),
                b"VALUE = 1\n".as_slice(),
            )
        })
        .collect::<Vec<_>>();
    write_wheel(path, "large-wheel", &files)
}

/// Write a wheel with a CRLF-terminated Python shebang and a Latin-1 encoded script.
pub(super) fn write_crlf_script_wheel(path: &Path) -> Result<()> {
    let files = [
        (
            "encoded_script/__init__.py".to_string(),
            b"VALUE = 1\n".as_slice(),
        ),
        (
            "encoded_script-1.0.0.data/scripts/encoded-script".to_string(),
            b"#!python\r\n# coding: latin-1\r\nprint('caf\xe9')\r\n".as_slice(),
        ),
    ];
    write_wheel(path, "encoded-script", &files)
}

fn write_wheel(path: &Path, project: &str, files: &[(String, &[u8])]) -> Result<()> {
    let project: PackageName = project.parse()?;
    let dist_info = format!("{}-1.0.0.dist-info", project.as_dist_info_name());
    let metadata = format!("Metadata-Version: 2.1\nName: {project}\nVersion: 1.0.0\n");
    let metadata_path = format!("{dist_info}/METADATA");
    let wheel_path = format!("{dist_info}/WHEEL");
    let wheel =
        b"Wheel-Version: 1.0\nGenerator: uv-test\nRoot-Is-Purelib: true\nTag: py3-none-any\n";

    let mut writer = ZipFileWriter::new(Vec::new());
    let mut record = String::new();
    for (name, contents) in files
        .iter()
        .map(|(name, contents)| (name.as_str(), *contents))
        .chain([
            (metadata_path.as_str(), metadata.as_bytes()),
            (wheel_path.as_str(), wheel.as_slice()),
        ])
    {
        let entry = ZipEntryBuilder::new(name.into(), Compression::Stored);
        block_on(writer.write_entry_whole(entry, contents))?;
        writeln!(record, "{name},,")?;
    }
    let record_path = format!("{dist_info}/RECORD");
    writeln!(record, "{record_path},,")?;
    let entry = ZipEntryBuilder::new(record_path.into(), Compression::Stored);
    block_on(writer.write_entry_whole(entry, record.as_bytes()))?;

    fs_err::write(path, block_on(writer.close())?)?;
    Ok(())
}
