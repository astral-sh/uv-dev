use std::io;
#[cfg(windows)]
use std::io::Write;
#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[cfg(windows)]
use crate::Cache;
use crate::{ARCHIVE_VERSION, ArchiveId};

/// A file-based reference to an archive in the cache.
#[derive(Debug)]
struct Link {
    /// The unique ID of the entry in the archive bucket.
    id: ArchiveId,
    /// The version of the archive bucket.
    version: u8,
}

impl Link {
    /// Encode a reference to an archive in the current bucket.
    fn serialize(id: &ArchiveId) -> String {
        format!("archive-v{ARCHIVE_VERSION}/{id}")
    }
}

#[cfg(windows)]
impl Cache {
    /// Create a file containing the archive ID and bucket version.
    #[expect(clippy::unused_self)]
    pub(super) fn create_link(&self, id: &ArchiveId, dst: impl AsRef<Path>) -> io::Result<()> {
        // Serialize the link.
        let contents = Link::serialize(id);

        // First, attempt to create a file at the location, but fail if it already exists.
        match fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dst.as_ref())
        {
            Ok(mut file) => {
                // Write the target path to the file.
                file.write_all(contents.as_bytes())?;
                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                uv_fs::write_atomic_sync(dst, contents.as_bytes())
            }
            Err(err) => Err(err),
        }
    }

    /// Resolve an archive link, returning the fully-resolved path.
    ///
    /// Returns an error if the link target does not exist.
    pub fn resolve_link(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        // Deserialize the link.
        let contents = fs_err::read_to_string(path.as_ref())?;
        let link: Link = contents.parse()?;

        // Ignore stale links.
        if link.version != ARCHIVE_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "The link target does not exist.",
            ));
        }

        // Reconstruct the path.
        let path = self.archive(&link.id);
        path.canonicalize()
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
        let Ok(id) = ArchiveId::from_str(id);

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
