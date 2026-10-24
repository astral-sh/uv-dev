use std::future::{pending, ready};
use std::time::Duration;

use futures::poll;
use uv_once_map::OnceMap;

#[tokio::test]
async fn cancelled_initialization_wakes_a_waiter() {
    let map = OnceMap::<_, _>::default();
    let mut owner = Box::pin(map.get_or_init("package", pending::<u32>()));
    assert!(poll!(&mut owner).is_pending());

    let mut waiter = Box::pin(map.get_or_init("package", ready(42)));
    assert!(poll!(&mut waiter).is_pending());
    drop(owner);

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("cancelled initialization blocked a waiter"),
        42
    );
    assert_eq!(map.get_or_init("package", ready(13)).await, 42);
}

#[tokio::test]
async fn cancelled_initialization_does_not_remove_a_completed_entry() {
    let map = OnceMap::<_, _>::default();
    let mut owner = Box::pin(map.get_or_init("package", pending::<u32>()));
    assert!(poll!(&mut owner).is_pending());

    map.done("package", 42);
    drop(owner);

    assert_eq!(map.get(&"package"), Some(42));
}

#[tokio::test]
async fn cancelled_initialization_does_not_remove_a_pending_replacement() {
    let map = OnceMap::<_, _>::default();
    let mut owner = Box::pin(map.get_or_init("package", pending::<u32>()));
    assert!(poll!(&mut owner).is_pending());
    assert_eq!(map.remove(&"package"), None);

    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut replacement = Box::pin(map.get_or_init("package", async { receiver.await.unwrap() }));
    assert!(poll!(&mut replacement).is_pending());

    drop(owner);
    sender.send(42).unwrap();

    assert_eq!(replacement.await, 42);
    assert_eq!(map.get(&"package"), Some(42));
}

#[tokio::test]
async fn initialization_returns_a_result_published_by_another_caller() {
    let map = OnceMap::<_, _>::default();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut owner = Box::pin(map.get_or_init("package", async { receiver.await.unwrap() }));
    assert!(poll!(&mut owner).is_pending());

    map.done("package", 42);
    sender.send(13).unwrap();

    assert_eq!(owner.await, 42);
    assert_eq!(map.get(&"package"), Some(42));
}

#[tokio::test]
async fn initialization_does_not_reinsert_a_removed_entry() {
    let map = OnceMap::<_, _>::default();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut owner = Box::pin(map.get_or_init("package", async { receiver.await.unwrap() }));
    assert!(poll!(&mut owner).is_pending());

    assert_eq!(map.remove(&"package"), None);
    sender.send(13).unwrap();

    assert_eq!(owner.await, 13);
    assert_eq!(map.get(&"package"), None);
}

#[tokio::test]
async fn removing_a_pending_job_wakes_a_waiter() {
    let map = OnceMap::<_, u32>::default();
    assert_eq!(map.register_or_wait(&"package").await, None);
    let mut waiter = Box::pin(map.get_or_init("package", ready(42)));
    assert!(poll!(&mut waiter).is_pending());

    assert_eq!(map.remove(&"package"), None);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("removed pending job blocked a waiter"),
        42
    );
}
