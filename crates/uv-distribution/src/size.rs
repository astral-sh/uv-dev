use std::fmt::Display;

use uv_distribution_types::{BuildableSource, BuiltDist, RemoteSource, SourceDist};
use uv_warnings::warn_user_once;

use crate::Error;

/// How an archive's advertised size is checked against its measured size.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ArchiveSizePolicy {
    None,
    Advisory(u64),
    Required(u64),
}

impl ArchiveSizePolicy {
    pub(crate) fn registry(size: Option<u64>, authoritative: bool) -> Self {
        match (size, authoritative) {
            (None, _) => Self::None,
            (Some(size), false) => Self::Advisory(size),
            (Some(size), true) => Self::Required(size),
        }
    }

    pub(crate) fn for_built(dist: &BuiltDist) -> Self {
        match dist {
            BuiltDist::Registry(dist) => {
                let wheel = dist.best_wheel();
                Self::registry(wheel.file.size, wheel.size_is_authoritative)
            }
            BuiltDist::DirectUrl(dist) => dist.size.map_or(Self::None, Self::Required),
            BuiltDist::Path(_) | BuiltDist::GitPath(_) => Self::None,
        }
    }

    pub(crate) fn for_source(source: &BuildableSource<'_>) -> Self {
        match source {
            BuildableSource::Dist(SourceDist::Registry(dist)) => {
                Self::registry(dist.size(), dist.size_is_authoritative)
            }
            BuildableSource::Dist(SourceDist::DirectUrl(dist)) => {
                dist.size().map_or(Self::None, Self::Required)
            }
            BuildableSource::Dist(
                SourceDist::Path(_)
                | SourceDist::Directory(_)
                | SourceDist::GitPath(_)
                | SourceDist::GitDirectory(_),
            )
            | BuildableSource::Url(_) => Self::None,
        }
    }

    /// A required size must be known before a cached archive can be reused.
    pub(crate) fn required(self) -> Option<u64> {
        match self {
            Self::Required(size) => Some(size),
            Self::None | Self::Advisory(_) => None,
        }
    }

    pub(crate) fn is_some(self) -> bool {
        match self {
            Self::None => false,
            Self::Advisory(_) | Self::Required(_) => true,
        }
    }

    /// Check a measured size before publishing or reusing an archive.
    pub(crate) fn check(self, distribution: impl Display, actual: u64) -> Result<(), Error> {
        match self {
            Self::Required(expected) if expected != actual => Err(Error::MismatchedSize {
                distribution: distribution.to_string(),
                expected,
                actual,
            }),
            Self::Advisory(_) => {
                self.warn(distribution, actual);
                Ok(())
            }
            Self::None | Self::Required(_) => Ok(()),
        }
    }

    /// Report an advisory mismatch without preventing cache reuse.
    pub(crate) fn warn(self, distribution: impl Display, actual: u64) {
        if let Self::Advisory(expected) = self
            && expected != actual
        {
            warn_user_once!(
                "Size mismatch for `{distribution}`: expected {expected} bytes, but downloaded {actual} bytes. This will become an error in a future release."
            );
        }
    }
}
