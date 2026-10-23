use std::io;
use std::path::Path;
use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::Semaphore;
use tracing::Span;
use uv_cache::Cache;
use uv_python_interpreter::PythonEnvironment;

use crate::Error;

/// Create an isolated environment while owning its temporary directory and worker admission.
///
/// Admission ends before the environment is returned for recursive build-dependency installation.
pub(super) async fn create_isolated_environment(
    temp_dir: TempDir,
    cache: Cache,
    slots: Arc<Semaphore>,
    create: impl FnOnce(&Path) -> Result<PythonEnvironment, uv_virtualenv::Error> + Send + 'static,
) -> Result<(TempDir, PythonEnvironment), Error> {
    let permit = slots.acquire_owned().await.map_err(io::Error::other)?;
    let span = Span::current();
    Ok(tokio::task::spawn_blocking(move || {
        let _cache = cache;
        let _permit = permit;
        let _entered = span.enter();
        let environment = create(temp_dir.path())?;
        Ok::<_, uv_virtualenv::Error>((temp_dir, environment))
    })
    .await
    .map_err(io::Error::other)??)
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::io;
    use std::pin::pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;

    use tokio::runtime::Builder;
    use tokio::sync::{Semaphore, oneshot};
    use uv_cache::Cache;
    use uv_fs::{LockedFile, LockedFileMode};

    use crate::Error as BuildError;

    use super::create_isolated_environment;

    type Error = Box<dyn std::error::Error>;

    fn poll_once<F: Future>(future: F) -> Poll<F::Output> {
        pin!(future).poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn isolated_creation_waits_for_admission() -> Result<(), Error> {
        let cache = Cache::temp()?;
        let directory = cache.venv_dir()?;
        let path = directory.path().to_owned();
        let started = Arc::new(AtomicBool::new(false));
        let worker_started = started.clone();
        let future = create_isolated_environment(
            directory,
            cache.clone(),
            Arc::new(Semaphore::new(0)),
            move |_| {
                worker_started.store(true, Ordering::SeqCst);
                Err(io::Error::other("unexpected creation").into())
            },
        );
        assert!(poll_once(future).is_pending());
        assert!(!started.load(Ordering::SeqCst));
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn cancelled_creation_keeps_parent_cache_directory_and_slot() -> Result<(), Error> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()?;
        let cache = runtime.block_on(Cache::temp()?.init())?;
        let cache_root = cache.root().to_owned();
        let directory = cache.venv_dir()?;
        let path = directory.path().to_owned();
        let slots = Arc::new(Semaphore::new(1));
        runtime.block_on(async {
            let (started, start) = oneshot::channel();
            let (release, finish) = mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                finish.recv()
            });
            start.await?;
            let (created, completed) = oneshot::channel();
            let future =
                create_isolated_environment(directory, cache.clone(), slots.clone(), move |path| {
                    fs_err::write(path.join("created"), b"created by worker")?;
                    let _ = created.send(());
                    Err(io::Error::other("fixture creation failure").into())
                });
            assert!(poll_once(future).is_pending());
            drop(cache);
            assert!(cache_root.is_dir());
            assert!(path.is_dir());
            assert!(
                LockedFile::acquire_no_wait(
                    cache_root.join(".lock"),
                    LockedFileMode::Exclusive,
                    "test cache"
                )
                .is_none()
            );
            assert_eq!(slots.available_permits(), 0);
            tokio::task::yield_now().await;
            release.send(())?;
            blocker.await??;
            tokio::time::timeout(Duration::from_secs(10), completed).await??;
            // This cannot run until the sole worker finishes creation and drops its directory.
            tokio::task::spawn_blocking(|| {}).await?;
            assert!(!path.exists());
            assert!(!cache_root.exists());
            assert_eq!(slots.available_permits(), 1);
            Ok(())
        })
    }

    #[tokio::test]
    async fn failed_creation_keeps_error_context_and_releases_resources() -> Result<(), Error> {
        let cache = Cache::temp()?;
        let directory = cache.venv_dir()?;
        let path = directory.path().to_owned();
        let slots = Arc::new(Semaphore::new(1));
        let error = create_isolated_environment(directory, cache.clone(), slots.clone(), |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fixture permission failure",
            )
            .into())
        })
        .await
        .expect_err("creation should fail");
        assert_eq!(error.to_string(), "Failed to create temporary virtualenv");
        let BuildError::Virtualenv(uv_virtualenv::Error::Io(error)) = error else {
            return Err("expected the virtualenv error cause".into());
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(slots.available_permits(), 1);
        assert!(!path.exists());
        Ok(())
    }
}
