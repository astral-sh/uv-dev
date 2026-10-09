use std::future::Future;
use std::sync::Arc;

use crate::error::Result;

tokio::task_local! {
    static OPERATION_GUARD: Arc<dyn Send + Sync>;
}

/// Retain a lock while a keyring operation and any native blocking work it starts are running.
///
/// Blocking work keeps its own guard reference, so cancelling the calling future cannot release
/// the lock while the platform is still mutating credentials.
pub async fn with_operation_guard<G, T>(
    guard: Arc<G>,
    operation: impl Future<Output = Result<T>>,
) -> Result<T>
where
    G: Send + Sync + 'static,
{
    OPERATION_GUARD.scope(guard, operation).await
}

#[cfg(any(
    test,
    all(target_os = "macos", feature = "apple-native"),
    all(target_os = "windows", feature = "windows-native"),
))]
pub(crate) async fn spawn_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let guard = OPERATION_GUARD.try_with(Arc::clone).ok();
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        f()
    })
    .await
    .map_err(|e| crate::error::Error::PlatformFailure(Box::new(e)))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    struct DropNotification(Option<oneshot::Sender<()>>);

    impl Drop for DropNotification {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }

    #[tokio::test]
    async fn cancelled_operation_retains_guard_until_blocking_work_finishes() {
        let (dropped, mut drop_observed) = oneshot::channel();
        let guard = Arc::new(DropNotification(Some(dropped)));
        let (started, start_observed) = oneshot::channel();
        let (release, wait_for_release) = std::sync::mpsc::channel();
        let operation = tokio::spawn(with_operation_guard(
            guard,
            spawn_blocking(move || {
                started.send(()).unwrap();
                wait_for_release.recv().unwrap();
                Ok(())
            }),
        ));
        start_observed.await.unwrap();
        operation.abort();
        assert!(operation.await.unwrap_err().is_cancelled());
        assert!(matches!(
            drop_observed.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        release.send(()).unwrap();
        drop_observed.await.unwrap();
    }
}
