use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;

use tokio::sync::Semaphore;

/// Concurrency limit settings.
// TODO(konsti): We should find a pattern that doesn't require having both semaphores and counts.
#[derive(Clone)]
pub struct Concurrency {
    /// The maximum number of concurrent downloads.
    pub downloads: NonZeroUsize,
    /// The maximum number of concurrent builds.
    pub builds: NonZeroUsize,
    /// The maximum number of concurrent installs.
    pub installs: NonZeroUsize,
    /// The maximum number of concurrent cache reads.
    pub cache_reads: NonZeroUsize,
    /// A global semaphore to limit the number of concurrent downloads.
    pub downloads_semaphore: Arc<Semaphore>,
    /// A global semaphore to limit the number of concurrent builds.
    pub builds_semaphore: Arc<Semaphore>,
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
        Self::new(
            Self::DEFAULT_DOWNLOADS,
            Self::threads(),
            Self::threads(),
            Self::DEFAULT_CACHE_READS,
        )
    }
}

impl Concurrency {
    // The default concurrent downloads limit.
    pub const DEFAULT_DOWNLOADS: NonZeroUsize = NonZeroUsize::new(50).expect("nonzero default");

    // The default concurrent cache reads limit.
    pub const DEFAULT_CACHE_READS: NonZeroUsize = NonZeroUsize::new(4).expect("nonzero default");

    /// Create a new [`Concurrency`] with the given limits.
    pub fn new(
        downloads: NonZeroUsize,
        builds: NonZeroUsize,
        installs: NonZeroUsize,
        cache_reads: NonZeroUsize,
    ) -> Self {
        Self {
            downloads,
            builds,
            installs,
            cache_reads,
            downloads_semaphore: Arc::new(Semaphore::new(downloads.get())),
            builds_semaphore: Arc::new(Semaphore::new(builds.get())),
        }
    }

    // The default concurrent builds and install limit.
    pub fn threads() -> NonZeroUsize {
        std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
    }
}
