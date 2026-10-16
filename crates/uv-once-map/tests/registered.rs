use std::error::Error;
use std::pin::pin;
use std::sync::Arc;

use futures::poll;
use uv_once_map::{OnceMap, RegisteredOnceMap, Registration};

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
        assert_eq!(first_wait.await, Some(42));
        assert_eq!(second_wait.await, Some(42));
        assert_eq!(cloned.wait_blocking(), Some(42));
        assert_eq!(first.wait().await, Some(42));
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
        assert_eq!(*entry.wait_blocking().ok_or("missing result")?, 42);
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

#[tokio::test]
async fn abandoned_producer_allows_retry() -> Result<(), Box<dyn Error>> {
    let map = RegisteredOnceMap::<_, _>::default();
    let producer = map
        .register_with_producer("package")
        .ok_or("expected a new registration")?;
    let entry = map
        .get_registered("package")
        .ok_or("missing registration")?;
    let mut wait = pin!(entry.wait());
    assert!(poll!(&mut wait).is_pending());

    drop(producer);
    assert_eq!(wait.await, None);

    let producer = map
        .register_with_producer("package")
        .ok_or("expected a replacement registration")?;
    let entry = map
        .get_registered("package")
        .ok_or("missing replacement registration")?;
    producer.done(42);
    assert_eq!(entry.wait().await, Some(42));
    Ok(())
}

#[tokio::test]
async fn register_or_wait_retries_abandoned_producer() -> Result<(), Box<dyn Error>> {
    let map = RegisteredOnceMap::<_, _>::default();
    let producer = map
        .register_with_producer("package")
        .ok_or("expected a new registration")?;
    let mut first = pin!(map.register_or_wait(&"package"));
    let mut second = pin!(map.register_or_wait(&"package"));
    assert!(poll!(&mut first).is_pending());
    assert!(poll!(&mut second).is_pending());

    drop(producer);
    tokio::select! {
        result = &mut first => {
            assert_eq!(result, None);
            map.done("package", 42);
            assert_eq!(second.await, Some(42));
        }
        result = &mut second => {
            assert_eq!(result, None);
            map.done("package", 42);
            assert_eq!(first.await, Some(42));
        }
    }
    Ok(())
}

#[tokio::test]
async fn removed_producer_cannot_replace_retry() -> Result<(), Box<dyn Error>> {
    let map = OnceMap::<_, _>::default();
    let producer = map
        .register_with_producer("package")
        .ok_or("expected a new registration")?;
    let mut wait = pin!(map.register_or_wait(&"package"));
    assert!(poll!(&mut wait).is_pending());

    assert_eq!(map.remove(&"package"), None);
    assert_eq!(wait.await, None);
    producer.done(1);
    assert_eq!(map.get(&"package"), None);

    map.done("package", 2);
    assert_eq!(map.get(&"package"), Some(2));
    Ok(())
}

#[test]
fn seeded_result_supersedes_pending_producer() -> Result<(), Box<dyn Error>> {
    let map = RegisteredOnceMap::<_, _>::default();
    let producer = map
        .register_with_producer("package")
        .ok_or("expected a new registration")?;

    map.done("package", 42);
    producer.done(0);

    assert_eq!(map.get(&"package"), Some(42));
    Ok(())
}
