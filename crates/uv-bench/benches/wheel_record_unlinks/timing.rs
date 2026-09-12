//! Time deletion calls on fresh fixtures without measuring their reconstruction or audit.

use std::hint::black_box;
use std::thread;
use std::time::{Duration, Instant};

pub(super) fn isolated<Output: Send>(operation: impl FnOnce() -> Output + Send) -> Output {
    thread::scope(|scope| {
        scope
            .spawn(operation)
            .join()
            .expect("Wheel-unlink benchmark thread panicked")
    })
}

/// A sample batch owns one fresh submitting thread and one initialized backend. Each operation
/// gets a different, fully audited fixture. Only the operation call is timed; result and fixture
/// destruction happen after the untimed oracle. The state and outputs need not be `Send`.
pub(super) fn isolated_trials<State, Trial, Output>(
    iterations: u64,
    setup: impl FnOnce() -> State + Send,
    mut materialize: impl FnMut() -> Trial + Send,
    mut operation: impl FnMut(&mut State, &Trial) -> Output + Send,
    mut audit: impl FnMut(&Trial, &Output) + Send,
    finish: impl FnOnce(&mut State) + Send,
) -> Duration {
    isolated(move || {
        let mut state = setup();
        let mut measured = Duration::ZERO;
        for _ in 0..iterations {
            let trial = materialize();
            let start = Instant::now();
            let output = black_box(operation(&mut state, black_box(&trial)));
            measured += start.elapsed();
            audit(&trial, &output);
            drop(output);
            drop(trial);
        }
        finish(&mut state);
        measured
    })
}
