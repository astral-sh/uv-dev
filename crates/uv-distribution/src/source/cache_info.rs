use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;
use tracing::Span;
use uv_cache_info::CacheInfo;

use crate::Error;

/// Scan all source cache keys while retaining I/O admission in the blocking worker.
pub(super) async fn read_source_cache_info(
    path: PathBuf,
    slots: Arc<Semaphore>,
) -> Result<CacheInfo, Error> {
    let permit = slots
        .acquire_owned()
        .await
        .map_err(|err| Error::CacheRead(io::Error::other(err)))?;
    let span = Span::current();
    Ok(tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _entered = span.enter();
        CacheInfo::from_directory(&path)
    })
    .await??)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use futures::FutureExt;
    use tokio::runtime::Builder;
    use tokio::sync::{Semaphore, oneshot};
    use uv_cache_info::CacheInfo;

    use super::read_source_cache_info;

    type Error = Box<dyn std::error::Error>;

    #[test]
    fn cancelled_source_scan_retains_admission() -> Result<(), Error> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()?;
        let directory = tempfile::tempdir()?;
        let slots = Arc::new(Semaphore::new(1));
        let reserved = slots.clone().try_acquire_owned()?;
        assert!(
            read_source_cache_info(directory.path().to_owned(), slots.clone())
                .now_or_never()
                .is_none()
        );
        drop(reserved);
        runtime.block_on(async {
            let (started, start) = oneshot::channel();
            let (release, finish) = mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                finish.recv()
            });
            start.await?;
            assert!(
                read_source_cache_info(directory.path().to_owned(), slots.clone())
                    .now_or_never()
                    .is_none()
            );
            assert_eq!(slots.available_permits(), 0);
            tokio::task::yield_now().await;
            assert_eq!(slots.available_permits(), 0);
            release.send(())?;
            blocker.await??;
            let permit = tokio::time::timeout(Duration::from_secs(10), slots.acquire()).await??;
            drop(permit);
            Ok(())
        })
    }

    #[tokio::test]
    async fn source_scan_observes_current_custom_keys() -> Result<(), Error> {
        let directory = tempfile::tempdir()?;
        fs_err::write(
            directory.path().join("pyproject.toml"),
            "[tool.uv]\ncache-keys = [{file = '**/*.txt'}, {dir = 'generated'}]\n",
        )?;
        fs_err::create_dir(directory.path().join("nested"))?;
        for index in 0..64 {
            fs_err::write(directory.path().join(format!("nested/{index}.txt")), "data")?;
        }
        let slots = Arc::new(Semaphore::new(1));
        let before = read_source_cache_info(directory.path().to_owned(), slots.clone()).await?;
        assert_eq!(before, CacheInfo::from_directory(directory.path())?);
        fs_err::create_dir(directory.path().join("generated"))?;
        let after = read_source_cache_info(directory.path().to_owned(), slots).await?;
        assert_eq!(after, CacheInfo::from_directory(directory.path())?);
        assert_ne!(before, after);
        Ok(())
    }
}
