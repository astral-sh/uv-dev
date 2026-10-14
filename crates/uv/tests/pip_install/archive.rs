use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use async_zip::base::read::seek::ZipFileReader;
use futures::executor::block_on;
use futures::io::Cursor;
use predicates::prelude::predicate;

use uv_test::TestContext;
use uv_test::find_links::FindLinksServer;

const WHEEL_FILENAME: &str = "ok-1.0.0-py3-none-any.whl";

enum Corruption {
    UncompressedSize,
    Crc32,
}

struct CorruptWheel {
    server: FindLinksServer,
    actual: u32,
    declared: u32,
}

fn corrupt_streamed_wheel(context: &TestContext, corruption: Corruption) -> Result<CorruptWheel> {
    let mut wheel = fs_err::read(
        context
            .workspace_root
            .join("test/links")
            .join(WHEEL_FILENAME),
    )?;
    let (offset, actual) = {
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
            ),
            Corruption::Crc32 => (entry.header_offset() + 14, entry.crc32()),
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
    wheelhouse.child(WHEEL_FILENAME).write_binary(&wheel)?;
    let server = FindLinksServer::new(wheelhouse.path());

    Ok(CorruptWheel {
        server,
        actual,
        declared,
    })
}

#[test]
fn reject_incorrect_uncompressed_size() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let fixture = corrupt_streamed_wheel(&context, Corruption::UncompressedSize)?;
    context
        .pip_install()
        .arg("--no-index")
        .arg(format!("ok @ {}/{WHEEL_FILENAME}", fixture.server.url()))
        .assert()
        .code(1)
        .stderr(predicate::str::contains(format!(
            "Failed to extract archive: {WHEEL_FILENAME}"
        )))
        .stderr(predicate::str::contains(format!(
            "Bad uncompressed size (got {:08x}, expected {:08x}) for file:",
            fixture.actual, fixture.declared
        )))
        .stderr(predicate::str::contains("__init__.py"));
    context.assert_not_installed("ok");
    Ok(())
}

#[test]
fn reject_incorrect_crc32() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let fixture = corrupt_streamed_wheel(&context, Corruption::Crc32)?;
    context
        .pip_install()
        .arg("--no-index")
        .arg(format!("ok @ {}/{WHEEL_FILENAME}", fixture.server.url()))
        .assert()
        .code(1)
        .stderr(predicate::str::contains(format!(
            "Failed to extract archive: {WHEEL_FILENAME}"
        )))
        .stderr(predicate::str::contains(format!(
            "Bad CRC (got {:08x}, expected {:08x}) for file:",
            fixture.actual, fixture.declared
        )))
        .stderr(predicate::str::contains("__init__.py"));
    context.assert_not_installed("ok");
    Ok(())
}
