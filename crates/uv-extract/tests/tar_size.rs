use std::path::Path;

use anyhow::Result;
use tokio_tar::{EntryType, Header};
use uv_distribution_filename::{LegacySourceDistExtension, SourceDistExtension};
use uv_preview::PreviewFeature;

fn append_member(
    archive: &mut Vec<u8>,
    path: &str,
    entry_type: EntryType,
    header_size: u64,
    payload: &[u8],
) -> Result<()> {
    let mut header = Header::new_ustar();
    header.set_path(path)?;
    header.set_entry_type(entry_type);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(header_size);
    header.set_cksum();
    archive.extend_from_slice(header.as_bytes());
    archive.extend_from_slice(payload);
    archive.resize(archive.len().next_multiple_of(512), 0);
    Ok(())
}

#[tokio::test]
async fn pax_size_controls_extraction_and_reported_size() -> Result<()> {
    let payload = [b'x'; 600];
    let mut archive = Vec::new();
    append_member(
        &mut archive,
        "PaxHeaders/payload",
        EntryType::XHeader,
        12,
        b"12 size=600\n",
    )?;
    // The PAX size crosses a block boundary and overrides this zero-sized ustar header.
    append_member(&mut archive, "payload.bin", EntryType::Regular, 0, &payload)?;
    append_member(&mut archive, "after.txt", EntryType::Regular, 5, b"after")?;
    archive.resize(archive.len() + 1024, 0);

    for features in [&[][..], &[PreviewFeature::TarCodec][..]] {
        let _guard = uv_preview::test::with_features(features);
        let (target, files) = uv_extract::stream::archive(
            archive.as_slice(),
            SourceDistExtension::Legacy(LegacySourceDistExtension::Tar),
            tempfile::tempdir()?,
        )
        .await?;

        assert_eq!(fs_err::read(target.path().join("payload.bin"))?, payload);
        assert_eq!(fs_err::read(target.path().join("after.txt"))?, b"after");
        assert_eq!(
            files
                .iter()
                .map(|file| (file.path(), file.size()))
                .collect::<Vec<_>>(),
            [(Path::new("payload.bin"), 600), (Path::new("after.txt"), 5)],
        );
    }
    Ok(())
}
