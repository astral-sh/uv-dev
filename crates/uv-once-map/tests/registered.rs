use std::error::Error;
use std::future::pending;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use futures::poll;
use tokio::time::timeout;
use uv_once_map::{RegisteredOnceMap, Registration};

#[tokio::test]
async fn registered_waiters() -> Result<(), Box<dyn Error>> {
    let mut map = RegisteredOnceMap::<_, _>::default();
    assert!(map.get_registered("package").is_none());
    {
        let Registration::New(first) = map.register_entry("package") else {
            return Err("lookup must not register an absent key".into());
        };
        let Registration::Existing(second) = map.register_entry("package") else {
            return Err("expected the existing registration".into());
        };
        let cloned = first.clone();
        let mut first_wait = pin!(first.wait());
        let mut second_wait = pin!(second.wait());
        assert!(poll!(&mut first_wait).is_pending());
        assert!(poll!(&mut second_wait).is_pending());

        map.done("package", 42);
        assert_eq!(first_wait.await, 42);
        assert_eq!(second_wait.await, 42);
        assert_eq!(cloned.wait_blocking(), 42);
        assert_eq!(first.wait().await, 42);
    }
    assert_eq!(map.remove(&"package"), Some(42));
    assert!(map.get_registered("package").is_none());
    assert!(map.register("package"));
    Ok(())
}

#[test]
fn preloaded_entry() -> Result<(), Box<dyn Error>> {
    let mut map = RegisteredOnceMap::<_, _>::from_iter([("package", Arc::new(42))]);
    {
        let entry = map
            .get_registered("package")
            .ok_or("missing preloaded entry")?;
        assert_eq!(entry.key(), &"package");
        assert_eq!(*entry.wait_blocking(), 42);
    }
    let result = map.remove(&"package").ok_or("missing result")?;
    assert_eq!(*result, 42);
    assert_eq!(Arc::strong_count(&result), 1);
    Ok(())
}

#[tokio::test]
async fn register_or_wait() {
    let map = RegisteredOnceMap::<_, _>::default();
    assert_eq!(map.register_or_wait(&"package").await, None);
    let mut wait = pin!(map.register_or_wait(&"package"));
    assert!(poll!(&mut wait).is_pending());
    map.done("package", 42);
    assert_eq!(wait.await, Some(42));
    assert_eq!(map.register_or_wait(&"package").await, Some(42));
}

/// Completion can precede the first poll of an already-created wait future.
#[tokio::test]
async fn completion_before_first_poll() -> Result<(), Box<dyn Error>> {
    let map = RegisteredOnceMap::<_, _>::default();
    assert!(map.register("package"));
    let entry = map
        .get_registered("package")
        .ok_or("missing registered entry")?;
    let wait = entry.wait();

    map.done("package", 42);

    assert_eq!(timeout(Duration::from_secs(2), wait).await?, 42);
    assert_eq!(map.register_or_wait(&"package").await, Some(42));
    Ok(())
}

/// Cancelling one consumer must not consume the notification for another consumer.
#[tokio::test]
async fn cancelled_waiter_does_not_block_other_waiters() -> Result<(), Box<dyn Error>> {
    let map = RegisteredOnceMap::<_, _>::default();
    assert!(map.register("package"));
    let mut cancelled = Box::pin(map.register_or_wait(&"package"));
    let mut retained = pin!(map.register_or_wait(&"package"));
    assert!(poll!(&mut cancelled).is_pending());
    assert!(poll!(&mut retained).is_pending());

    drop(cancelled);
    map.done("package", 42);

    assert_eq!(timeout(Duration::from_secs(2), retained).await?, Some(42));
    assert_eq!(map.register_or_wait(&"package").await, Some(42));
    Ok(())
}

/// Completion remains available when the last consumer cancels before the producer finishes.
#[tokio::test]
async fn completion_after_last_waiter_cancels() -> Result<(), Box<dyn Error>> {
    let map = RegisteredOnceMap::<_, _>::default();
    assert!(map.register("package"));
    {
        let entry = map
            .get_registered("package")
            .ok_or("missing registered entry")?;
        let mut cancelled = Box::pin(entry.wait());
        assert!(poll!(&mut cancelled).is_pending());
        drop(cancelled);
    }

    assert!(!map.register("package"));
    map.done("package", 42);

    let Registration::Existing(later) = map.register_entry("package") else {
        return Err("completion must retain the registration".into());
    };
    assert_eq!(timeout(Duration::from_secs(2), later.wait()).await?, 42);
    Ok(())
}

/// Exclusive removal after an error lets fresh consumers wait for a new producer's result.
#[tokio::test]
async fn failed_registration_can_be_removed_and_retried() -> Result<(), Box<dyn Error>> {
    let mut map = RegisteredOnceMap::<_, Result<u32, &str>>::default();
    {
        let Registration::New(entry) = map.register_entry("package") else {
            return Err("expected a new registration".into());
        };
        let cloned = entry.clone();
        let mut waiter = pin!(entry.wait());
        assert!(poll!(&mut waiter).is_pending());
        map.done("package", Err("producer cancelled"));
        assert_eq!(
            timeout(Duration::from_secs(2), waiter).await?,
            Err("producer cancelled")
        );
        assert_eq!(cloned.wait().await, Err("producer cancelled"));
    }

    assert_eq!(map.remove(&"package"), Some(Err("producer cancelled")));
    assert!(map.get_registered("package").is_none());
    let Registration::New(retried) = map.register_entry("package") else {
        return Err("removal must permit a new registration".into());
    };
    let Registration::Existing(follower) = map.register_entry("package") else {
        return Err("expected the retry registration".into());
    };
    let mut retried_wait = pin!(retried.wait());
    let mut follower_wait = pin!(follower.wait());
    assert!(poll!(&mut retried_wait).is_pending());
    assert!(poll!(&mut follower_wait).is_pending());

    map.done("package", Ok(42));

    assert_eq!(timeout(Duration::from_secs(2), retried_wait).await?, Ok(42));
    assert_eq!(
        timeout(Duration::from_secs(2), follower_wait).await?,
        Ok(42)
    );
    Ok(())
}

/// The owner of cancelled work must still publish a result for its registered consumers.
#[tokio::test]
async fn aborted_producer_can_publish_an_error() -> Result<(), Box<dyn Error>> {
    let map = Arc::new(RegisteredOnceMap::<_, Result<u32, &str>>::default());
    assert!(map.register("package"));
    let (started, ready) = tokio::sync::oneshot::channel();
    let producer = tokio::spawn({
        let map = Arc::clone(&map);
        async move {
            started.send(()).expect("consumer is waiting for producer");
            pending::<()>().await;
            map.done("package", Ok(42));
        }
    });
    ready.await?;
    let mut waiter = pin!(map.register_or_wait(&"package"));
    assert!(poll!(&mut waiter).is_pending());

    producer.abort();
    let error = producer.await.expect_err("producer must be aborted");
    assert!(error.is_cancelled());
    assert!(poll!(&mut waiter).is_pending());
    map.done("package", Err("producer cancelled"));

    assert_eq!(
        timeout(Duration::from_secs(2), waiter).await?,
        Some(Err("producer cancelled"))
    );
    assert_eq!(
        map.register_or_wait(&"package").await,
        Some(Err("producer cancelled"))
    );
    Ok(())
}
