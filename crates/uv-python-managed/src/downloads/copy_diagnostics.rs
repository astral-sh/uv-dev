use std::fmt::Write;
use std::future::{Future, poll_fn};
use std::io::{self, IoSlice};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tracing::{Dispatch, Span, trace};

const SNAPSHOT_INTERVAL_NS: u64 = 10_000_000_000;
const SNAPSHOT_INTERVAL: Duration = Duration::from_nanos(SNAPSHOT_INTERVAL_NS);
const MAX_SNAPSHOTS: u64 = 12;
type Registry = Arc<Mutex<Vec<Weak<State>>>>;

static WATCHDOG: OnceLock<io::Result<Registry>> = OnceLock::new();
static NEXT_COPY_ID: AtomicU64 = AtomicU64::new(1);

/// Opt-in measurements of the existing copy future and its I/O boundaries.
pub(super) struct CopyDiagnostics {
    state: Arc<State>,
}

impl CopyDiagnostics {
    pub(super) fn new(download: String) -> Self {
        let state = Arc::new(State::new(download));
        match WATCHDOG.get_or_init(start_watchdog) {
            Ok(registry) => registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Arc::downgrade(&state)),
            Err(error) => tracing::error!(
                %error,
                "Failed to start Python archive copy diagnostics watchdog"
            ),
        }
        Self { state }
    }

    pub(super) fn reader<R>(&self, inner: R) -> TimedReader<R> {
        TimedReader {
            inner,
            state: Arc::clone(&self.state),
        }
    }

    pub(super) fn writer<W>(&self, inner: W) -> TimedWriter<W> {
        TimedWriter {
            inner,
            state: Arc::clone(&self.state),
        }
    }

    pub(super) fn progress<T>(&self, callback: impl FnOnce() -> T) -> T {
        let start = self.state.enter(Stage::Progress, &self.state.progress);
        let result = callback();
        self.state.leave(&self.state.progress, start, false, 0);
        result
    }

    pub(super) async fn copy<R, W>(&self, reader: &mut R, writer: &mut W) -> io::Result<u64>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut copy = std::pin::pin!(tokio::io::copy(reader, writer));
        let result = poll_fn(|cx| {
            let now = self.state.now();
            if let Some(gap) = self.state.copy.enter(now) {
                let mask = usize::from(self.state.suspended_mask.load(Ordering::Relaxed));
                self.state.suspended_ns[mask].fetch_add(gap, Ordering::Relaxed);
            }
            self.state.stage(Stage::Copy, now);
            let result = copy.as_mut().poll(cx);
            let end = self.state.now();
            self.state.copy.leave(now, end, result.is_pending(), 0);
            if result.is_pending() {
                self.state
                    .suspended_mask
                    .store(self.state.pending_mask(), Ordering::Relaxed);
                self.state.stage(Stage::Suspended, end);
            }
            result
        })
        .await;
        self.state.finished.store(true, Ordering::Relaxed);
        self.state.stage(Stage::Finished, self.state.now());
        self.state
            .report(if result.is_ok() { "complete" } else { "error" });
        result
    }
}

pub(super) struct TimedReader<R> {
    inner: R,
    state: Arc<State>,
}

impl<R: AsyncRead + Unpin> AsyncRead for TimedReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let start = this.state.enter(Stage::Read, &this.state.read);
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        let bytes = buf.filled().len().saturating_sub(before) as u64;
        this.state
            .leave(&this.state.read, start, result.is_pending(), bytes);
        result
    }
}

pub(super) struct TimedWriter<W> {
    inner: W,
    state: Arc<State>,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for TimedWriter<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let start = this.state.enter(Stage::Write, &this.state.write);
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        let bytes = match &result {
            Poll::Ready(Ok(bytes)) => *bytes as u64,
            _ => 0,
        };
        this.state
            .leave(&this.state.write, start, result.is_pending(), bytes);
        result
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let start = this.state.enter(Stage::Write, &this.state.write);
        let result = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        let bytes = match &result {
            Poll::Ready(Ok(bytes)) => *bytes as u64,
            _ => 0,
        };
        this.state
            .leave(&this.state.write, start, result.is_pending(), bytes);
        result
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let start = this.state.enter(Stage::Flush, &this.state.flush);
        let result = Pin::new(&mut this.inner).poll_flush(cx);
        this.state
            .leave(&this.state.flush, start, result.is_pending(), 0);
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

#[derive(Default)]
struct Boundary {
    calls: AtomicU64,
    pending_returns: AtomicU64,
    bytes: AtomicU64,
    sync_total_ns: AtomicU64,
    sync_max_ns: AtomicU64,
    pending_total_ns: AtomicU64,
    pending_max_ns: AtomicU64,
    pending_since: AtomicU64,
}

impl Boundary {
    fn enter(&self, now: u64) -> Option<u64> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let previous = self.pending_since.swap(0, Ordering::Relaxed);
        if previous == 0 {
            return None;
        }
        let elapsed = now.saturating_sub(previous - 1);
        self.pending_total_ns.fetch_add(elapsed, Ordering::Relaxed);
        self.pending_max_ns.fetch_max(elapsed, Ordering::Relaxed);
        Some(elapsed)
    }

    fn leave(&self, start: u64, end: u64, pending: bool, bytes: u64) {
        let elapsed = end.saturating_sub(start);
        self.sync_total_ns.fetch_add(elapsed, Ordering::Relaxed);
        self.sync_max_ns.fetch_max(elapsed, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
        if pending {
            self.pending_returns.fetch_add(1, Ordering::Relaxed);
            self.pending_since.store(end + 1, Ordering::Relaxed);
        }
    }

    fn append(&self, output: &mut String, name: &str, now: u64) {
        let pending_since = self.pending_since.load(Ordering::Relaxed);
        let open = if pending_since == 0 {
            0
        } else {
            now.saturating_sub(pending_since - 1)
        };
        let _ = write!(
            output,
            " {name}=[calls={},pending={},bytes={},sync_total_us={},sync_max_us={},pending_total_us={},pending_max_us={},open_pending_us={}]",
            self.calls.load(Ordering::Relaxed),
            self.pending_returns.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
            self.sync_total_ns.load(Ordering::Relaxed) / 1_000,
            self.sync_max_ns.load(Ordering::Relaxed) / 1_000,
            self.pending_total_ns.load(Ordering::Relaxed) / 1_000,
            self.pending_max_ns.load(Ordering::Relaxed) / 1_000,
            open / 1_000,
        );
    }
}

struct State {
    id: u64,
    download: String,
    started: Instant,
    span: Span,
    dispatch: Dispatch,
    active: AtomicU64,
    read: Boundary,
    write: Boundary,
    flush: Boundary,
    progress: Boundary,
    copy: Boundary,
    // Completed copy-poll gaps are grouped by the last-observed Pending states; open gaps are
    // reported separately. These states do not identify the cause of suspension. For example,
    // a Pending flush can remain unpolled while copy subsequently reads or writes more data.
    suspended_mask: AtomicU8,
    suspended_ns: [AtomicU64; 4],
    finished: AtomicBool,
    snapshots: AtomicU64,
    last_snapshot: AtomicU64,
    #[cfg(test)]
    test_clock: Option<Arc<AtomicU64>>,
}

impl State {
    fn new(download: String) -> Self {
        Self {
            id: NEXT_COPY_ID.fetch_add(1, Ordering::Relaxed),
            download,
            started: Instant::now(),
            span: Span::current(),
            dispatch: tracing::dispatcher::get_default(Clone::clone),
            active: AtomicU64::new(0),
            read: Boundary::default(),
            write: Boundary::default(),
            flush: Boundary::default(),
            progress: Boundary::default(),
            copy: Boundary::default(),
            suspended_mask: AtomicU8::new(0),
            suspended_ns: std::array::from_fn(|_| AtomicU64::new(0)),
            finished: AtomicBool::new(false),
            snapshots: AtomicU64::new(0),
            last_snapshot: AtomicU64::new(0),
            #[cfg(test)]
            test_clock: None,
        }
    }

    fn now(&self) -> u64 {
        #[cfg(test)]
        if let Some(clock) = &self.test_clock {
            return clock.load(Ordering::Relaxed);
        }
        u64::try_from(self.started.elapsed().as_nanos())
            .unwrap_or(u64::MAX)
            .min(u64::MAX - 1)
    }

    fn stage(&self, stage: Stage, now: u64) {
        // Pack the stage and its start time so live snapshots cannot mix their updates.
        self.active
            .store(((now / 1_000) << 3) | stage as u64, Ordering::Relaxed);
    }

    fn enter(&self, stage: Stage, boundary: &Boundary) -> u64 {
        let now = self.now();
        let _ = boundary.enter(now);
        self.stage(stage, now);
        now
    }

    fn leave(&self, boundary: &Boundary, start: u64, pending: bool, bytes: u64) {
        let now = self.now();
        boundary.leave(start, now, pending, bytes);
        self.stage(Stage::Copy, now);
    }

    fn pending_mask(&self) -> u8 {
        u8::from(self.read.pending_since.load(Ordering::Relaxed) != 0)
            | (u8::from(
                self.write.pending_since.load(Ordering::Relaxed) != 0
                    || self.flush.pending_since.load(Ordering::Relaxed) != 0,
            ) << 1)
    }

    fn snapshot_due(&self, now: u64) -> bool {
        if self.finished.load(Ordering::Relaxed)
            || self.snapshots.load(Ordering::Relaxed) >= MAX_SNAPSHOTS
            || now.saturating_sub(self.last_snapshot.load(Ordering::Relaxed)) < SNAPSHOT_INTERVAL_NS
        {
            return false;
        }
        self.last_snapshot.store(now, Ordering::Relaxed);
        self.snapshots.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn summary(&self) -> String {
        let now = self.now();
        let active = self.active.load(Ordering::Relaxed);
        let mut output = format!(
            "elapsed_us={} active={} active_us={} snapshot={} pending_intervals_may_overlap=true copy_sync_includes_boundaries=true suspended_last_observed_us=[neither={},reader={},writer={},both={}]",
            now / 1_000,
            Stage::name(active & 7),
            (now / 1_000).saturating_sub(active >> 3),
            self.snapshots.load(Ordering::Relaxed),
            self.suspended_ns[0].load(Ordering::Relaxed) / 1_000,
            self.suspended_ns[1].load(Ordering::Relaxed) / 1_000,
            self.suspended_ns[2].load(Ordering::Relaxed) / 1_000,
            self.suspended_ns[3].load(Ordering::Relaxed) / 1_000,
        );
        self.read.append(&mut output, "read", now);
        self.write.append(&mut output, "write", now);
        self.flush.append(&mut output, "flush", now);
        self.progress.append(&mut output, "progress", now);
        self.copy.append(&mut output, "copy", now);
        output
    }

    fn report(&self, outcome: &'static str) {
        tracing::dispatcher::with_default(&self.dispatch, || {
            self.span.in_scope(|| {
                trace!(
                    copy_id = self.id,
                    download = %self.download,
                    outcome,
                    measurements = %self.summary(),
                    "Python archive copy diagnostics"
                );
            });
        });
    }
}

fn start_watchdog() -> io::Result<Registry> {
    let registry: Registry = Arc::default();
    let watched = Arc::clone(&registry);
    std::thread::Builder::new()
        .name("uv-copy-diagnostics".to_owned())
        .spawn(move || {
            loop {
                std::thread::park_timeout(SNAPSHOT_INTERVAL);
                let states: Vec<_> = {
                    let mut registry = watched
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let states = registry.iter().filter_map(Weak::upgrade).collect();
                    registry.retain(|state| state.strong_count() != 0);
                    states
                };
                // Reporting can perform file I/O; do it after releasing the registry lock.
                for state in states {
                    if state.snapshot_due(state.now()) {
                        state.report("snapshot");
                    }
                }
            }
        })?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::task::Waker;

    use super::*;

    fn test_diagnostics() -> (CopyDiagnostics, Arc<AtomicU64>) {
        let clock = Arc::new(AtomicU64::new(0));
        let mut state = State::new("test-archive".to_owned());
        state.test_clock = Some(Arc::clone(&clock));
        (
            CopyDiagnostics {
                state: Arc::new(state),
            },
            clock,
        )
    }

    enum ReadStep {
        Pending,
        Data(&'static [u8]),
        Error,
    }

    struct Reader {
        steps: VecDeque<ReadStep>,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl AsyncRead for Reader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            assert!(cx.waker().will_wake(Waker::noop()));
            self.events.lock().unwrap().push("read");
            match self.steps.pop_front() {
                Some(ReadStep::Pending) => Poll::Pending,
                Some(ReadStep::Data(bytes)) => {
                    buf.put_slice(bytes);
                    Poll::Ready(Ok(()))
                }
                Some(ReadStep::Error) => Poll::Ready(Err(io::Error::other("reader error"))),
                None => Poll::Ready(Ok(())),
            }
        }
    }

    enum WriteStep {
        Pending,
        Limit(usize),
        Error,
    }

    #[derive(Default)]
    struct Writer {
        steps: VecDeque<WriteStep>,
        flush_pending: bool,
        bytes: Vec<u8>,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl AsyncWrite for Writer {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            assert!(cx.waker().will_wake(Waker::noop()));
            self.events.lock().unwrap().push("write");
            let bytes = match self.steps.pop_front() {
                Some(WriteStep::Pending) => return Poll::Pending,
                Some(WriteStep::Error) => {
                    return Poll::Ready(Err(io::Error::other("writer error")));
                }
                Some(WriteStep::Limit(limit)) => buf.len().min(limit),
                None => buf.len(),
            };
            self.bytes.extend_from_slice(&buf[..bytes]);
            Poll::Ready(Ok(bytes))
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            assert!(cx.waker().will_wake(Waker::noop()));
            self.events.lock().unwrap().push("flush");
            if std::mem::take(&mut self.flush_pending) {
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn reader_wait_and_error_are_reported() {
        let (diagnostics, clock) = test_diagnostics();
        let mut reader = diagnostics.reader(Reader {
            steps: [ReadStep::Pending, ReadStep::Error].into(),
            events: Arc::default(),
        });
        let mut writer = diagnostics.writer(Writer::default());
        let mut copy = std::pin::pin!(diagnostics.copy(&mut reader, &mut writer));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(copy.as_mut().poll(&mut cx).is_pending());
        clock.store(40_000_000, Ordering::Relaxed);
        assert!(matches!(copy.as_mut().poll(&mut cx), Poll::Ready(Err(_))));
        assert_eq!(
            diagnostics
                .state
                .read
                .pending_total_ns
                .load(Ordering::Relaxed),
            40_000_000
        );
        assert_eq!(
            diagnostics.state.suspended_ns[1].load(Ordering::Relaxed),
            40_000_000
        );
        assert!(diagnostics.state.finished.load(Ordering::Relaxed));
        assert_eq!(diagnostics.state.write.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn overlapping_reader_writer_and_flush_waits_preserve_poll_order() {
        let (diagnostics, clock) = test_diagnostics();
        let events = Arc::default();
        let mut reader = diagnostics.reader(Reader {
            steps: [ReadStep::Data(b"abc"), ReadStep::Pending, ReadStep::Pending].into(),
            events: Arc::clone(&events),
        });
        let mut writer = diagnostics.writer(Writer {
            steps: [WriteStep::Pending, WriteStep::Limit(1)].into(),
            flush_pending: true,
            events: Arc::clone(&events),
            ..Writer::default()
        });
        {
            let mut copy = std::pin::pin!(diagnostics.copy(&mut reader, &mut writer));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(copy.as_mut().poll(&mut cx).is_pending());
            clock.store(20_000_000, Ordering::Relaxed);
            assert!(copy.as_mut().poll(&mut cx).is_pending());
            clock.store(50_000_000, Ordering::Relaxed);
            assert!(matches!(copy.as_mut().poll(&mut cx), Poll::Ready(Ok(3))));
        }
        assert_eq!(writer.inner.bytes, b"abc");
        assert_eq!(
            *events.lock().unwrap(),
            [
                "read", "write", "read", "read", "write", "write", "read", "flush", "flush"
            ]
        );
        assert_eq!(
            diagnostics
                .state
                .read
                .pending_total_ns
                .load(Ordering::Relaxed),
            20_000_000
        );
        assert_eq!(
            diagnostics
                .state
                .write
                .pending_total_ns
                .load(Ordering::Relaxed),
            20_000_000
        );
        assert_eq!(
            diagnostics
                .state
                .flush
                .pending_total_ns
                .load(Ordering::Relaxed),
            30_000_000
        );
        assert_eq!(
            diagnostics.state.suspended_ns[3].load(Ordering::Relaxed),
            20_000_000
        );
        assert_eq!(
            diagnostics.state.suspended_ns[2].load(Ordering::Relaxed),
            30_000_000
        );
        assert_eq!(
            diagnostics
                .state
                .copy
                .pending_total_ns
                .load(Ordering::Relaxed),
            50_000_000
        );
    }

    #[test]
    fn writer_error_finishes_diagnostics() {
        let (diagnostics, _) = test_diagnostics();
        let mut reader = diagnostics.reader(Reader {
            steps: [ReadStep::Data(b"abc")].into(),
            events: Arc::default(),
        });
        let mut writer = diagnostics.writer(Writer {
            steps: [WriteStep::Error].into(),
            ..Writer::default()
        });
        let mut copy = std::pin::pin!(diagnostics.copy(&mut reader, &mut writer));
        assert!(matches!(
            copy.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Err(_))
        ));
        assert!(diagnostics.state.finished.load(Ordering::Relaxed));
        assert_eq!(diagnostics.state.write.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn progress_time_is_separate_from_raw_reader_time() {
        struct CallbackReader<'a, R> {
            reader: R,
            diagnostics: &'a CopyDiagnostics,
            clock: Arc<AtomicU64>,
        }

        impl<R: AsyncRead + Unpin> AsyncRead for CallbackReader<'_, R> {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                let this = self.get_mut();
                let result = Pin::new(&mut this.reader).poll_read(cx, buf);
                if matches!(result, Poll::Ready(Ok(()))) {
                    this.diagnostics.progress(|| {
                        this.clock.fetch_add(20_000_000, Ordering::Relaxed);
                        let summary = this.diagnostics.state.summary();
                        assert!(summary.contains("active=progress-callback active_us=20000"));
                    });
                }
                result
            }
        }

        let (diagnostics, clock) = test_diagnostics();
        let mut reader = CallbackReader {
            reader: diagnostics.reader(Reader {
                steps: [ReadStep::Data(b"abc")].into(),
                events: Arc::default(),
            }),
            diagnostics: &diagnostics,
            clock,
        };
        let mut writer = diagnostics.writer(Writer::default());
        let mut copy = std::pin::pin!(diagnostics.copy(&mut reader, &mut writer));
        assert!(matches!(
            copy.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(3))
        ));
        assert_eq!(
            diagnostics
                .state
                .progress
                .sync_total_ns
                .load(Ordering::Relaxed),
            40_000_000
        );
        assert_eq!(
            diagnostics
                .state
                .progress
                .sync_max_ns
                .load(Ordering::Relaxed),
            20_000_000
        );
        assert_eq!(
            diagnostics.state.read.sync_total_ns.load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            diagnostics.state.copy.sync_total_ns.load(Ordering::Relaxed),
            40_000_000
        );
    }

    #[test]
    fn snapshots_are_bounded_and_stop_after_completion() {
        let (diagnostics, _) = test_diagnostics();
        assert!(!diagnostics.state.snapshot_due(9_999_999_999));
        for snapshot in 1..=MAX_SNAPSHOTS {
            let now = snapshot * 10_000_000_000;
            assert!(diagnostics.state.snapshot_due(now));
            assert!(!diagnostics.state.snapshot_due(now));
        }
        assert!(!diagnostics.state.snapshot_due(130_000_000_000));
        let (diagnostics, _) = test_diagnostics();
        diagnostics.state.finished.store(true, Ordering::Relaxed);
        assert!(!diagnostics.state.snapshot_due(10_000_000_000));
    }
}
