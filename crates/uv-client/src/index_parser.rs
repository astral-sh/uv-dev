use std::sync::Arc;
use std::thread;

use tokio::sync::Semaphore;
use tracing::Span;

use crate::Error;

/// Shared worker and input-byte budgets for parsing index responses.
#[derive(Debug, Clone)]
pub(crate) struct IndexParser {
    concurrency: Arc<Semaphore>,
    memory: Arc<Semaphore>,
}

impl Default for IndexParser {
    fn default() -> Self {
        Self {
            concurrency: Arc::new(Semaphore::new(
                thread::available_parallelism().map_or(1, |parallelism| parallelism.get().min(4)),
            )),
            memory: Arc::new(Semaphore::new(8 * 1024 * 1024)),
        }
    }
}

impl IndexParser {
    /// Offload large index parsing so sibling HTTP futures can make progress.
    ///
    /// `body_size` counts decoded input bytes, excluding allocations produced by parsing. Small
    /// bodies or bodies that cannot fit the shared worker and byte budgets are parsed inline
    /// without waiting. Offloaded work retains both permits until its result is collected or
    /// dropped, even if the caller is cancelled.
    pub(crate) async fn parse<T: Send + 'static>(
        &self,
        body_size: usize,
        parse: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        // Small responses are cheaper to parse inline than to dispatch to another thread.
        if body_size < 512 * 1024 {
            return parse();
        }
        // Oversized permit requests are invalid on 32-bit platforms.
        if body_size > Semaphore::MAX_PERMITS {
            return parse();
        }
        let Ok(body_size) = u32::try_from(body_size) else {
            return parse();
        };
        // Fall back to inline parsing instead of retaining completed response bodies in a queue.
        let Ok(permit) = self.concurrency.clone().try_acquire_owned() else {
            return parse();
        };
        let Ok(memory) = self.memory.clone().try_acquire_many_owned(body_size) else {
            drop(permit);
            return parse();
        };
        let span = Span::current();
        let (result, _permits) =
            tokio::task::spawn_blocking(move || (span.in_scope(parse), (permit, memory)))
                .await
                .expect("The task executor is broken, did some other task panic?");
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use futures::FutureExt;
    use tokio::runtime::Builder;
    use tokio::sync::{Semaphore, oneshot};

    use super::IndexParser;

    type Error = Box<dyn std::error::Error>;

    #[test]
    fn small_and_over_budget_bodies_parse_inline() -> Result<(), Error> {
        let parser = IndexParser {
            concurrency: Arc::new(Semaphore::new(1)),
            memory: Arc::new(Semaphore::new(1024 * 1024)),
        };
        let caller = thread::current().id();
        for size in [0, 512 * 1024 - 1, 1024 * 1024 + 1, usize::MAX] {
            let thread = parser
                .parse(size, || Ok(thread::current().id()))
                .now_or_never()
                .ok_or("expected inline parsing")??;
            assert_eq!(thread, caller);
        }

        let worker = parser.concurrency.clone().try_acquire_owned()?;
        let thread = parser
            .clone()
            .parse(512 * 1024, || Ok(thread::current().id()))
            .now_or_never()
            .ok_or("expected inline fallback when workers are busy")??;
        assert_eq!(thread, caller);
        drop(worker);

        let memory = parser.memory.clone().try_acquire_many_owned(1024 * 1024)?;
        let thread = parser
            .clone()
            .parse(512 * 1024, || Ok(thread::current().id()))
            .now_or_never()
            .ok_or("expected inline fallback when the byte budget is full")??;
        assert_eq!(thread, caller);
        assert_eq!(parser.concurrency.available_permits(), 1);
        drop(memory);
        Ok(())
    }

    #[test]
    fn parser_budgets_survive_cancellation_and_result_collection() -> Result<(), Error> {
        for cancel in [false, true] {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .max_blocking_threads(1)
                .build()?;
            let parser = IndexParser {
                concurrency: Arc::new(Semaphore::new(1)),
                memory: Arc::new(Semaphore::new(512 * 1024)),
            };
            runtime.block_on(async {
                let (started, start) = oneshot::channel();
                let (release, finish) = mpsc::channel();
                let blocker = tokio::task::spawn_blocking(move || {
                    let _ = started.send(());
                    finish.recv()
                });
                start.await?;

                let (parsed, completed) = oneshot::channel();
                let mut parsing = Box::pin(parser.parse(512 * 1024, move || {
                    let _ = parsed.send(());
                    Ok(thread::current().id())
                }));
                assert!(parsing.as_mut().now_or_never().is_none());
                let pending = if cancel {
                    drop(parsing);
                    None
                } else {
                    Some(parsing)
                };
                assert_eq!(parser.concurrency.available_permits(), 0);
                assert_eq!(parser.memory.available_permits(), 0);
                tokio::task::yield_now().await;
                release.send(())?;
                blocker.await??;
                tokio::time::timeout(Duration::from_secs(10), completed).await??;
                // With one blocking thread, this runs only after the parse job has returned.
                tokio::task::spawn_blocking(|| {}).await?;

                if let Some(pending) = pending {
                    assert_eq!(parser.concurrency.available_permits(), 0);
                    assert_eq!(parser.memory.available_permits(), 0);
                    assert_ne!(pending.await?, thread::current().id());
                }
                assert_eq!(parser.concurrency.available_permits(), 1);
                assert_eq!(parser.memory.available_permits(), 512 * 1024);
                Ok::<_, Error>(())
            })?;
        }
        Ok(())
    }
}
