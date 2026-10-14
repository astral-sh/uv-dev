use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use async_zip::base::read::seek::ZipFileReader;
use futures::executor::block_on;
use futures::io::Cursor;
use predicates::prelude::predicate;

use uv_test::find_links::FindLinksServer;

enum Corruption {
    UncompressedSize,
    Crc32,
}

fn reject_corrupt_streamed_wheel(corruption: Corruption) -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let filename = "ok-1.0.0-py3-none-any.whl";
    let mut wheel = fs_err::read(context.workspace_root.join("test/links").join(filename))?;
    let (offset, actual, error_kind) = {
        let archive = block_on(ZipFileReader::new(Cursor::new(wheel.as_slice())))?;
        let entry = archive
            .file()
            .entries()
            .iter()
            .find(|entry| matches!(entry.filename().as_str(), Ok("ok/__init__.py")))
            .context("fixture wheel is missing its module")?;
        assert!(
            !entry.data_descriptor(),
            "expected values in the local header"
        );
        // ZIP local headers store CRC-32 at byte 14 and uncompressed size at byte 22.
        match corruption {
            Corruption::UncompressedSize => (
                entry.header_offset() + 22,
                u32::try_from(entry.uncompressed_size())?,
                "uncompressed size",
            ),
            Corruption::Crc32 => (entry.header_offset() + 14, entry.crc32(), "CRC"),
        }
    };
    let offset = usize::try_from(offset)?;
    let field = &mut wheel[offset..offset + 4];
    assert_eq!(u32::from_le_bytes(field[..].try_into()?), actual);
    let declared = actual
        .checked_sub(1)
        .context("fixture value must be nonzero")?;
    field.copy_from_slice(&declared.to_le_bytes());

    let wheelhouse = context.temp_dir.child("wheelhouse");
    wheelhouse.create_dir_all()?;
    wheelhouse.child(filename).write_binary(&wheel)?;
    let server = FindLinksServer::new(wheelhouse.path());
    context
        .pip_install()
        .arg("--no-index")
        .arg(format!("ok @ {}/{filename}", server.url()))
        .assert()
        .code(1)
        .stderr(predicate::str::contains(format!(
            "Failed to extract archive: {filename}"
        )))
        .stderr(predicate::str::contains(format!(
            "Bad {error_kind} (got {actual:08x}, expected {declared:08x}) for file:"
        )))
        .stderr(predicate::str::contains("__init__.py"));
    context.assert_not_installed("ok");
    Ok(())
}

#[test]
fn reject_incorrect_uncompressed_size() -> Result<()> {
    reject_corrupt_streamed_wheel(Corruption::UncompressedSize)
}

#[test]
fn reject_incorrect_crc32() -> Result<()> {
    reject_corrupt_streamed_wheel(Corruption::Crc32)
}
