use std::path::Path;

use anyhow::Result;
use async_compression::tokio::write::ZstdEncoder;
use tokio::io::AsyncWriteExt;
use uv_distribution_filename::{LegacySourceDistExtension, SourceDistExtension};

#[tokio::test]
async fn tar_zst_hardlink_effective_size() -> Result<()> {
    let mut tar = tokio_tar::Builder::new_non_terminated(ZstdEncoder::new(Vec::new()));
    let contents = b"VALUE = 1\n";

    let mut header = tokio_tar::Header::new_gnu();
    header.set_path("basic_package/__init__.py")?;
    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append(&header, contents.as_slice()).await?;

    let mut header = tokio_tar::Header::new_gnu();
    header.set_entry_type(tokio_tar::EntryType::Link);
    header.set_path("basic_package/linked.py")?;
    header.set_link_name("basic_package/__init__.py")?;
    header.set_size(0);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append(&header, tokio::io::empty()).await?;

    let mut encoder = tar.into_inner().await?;
    encoder.shutdown().await?;
    let archive = encoder.into_inner();
    let target = tempfile::tempdir()?;
    let _preview = uv_preview::test::with_features(&[]);

    let files = uv_extract::stream::archive(
        archive.as_slice(),
        SourceDistExtension::Legacy(LegacySourceDistExtension::TarZst),
        target.path(),
    )
    .await?;
    let files = files
        .iter()
        .map(|file| (file.path(), file.size()))
        .collect::<Vec<_>>();

    assert_eq!(
        files,
        [
            (
                Path::new("basic_package/__init__.py"),
                contents.len() as u64,
            ),
            (Path::new("basic_package/linked.py"), contents.len() as u64,),
        ]
    );
    assert_eq!(
        fs_err::read(target.path().join("basic_package/linked.py"))?,
        contents
    );

    Ok(())
}
