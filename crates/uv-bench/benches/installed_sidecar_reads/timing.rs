//! Keep task-owned I/O worker state local to one benchmark sample batch.

use std::hint::black_box;
use std::thread;
use std::time::{Duration, Instant};

pub(super) fn isolated<Output: Send>(operation: impl FnOnce() -> Output + Send) -> Output {
    thread::scope(|scope| {
        scope
            .spawn(operation)
            .join()
            .expect("Filesystem benchmark thread panicked")
    })
}

/// Run a Criterion batch on a fresh submitting thread without timing its creation or teardown.
///
/// The setup state and operation outputs need not be `Send`: they are created, used, and dropped
/// on the submitting thread. Only the measured duration crosses the thread boundary.
pub(super) fn isolated_time<State, Output>(
    iterations: u64,
    setup: impl FnOnce() -> State + Send,
    mut operation: impl FnMut(&mut State) -> Output + Send,
) -> Duration {
    isolated(move || {
        let mut state = setup();
        let start = Instant::now();
        for _ in 0..iterations {
            drop(black_box(operation(&mut state)));
        }
        start.elapsed()
    })
}
