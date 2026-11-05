use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;

use tokio::sync::Semaphore;

/// Concurrency limit settings.
// TODO(konsti): We should find a pattern that doesn't require having both semaphores and counts.
#[derive(Clone)]
pub struct Concurrency {
    /// The maximum number of concurrent downloads.
    ///
    /// Note this value must be non-zero.
    pub downloads: usize,
    /// The maximum number of concurrent builds.
    ///
    /// Note this value must be non-zero.
    pub builds: usize,
    /// The maximum number of concurrent installs.
    ///
    /// Note this value must be non-zero.
    pub installs: usize,
    /// The maximum number of concurrent cache reads.
    ///
    /// Note this value must be non-zero.
    pub cache_reads: usize,
    /// A global semaphore to limit the number of concurrent downloads.
    pub downloads_semaphore: Arc<Semaphore>,
    /// A global semaphore to limit the number of concurrent builds.
    pub builds_semaphore: Arc<Semaphore>,
}

#[derive(Debug, thiserror::Error)]
#[error("The `{setting}` setting must be at most {}", Semaphore::MAX_PERMITS)]
pub struct ConcurrencyLimitError {
    setting: &'static str,
}

/// Custom `Debug` to hide semaphore fields from `--show-settings` output.
#[expect(clippy::missing_fields_in_debug)]
impl fmt::Debug for Concurrency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Concurrency")
            .field("downloads", &self.downloads)
            .field("builds", &self.builds)
            .field("installs", &self.installs)
            .field("cache_reads", &self.cache_reads)
            .finish()
    }
}

impl Default for Concurrency {
    fn default() -> Self {
        Self::new_unchecked(
            Self::DEFAULT_DOWNLOADS,
            Self::threads(),
            Self::threads(),
            Self::DEFAULT_CACHE_READS,
        )
    }
}

impl Concurrency {
    // The default concurrent downloads limit.
    pub const DEFAULT_DOWNLOADS: usize = 50;

    // The default concurrent cache reads limit.
    pub const DEFAULT_CACHE_READS: usize = 4;

    /// Create a new [`Concurrency`] with the given limits.
    pub fn new(
        downloads: usize,
        builds: usize,
        installs: usize,
        cache_reads: usize,
    ) -> Result<Self, ConcurrencyLimitError> {
        for (setting, value) in [
            ("concurrent-downloads", downloads),
            ("concurrent-builds", builds),
        ] {
            if value > Semaphore::MAX_PERMITS {
                return Err(ConcurrencyLimitError { setting });
            }
        }
        Ok(Self::new_unchecked(
            downloads,
            builds,
            installs,
            cache_reads,
        ))
    }

    fn new_unchecked(downloads: usize, builds: usize, installs: usize, cache_reads: usize) -> Self {
        Self {
            downloads,
            builds,
            installs,
            cache_reads,
            downloads_semaphore: Arc::new(Semaphore::new(downloads)),
            builds_semaphore: Arc::new(Semaphore::new(builds)),
        }
    }

    // The default concurrent builds and install limit.
    pub fn threads() -> usize {
        std::thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(1)
    }
}
