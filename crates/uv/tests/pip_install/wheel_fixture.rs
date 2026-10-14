use std::fmt::Write;
use std::path::Path;

use anyhow::Result;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::executor::block_on;
use indoc::indoc;

/// Write a wheel containing exactly `source_files` numbered Python modules.
pub(super) fn write_many_files_wheel(path: &Path, source_files: usize) -> Result<()> {
    let mut writer = ZipFileWriter::new(Vec::new());
    let mut record = String::new();

    for index in 0..source_files {
        let name = format!("large_wheel/module_{index:05}.py");
        let entry = ZipEntryBuilder::new(name.clone().into(), Compression::Stored);
        block_on(writer.write_entry_whole(entry, b"VALUE = 1\n"))?;
        writeln!(record, "{name},,")?;
    }

    let metadata = indoc! {"
        Metadata-Version: 2.1
        Name: large-wheel
        Version: 1.0.0
    "};
    let wheel = indoc! {"
        Wheel-Version: 1.0
        Generator: uv-test
        Root-Is-Purelib: true
        Tag: py3-none-any
    "};
    for (name, contents) in [
        ("large_wheel-1.0.0.dist-info/METADATA", metadata),
        ("large_wheel-1.0.0.dist-info/WHEEL", wheel),
    ] {
        let entry = ZipEntryBuilder::new(name.into(), Compression::Stored);
        block_on(writer.write_entry_whole(entry, contents.as_bytes()))?;
        writeln!(record, "{name},,")?;
    }
    record.push_str("large_wheel-1.0.0.dist-info/RECORD,,\n");
    let entry = ZipEntryBuilder::new(
        "large_wheel-1.0.0.dist-info/RECORD".into(),
        Compression::Stored,
    );
    block_on(writer.write_entry_whole(entry, record.as_bytes()))?;

    fs_err::write(path, block_on(writer.close())?)?;
    Ok(())
}
