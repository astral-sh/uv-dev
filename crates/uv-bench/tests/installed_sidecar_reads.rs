#[path = "../benches/installed_sidecar_reads/timing.rs"]
mod timing;

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]
mod uring {
    include!("../benches/installed_sidecar_reads/uring.rs");
    include!("installed_sidecar_reads/uring_cases.rs");
}

mod timing_tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::thread::{self, ThreadId};

    use super::timing::{isolated, isolated_time};

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Event {
        Setup,
        Operation(u64),
        OutputDropped(u64),
        StateDropped,
    }

    type Events = Arc<Mutex<Vec<(ThreadId, Event)>>>;

    fn record(events: &Events, event: Event) {
        events
            .lock()
            .expect("Failed to lock timing-test events")
            .push((thread::current().id(), event));
    }

    struct State {
        next: Rc<Cell<u64>>,
        events: Events,
    }

    impl Drop for State {
        fn drop(&mut self) {
            record(&self.events, Event::StateDropped);
        }
    }

    struct Output {
        next: Rc<Cell<u64>>,
        index: u64,
        events: Events,
    }

    impl Drop for Output {
        fn drop(&mut self) {
            assert_eq!(self.next.get(), self.index + 1);
            record(&self.events, Event::OutputDropped(self.index));
        }
    }

    #[test]
    fn isolated_time_keeps_setup_work_and_drops_on_one_fresh_thread() {
        let caller = thread::current().id();
        let events = Events::default();
        let setup_events = Arc::clone(&events);
        let _elapsed = isolated_time(
            3,
            move || {
                record(&setup_events, Event::Setup);
                State {
                    next: Rc::new(Cell::new(0)),
                    events: setup_events,
                }
            },
            |state| {
                let index = state.next.get();
                state.next.set(index + 1);
                record(&state.events, Event::Operation(index));
                Output {
                    next: Rc::clone(&state.next),
                    index,
                    events: Arc::clone(&state.events),
                }
            },
        );

        let events = events.lock().expect("Failed to lock timing-test events");
        let submitting_thread = events[0].0;
        assert_ne!(submitting_thread, caller);
        assert!(
            events
                .iter()
                .all(|(thread, _)| *thread == submitting_thread)
        );
        assert_eq!(
            events.iter().map(|(_, event)| event).collect::<Vec<_>>(),
            [
                &Event::Setup,
                &Event::Operation(0),
                &Event::OutputDropped(0),
                &Event::Operation(1),
                &Event::OutputDropped(1),
                &Event::Operation(2),
                &Event::OutputDropped(2),
                &Event::StateDropped,
            ]
        );

        let preflight_thread = isolated(|| thread::current().id());
        assert_ne!(preflight_thread, caller);
        assert_ne!(preflight_thread, submitting_thread);
    }
}
