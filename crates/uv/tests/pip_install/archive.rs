use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use async_zip::base::read::seek::ZipFileReader;
use futures::executor::block_on;
use futures::io::Cursor;
use indoc::indoc;
use predicates::prelude::predicate;

use uv_cache::CacheBucket;
use uv_test::archive::generate_source_archive;
use uv_test::find_links::FindLinksServer;
use uv_test::{TestContext, assert_link_target, assert_path_missing, uv_snapshot};

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

/// A tar symlink can precede its target, as in `pgpdump==1.5`. See astral-sh/uv#1858.
#[test]
fn install_source_archive_symlink() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let original = context.temp_dir.child("original.tar.gz");
    original.write_binary(&generate_source_archive(
        &"linked".parse()?,
        &"1.0.0".parse()?,
        "",
        None,
    )?)?;
    let archive = context.temp_dir.child("linked-1.0.0.tar.gz");
    context
        .python_command()
        .arg("-c")
        .arg(indoc! {r#"
            import io
            import sys
            import tarfile

            with tarfile.open(sys.argv[1]) as original, tarfile.open(sys.argv[2], "w:gz") as archive:
                link = tarfile.TarInfo("linked-1.0.0/README")
                link.type = tarfile.SYMTYPE
                link.linkname = "README.md"
                archive.addfile(link)

                contents = b"Linked package\n"
                target = tarfile.TarInfo("linked-1.0.0/README.md")
                target.size = len(contents)
                archive.addfile(target, io.BytesIO(contents))

                for member in original:
                    archive.addfile(member, original.extractfile(member))
        "#})
        .arg(original.path())
        .arg(archive.path())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_install().arg(archive.path()).args(["--offline", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + linked==1.0.0 (from file://[TEMP_DIR]/linked-1.0.0.tar.gz)
    ");
    context.assert_installed("linked", "1.0.0");

    let target = context
        .cache_files(CacheBucket::SourceDistributions)?
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name == "README.md"))
        .context("source distribution was not extracted into the cache")?;
    let link = target.with_file_name("README");
    if cfg!(windows) {
        assert_path_missing(link);
    } else {
        assert_link_target(link, "README.md");
    }

    uv_snapshot!(context.filters(), context.pip_uninstall().arg("linked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - linked==1.0.0 (from file://[TEMP_DIR]/linked-1.0.0.tar.gz)
    ");
    context.assert_not_installed("linked");
    Ok(())
}
