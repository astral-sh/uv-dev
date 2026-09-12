use std::fmt;
use std::num::NonZeroUsize;
use std::sync::{Arc, Weak};

use rustc_hash::FxHashMap;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

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
    /// Shared admission for source preparation at each build-dependency depth.
    pub source_preparation: Arc<SourcePreparationConcurrency>,
}

/// Bound the width of source preparation without blocking recursive build dependencies.
///
/// A source can retain its cache lock while its isolated build environment prepares another
/// source. Those descendants need their own admission level; sharing the parent's semaphore can
/// deadlock when all permits are held by ancestors. The limit applies to each depth, not to the
/// total number of file descriptors retained by a recursive build graph.
#[derive(Debug)]
pub struct SourcePreparationConcurrency {
    limit: usize,
    semaphores: Mutex<FxHashMap<usize, Weak<Semaphore>>>,
}

/// Admission to one source-preparation level.
#[derive(Debug)]
pub struct SourcePreparationPermit {
    _permit: OwnedSemaphorePermit,
}

impl SourcePreparationConcurrency {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            semaphores: Mutex::new(FxHashMap::default()),
        }
    }

    /// Acquire admission at the given build-dependency depth.
    pub async fn acquire(&self, depth: usize) -> SourcePreparationPermit {
        let semaphore = {
            let mut semaphores = self.semaphores.lock().await;
            // Queued acquisitions and active permits keep their semaphore alive. Remove levels
            // that have finished so repeated or cancelled build graphs do not retain every depth.
            semaphores.retain(|_, semaphore| semaphore.strong_count() > 0);
            if let Some(semaphore) = semaphores.get(&depth).and_then(Weak::upgrade) {
                semaphore
            } else {
                let semaphore = Arc::new(Semaphore::new(self.limit));
                semaphores.insert(depth, Arc::downgrade(&semaphore));
                semaphore
            }
        };
        SourcePreparationPermit {
            _permit: semaphore
                .acquire_owned()
                .await
                .expect("source preparation semaphores are never closed"),
        }
    }
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
            downloads_semaphore: Arc::new(Semaphore::new(downloads)),
            builds_semaphore: Arc::new(Semaphore::new(builds)),
            source_preparation: Arc::new(SourcePreparationConcurrency::new(builds)),
        }
    }

    // The default concurrent builds and install limit.
    pub fn threads() -> usize {
        std::thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::time::timeout;

    use super::{Concurrency, SourcePreparationConcurrency};

    #[tokio::test]
    async fn source_preparation_shares_width_per_depth() -> anyhow::Result<()> {
        let concurrency = Concurrency::new(50, 2, 2, 4);
        let sibling = concurrency.clone();
        let first = concurrency.source_preparation.acquire(0).await;
        let second = sibling.source_preparation.acquire(0).await;

        assert!(
            timeout(
                Duration::from_millis(25),
                sibling.source_preparation.acquire(0),
            )
            .await
            .is_err()
        );
        let nested = timeout(
            Duration::from_secs(5),
            sibling.source_preparation.acquire(1),
        )
        .await?;
        assert_eq!(concurrency.downloads_semaphore.available_permits(), 50);
        assert_eq!(concurrency.builds_semaphore.available_permits(), 2);

        drop(first);
        let replacement = timeout(
            Duration::from_secs(5),
            sibling.source_preparation.acquire(0),
        )
        .await?;
        drop((second, nested, replacement));
        Ok(())
    }

    #[tokio::test]
    async fn source_preparation_reclaims_cancelled_levels() -> anyhow::Result<()> {
        let concurrency = Arc::new(SourcePreparationConcurrency::new(1));
        let held = concurrency.acquire(0).await;
        let waiter = tokio::spawn({
            let concurrency = concurrency.clone();
            async move { concurrency.acquire(0).await }
        });
        tokio::task::yield_now().await;
        waiter.abort();
        assert!(
            waiter
                .await
                .expect_err("the queued acquisition was aborted")
                .is_cancelled()
        );
        drop(held);

        let replacement = timeout(Duration::from_secs(5), concurrency.acquire(0)).await?;
        drop(replacement);
        for depth in 1..256 {
            drop(concurrency.acquire(depth).await);
        }
        assert!(concurrency.semaphores.lock().await.len() <= 1);
        Ok(())
    }
}
