use std::error::Error;
use std::path::Path;
use std::str::FromStr;

use uv_cache::{ARCHIVE_VERSION, ArchiveId, Cache};

fn assert_string_encoding(id: &ArchiveId, expected: &str) -> Result<(), Box<dyn Error>> {
    // Encode the original string independently of ArchiveId's serializer.
    let bytes = rmp_serde::to_vec(&expected)?;
    let decoded: ArchiveId = rmp_serde::from_slice(&bytes)?;
    assert_eq!(&decoded, id);
    assert_eq!(rmp_serde::to_vec(id)?, bytes);
    assert_eq!(id.to_string(), expected);
    assert_eq!(id.as_ref(), Path::new(expected));

    let cache = Cache::from_path("archive-id-cache");
    assert_eq!(
        cache.archive(id),
        Path::new("archive-id-cache")
            .join(format!("archive-v{ARCHIVE_VERSION}"))
            .join(expected)
    );
    Ok(())
}

#[test]
fn stored_archive_id_formats_remain_compatible() -> Result<(), Box<dyn Error>> {
    for stored in [
        // The 21-character nanoid format used before uv 0.11.9.
        "_Legacy-0123456789AbC",
        // A current 16-character uv-fastid value.
        "Ab_0123456789-xy",
        // A content-addressed, lowercase base-36 directory digest.
        "0123456789abcdefghijklmn",
    ] {
        assert_string_encoding(&ArchiveId::from_str(stored)?, stored)?;
    }
    Ok(())
}

#[test]
fn constructed_archive_ids_round_trip() -> Result<(), Box<dyn Error>> {
    let digest = "0123456789abcdefghijklmn";
    assert_string_encoding(&ArchiveId::from_digest(digest.to_owned()), digest)?;

    let generated = ArchiveId::default();
    let value = generated.to_string();
    assert!(!value.is_empty());
    assert_string_encoding(&generated, &value)?;
    Ok(())
}
