use std::error::Error;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;

use futures::executor::block_on;

use uv_once_map::OnceMap;

#[test]
fn registration_has_one_winner() {
    const TASKS: usize = 8;

    let map: OnceMap<&str, usize> = OnceMap::default();
    let barrier = Barrier::new(TASKS);
    let winners = thread::scope(|scope| {
        let tasks: Vec<_> = (0..TASKS)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    map.register("package")
                })
            })
            .collect();

        tasks
            .into_iter()
            .map(|task| task.join().expect("registration thread failed"))
            .filter(|registered| *registered)
            .count()
    });

    assert_eq!(winners, 1);
    assert_eq!(map.get("package"), None);
    map.done("package", 42);
    assert_eq!(map.get("package"), Some(42));
    assert!(!map.register("package"));
}

#[derive(Default)]
struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn done_wakes_registered_waiters() -> Result<(), Box<dyn Error>> {
    let map: OnceMap<&str, Arc<Vec<usize>>> = OnceMap::default();
    let key = "package";
    assert_eq!(block_on(map.register_or_wait(&key)), None);

    let first_wakes = Arc::new(WakeCounter::default());
    let second_wakes = Arc::new(WakeCounter::default());
    let first_waker = Waker::from(first_wakes.clone());
    let second_waker = Waker::from(second_wakes.clone());
    let mut first_context = Context::from_waker(&first_waker);
    let mut second_context = Context::from_waker(&second_waker);
    let mut first = Box::pin(map.register_or_wait(&key));
    let mut second = Box::pin(map.register_or_wait(&key));

    assert_eq!(first.as_mut().poll(&mut first_context), Poll::Pending);
    assert_eq!(second.as_mut().poll(&mut second_context), Poll::Pending);
    let value = Arc::new(vec![1, 2, 3]);
    map.done(key, value.clone());

    assert!(first_wakes.0.load(Ordering::Relaxed) > 0);
    assert!(second_wakes.0.load(Ordering::Relaxed) > 0);
    let Poll::Ready(Some(first_value)) = first.as_mut().poll(&mut first_context) else {
        return Err("first waiter was not ready".into());
    };
    let Poll::Ready(Some(second_value)) = second.as_mut().poll(&mut second_context) else {
        return Err("second waiter was not ready".into());
    };
    assert!(Arc::ptr_eq(&first_value, &value));
    assert!(Arc::ptr_eq(&second_value, &value));

    let cached = block_on(map.register_or_wait(&key)).expect("completed job was not cached");
    assert!(Arc::ptr_eq(&cached, &value));
    Ok(())
}

#[test]
fn values_are_cloned_and_borrowed_keys_are_supported() {
    let map: OnceMap<String, Vec<usize>> = [(String::from("package"), vec![1, 2, 3])]
        .into_iter()
        .collect();

    let mut cloned = map.get("package").expect("completed job was not cached");
    cloned.push(4);
    assert_eq!(map.get("package"), Some(vec![1, 2, 3]));
    assert_eq!(map.get("missing"), None);

    assert_eq!(map.remove("package"), Some(vec![1, 2, 3]));
    assert_eq!(map.get("package"), None);
    assert_eq!(map.remove("package"), None);
    assert!(map.register(String::from("package")));
}

#[test]
fn waiting_for_unregistered_tasks_returns_errors() {
    let map: OnceMap<&str, usize> = OnceMap::default();
    let asynchronous = "asynchronous";
    let blocking = "blocking";

    let error = block_on(map.wait(&asynchronous)).expect_err("task was not registered");
    assert_eq!(
        error.to_string(),
        "Attempted to wait on an unregistered task: asynchronous"
    );
    let error = map
        .wait_blocking(&blocking)
        .expect_err("task was not registered");
    assert_eq!(
        error.to_string(),
        "Attempted to wait on an unregistered task: blocking"
    );

    assert!(!map.register(asynchronous));
    assert!(!map.register(blocking));
    map.done(asynchronous, 1);
    map.done(blocking, 2);
    assert_eq!(block_on(map.wait(&asynchronous)).expect("job is done"), 1);
    assert_eq!(map.wait_blocking(&blocking).expect("job is done"), 2);
}
