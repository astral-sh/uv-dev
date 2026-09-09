use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;

use tokio::sync::Semaphore;

/// Concurrency limit settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub const DEFAULT_DOWNLOADS: usize = 50;

    // The default concurrent cache reads limit.
    pub const DEFAULT_CACHE_READS: usize = 4;

    /// Create a new [`Concurrency`] with the given limits.
    pub fn new(downloads: usize, builds: usize, installs: usize, cache_reads: usize) -> Self {
        Self {
            downloads,
            builds,
            installs,
            cache_reads,
        }
    }

    // The default concurrent builds and install limit.
    pub fn threads() -> usize {
        std::thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(1)
    }
}

/// Shared runtime state for the configured [`Concurrency`] limits.
#[derive(Clone)]
pub struct ConcurrencyState {
    limits: Concurrency,
    downloads_semaphore: Arc<Semaphore>,
    builds_semaphore: Arc<Semaphore>,
}

impl fmt::Debug for ConcurrencyState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.limits.fmt(f)
    }
}

impl Default for ConcurrencyState {
    fn default() -> Self {
        Self::new(Concurrency::default())
    }
}

impl ConcurrencyState {
    /// Create shared runtime state for the given limits.
    pub fn new(limits: Concurrency) -> Self {
        Self {
            downloads_semaphore: Arc::new(Semaphore::new(limits.downloads)),
            builds_semaphore: Arc::new(Semaphore::new(limits.builds)),
            limits,
        }
    }

    /// Return the configured limits.
    pub fn limits(&self) -> &Concurrency {
        &self.limits
    }

    /// Return the shared semaphore for concurrent downloads.
    pub fn downloads_semaphore(&self) -> Arc<Semaphore> {
        self.downloads_semaphore.clone()
    }

    /// Return the shared semaphore for concurrent builds.
    pub fn builds_semaphore(&self) -> Arc<Semaphore> {
        self.builds_semaphore.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Concurrency, ConcurrencyState};

    #[test]
    fn state_preserves_limits() -> Result<(), tokio::sync::TryAcquireError> {
        let limits = Concurrency::new(2, 3, 4, 5);
        let state = ConcurrencyState::new(limits);
        let cloned = state.clone();

        assert_eq!(state.limits(), &limits);
        assert_eq!(cloned.limits(), &limits);
        assert!(Arc::ptr_eq(
            &state.downloads_semaphore(),
            &cloned.downloads_semaphore()
        ));
        assert!(Arc::ptr_eq(
            &state.builds_semaphore(),
            &cloned.builds_semaphore()
        ));

        let download = state.downloads_semaphore().try_acquire_owned()?;
        let build = state.builds_semaphore().try_acquire_owned()?;
        assert_eq!(cloned.downloads_semaphore().available_permits(), 1);
        assert_eq!(cloned.builds_semaphore().available_permits(), 2);
        assert_eq!(cloned.limits().downloads, 2);
        assert_eq!(cloned.limits().builds, 3);
        drop((download, build));
        assert_eq!(cloned.downloads_semaphore().available_permits(), 2);
        assert_eq!(cloned.builds_semaphore().available_permits(), 3);
        Ok(())
    }

    #[test]
    fn settings_debug_format_is_unchanged() {
        let limits = Concurrency::new(2, 3, 4, 5);
        insta::assert_debug_snapshot!(limits, @r"
        Concurrency {
            downloads: 2,
            builds: 3,
            installs: 4,
            cache_reads: 5,
        }
        ");
        assert_eq!(
            format!("{limits:?}"),
            format!("{:?}", ConcurrencyState::new(limits))
        );
    }
}
