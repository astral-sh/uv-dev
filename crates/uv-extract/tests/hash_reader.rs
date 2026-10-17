use anyhow::Result;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use uv_extract::hash::{HashReader, Hasher};
use uv_pypi_types::{HashAlgorithm, HashDigest};

#[tokio::test]
async fn finish_hashes_the_remaining_bytes() -> Result<()> {
    let contents = (0u8..=250).cycle().take(20_000).collect::<Vec<_>>();
    let mut hashers = [Hasher::from(HashAlgorithm::Sha256)];
    {
        let mut reader = HashReader::new(contents.as_slice(), &mut hashers);
        let mut prefix = [0; 13];
        reader.read_exact(&mut prefix).await?;
        assert_eq!(prefix, contents[..prefix.len()]);
        assert_eq!(reader.bytes_read(), prefix.len() as u64);

        reader.finish().await?;
        assert_eq!(reader.bytes_read(), contents.len() as u64);
        reader.finish().await?;
        assert_eq!(reader.bytes_read(), contents.len() as u64);
    }

    assert_eq!(
        hashers.map(|hasher| HashDigest::from(hasher).to_string()),
        [format!("sha256:{}", hex::encode(Sha256::digest(&contents)))]
    );
    Ok(())
}
