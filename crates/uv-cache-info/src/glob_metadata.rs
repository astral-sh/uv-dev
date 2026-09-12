use std::io;
use std::path::PathBuf;

use globwalk::GlobWalker;
use tracing::warn;
use walkdir::DirEntry;

use crate::Timestamp;

/// A collector for the leaf metadata of one clustered cache-key glob.
///
/// Implementations must return results in the same order as the walker. Metadata collection may
/// run concurrently, but diagnostics are emitted only when the caller consumes each result.
#[doc(hidden)]
pub trait GlobMetadataCollector {
    fn collect(
        &mut self,
        walker: GlobWalker,
    ) -> io::Result<impl Iterator<Item = GlobEntryMetadata>>;
}

pub(crate) struct OrdinaryGlobMetadata;

impl GlobMetadataCollector for OrdinaryGlobMetadata {
    fn collect(
        &mut self,
        walker: GlobWalker,
    ) -> io::Result<impl Iterator<Item = GlobEntryMetadata>> {
        Ok(walker.map(GlobEntryMetadata::read))
    }
}

/// The result of reading one glob entry, with diagnostics deferred until ordered consumption.
#[doc(hidden)]
#[derive(Debug)]
pub struct GlobEntryMetadata(EntryResult);

#[derive(Debug)]
enum EntryResult {
    WalkError(walkdir::Error),
    Metadata {
        entry: DirEntry,
        timestamp: Result<Option<Timestamp>, MetadataError>,
    },
}

#[derive(Debug)]
enum MetadataError {
    Symlink(io::Error),
    Entry(walkdir::Error),
}

impl GlobEntryMetadata {
    /// Read metadata using the ordinary cache-key behavior without emitting diagnostics.
    pub fn read(entry: Result<DirEntry, walkdir::Error>) -> Self {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Self(EntryResult::WalkError(error)),
        };
        let metadata = if entry.path_is_symlink() {
            // Resolve leaf symlinks without following symlink directories while globbing.
            fs_err::metadata(entry.path()).map_err(MetadataError::Symlink)
        } else {
            entry.metadata().map_err(MetadataError::Entry)
        };
        match metadata {
            Ok(metadata) => Self::from_timestamp(
                entry,
                metadata
                    .is_file()
                    .then(|| Timestamp::from_metadata(&metadata)),
            ),
            Err(error) => Self(EntryResult::Metadata {
                entry,
                timestamp: Err(error),
            }),
        }
    }

    /// Construct a result from a regular file's timestamp, or `None` for another file type.
    pub fn from_timestamp(entry: DirEntry, timestamp: Option<Timestamp>) -> Self {
        Self(EntryResult::Metadata {
            entry,
            timestamp: Ok(timestamp),
        })
    }

    /// Emit any diagnostic and return the timestamp of a regular file.
    pub fn into_timestamp(self) -> Option<(PathBuf, Timestamp)> {
        let (entry, timestamp) = match self.0 {
            EntryResult::WalkError(error) => {
                warn!("Failed to read glob entry: {error}");
                return None;
            }
            EntryResult::Metadata { entry, timestamp } => (entry, timestamp),
        };
        match timestamp {
            Ok(Some(timestamp)) => Some((entry.into_path(), timestamp)),
            Ok(None) => {
                // A leaf symlink can legitimately resolve to a directory.
                if !entry.path_is_symlink() {
                    warn!(
                        "Expected file for cache key, but found directory: `{}`",
                        entry.path().display()
                    );
                }
                None
            }
            Err(MetadataError::Symlink(error)) => {
                warn!("Failed to resolve symlink for glob entry: {error}");
                None
            }
            Err(MetadataError::Entry(error)) => {
                warn!("Failed to read metadata for glob entry: {error}");
                None
            }
        }
    }
}
