//! Bounded, best-effort transfer of context-free manifest syntax from workers to the parent.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Result, bail, ensure};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use serde::de::{self, DeserializeSeed, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use socket2::{Domain, SockAddr, Socket, Type};

use uv_workspace::pyproject::PyProjectToml;

use super::{CACHE_SOCKET, Request, Response, verify_peer};

const MAX_PENDING_SOURCES: usize = 8192;
const MAX_PENDING_BYTES: usize = 8 * 1024 * 1024;
const MAX_BATCH_SOURCES: usize = 256;
const MAX_BATCH_BYTES: usize = 1024 * 1024;
const MAX_BATCH_FRAME: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_FRAME: usize = 4096;
const BATCH_TIMEOUT: Duration = Duration::from_millis(250);
const FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

static OBSERVATIONS: OnceLock<Observations> = OnceLock::new();

#[derive(Default)]
pub(super) struct Counters {
    pub(super) hits: u64,
    /// Known source observations omitted while queueing or warming the parent.
    pub(super) dropped: u64,
}

#[derive(Serialize, Deserialize)]
struct Batch {
    #[serde(deserialize_with = "deserialize_sources")]
    sources: Vec<String>,
    hits: u64,
    dropped: u64,
}

/// Reject an oversized sequence before allocating or reading its elements.
fn deserialize_sources<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct SourcesVisitor;

    impl<'de> Visitor<'de> for SourcesVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded sequence of manifest source strings")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let capacity = sequence.size_hint().unwrap_or(0);
            if capacity > MAX_BATCH_SOURCES {
                return Err(de::Error::custom(
                    "Too many manifest sources in daemon cache batch",
                ));
            }
            let mut sources = Vec::with_capacity(capacity);
            let mut bytes = 0;
            while let Some(source) = sequence.next_element_seed(SourceSeed {
                remaining_sources: MAX_BATCH_SOURCES - sources.len(),
                remaining_bytes: MAX_BATCH_BYTES - bytes,
            })? {
                bytes += source.len();
                sources.push(source);
            }
            Ok(sources)
        }
    }

    deserializer.deserialize_seq(SourcesVisitor)
}

struct SourceSeed {
    remaining_sources: usize,
    remaining_bytes: usize,
}

impl SourceSeed {
    fn check_length<E: de::Error>(&self, length: usize) -> Result<(), E> {
        if length > PyProjectToml::PROCESS_CACHE_MAX_ENTRY_BYTES {
            return Err(E::custom(
                "Manifest source exceeds daemon cache entry limit",
            ));
        }
        if length > self.remaining_bytes {
            return Err(E::custom(
                "Manifest sources exceed daemon cache batch limit",
            ));
        }
        Ok(())
    }
}

impl<'de> DeserializeSeed<'de> for SourceSeed {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        if self.remaining_sources == 0 {
            return Err(de::Error::custom(
                "Too many manifest sources in daemon cache batch",
            ));
        }
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for SourceSeed {
    type Value = String;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded manifest source string")
    }

    fn visit_str<E: de::Error>(self, source: &str) -> Result<Self::Value, E> {
        self.check_length(source.len())?;
        Ok(source.to_owned())
    }

    fn visit_string<E: de::Error>(self, source: String) -> Result<Self::Value, E> {
        self.check_length(source.len())?;
        Ok(source)
    }
}

impl Batch {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.sources.len() <= MAX_BATCH_SOURCES,
            "Too many manifest sources in daemon cache batch"
        );
        let mut bytes = 0;
        for source in &self.sources {
            ensure!(
                source.len() <= PyProjectToml::PROCESS_CACHE_MAX_ENTRY_BYTES,
                "Manifest source exceeds daemon cache entry limit"
            );
            bytes += source.len();
            ensure!(
                bytes <= MAX_BATCH_BYTES,
                "Manifest sources exceed daemon cache batch limit"
            );
        }
        Ok(())
    }
}

struct Pending {
    sources: VecDeque<String>,
    source_bytes: usize,
    hits: u64,
    dropped: u64,
    closed: bool,
    max_sources: usize,
    max_source_bytes: usize,
}

impl Default for Pending {
    fn default() -> Self {
        Self::new(MAX_PENDING_SOURCES, MAX_PENDING_BYTES)
    }
}

impl Pending {
    fn new(max_sources: usize, max_source_bytes: usize) -> Self {
        Self {
            sources: VecDeque::new(),
            source_bytes: 0,
            hits: 0,
            dropped: 0,
            closed: false,
            max_sources,
            max_source_bytes,
        }
    }

    fn observe(&mut self, source: &str, hit: bool) {
        if self.closed {
            return;
        }
        if hit {
            self.hits = self.hits.saturating_add(1);
        } else if source.len() > PyProjectToml::PROCESS_CACHE_MAX_ENTRY_BYTES
            || self.sources.len() >= self.max_sources
            || self.source_bytes.saturating_add(source.len()) > self.max_source_bytes
        {
            self.dropped = self.dropped.saturating_add(1);
        } else {
            self.source_bytes += source.len();
            self.sources.push_back(source.to_owned());
        }
    }

    fn take_batch(&mut self) -> Option<Batch> {
        if self.sources.is_empty() && self.hits == 0 && self.dropped == 0 {
            return None;
        }
        let mut sources = Vec::new();
        let mut bytes = 0;
        while let Some(source) = self.sources.front() {
            if sources.len() >= MAX_BATCH_SOURCES || bytes + source.len() > MAX_BATCH_BYTES {
                break;
            }
            bytes += source.len();
            // The front element cannot change while this queue is exclusively borrowed.
            if let Some(source) = self.sources.pop_front() {
                sources.push(source);
            }
        }
        self.source_bytes -= bytes;
        Some(Batch {
            sources,
            hits: std::mem::take(&mut self.hits),
            dropped: std::mem::take(&mut self.dropped),
        })
    }
}

#[derive(Default)]
struct Observations {
    pending: Mutex<Pending>,
    sending: Mutex<bool>,
    sender_idle: Condvar,
}

struct ActiveSender<'a>(&'a Observations);

impl Drop for ActiveSender<'_> {
    fn drop(&mut self) {
        if let Ok(mut sending) = self.0.sending.lock() {
            *sending = false;
            self.0.sender_idle.notify_all();
        }
    }
}

impl Observations {
    fn observe(&self, source: &str, hit: bool) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.observe(source, hit);
        }
    }

    fn flush(&self, close: bool, send: impl FnMut(&Batch, Instant) -> Result<()>) {
        self.flush_until(close, Instant::now() + FLUSH_TIMEOUT, send);
    }

    fn acquire_sender(&self, close: bool, deadline: Instant) -> Option<ActiveSender<'_>> {
        let mut sending = self.sending.lock().ok()?;
        if *sending && !close {
            return None;
        }
        while *sending {
            let (guard, timeout) = self
                .sender_idle
                .wait_timeout(sending, remaining(deadline).ok()?)
                .ok()?;
            sending = guard;
            if timeout.timed_out() && *sending {
                return None;
            }
        }
        remaining(deadline).ok()?;
        *sending = true;
        Some(ActiveSender(self))
    }

    fn flush_until(
        &self,
        close: bool,
        deadline: Instant,
        mut send: impl FnMut(&Batch, Instant) -> Result<()>,
    ) {
        // Closing and recording an observation share the same mutex. Detached runtime tasks
        // cannot add new observations after the final drain begins.
        if close && let Ok(mut pending) = self.pending.lock() {
            pending.closed = true;
        }
        // Non-final checkpoints skip an active sender. A final flush waits only until its own
        // deadline, so contention cannot add another sender's budget to command latency.
        let Some(_sender) = self.acquire_sender(close, deadline) else {
            return;
        };
        while Instant::now() < deadline {
            let batch = self
                .pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.take_batch());
            let Some(batch) = batch else {
                break;
            };
            // Never retry an ambiguously acknowledged batch: its hit counters may already
            // have been applied. Cache observations must not affect the command's outcome.
            if send(&batch, deadline).is_err() {
                break;
            }
        }
    }
}

pub(super) fn initialize_worker() {
    OBSERVATIONS.get_or_init(Observations::default);
    PyProjectToml::observe_process_cache(observe);
}

fn observe(source: &str, hit: bool) {
    if let Some(observations) = OBSERVATIONS.get() {
        observations.observe(source, hit);
    }
}

pub(super) fn flush(close: bool) {
    if let Some(observations) = OBSERVATIONS.get()
        && let Some(socket) = CACHE_SOCKET.get()
    {
        observations.flush(close, |batch, deadline| send_batch(socket, batch, deadline));
    }
}

/// Accept one batch from a peer already authenticated as an active worker.
pub(super) fn accept(stream: &mut UnixStream, counters: &mut Counters) -> Result<()> {
    let deadline = Instant::now() + BATCH_TIMEOUT;
    stream.set_nonblocking(true)?;
    let batch: Batch = read_frame_until(stream, MAX_BATCH_FRAME, deadline)?;
    batch.validate()?;
    let mut sources = batch.sources.into_iter();
    while let Some(source) = sources.next() {
        if Instant::now() >= deadline {
            counters.dropped = counters
                .dropped
                .saturating_add(1)
                .saturating_add(sources.len() as u64);
            break;
        }
        // Parsing syntax has no dependency on cwd, environment, configuration, or clocks.
        // A malformed source cannot make the daemon fail or alter another request's policy.
        if PyProjectToml::warm_process_cache(source).is_err() {
            counters.dropped = counters.dropped.saturating_add(1);
        }
    }
    counters.hits = counters.hits.saturating_add(batch.hits);
    counters.dropped = counters.dropped.saturating_add(batch.dropped);
    write_frame_until(stream, &Response::Observed, MAX_RESPONSE_FRAME, deadline)
}

fn send_batch(path: &Path, batch: &Batch, flush_deadline: Instant) -> Result<()> {
    let deadline = flush_deadline.min(Instant::now() + BATCH_TIMEOUT);
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    socket.connect_timeout(&SockAddr::unix(path)?, remaining(deadline)?)?;
    socket.set_nonblocking(true)?;
    let mut stream = UnixStream::from(socket);
    verify_peer(&stream)?;
    write_frame_until(
        &mut stream,
        &Request::Manifests,
        MAX_RESPONSE_FRAME,
        deadline,
    )?;
    write_frame_until(&mut stream, batch, MAX_BATCH_FRAME, deadline)?;
    match read_frame_until(&mut stream, MAX_RESPONSE_FRAME, deadline)? {
        Response::Observed => Ok(()),
        Response::Status(_)
        | Response::Accepted(_)
        | Response::Completed(_)
        | Response::RunLocally(_)
        | Response::Error(_) => bail!("Daemon did not acknowledge manifest cache batch"),
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::ErrorKind::TimedOut.into())
}

fn wait_ready(stream: &UnixStream, events: PollFlags, deadline: Instant) -> io::Result<()> {
    loop {
        let timeout = PollTimeout::try_from(remaining(deadline)?.as_millis().saturating_add(1))
            .map_err(io::Error::other)?;
        let mut descriptors = [PollFd::new(stream.as_fd(), events)];
        match poll(&mut descriptors, timeout) {
            Ok(0) => return Err(io::ErrorKind::TimedOut.into()),
            Ok(_) => {
                let observed = descriptors[0]
                    .revents()
                    .ok_or_else(|| io::Error::other("Unknown daemon cache socket event"))?;
                if observed.contains(PollFlags::POLLNVAL) {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
                if observed.contains(PollFlags::POLLERR)
                    && let Some(error) = stream.take_error()?
                {
                    return Err(error);
                }
                if observed.intersects(events | PollFlags::POLLHUP | PollFlags::POLLERR) {
                    return Ok(());
                }
            }
            Err(Errno::EINTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn write_all_until(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        remaining(deadline)?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_ready(stream, PollFlags::POLLOUT, deadline)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn read_exact_until(
    stream: &mut UnixStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        remaining(deadline)?;
        match stream.read(bytes) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => bytes = &mut bytes[read..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_ready(stream, PollFlags::POLLIN, deadline)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn write_frame_until<T: Serialize>(
    stream: &mut UnixStream,
    message: &T,
    limit: usize,
    deadline: Instant,
) -> Result<()> {
    let encoded = rmp_serde::to_vec(message)?;
    ensure!(
        encoded.len() <= limit,
        "Daemon cache message exceeds size limit"
    );
    write_all_until(
        stream,
        &u32::try_from(encoded.len())?.to_be_bytes(),
        deadline,
    )?;
    write_all_until(stream, &encoded, deadline)?;
    Ok(())
}

fn read_frame_until<T: serde::de::DeserializeOwned>(
    stream: &mut UnixStream,
    limit: usize,
    deadline: Instant,
) -> Result<T> {
    let mut length = [0; 4];
    read_exact_until(stream, &mut length, deadline)?;
    let length = u32::from_be_bytes(length) as usize;
    ensure!(length <= limit, "Daemon cache message exceeds size limit");
    let mut encoded = vec![0; length];
    read_exact_until(stream, &mut encoded, deadline)?;
    Ok(rmp_serde::from_slice(&encoded)?)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use anyhow::{Result, anyhow};
    use uv_workspace::pyproject::PyProjectToml;

    use super::{
        Batch, MAX_BATCH_BYTES, MAX_BATCH_FRAME, MAX_BATCH_SOURCES, Observations, Pending,
        read_frame_until,
    };

    fn decode_raw_batch(encoded: Vec<u8>) -> Result<Batch> {
        let (mut writer, mut reader) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        thread::scope(|scope| {
            let writing = scope.spawn(move || -> Result<()> {
                writer.write_all(&u32::try_from(encoded.len())?.to_be_bytes())?;
                writer.write_all(&encoded)?;
                Ok(())
            });
            let result = read_frame_until(
                &mut reader,
                MAX_BATCH_FRAME,
                Instant::now() + Duration::from_secs(5),
            );
            drop(reader);
            writing.join().expect("raw frame writer")?;
            result
        })
    }

    /// Encode the compact three-field batch shape without constructing owned source strings.
    fn raw_batch(source_lengths: &[usize]) -> Result<Vec<u8>> {
        let mut encoded = vec![0x93, 0xdd];
        encoded.extend(u32::try_from(source_lengths.len())?.to_be_bytes());
        for &length in source_lengths {
            encoded.push(0xdb);
            encoded.extend(u32::try_from(length)?.to_be_bytes());
            encoded.resize(encoded.len() + length, b'x');
        }
        encoded.extend([0, 0]);
        Ok(encoded)
    }

    #[test]
    fn source_count_is_rejected_before_decoding_elements() -> Result<()> {
        for count in [u32::try_from(MAX_BATCH_SOURCES + 1)?, u32::MAX] {
            // The sequence has no elements or trailing fields. Reject its declared count,
            // rather than allocating from the hint or reporting a truncated element.
            let mut encoded = vec![0x93, 0xdd];
            encoded.extend(count.to_be_bytes());
            let error = decode_raw_batch(encoded)
                .err()
                .expect("oversized source sequence");
            assert_eq!(
                error.to_string(),
                "Too many manifest sources in daemon cache batch"
            );
        }
        Ok(())
    }

    #[test]
    fn source_byte_limits_are_enforced_during_decoding() -> Result<()> {
        let entry_limit = PyProjectToml::PROCESS_CACHE_MAX_ENTRY_BYTES;
        let error = decode_raw_batch(raw_batch(&[entry_limit + 1])?)
            .err()
            .expect("oversized source string");
        assert_eq!(
            error.to_string(),
            "Manifest source exceeds daemon cache entry limit"
        );

        let error = decode_raw_batch(raw_batch(&[entry_limit; 5])?)
            .err()
            .expect("oversized source total");
        assert_eq!(
            error.to_string(),
            "Manifest sources exceed daemon cache batch limit"
        );
        Ok(())
    }

    #[test]
    fn bounded_source_batches_keep_the_wire_shape() -> Result<()> {
        let batch = Batch {
            sources: vec!["first".to_owned(), "second".to_owned()],
            hits: 7,
            dropped: 3,
        };
        let decoded = decode_raw_batch(rmp_serde::to_vec(&batch)?)?;
        assert_eq!(decoded.sources, batch.sources);
        assert_eq!(decoded.hits, batch.hits);
        assert_eq!(decoded.dropped, batch.dropped);

        let decoded = decode_raw_batch(raw_batch(
            &[PyProjectToml::PROCESS_CACHE_MAX_ENTRY_BYTES; 4],
        )?)?;
        decoded.validate()?;
        assert_eq!(
            decoded.sources.iter().map(String::len).sum::<usize>(),
            MAX_BATCH_BYTES
        );
        Ok(())
    }

    #[test]
    fn queue_enforces_source_and_byte_limits() {
        let mut pending = Pending::new(2, 8);
        pending.observe("aaaa", false);
        pending.observe("bb", false);
        pending.observe("c", false);
        pending.observe("aaaa", true);
        let batch = pending.take_batch().expect("queued batch");
        assert_eq!(batch.sources, ["aaaa", "bb"]);
        assert_eq!(batch.hits, 1);
        assert_eq!(batch.dropped, 1);
        assert_eq!(pending.source_bytes, 0);
        pending.observe("123456789", false);
        let batch = pending.take_batch().expect("reported overflow");
        assert!(batch.sources.is_empty());
        assert_eq!(batch.dropped, 1);
        assert!(pending.take_batch().is_none());
    }

    #[test]
    fn batches_enforce_count_and_source_limits() -> Result<()> {
        let mut pending = Pending::new(MAX_BATCH_SOURCES + 1, MAX_BATCH_BYTES * 2);
        for _ in 0..=MAX_BATCH_SOURCES {
            pending.observe("x", false);
        }
        let first = pending.take_batch().expect("first batch");
        first.validate()?;
        assert_eq!(first.sources.len(), MAX_BATCH_SOURCES);
        assert_eq!(pending.take_batch().expect("second batch").sources.len(), 1);

        let source = "x".repeat(PyProjectToml::PROCESS_CACHE_MAX_ENTRY_BYTES);
        for _ in 0..5 {
            pending.observe(&source, false);
        }
        let first = pending.take_batch().expect("byte-limited batch");
        first.validate()?;
        assert_eq!(
            first.sources.iter().map(String::len).sum::<usize>(),
            MAX_BATCH_BYTES
        );
        assert_eq!(
            pending
                .take_batch()
                .expect("remaining source")
                .sources
                .len(),
            1
        );
        Ok(())
    }

    #[test]
    fn failed_acknowledgment_is_not_retried() {
        let observations = Observations::default();
        observations.observe("first", false);
        observations.observe("first", true);
        let mut attempts = 0;
        observations.flush(false, |batch, _| {
            attempts += 1;
            assert_eq!(batch.hits, 1);
            Err(anyhow!("lost acknowledgment"))
        });
        observations.flush(true, |_, _| {
            attempts += 1;
            Ok(())
        });
        assert_eq!(attempts, 1);
    }

    #[test]
    fn final_flush_closes_before_waiting_for_an_active_sender() {
        let observations = Arc::new(Observations::default());
        observations.observe("first", false);
        let (entered, entered_receiver) = mpsc::channel();
        let (release, release_receiver) = mpsc::channel();
        thread::scope(|scope| {
            let first_observations = Arc::clone(&observations);
            let first = scope.spawn(move || {
                first_observations.flush(false, |_, _| {
                    entered.send(())?;
                    release_receiver.recv_timeout(Duration::from_secs(5))?;
                    Ok(())
                });
            });
            entered_receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("active sender");
            let final_flush = scope.spawn(|| observations.flush(true, |_, _| Ok(())));
            let deadline = Instant::now() + Duration::from_secs(5);
            while !observations.pending.lock().expect("pending queue").closed
                && Instant::now() < deadline
            {
                thread::yield_now();
            }
            let closed = observations.pending.lock().expect("pending queue").closed;
            observations.observe("late", false);
            observations.observe("late", true);
            let _ = release.send(());
            first.join().expect("interim flusher");
            final_flush.join().expect("final flusher");
            assert!(closed, "final flush did not close the queue");
        });
        assert!(
            observations
                .pending
                .lock()
                .expect("pending queue")
                .take_batch()
                .is_none()
        );
    }

    #[test]
    fn sender_contention_obeys_the_callers_deadline() {
        let observations = Arc::new(Observations::default());
        observations.observe("first", false);
        let (entered, entered_receiver) = mpsc::channel();
        let (release, release_receiver) = mpsc::channel();
        thread::scope(|scope| {
            let active_observations = Arc::clone(&observations);
            let active = scope.spawn(move || {
                let mut first_batch = true;
                active_observations.flush(false, |_, _| {
                    if first_batch {
                        first_batch = false;
                        entered.send(())?;
                        release_receiver.recv_timeout(Duration::from_secs(5))?;
                    }
                    Ok(())
                });
            });
            entered_receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("active sender");
            observations.observe("second", false);
            let started = Instant::now();
            let mut attempts = 0;
            observations.flush_until(false, started + Duration::from_millis(20), |_, _| {
                attempts += 1;
                Ok(())
            });
            observations.flush_until(true, started + Duration::from_millis(20), |_, _| {
                attempts += 1;
                Ok(())
            });
            let elapsed = started.elapsed();
            let _ = release.send(());
            active.join().expect("active sender");
            assert_eq!(attempts, 0);
            assert!(elapsed < Duration::from_secs(1));
            assert!(observations.pending.lock().expect("pending queue").closed);
        });
    }

    #[test]
    fn malformed_and_oversized_batches_are_rejected() {
        assert!(
            Batch {
                sources: vec![String::new(); MAX_BATCH_SOURCES + 1],
                hits: 0,
                dropped: 0,
            }
            .validate()
            .is_err()
        );
        assert!(
            Batch {
                sources: vec!["x".repeat(PyProjectToml::PROCESS_CACHE_MAX_ENTRY_BYTES + 1)],
                hits: 0,
                dropped: 0,
            }
            .validate()
            .is_err()
        );
        assert!(
            Batch {
                sources: vec!["x".repeat(PyProjectToml::PROCESS_CACHE_MAX_ENTRY_BYTES); 5],
                hits: 0,
                dropped: 0,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn frame_limit_is_checked_before_allocating_payload() -> Result<()> {
        let (mut writer, mut reader) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.write_all(&u32::try_from(MAX_BATCH_FRAME + 1)?.to_be_bytes())?;
        let result = read_frame_until::<Batch>(
            &mut reader,
            MAX_BATCH_FRAME,
            Instant::now() + Duration::from_secs(1),
        );
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn malformed_frame_is_rejected() -> Result<()> {
        let (mut writer, mut reader) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.write_all(&1_u32.to_be_bytes())?;
        writer.write_all(&[0xc1])?;
        assert!(
            read_frame_until::<Batch>(
                &mut reader,
                MAX_BATCH_FRAME,
                Instant::now() + Duration::from_secs(1),
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn incomplete_frame_stops_at_the_deadline() -> Result<()> {
        let (_writer, mut reader) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        let result = read_frame_until::<Batch>(
            &mut reader,
            MAX_BATCH_FRAME,
            Instant::now() + Duration::from_millis(1),
        );
        assert_eq!(
            result
                .err()
                .expect("incomplete frame")
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::TimedOut),
        );
        Ok(())
    }
}
