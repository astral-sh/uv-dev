//! Deterministic ZIP integrity regressions that do not require the external archive corpus.

use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::io::AsyncWriteExt;

async fn unzip_seekable_bytes(bytes: &[u8]) -> anyhow::Result<(), uv_extract::Error> {
    let archive = tempfile::NamedTempFile::new().map_err(uv_extract::Error::Io)?;
    fs_err::write(archive.path(), bytes).map_err(uv_extract::Error::Io)?;
    let archive = fs_err::File::open(archive.path()).map_err(uv_extract::Error::Io)?;

    let target = tempfile::TempDir::new().map_err(uv_extract::Error::Io)?;
    let target_path = target.path().to_path_buf();
    tokio::task::spawn_blocking(move || uv_extract::unzip(archive, &target_path))
        .await
        .expect("seekable ZIP extraction task should not panic")?;
    Ok(())
}

/// Both extraction paths must reject a local-file-header CRC that disagrees with the entry data
/// and central-directory record.
#[tokio::test]
async fn reject_conflicting_local_header_crc() {
    let mut writer = ZipFileWriter::new(Vec::new());
    let entry = ZipEntryBuilder::new("file".into(), Compression::Stored);
    writer
        .write_entry_whole(entry, b"hello")
        .await
        .expect("ZIP entry should be written");
    let mut bytes = writer.close().await.expect("ZIP should be written");
    bytes[14..18].copy_from_slice(&0u32.to_le_bytes());

    let seekable = unzip_seekable_bytes(&bytes).await;
    insta::assert_debug_snapshot!(seekable, @r#"
    Err(
        ConflictingChecksums {
            path: "file",
            offset: 0,
            local_crc32: 0,
            central_directory_crc32: 907060870,
        },
    )
    "#);

    let target = tempfile::TempDir::new().expect("target directory should be created");
    let streaming = uv_extract::stream::unzip(bytes.as_slice(), target.path()).await;
    insta::assert_debug_snapshot!(streaming, @r#"
    Err(
        BadCrc32 {
            path: "file",
            computed: 907060870,
            expected: 0,
        },
    )
    "#);
}

/// Both extraction paths must reject a data-descriptor CRC that disagrees with the entry data and
/// central-directory record, for both the standard and ZIP64 descriptor layouts.
#[tokio::test]
async fn reject_conflicting_data_descriptor_crc() {
    let mut results = Vec::new();
    for zip64 in [false, true] {
        let mut writer = ZipFileWriter::new(Vec::new());
        if !zip64 {
            writer = writer.force_no_zip64();
        }
        let entry = ZipEntryBuilder::new("file".into(), Compression::Deflate);
        let mut entry = writer
            .write_entry_stream(entry)
            .await
            .expect("ZIP entry stream should be created");
        entry
            .write_all(b"hello")
            .await
            .expect("ZIP entry should be written");
        entry.close().await.expect("ZIP entry should be finalized");
        let mut bytes = writer.close().await.expect("ZIP should be written");

        if !zip64 {
            // `write_entry_stream` currently swaps the standard central-directory sizes.
            // Restore them so this exercises a valid descriptor-backed archive with only its
            // checksum corrupted.
            let central_directory = bytes
                .windows(4)
                .position(|candidate| candidate == b"PK\x01\x02")
                .expect("central-directory entry should be present");
            bytes[central_directory + 20..central_directory + 24]
                .copy_from_slice(&7u32.to_le_bytes());
            bytes[central_directory + 24..central_directory + 28]
                .copy_from_slice(&5u32.to_le_bytes());
        }

        let signature = b"PK\x07\x08";
        let descriptor_offset = bytes
            .windows(signature.len())
            .position(|candidate| candidate == signature)
            .expect("data descriptor should be present");
        bytes[descriptor_offset + 4..descriptor_offset + 8].copy_from_slice(&0u32.to_le_bytes());

        let seekable = unzip_seekable_bytes(&bytes).await;
        let target = tempfile::TempDir::new().expect("target directory should be created");
        let streaming = uv_extract::stream::unzip(bytes.as_slice(), target.path()).await;
        results.push((seekable, streaming));
    }

    insta::assert_debug_snapshot!(results, @r#"
    [
        (
            Err(
                ConflictingChecksums {
                    path: "file",
                    offset: 0,
                    local_crc32: 0,
                    central_directory_crc32: 907060870,
                },
            ),
            Err(
                BadCrc32 {
                    path: "file",
                    computed: 907060870,
                    expected: 0,
                },
            ),
        ),
        (
            Err(
                ConflictingChecksums {
                    path: "file",
                    offset: 0,
                    local_crc32: 0,
                    central_directory_crc32: 907060870,
                },
            ),
            Err(
                BadCrc32 {
                    path: "file",
                    computed: 907060870,
                    expected: 0,
                },
            ),
        ),
    ]
    "#);
}

/// An appended, independently valid central-directory chain must not hide contents following the
/// first end-of-central-directory record, even when padding moves that record outside the maximum
/// ZIP comment window.
#[tokio::test]
async fn reject_padded_appended_central_directory_chain() {
    let mut writer = ZipFileWriter::new(Vec::new());
    let entry = ZipEntryBuilder::new("file".into(), Compression::Stored);
    writer
        .write_entry_whole(entry, b"hello")
        .await
        .expect("ZIP entry should be written");
    let mut bytes = writer.close().await.expect("ZIP should be written");

    let eocd_offset = bytes.len() - 22;
    let central_directory_offset = usize::try_from(u32::from_le_bytes(
        bytes[eocd_offset + 16..eocd_offset + 20]
            .try_into()
            .expect("EOCD central-directory offset should be present"),
    ))
    .expect("central-directory offset should fit in usize");
    let central_directory = bytes[central_directory_offset..eocd_offset].to_vec();
    let mut eocd = bytes[eocd_offset..].to_vec();

    bytes.resize(bytes.len() + usize::from(u16::MAX) + 23, 0);
    let appended_directory_offset =
        u32::try_from(bytes.len()).expect("appended central-directory offset should fit in u32");
    eocd[16..20].copy_from_slice(&appended_directory_offset.to_le_bytes());
    bytes.extend(central_directory);
    bytes.extend(eocd);

    let seekable = unzip_seekable_bytes(&bytes).await;
    insta::assert_debug_snapshot!(seekable, @"
    Err(
        TrailingContents,
    )
    ");

    let target = tempfile::TempDir::new().expect("target directory should be created");
    let streaming = uv_extract::stream::unzip(bytes.as_slice(), target.path()).await;
    insta::assert_debug_snapshot!(streaming, @"
    Err(
        TrailingContents,
    )
    ");
}
