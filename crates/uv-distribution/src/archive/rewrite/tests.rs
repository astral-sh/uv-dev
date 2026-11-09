use std::collections::BTreeMap;

use anyhow::{Context, Result};
use async_compression::tokio::bufread::GzipDecoder;
use async_compression::tokio::write::GzipEncoder;
use async_zip::base::read::seek::ZipFileReader;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::io::Cursor;
use sha2::{Digest, Sha256};
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
    let rewritten = fs_err::read(destination)?;
    insta::assert_snapshot!(
        hex::encode(Sha256::digest(&rewritten)),
        @"a87875a16536209d7de7a7505f836ef8003f841be7b2124eac2d61a5e361b50a"
    );
    let mut archive = ZipFileReader::new(Cursor::new(rewritten)).await?;
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

/// ZIP archive comments can contain bytes that are invalid UTF-8.
#[tokio::test]
async fn rewrites_wheel_with_non_utf8_comment() -> Result<()> {
    let basename = "example-1.0-py3-none-any.whl";
    let filename = DistFilename::try_from_normalized_filename(basename)
        .context("expected a wheel filename")?;
    let source_dir = tempfile::tempdir()?;
    let destination_dir = tempfile::tempdir()?;
    let source = source_dir.path().join(basename);
    let destination = destination_dir.path().join(basename);

    let wheel = b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n";
    let record =
        b"example-1.0.dist-info/METADATA,sha256=IuYghFRt34TsLZ-FfBumt9tECIRZAO8FFv5bQZUC-Go,49\n\
        example-1.0.dist-info/WHEEL,sha256=JCVX9z8V-js2aV5qmQR2E3fiCs-Yu3vPO91cCBmO1JM,59\n\
        example-1.0.dist-info/RECORD,,\n";
    let mut writer = ZipFileWriter::new(Vec::new());
    writer.comment("comment?".to_owned());
    for (path, contents) in [
        ("example-1.0.dist-info/METADATA", METADATA),
        ("example-1.0.dist-info/WHEEL", wheel.as_slice()),
        ("example-1.0.dist-info/RECORD", record.as_slice()),
    ] {
        writer
            .write_entry_whole(
                ZipEntryBuilder::new(path.into(), Compression::Stored),
                contents,
            )
            .await?;
    }
    let mut original = writer.close().await?;
    let comment = b"comment\xe9";
    let comment_start = original.len() - comment.len();
    original[comment_start..].copy_from_slice(comment);
    fs_err::write(&source, original)?;

    add_lock_to_distribution(&source, &destination, &filename, LOCK).await?;

    let rewritten = fs_err::read(destination)?;
    let archive = ZipFileReader::new(Cursor::new(rewritten)).await?;
    assert!(archive.file().comment().as_bytes().is_empty());
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
    insta::assert_snapshot!(
        hex::encode(Sha256::digest(&rewritten)),
        @"3e2b405f1a1608998bb832bdff8c94a0eb084c587a0817958855373be11f98bc"
    );
    assert_eq!(
        sdist_contents(&rewritten).await?,
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

#[tokio::test]
async fn rejects_sdist_hard_link_to_replaced_lock() -> Result<()> {
    let basename = "example-1.0.tar.gz";
    let filename = DistFilename::try_from_normalized_filename(basename)
        .context("expected a source distribution filename")?;
    let directory = tempfile::tempdir()?;
    let source = directory.path().join(basename);
    let destination = directory.path().join("rewritten.tar.gz");

    let mut builder = tokio_tar::Builder::new(GzipEncoder::new(Vec::new()));
    let mut header = tokio_tar::Header::new_gnu();
    header.set_entry_type(tokio_tar::EntryType::Regular);
    header.set_size(0);
    builder
        .append_data(
            &mut header,
            "example-1.0/pyproject.toml",
            tokio::io::empty(),
        )
        .await?;
    header.set_size(3);
    builder
        .append_data(&mut header, "example-1.0/pylock.toml", b"old".as_slice())
        .await?;
    header.set_entry_type(tokio_tar::EntryType::Symlink);
    header.set_size(0);
    header.set_link_name(".")?;
    builder
        .append_data(&mut header, "example-1.0/here", tokio::io::empty())
        .await?;
    header.set_entry_type(tokio_tar::EntryType::Link);
    header.set_link_name("example-1.0/here/pylock.toml")?;
    builder
        .append_data(&mut header, "example-1.0/lock-copy", tokio::io::empty())
        .await?;
    let mut gzip = builder.into_inner().await?;
    gzip.shutdown().await?;
    fs_err::write(&source, gzip.into_inner())?;

    let error = add_lock_to_distribution(&source, &destination, &filename, LOCK)
        .await
        .expect_err("the hard link target would be relocated");
    insta::assert_snapshot!(error, @"Cannot replace pylock.toml in a source distribution that contains hard links");
    assert!(!destination.exists());
    Ok(())
}

#[tokio::test]
async fn rewrites_sdist_with_multiple_gzip_members() -> Result<()> {
    let basename = "example-1.0.tar.gz";
    let filename = DistFilename::try_from_normalized_filename(basename)
        .context("expected a source distribution filename")?;
    let directory = tempfile::tempdir()?;
    let source = directory.path().join(basename);
    let destination = directory.path().join("rewritten.tar.gz");

    let project = b"[project]\nname = \"example\"\nversion = \"1.0\"\n";
    let mut tar_bytes = Vec::new();
    let mut tar = TarEncoder::new(&mut tar_bytes).builder();
    tar.add_file(
        "example-1.0/pyproject.toml",
        project.as_slice(),
        EntryMetadata::default(),
    )
    .await?;
    tar.add_file("example-1.0/PKG-INFO", METADATA, EntryMetadata::default())
        .await?;
    tar.finish().await?;

    // The first gzip member contains the first tar header and its padded payload.
    let (first, second) = tar_bytes.split_at(1024);
    let mut original = Vec::new();
    for member in [first, second] {
        let mut gzip = GzipEncoder::new(&mut original);
        gzip.write_all(member).await?;
        gzip.shutdown().await?;
    }
    fs_err::write(&source, original)?;

    add_lock_to_distribution(&source, &destination, &filename, LOCK).await?;

    assert_eq!(
        sdist_contents(&fs_err::read(destination)?).await?,
        BTreeMap::from([
            ("example-1.0/pyproject.toml".to_owned(), project.to_vec()),
            ("example-1.0/PKG-INFO".to_owned(), METADATA.to_vec()),
            ("example-1.0/pylock.toml".to_owned(), LOCK.to_vec()),
        ])
    );
    Ok(())
}

async fn sdist_contents(rewritten: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let archive = TarArchive::new(GzipDecoder::new(rewritten));
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
    Ok(contents)
}
