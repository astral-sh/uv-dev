use std::error::Error;
use std::pin::pin;
use std::sync::Arc;

use futures::{future, poll};
use uv_once_map::{Abandoned, OnceMap, RegisteredOnceMap, Registration};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

#[tokio::test]
async fn cancelled_producer_has_one_retry_winner() -> Result {
    let map = Arc::new(OnceMap::<_, usize>::default());
    let (started, ready) = tokio::sync::oneshot::channel();
    let producer = {
        let map = map.clone();
        tokio::spawn(async move {
            let Registration::New(producer) = map.register_or_wait(&"package").await else {
                return;
            };
            let _ = started.send(());
            future::pending::<()>().await;
            producer.done(0);
        })
    };
    ready.await?;
    let mut waiters = (0..8)
        .map(|_| Box::pin(map.register_or_wait(&"package")))
        .collect::<Vec<_>>();
    for waiter in &mut waiters {
        assert!(poll!(waiter.as_mut()).is_pending());
    }
    producer.abort();
    assert!(
        producer
            .await
            .err()
            .is_some_and(|error| error.is_cancelled())
    );
    let mut winner = None;
    for waiter in &mut waiters {
        match poll!(waiter.as_mut()) {
            std::task::Poll::Ready(Registration::New(producer)) => {
                assert!(winner.is_none());
                winner = Some(producer);
            }
            std::task::Poll::Pending => {}
            std::task::Poll::Ready(Registration::Existing(_)) => {
                return Err("abandoned producer must not publish a result".into());
            }
        }
    }
    winner.ok_or("missing retry producer")?.done(42);
    // The first waiter became the producer and its completed future must not be polled again.
    for waiter in waiters.into_iter().skip(1) {
        let Registration::Existing(value) = waiter.await else {
            return Err("only one retry producer may run".into());
        };
        assert_eq!(value, 42);
    }
    assert_eq!(map.get("package"), Some(42));
    Ok(())
}

#[tokio::test]
async fn dropping_a_waiter_does_not_abandon_its_producer() -> Result {
    let map = OnceMap::<_, usize>::default();
    let Registration::New(producer) = map.register_or_wait(&"package").await else {
        return Err("expected producer".into());
    };
    let mut waiter = Box::pin(map.register_or_wait(&"package"));
    assert!(poll!(waiter.as_mut()).is_pending());
    drop(waiter);
    producer.done(42);
    let Registration::Existing(value) = map.register_or_wait(&"package").await else {
        return Err("completed result must remain cached".into());
    };
    assert_eq!(value, 42);
    Ok(())
}

#[tokio::test]
async fn ordinary_failure_remains_memoized() -> Result {
    let map = OnceMap::<_, std::result::Result<usize, &str>>::default();
    let Registration::New(producer) = map.register_or_wait(&"package").await else {
        return Err("expected producer".into());
    };
    producer.done(Err("failure"));
    let Registration::Existing(result) = map.register_or_wait(&"package").await else {
        return Err("ordinary errors must not be retried".into());
    };
    assert_eq!(result, Err("failure"));
    Ok(())
}

#[tokio::test]
async fn old_completion_does_not_replace_a_new_registration() -> Result {
    let map = OnceMap::<_, usize>::default();
    let Registration::New(old) = map.register_or_wait(&"package").await else {
        return Err("expected producer".into());
    };
    let mut waiter = pin!(map.register_or_wait(&"package"));
    assert!(poll!(&mut waiter).is_pending());
    assert_eq!(map.remove(&"package"), None);
    let Registration::New(new) = waiter.await else {
        return Err("removed generation must be retried".into());
    };
    old.done(1);
    assert_eq!(map.get("package"), None);
    new.done(2);
    assert_eq!(map.get("package"), Some(2));
    Ok(())
}

#[tokio::test]
async fn seeded_result_supersedes_a_pending_producer() -> Result {
    let map = OnceMap::<_, usize>::default();
    let Registration::New(producer) = map.register_or_wait(&"package").await else {
        return Err("expected producer".into());
    };
    map.done("package", 42);
    producer.done(0);
    assert_eq!(map.get("package"), Some(42));
    Ok(())
}

#[tokio::test]
async fn registered_waiter_observes_abandonment() -> Result {
    let map = RegisteredOnceMap::<_, usize>::default();
    let Registration::New(producer) = map.register_or_wait(&"package").await else {
        return Err("expected producer".into());
    };
    let entry = map.get_registered("package").ok_or("missing entry")?;
    let mut waiter = pin!(entry.wait());
    assert!(poll!(&mut waiter).is_pending());
    drop(producer);
    let Registration::New(retry) = map.register_or_wait(&"package").await else {
        return Err("expected retry producer".into());
    };
    retry.done(42);
    assert_eq!(waiter.await, Err(Abandoned));
    assert_eq!(entry.wait_blocking(), Ok(42));
    Ok(())
}

#[tokio::test]
async fn cancellation_after_publication_keeps_the_result() -> Result {
    let map = Arc::new(OnceMap::<_, usize>::default());
    let (published, ready) = tokio::sync::oneshot::channel();
    let producer = {
        let map = map.clone();
        tokio::spawn(async move {
            let Registration::New(producer) = map.register_or_wait(&"package").await else {
                return;
            };
            producer.done(42);
            let _ = published.send(());
            future::pending::<()>().await;
        })
    };
    ready.await?;
    producer.abort();
    assert!(
        producer
            .await
            .err()
            .is_some_and(|error| error.is_cancelled())
    );
    assert_eq!(map.get("package"), Some(42));
    Ok(())
}
