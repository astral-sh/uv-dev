use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::io::{self, IoSlice};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::runtime::{Handle, Id};
use tracing::{Dispatch, Span, trace};

const SNAPSHOT_INTERVAL_US: u64 = 10_000_000;
const MAX_SNAPSHOTS: u64 = 12;
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(250);

static WATCHDOG: OnceLock<io::Result<Arc<Mutex<Registry>>>> = OnceLock::new();
static NEXT_COPY_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
struct Registry {
    copies: Vec<Weak<State>>,
    heartbeats: HashMap<Id, Weak<Heartbeat>>,
}

/// Opt-in observations of copy progress, notifications, and runtime heartbeat activity.
pub(super) struct CopyDiagnostics {
    state: Arc<State>,
    _heartbeat: Arc<Heartbeat>,
}

impl CopyDiagnostics {
    pub(super) fn new(download: String) -> Self {
        let registry = match WATCHDOG.get_or_init(start_watchdog) {
            Ok(registry) => Some(registry),
            Err(error) => {
                tracing::error!(%error, "Failed to start Python copy diagnostics watchdog");
                None
            }
        };
        let mut registry = registry.map(|registry| {
            registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        });
        let runtime = Handle::current().id();
        let heartbeat = registry
            .as_ref()
            .and_then(|registry| registry.heartbeats.get(&runtime))
            .and_then(Weak::upgrade)
            .unwrap_or_else(start_heartbeat);
        let state = Arc::new(State::new(download, Arc::downgrade(&heartbeat)));
        if let Some(registry) = registry.as_mut() {
            registry.copies.push(Arc::downgrade(&state));
            registry
                .heartbeats
                .insert(runtime, Arc::downgrade(&heartbeat));
        }
        Self {
            state,
            _heartbeat: heartbeat,
        }
    }

    pub(super) fn reader<R>(&self, inner: R) -> ObservedReader<R> {
        ObservedReader {
            inner,
            state: Arc::clone(&self.state),
        }
    }

    pub(super) fn writer<W>(&self, inner: W) -> ObservedWriter<W> {
        ObservedWriter {
            inner,
            state: Arc::clone(&self.state),
        }
    }

    pub(super) fn progress<T>(&self, callback: impl FnOnce() -> T) -> T {
        self.state.stage(Stage::Progress);
        let result = callback();
        self.state.stage(Stage::Copy);
        result
    }

    pub(super) async fn copy<R, W>(&self, reader: &mut R, writer: &mut W) -> io::Result<u64>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut copy = std::pin::pin!(tokio::io::copy(reader, writer));
        let mut forward: Option<Arc<CopyWake>> = None;
        let result = poll_fn(|cx| {
            let first_wake = self.state.first_wake.swap(0, Ordering::Relaxed);
            let now = self.state.now();
            if first_wake != 0 {
                self.state
                    .max_wake_to_poll
                    .fetch_max(now.saturating_sub(first_wake - 1), Ordering::Relaxed);
            }
            self.state.copy_polls.fetch_add(1, Ordering::Relaxed);
            self.state.stage_at(Stage::Copy, now);
            let wake = forward.get_or_insert_with(|| {
                Arc::new(CopyWake {
                    parent: cx.waker().clone(),
                    state: Arc::clone(&self.state),
                })
            });
            if !wake.parent.will_wake(cx.waker()) {
                *wake = Arc::new(CopyWake {
                    parent: cx.waker().clone(),
                    state: Arc::clone(&self.state),
                });
            }
            let waker = Waker::from(Arc::clone(wake));
            let result = copy.as_mut().poll(&mut Context::from_waker(&waker));
            let end = self.state.now();
            self.state
                .max_copy_poll
                .fetch_max(end.saturating_sub(now), Ordering::Relaxed);
            if result.is_pending() {
                let mask = u8::from(self.state.reader_pending.load(Ordering::Relaxed))
                    | (u8::from(self.state.writer_pending.load(Ordering::Relaxed)) << 1)
                    | (u8::from(self.state.flush_pending.load(Ordering::Relaxed)) << 2);
                self.state.pending_mask.store(mask, Ordering::Relaxed);
                self.state.stage_at(Stage::Suspended, end);
            }
            result
        })
        .await;
        self.state.finished.store(true, Ordering::Relaxed);
        self.state.stage(Stage::Finished);
        self.state
            .report(if result.is_ok() { "complete" } else { "error" });
        result
    }
}

impl Drop for CopyDiagnostics {
    fn drop(&mut self) {
        self.state.finished.store(true, Ordering::Relaxed);
    }
}

struct CopyWake {
    parent: Waker,
    state: Arc<State>,
}

impl Wake for CopyWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self.state.first_wake.compare_exchange(
            0,
            self.state.now() + 1,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        self.parent.wake_by_ref();
    }
}

pub(super) struct ObservedReader<R> {
    inner: R,
    state: Arc<State>,
}

impl<R: AsyncRead + Unpin> AsyncRead for ObservedReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.state.stage(Stage::Read);
        let before = buffer.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buffer);
        let now = this.state.now();
        this.state
            .reader_pending
            .store(result.is_pending(), Ordering::Relaxed);
        let bytes = buffer.filled().len().saturating_sub(before) as u64;
        if bytes != 0 {
            this.state.read_bytes.fetch_add(bytes, Ordering::Relaxed);
            this.state.last_read.store(now, Ordering::Relaxed);
        }
        this.state.stage_at(Stage::Copy, now);
        result
    }
}

pub(super) struct ObservedWriter<W> {
    inner: W,
    state: Arc<State>,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for ObservedWriter<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.state.stage(Stage::Write);
        let result = Pin::new(&mut this.inner).poll_write(cx, buffer);
        this.state.written(&result);
        result
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffers: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.state.stage(Stage::Write);
        let result = Pin::new(&mut this.inner).poll_write_vectored(cx, buffers);
        this.state.written(&result);
        result
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.state.stage(Stage::Flush);
        let result = Pin::new(&mut this.inner).poll_flush(cx);
        this.state
            .flush_pending
            .store(result.is_pending(), Ordering::Relaxed);
        this.state.stage(Stage::Copy);
        result
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[derive(Clone, Copy)]
#[repr(u64)]
enum Stage {
    Copy,
    Read,
    Write,
    Flush,
    Progress,
    Suspended,
    Finished,
}

impl Stage {
    fn name(value: u64) -> &'static str {
        match value {
            0 => "copy",
            1 => "read-poll",
            2 => "write-poll",
            3 => "flush-poll",
            4 => "progress-callback",
            5 => "suspended",
            6 => "finished",
            _ => "unknown",
        }
    }
}

struct State {
    id: u64,
    download: String,
    started: Instant,
    span: Span,
    dispatch: Dispatch,
    heartbeat: Weak<Heartbeat>,
    active: AtomicU64,
    reader_pending: AtomicBool,
    writer_pending: AtomicBool,
    flush_pending: AtomicBool,
    // Bits 0, 1, and 2 capture the reader, writer, and flush Pending states when copy last
    // suspended. Each state is that boundary's most recent poll result; these results can overlap
    // or predate the suspension. The mask changes only when the outer copy suspends again.
    pending_mask: AtomicU8,
    read_bytes: AtomicU64,
    // Accepted writes can still be buffered or waiting on a filesystem worker.
    accepted_bytes: AtomicU64,
    // Before any nonempty read or accepted write, the corresponding age is measured from
    // diagnostics creation.
    last_read: AtomicU64,
    last_write: AtomicU64,
    copy_polls: AtomicU64,
    // Copy polls include nested reader, writer, and progress callback work.
    max_copy_poll: AtomicU64,
    // A notification does not guarantee readiness. Record the first notification until copy is
    // polled again, including notifications received while an earlier poll is still running.
    first_wake: AtomicU64,
    max_wake_to_poll: AtomicU64,
    finished: AtomicBool,
    snapshots: AtomicU64,
    last_snapshot: AtomicU64,
}

impl State {
    fn new(download: String, heartbeat: Weak<Heartbeat>) -> Self {
        Self {
            id: NEXT_COPY_ID.fetch_add(1, Ordering::Relaxed),
            download,
            started: Instant::now(),
            span: Span::current(),
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            heartbeat,
            active: AtomicU64::new(0),
            reader_pending: AtomicBool::new(false),
            writer_pending: AtomicBool::new(false),
            flush_pending: AtomicBool::new(false),
            pending_mask: AtomicU8::new(0),
            read_bytes: AtomicU64::new(0),
            accepted_bytes: AtomicU64::new(0),
            last_read: AtomicU64::new(0),
            last_write: AtomicU64::new(0),
            copy_polls: AtomicU64::new(0),
            max_copy_poll: AtomicU64::new(0),
            first_wake: AtomicU64::new(0),
            max_wake_to_poll: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            snapshots: AtomicU64::new(0),
            last_snapshot: AtomicU64::new(0),
        }
    }

    fn now(&self) -> u64 {
        micros_since(self.started)
    }

    fn stage(&self, stage: Stage) {
        self.stage_at(stage, self.now());
    }

    fn stage_at(&self, stage: Stage, now: u64) {
        self.active
            .store((now << 3) | stage as u64, Ordering::Relaxed);
    }

    fn written(&self, result: &Poll<io::Result<usize>>) {
        let now = self.now();
        self.writer_pending
            .store(result.is_pending(), Ordering::Relaxed);
        if let Poll::Ready(Ok(bytes)) = result
            && *bytes != 0
        {
            self.accepted_bytes
                .fetch_add(*bytes as u64, Ordering::Relaxed);
            self.last_write.store(now, Ordering::Relaxed);
        }
        self.stage_at(Stage::Copy, now);
    }

    fn report(&self, outcome: &'static str) {
        // Live snapshots read independently updated counters and boundary states.
        let now = self.now();
        let active = self.active.load(Ordering::Relaxed);
        let wake = self.first_wake.load(Ordering::Relaxed);
        let wake_age = (wake != 0).then(|| now.saturating_sub(wake - 1));
        let heartbeat = self.heartbeat.upgrade();
        let heartbeat_age = heartbeat.as_ref().map(|heartbeat| heartbeat.age());
        // This shared maximum can include a gap before this particular copy began.
        let heartbeat_max_gap = heartbeat.as_ref().map(|heartbeat| {
            heartbeat
                .max_gap
                .load(Ordering::Relaxed)
                .max(heartbeat.age())
        });
        tracing::dispatcher::with_default(&self.dispatch, || {
            self.span.in_scope(|| {
                trace!(
                    copy_id = self.id,
                    download = %self.download,
                    outcome,
                    elapsed_us = now,
                    active = Stage::name(active & 7),
                    active_us = now.saturating_sub(active >> 3),
                    read_bytes = self.read_bytes.load(Ordering::Relaxed),
                    writer_accepted_bytes = self.accepted_bytes.load(Ordering::Relaxed),
                    since_read_us = now.saturating_sub(self.last_read.load(Ordering::Relaxed)),
                    since_write_us = now.saturating_sub(self.last_write.load(Ordering::Relaxed)),
                    last_pending_mask = self.pending_mask.load(Ordering::Relaxed),
                    copy_polls = self.copy_polls.load(Ordering::Relaxed),
                    max_copy_poll_us = self.max_copy_poll.load(Ordering::Relaxed),
                    wake_age_us = ?wake_age,
                    max_wake_to_poll_us = self.max_wake_to_poll.load(Ordering::Relaxed),
                    runtime_heartbeat_age_us = ?heartbeat_age,
                    runtime_shared_max_heartbeat_gap_us = ?heartbeat_max_gap,
                    "Python archive copy observation"
                );
            });
        });
    }
}

struct Heartbeat {
    started: Instant,
    last_tick: AtomicU64,
    max_gap: AtomicU64,
}

impl Heartbeat {
    fn age(&self) -> u64 {
        micros_since(self.started).saturating_sub(self.last_tick.load(Ordering::Relaxed))
    }
}

fn start_heartbeat() -> Arc<Heartbeat> {
    let heartbeat = Arc::new(Heartbeat {
        started: Instant::now(),
        last_tick: AtomicU64::new(0),
        max_gap: AtomicU64::new(0),
    });
    let watched = Arc::downgrade(&heartbeat);
    // Copies on the same runtime share this pulse. The task keeps no strong reference while
    // asleep, so it stops after the last diagnostics owner is dropped.
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(HEARTBEAT_INTERVAL).await;
            let Some(heartbeat) = watched.upgrade() else {
                return;
            };
            let now = micros_since(heartbeat.started);
            let previous = heartbeat.last_tick.swap(now, Ordering::Relaxed);
            heartbeat
                .max_gap
                .fetch_max(now.saturating_sub(previous), Ordering::Relaxed);
        }
    });
    heartbeat
}

fn start_watchdog() -> io::Result<Arc<Mutex<Registry>>> {
    let registry: Arc<Mutex<Registry>> = Arc::default();
    let watched = Arc::clone(&registry);
    std::thread::Builder::new()
        .name("uv-copy-diagnostics".to_owned())
        .spawn(move || {
            loop {
                std::thread::park_timeout(Duration::from_secs(1));
                let copies: Vec<_> = {
                    let mut registry = watched
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let copies = registry.copies.iter().filter_map(Weak::upgrade).collect();
                    registry.copies.retain(|copy| copy.strong_count() != 0);
                    registry
                        .heartbeats
                        .retain(|_, heartbeat| heartbeat.strong_count() != 0);
                    copies
                };
                for copy in copies {
                    let now = copy.now();
                    if !copy.finished.load(Ordering::Relaxed)
                        && copy.snapshots.load(Ordering::Relaxed) < MAX_SNAPSHOTS
                        && now.saturating_sub(copy.last_snapshot.load(Ordering::Relaxed))
                            >= SNAPSHOT_INTERVAL_US
                    {
                        copy.last_snapshot.store(now, Ordering::Relaxed);
                        copy.snapshots.fetch_add(1, Ordering::Relaxed);
                        copy.report("snapshot");
                    }
                }
            }
        })?;
    Ok(registry)
}

fn micros_since(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros())
        .unwrap_or(u64::MAX >> 3)
        .min(u64::MAX >> 3)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    struct CountWake(AtomicUsize);

    impl Wake for CountWake {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    struct Reader {
        remaining: usize,
        pending: bool,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl AsyncRead for Reader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            self.events.lock().expect("events lock").push("read");
            if std::mem::take(&mut self.pending) {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            if self.remaining != 0 && buffer.remaining() != 0 {
                buffer.put_slice(b"x");
                self.remaining -= 1;
            }
            Poll::Ready(Ok(()))
        }
    }

    struct Writer {
        bytes: Vec<u8>,
        write_pending: bool,
        flush_pending: bool,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl AsyncWrite for Writer {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.events.lock().expect("events lock").push("write");
            if std::mem::take(&mut self.write_pending) {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            let count = bytes.len().min(1);
            self.bytes.extend_from_slice(&bytes[..count]);
            Poll::Ready(Ok(count))
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.events.lock().expect("events lock").push("flush");
            if std::mem::take(&mut self.flush_pending) {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.poll_flush(cx)
        }
    }

    fn streams(events: &Arc<Mutex<Vec<&'static str>>>) -> (Reader, Writer) {
        (
            Reader {
                remaining: 7,
                pending: true,
                events: Arc::clone(events),
            },
            Writer {
                bytes: Vec::new(),
                write_pending: true,
                flush_pending: true,
                events: Arc::clone(events),
            },
        )
    }

    #[test]
    fn copy_forwards_pending_and_short_writes() -> io::Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let plain_events = Arc::default();
                let (mut plain_reader, mut plain_writer) = streams(&plain_events);
                let plain_count = tokio::io::copy(&mut plain_reader, &mut plain_writer).await?;

                let observed_events = Arc::default();
                let (reader, mut writer) = streams(&observed_events);
                let diagnostics = CopyDiagnostics::new("forwarding".to_owned());
                let observed_count = diagnostics
                    .copy(
                        &mut diagnostics.reader(reader),
                        &mut diagnostics.writer(&mut writer),
                    )
                    .await?;
                assert_eq!(observed_count, plain_count);
                assert_eq!(writer.bytes, plain_writer.bytes);
                assert_eq!(
                    *observed_events.lock().expect("events lock"),
                    *plain_events.lock().expect("events lock")
                );
                assert_eq!(diagnostics.state.read_bytes.load(Ordering::Relaxed), 7);
                assert_eq!(diagnostics.state.accepted_bytes.load(Ordering::Relaxed), 7);
                Ok(())
            })
    }

    #[test]
    fn wakes_are_forwarded_without_replacing_the_first_notification() {
        let parent = Arc::new(CountWake(AtomicUsize::new(0)));
        let state = Arc::new(State::new("wake".to_owned(), Weak::new()));
        let forward = Arc::new(CopyWake {
            parent: Waker::from(Arc::clone(&parent)),
            state: Arc::clone(&state),
        });
        forward.wake_by_ref();
        let first = state.first_wake.load(Ordering::Relaxed);
        forward.wake_by_ref();
        forward.wake();
        assert_ne!(first, 0);
        assert_eq!(state.first_wake.load(Ordering::Relaxed), first);
        assert_eq!(parent.0.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn runtime_heartbeat_is_shared_and_released() -> io::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let _guard = runtime.enter();
        let first = CopyDiagnostics::new("first".to_owned());
        let second = CopyDiagnostics::new("second".to_owned());
        assert!(Weak::ptr_eq(
            &first.state.heartbeat,
            &second.state.heartbeat
        ));
        let state = Arc::clone(&first.state);
        let heartbeat = first.state.heartbeat.clone();
        drop(first);
        assert!(state.finished.load(Ordering::Relaxed));
        assert!(heartbeat.upgrade().is_some());
        drop(second);
        assert!(heartbeat.upgrade().is_none());
        Ok(())
    }
}
