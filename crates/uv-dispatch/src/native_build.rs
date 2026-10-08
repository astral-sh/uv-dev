use std::sync::Arc;

use tokio::sync::{AcquireError, Semaphore};
use tokio::task::JoinHandle;

/// Admit native backend work with the same quota as PEP 517 subprocesses.
pub(super) async fn spawn_native_build<T: Send + 'static>(
    slots: Arc<Semaphore>,
    build: impl FnOnce() -> T + Send + 'static,
) -> Result<JoinHandle<T>, AcquireError> {
    let permit = slots.acquire_owned().await?;
    Ok(tokio::task::spawn_blocking(move || {
        // A blocking build can outlive the caller that awaits its result.
        let _permit = permit;
        build()
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};

    use anyhow::Result;
    use futures::poll;
    use tokio::sync::{Semaphore, oneshot};

    use super::spawn_native_build;

    #[tokio::test]
    async fn cancelled_native_build_retains_its_slot() -> Result<()> {
        for limit in [1, 2] {
            let slots = Arc::new(Semaphore::new(limit));
            let mut workers = Vec::new();
            let mut releases = Vec::new();
            for _ in 0..limit {
                let (started, start) = oneshot::channel();
                let (release, finish) = mpsc::channel();
                let worker = spawn_native_build(slots.clone(), move || {
                    let _ = started.send(());
                    finish.recv()
                })
                .await?;
                start.await?;
                workers.push(worker);
                releases.push(release);
            }

            let next = spawn_native_build(slots.clone(), || ());
            tokio::pin!(next);
            assert!(poll!(next.as_mut()).is_pending());

            // Dropping the wait for one build must not admit another build yet.
            drop(workers.pop());
            assert!(poll!(next.as_mut()).is_pending());
            assert_eq!(slots.available_permits(), 0);

            if let Some(release) = releases.pop() {
                release.send(())?;
            }
            next.await?.await?;

            for release in releases {
                release.send(())?;
            }
            for worker in workers {
                worker.await??;
            }
            assert_eq!(slots.available_permits(), limit);
        }
        Ok(())
    }

    #[tokio::test]
    async fn native_build_waits_for_subprocess_slot() -> Result<()> {
        let slots = Arc::new(Semaphore::new(1));
        let subprocess = slots.acquire().await?;
        let native = spawn_native_build(slots.clone(), || ());
        tokio::pin!(native);
        assert!(poll!(native.as_mut()).is_pending());
        drop(subprocess);
        native.await?.await?;
        assert_eq!(slots.available_permits(), 1);
        Ok(())
    }
}
