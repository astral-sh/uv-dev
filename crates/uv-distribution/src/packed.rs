use uv_cache::Cache;
use uv_client::PackedArchiveEntry;
use uv_distribution_types::{BuiltDist, Dist, SourceDist};

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
