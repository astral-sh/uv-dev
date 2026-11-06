use std::collections::BTreeMap;

use anyhow::{Context, Result};
use async_compression::tokio::bufread::GzipDecoder;
use async_compression::tokio::write::GzipEncoder;
use async_zip::base::read::seek::ZipFileReader;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::io::Cursor;
use tar_codec::{Archive as _, ArchiveBuilder as _, EntryMetadata, Member, MemberPayload as _};
use tar_codec::{TarArchive, TarEncoder};
use tokio::io::AsyncWriteExt as _;

use uv_distribution_filename::DistFilename;

use super::add_lock_to_distribution;

const LOCK: &[u8] = b"lock-version = \"1.0\"\ncreated-by = \"uv\"\npackages = []\n";
const METADATA: &[u8] = b"Metadata-Version: 2.4\nName: example\nVersion: 1.0\n";

#[tokio::test]
async fn replaces_wheel_lock_and_record_entry() -> Result<()> {
    let basename = "example-1.0-py3-none-any.whl";
    let filename = DistFilename::try_from_normalized_filename(basename)
        .context("expected a wheel filename")?;
    let source_dir = tempfile::tempdir()?;
    let destination_dir = tempfile::tempdir()?;
    let source = source_dir.path().join(basename);
    let destination = destination_dir.path().join(basename);

    let module = b"";
    let wheel = b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n";
    let record = b"example/__init__.py,sha256=47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU,0\n\
        example-1.0.dist-info/METADATA,sha256=IuYghFRt34TsLZ-FfBumt9tECIRZAO8FFv5bQZUC-Go,49\n\
        example-1.0.dist-info/WHEEL,sha256=JCVX9z8V-js2aV5qmQR2E3fiCs-Yu3vPO91cCBmO1JM,59\n\
        example-1.0.dist-info/pylock.toml,sha256=y6BrVzb69n5UsHtWHq6UOV53TFF6fZEKVDaeEmPM-9Q,3\n\
        example-1.0.dist-info/RECORD,,\n";
    let mut writer = ZipFileWriter::new(Vec::new());
    for (path, contents) in [
        ("example/__init__.py", module.as_slice()),
        ("example-1.0.dist-info/METADATA", METADATA),
        ("example-1.0.dist-info/WHEEL", wheel.as_slice()),
        ("example-1.0.dist-info/pylock.toml", b"old"),
        ("example-1.0.dist-info/RECORD", record.as_slice()),
    ] {
        writer
            .write_entry_whole(
                ZipEntryBuilder::new(path.into(), Compression::Stored),
                contents,
            )
            .await?;
    }
    let original = writer.close().await?;
    fs_err::write(&source, &original)?;

    add_lock_to_distribution(&source, &destination, &filename, LOCK).await?;

    assert_eq!(fs_err::read(source)?, original);
    let mut archive = ZipFileReader::new(Cursor::new(fs_err::read(destination)?)).await?;
    let mut contents = BTreeMap::new();
    for index in 0..archive.file().entries().len() {
        let mut entry = archive.reader_with_entry(index).await?;
        let path = entry.entry().filename().as_str()?.to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end_checked(&mut bytes).await?;
        assert!(
            contents.insert(path, bytes).is_none(),
            "duplicate ZIP entry"
        );
    }
    let record = contents
        .remove("example-1.0.dist-info/RECORD")
        .context("expected a wheel RECORD")?;
    let record = String::from_utf8(record)?;
    insta::assert_snapshot!(record, @"
    example/__init__.py,sha256=47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU,0
    example-1.0.dist-info/METADATA,sha256=IuYghFRt34TsLZ-FfBumt9tECIRZAO8FFv5bQZUC-Go,49
    example-1.0.dist-info/WHEEL,sha256=JCVX9z8V-js2aV5qmQR2E3fiCs-Yu3vPO91cCBmO1JM,59
    example-1.0.dist-info/pylock.toml,sha256=q_aGAyAnc99Gl2C-DFMUQAGq74FpqCE5WqLACGTH5HM,53
    example-1.0.dist-info/RECORD,,
    ");
    assert_eq!(
        contents,
        BTreeMap::from([
            ("example/__init__.py".to_owned(), module.to_vec()),
            (
                "example-1.0.dist-info/METADATA".to_owned(),
                METADATA.to_vec()
            ),
            ("example-1.0.dist-info/WHEEL".to_owned(), wheel.to_vec()),
            (
                "example-1.0.dist-info/pylock.toml".to_owned(),
                LOCK.to_vec()
            ),
        ])
    );
    Ok(())
}

#[tokio::test]
async fn replaces_only_the_sdist_lock_next_to_pyproject() -> Result<()> {
    let basename = "example-1.0.tar.gz";
    let filename = DistFilename::try_from_normalized_filename(basename)
        .context("expected a source distribution filename")?;
    let source_dir = tempfile::tempdir()?;
    let destination_dir = tempfile::tempdir()?;
    let source = source_dir.path().join(basename);
    let destination = destination_dir.path().join(basename);

    let project = b"[project]\nname = \"example\"\nversion = \"1.0\"\n";
    let nested_lock = b"example lock for a test fixture";
    let mut original = Vec::new();
    let mut gzip = GzipEncoder::new(&mut original);
    let mut tar = TarEncoder::new(&mut gzip).builder();
    for (path, contents) in [
        ("example-1.0/pyproject.toml", project.as_slice()),
        ("example-1.0/PKG-INFO", METADATA),
        ("example-1.0/pylock.toml", b"old"),
        ("example-1.0/tests/pylock.toml", nested_lock.as_slice()),
    ] {
        tar.add_file(path, contents, EntryMetadata::default())
            .await?;
    }
    tar.finish().await?;
    gzip.shutdown().await?;
    fs_err::write(&source, &original)?;

    add_lock_to_distribution(&source, &destination, &filename, LOCK).await?;

    assert_eq!(fs_err::read(source)?, original);
    let rewritten = fs_err::read(destination)?;
    let archive = TarArchive::new(GzipDecoder::new(rewritten.as_slice()));
    let mut members = archive.members();
    let mut contents = BTreeMap::new();
    while let Some(member) = members.next().await? {
        let path = member.metadata().path.clone();
        let Member::File { mut payload, .. } = member else {
            anyhow::bail!("expected a regular archive member at {path}");
        };
        let mut bytes = Vec::new();
        let mut chunk = Vec::new();
        while payload.next_chunk(&mut chunk, 8 * 1024).await? {
            bytes.extend_from_slice(&chunk);
        }
        assert!(
            contents.insert(path, bytes).is_none(),
            "duplicate tar member"
        );
    }
    assert_eq!(
        contents,
        BTreeMap::from([
            ("example-1.0/pyproject.toml".to_owned(), project.to_vec()),
            ("example-1.0/PKG-INFO".to_owned(), METADATA.to_vec()),
            ("example-1.0/pylock.toml".to_owned(), LOCK.to_vec()),
            (
                "example-1.0/tests/pylock.toml".to_owned(),
                nested_lock.to_vec()
            ),
        ])
    );
    Ok(())
}
