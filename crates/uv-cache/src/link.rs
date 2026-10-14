use std::io;
use std::str::FromStr;

use crate::{ARCHIVE_VERSION, ArchiveId};

/// A file-based reference to an archive in the cache.
#[derive(Debug)]
pub(super) struct Link {
    /// The unique ID of the entry in the archive bucket.
    pub(super) id: ArchiveId,
    /// The version of the archive bucket.
    pub(super) version: u8,
}

impl Link {
    /// Encode a reference to an archive in the current bucket.
    pub(super) fn serialize(id: &ArchiveId) -> String {
        format!("archive-v{ARCHIVE_VERSION}/{id}")
    }
}

impl FromStr for Link {
    type Err = io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, '/');
        let version = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing version"))?;
        let id = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing ID"))?;

        // Parse the archive version from `archive-v{version}/{id}`.
        let version = version
            .strip_prefix("archive-v")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing version prefix"))?;
        let version = u8::from_str(version).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse version: {err}"),
            )
        })?;

        // Parse the ID from `archive-v{version}/{id}`.
        let id = ArchiveId::from_str(id).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse ID: {err}"),
            )
        })?;

        Ok(Self { id, version })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::Link;
    use crate::{ARCHIVE_VERSION, ArchiveId};

    #[test]
    fn round_trip() {
        let id = ArchiveId::new();
        let serialized = Link::serialize(&id);
        let parsed = Link::from_str(&serialized).unwrap();
        assert_eq!(id, parsed.id);
        assert_eq!(ARCHIVE_VERSION, parsed.version);
    }

    #[test]
    fn deserialize() {
        assert!(Link::from_str("archive-v0/foo").is_ok());
        assert!(Link::from_str("archive/foo").is_err());
        assert!(Link::from_str("v1/foo").is_err());
        assert!(Link::from_str("archive-v0/").is_err());
    }
}
