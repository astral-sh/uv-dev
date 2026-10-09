use uv_cache::Cache;
use uv_cache_info::{CacheInfo, CacheInfoError};
use uv_client::PackedArchiveEntry;
use uv_distribution_filename::{DistExtension, WheelFilename};
use uv_distribution_types::{BuiltDist, Dist, RemoteSource, RequirementSource, SourceDist};
use uv_normalize::PackageName;

/// Return whether a direct local archive has a packed pointer for deferred preparation.
///
/// This only permits metadata and installation planning to reach the archive reader, which
/// validates the pointer and retained bytes before using them.
pub fn has_cached_local_archive(cache: &Cache, dist: &Dist) -> bool {
    match dist {
        Dist::Built(BuiltDist::Path(wheel)) => {
            PackedArchiveEntry::wheel(cache, None, &wheel.url, &wheel.filename).has_local_pointer()
        }
        Dist::Source(SourceDist::Path(sdist)) => {
            PackedArchiveEntry::source(cache, None, &sdist.name, &sdist.url, sdist.ext)
                .has_local_pointer()
        }
        _ => false,
    }
}

/// A removed local archive retains its original revision in the packed cache.
pub fn local_archive_cache_info(
    cache: &Cache,
    name: &PackageName,
    source: &RequirementSource,
    path: &std::path::Path,
) -> Result<CacheInfo, CacheInfoError> {
    let err = match CacheInfo::from_path(path) {
        Ok(info) => return Ok(info),
        Err(err) if matches!(&err, CacheInfoError::Io(io) if io.kind() == std::io::ErrorKind::NotFound) => {
            err
        }
        Err(err) => return Err(err),
    };
    let (RequirementSource::Path { url, ext, .. } | RequirementSource::Url { url, ext, .. }) =
        source
    else {
        return Err(err);
    };
    let entry = match ext {
        DistExtension::Wheel => {
            let Some(filename) = url
                .filename()
                .ok()
                .and_then(|filename| filename.parse::<WheelFilename>().ok())
            else {
                return Err(err);
            };
            PackedArchiveEntry::wheel(cache, None, url, &filename)
        }
        DistExtension::Source(ext) => PackedArchiveEntry::source(cache, None, name, url, *ext),
    };
    match entry.local_timestamp() {
        Ok(Some(timestamp)) => Ok(CacheInfo::from_timestamp(timestamp)),
        Ok(None) => Err(err),
        Err(err) => Err(std::io::Error::other(err).into()),
    }
}
