//! Shared HTTP header types.

pub use http_content_range::{ContentRange, ContentRangeBytes, ContentRangeUnbound};

pub use crate::httpcache::ETag;

#[cfg(test)]
mod tests {
    use rkyv::util::AlignedVec;

    use crate::OwnedArchive;

    use super::ETag;

    const ETAGS: &[(&[u8], &[u8], bool)] = &[
        (b"\"strong\"", b"\"strong\"", false),
        (b"W/\"weak\"", b"\"weak\"", true),
        (b"w/\"lowercase\"", b"w/\"lowercase\"", false),
        (b"W/W/\"one-prefix\"", b"W/\"one-prefix\"", true),
        (b"", b"", false),
        (b"W/", b"", true),
        (b"unquoted", b"unquoted", false),
        (b"\xff\0", b"\xff\0", false),
        (b"W/\xff\0", b"\xff\0", true),
    ];

    #[test]
    fn etag_preserves_opaque_bytes() {
        for &(input, expected, weak) in ETAGS {
            let etag = ETag::parse(input);
            assert_eq!(etag.as_bytes(), expected);
            assert_eq!(etag.is_weak(), weak);
        }
    }

    #[derive(Debug, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
    #[rkyv(derive(Debug))]
    struct LegacyETag {
        value: Vec<u8>,
        weak: bool,
    }

    #[test]
    fn etag_preserves_legacy_archive_layout() -> Result<(), crate::Error> {
        for &(input, expected, weak) in ETAGS {
            let legacy = OwnedArchive::from_unarchived(&LegacyETag {
                value: expected.to_vec(),
                weak,
            })?;
            let current = OwnedArchive::from_unarchived(&ETag::parse(input))?;
            assert_eq!(
                OwnedArchive::as_bytes(&current),
                OwnedArchive::as_bytes(&legacy)
            );

            let mut bytes = AlignedVec::new();
            bytes.extend_from_slice(OwnedArchive::as_bytes(&legacy));
            let archived = OwnedArchive::<ETag>::new(bytes)?;
            let etag = OwnedArchive::deserialize(&archived);
            assert_eq!(etag.as_bytes(), expected);
            assert_eq!(etag.is_weak(), weak);
        }
        Ok(())
    }
}
