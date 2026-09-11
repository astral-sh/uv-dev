//! A bounded, benchmark-only Linux small-file reader.
//!
//! Every file is opened with `OPENAT`, read with `READ` until an EOF completion, and closed by its
//! owned descriptor. There is no ordinary-I/O fallback: an unavailable or retired ring cannot be
//! mistaken for an active backend in a benchmark result.

use std::cell::UnsafeCell;
use std::ffi::CString;
use std::io;
use std::mem;
use std::num::NonZeroU32;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use io_uring::{EnterFlags, IoUring, Probe, opcode, squeue, types};
use rustix::fs::{CWD, OFlags};
use rustix::io::Errno;

const READ_SIZE: usize = 8 * 1024;
const MAX_QUEUE_DEPTH: u32 = 256;
const MAX_STALLED_ATTEMPTS: usize = 64;

type ReadResult = io::Result<Option<Vec<u8>>>;
pub(super) type ReadResults = Vec<ReadResult>;
type ReadBuffer = Box<UnsafeCell<[u8; READ_SIZE]>>;

pub(super) struct Reader {
    ring: Option<IoUring>,
    capacity: usize,
    buffers: Vec<ReadBuffer>,
    pending: Option<PendingBatch>,
    #[cfg(test)]
    drain_observer: Option<std::sync::mpsc::SyncSender<()>>,
}

impl Reader {
    pub(super) fn new(queue_depth: NonZeroU32) -> io::Result<Self> {
        if queue_depth.get() > MAX_QUEUE_DEPTH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "io_uring read queue depth exceeds the bounded experiment",
            ));
        }
        let ring: IoUring = IoUring::builder().dontfork().build(queue_depth.get())?;
        let mut probe = Probe::new();
        ring.submitter().register_probe(&mut probe)?;
        if !probe.is_supported(opcode::OpenAt::CODE) || !probe.is_supported(opcode::Read::CODE) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "io_uring openat/read is unavailable",
            ));
        }
        let capacity = usize::try_from(queue_depth.get().min(ring.params().sq_entries()))
            .map_err(|_| invalid_completion())?;
        if capacity == 0 {
            return Err(invalid_completion());
        }
        Ok(Self {
            ring: Some(ring),
            capacity,
            buffers: (0..capacity).map(|_| new_buffer()).collect(),
            pending: None,
            #[cfg(test)]
            drain_observer: None,
        })
    }

    pub(super) fn read(&mut self, paths: &[PathBuf]) -> io::Result<ReadResults> {
        self.read_with(paths, &mut IoUring::submit_and_wait)
    }

    fn read_with(
        &mut self,
        paths: &[PathBuf],
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> io::Result<ReadResults> {
        if self.pending.is_some() {
            self.retire();
            return Err(invalid_completion());
        }
        if self.ring.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "io_uring small-file reader was retired",
            ));
        }

        let mut results = Vec::with_capacity(paths.len());
        for paths in paths.chunks(self.capacity) {
            let unused = self
                .buffers
                .len()
                .checked_sub(paths.len())
                .ok_or_else(invalid_completion)?;
            let buffers = self.buffers.split_off(unused);
            self.pending = Some(PendingBatch::new(paths, buffers)?);
            if let Err(error) = self.complete_batch(submit) {
                self.retire();
                return Err(error);
            }
            let mut batch = self.pending.take().ok_or_else(invalid_completion)?;
            results.extend(batch.take_results()?);
            batch.return_buffers(&mut self.buffers)?;
        }
        Ok(results)
    }

    fn complete_batch(
        &mut self,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> io::Result<()> {
        loop {
            let batch = self.pending.as_ref().ok_or_else(invalid_completion)?;
            if batch
                .requests
                .iter()
                .all(|request| request.result.is_some())
            {
                return Ok(());
            }
            self.queue()?;
            self.finish_queued(submit)?;
        }
    }

    #[expect(unsafe_code)]
    fn queue(&mut self) -> io::Result<()> {
        let (Some(ring), Some(batch)) = (&mut self.ring, &mut self.pending) else {
            return Err(invalid_completion());
        };
        if batch.queued != batch.completed {
            return Err(invalid_completion());
        }
        let mut submissions = ring.submission();
        if !submissions.is_empty() {
            return Err(invalid_completion());
        }
        for (index, request) in batch.requests.iter_mut().enumerate() {
            if request.result.is_some() {
                continue;
            }
            if request.in_flight.is_some() {
                return Err(invalid_completion());
            }
            let user_data = u64::try_from(batch.queued)
                .ok()
                .and_then(|sequence| sequence.checked_mul(u64::from(MAX_QUEUE_DEPTH)))
                .and_then(|sequence| sequence.checked_add(u64::try_from(index).ok()?))
                .ok_or_else(invalid_completion)?;
            let (operation, entry) = request.entry(user_data)?;
            // SAFETY: The pending batch owns every pathname, buffer, and file descriptor. Its
            // boxed buffers have stable addresses, and none of this storage is reclaimed until
            // all submitted completions are accounted for, including on a control failure.
            unsafe { submissions.push(&entry) }.map_err(|_| invalid_completion())?;
            request.in_flight = Some(Queued {
                operation,
                user_data,
            });
            batch.queued += 1;
        }
        Ok(())
    }

    fn finish_queued(
        &mut self,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> io::Result<()> {
        let mut progress = Progress::default();
        loop {
            self.reap()?;
            let (Some(ring), Some(batch)) = (&mut self.ring, &self.pending) else {
                return Err(invalid_completion());
            };
            if batch.completed == batch.queued {
                return Ok(());
            }
            progress.observe(batch.completed, ring.submission().len())?;
            match submit(ring, 1) {
                Ok(_) => {}
                Err(error) if retryable_control_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn reap(&mut self) -> io::Result<()> {
        let (Some(ring), Some(batch)) = (&mut self.ring, &mut self.pending) else {
            return Err(invalid_completion());
        };
        for completion in ring.completion() {
            let index = usize::try_from(completion.user_data() % u64::from(MAX_QUEUE_DEPTH))
                .map_err(|_| invalid_completion())?;
            let Some(request) = batch.requests.get_mut(index) else {
                batch.valid_completions = false;
                return Err(invalid_completion());
            };
            let Some(queued) = request.in_flight else {
                batch.valid_completions = false;
                return Err(invalid_completion());
            };
            if queued.user_data != completion.user_data() {
                batch.valid_completions = false;
                return Err(invalid_completion());
            }
            request.in_flight = None;
            batch.completed += 1;
            if let Err(error) = request.complete(queued.operation, completion.result()) {
                batch.valid_completions = false;
                return Err(error);
            }
        }
        Ok(())
    }

    fn retire(&mut self) {
        let drained = self.drain_submitted();
        self.retire_after_drain(drained);
    }

    fn retire_after_drain(&mut self, drained: io::Result<()>) {
        if drained.is_err()
            && let Some(mut batch) = self.pending.take()
        {
            // Ring shutdown can cancel asynchronously on supported kernels. If the submitted
            // CQEs cannot be observed, retain this single bounded batch until process exit so a
            // late open/read cannot access freed storage or a recycled descriptor. The reader
            // stays retired, so another call cannot accumulate more retained batches.
            batch.discard_results();
            mem::forget(batch);
        }
        // Without SQPOLL, closing the ring also discards entries never submitted to the kernel.
        // Submitted operations have either completed or kept all of their storage alive above.
        drop(self.ring.take());
        drop(self.pending.take());
    }

    #[expect(unsafe_code)]
    fn drain_submitted(&mut self) -> io::Result<()> {
        let mut progress = Progress::default();
        loop {
            self.reap()?;
            let (Some(ring), Some(batch)) = (&mut self.ring, &self.pending) else {
                return Err(invalid_completion());
            };
            if !batch.valid_completions {
                return Err(invalid_completion());
            }
            let unsubmitted = ring.submission().len();
            match batch.queued.checked_sub(unsubmitted) {
                Some(submitted) if batch.completed == submitted => return Ok(()),
                Some(submitted) if batch.completed < submitted => {}
                _ => return Err(invalid_completion()),
            }
            progress.observe(batch.completed, unsubmitted)?;
            #[cfg(test)]
            if let Some(observer) = self.drain_observer.take() {
                let _ = observer.send(());
            }
            // SAFETY: GETEVENTS is called without a signal mask or extended arguments. Passing
            // zero submissions leaves unsubmitted SQEs untouched while the batch continues to
            // own every kernel-accessed pathname, buffer, and descriptor.
            match unsafe {
                ring.submitter()
                    .enter::<()>(0, 1, EnterFlags::GETEVENTS.bits(), None)
            } {
                Ok(_) => {}
                Err(error) if retryable_control_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if self.pending.is_some() {
            self.retire();
        }
    }
}

struct PendingBatch {
    requests: Box<[Request]>,
    queued: usize,
    completed: usize,
    valid_completions: bool,
    #[cfg(test)]
    drop_counter: Option<Arc<AtomicUsize>>,
}

impl PendingBatch {
    fn new(paths: &[PathBuf], buffers: Vec<ReadBuffer>) -> io::Result<Self> {
        if paths.len() != buffers.len() {
            return Err(invalid_completion());
        }
        Ok(Self {
            requests: paths
                .iter()
                .zip(buffers)
                .map(|(path, buffer)| Request::new(path, buffer))
                .collect(),
            queued: 0,
            completed: 0,
            valid_completions: true,
            #[cfg(test)]
            drop_counter: None,
        })
    }

    fn take_results(&mut self) -> io::Result<ReadResults> {
        self.requests
            .iter_mut()
            .map(|request| request.result.take().ok_or_else(invalid_completion))
            .collect()
    }

    fn return_buffers(&mut self, buffers: &mut Vec<ReadBuffer>) -> io::Result<()> {
        for request in &mut self.requests {
            buffers.push(request.buffer.take().ok_or_else(invalid_completion)?);
        }
        Ok(())
    }

    fn discard_results(&mut self) {
        for request in &mut self.requests {
            // Accumulated and completed file contents are never kernel-accessed. Only the fixed
            // read buffer, pathname, and descriptor must outlive an unobserved completion.
            drop(mem::take(&mut request.contents));
            drop(request.result.take());
        }
    }
}

#[cfg(test)]
impl Drop for PendingBatch {
    fn drop(&mut self) {
        if let Some(counter) = &self.drop_counter {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Open,
    Read,
}

#[derive(Clone, Copy)]
struct Queued {
    operation: Operation,
    user_data: u64,
}

struct Request {
    path: Option<CString>,
    file: Option<OwnedFd>,
    buffer: Option<ReadBuffer>,
    contents: Vec<u8>,
    in_flight: Option<Queued>,
    result: Option<ReadResult>,
}

impl Request {
    fn new(path: &Path, buffer: ReadBuffer) -> Self {
        let (path, result) = match CString::new(path.as_os_str().as_bytes()) {
            Ok(path) => (Some(path), None),
            Err(error) => (
                None,
                Some(Err(io::Error::new(io::ErrorKind::InvalidInput, error))),
            ),
        };
        Self {
            path,
            file: None,
            buffer: Some(buffer),
            contents: Vec::new(),
            in_flight: None,
            result,
        }
    }

    fn entry(&self, user_data: u64) -> io::Result<(Operation, squeue::Entry)> {
        let (operation, entry) = if let Some(file) = &self.file {
            let buffer = self.buffer.as_ref().ok_or_else(invalid_completion)?;
            (
                Operation::Read,
                opcode::Read::new(
                    types::Fd(file.as_raw_fd()),
                    buffer.get().cast::<u8>(),
                    u32::try_from(READ_SIZE).map_err(|_| invalid_completion())?,
                )
                // Each descriptor has at most one in-flight read. Advancing its current offset
                // matches ordinary `read`, including descriptors that do not support seeking.
                .offset(u64::MAX)
                .build(),
            )
        } else {
            let path = self.path.as_ref().ok_or_else(invalid_completion)?;
            (
                Operation::Open,
                opcode::OpenAt::new(types::Fd(CWD.as_raw_fd()), path.as_ptr())
                    .flags((OFlags::RDONLY | OFlags::CLOEXEC).bits().cast_signed())
                    .build(),
            )
        };
        Ok((operation, entry.user_data(user_data)))
    }

    fn complete(&mut self, operation: Operation, result: i32) -> io::Result<()> {
        match (operation, self.file.is_some()) {
            (Operation::Open, false) | (Operation::Read, true) => {}
            (Operation::Open, true) | (Operation::Read, false) => {
                return Err(invalid_completion());
            }
        }
        if result < 0 {
            let error =
                io::Error::from_raw_os_error(result.checked_neg().ok_or_else(invalid_completion)?);
            if error.kind() != io::ErrorKind::Interrupted {
                let result = match operation {
                    Operation::Open if error.kind() == io::ErrorKind::NotFound => Ok(None),
                    Operation::Open | Operation::Read => Err(error),
                };
                self.finish(result);
            }
            return Ok(());
        }
        match operation {
            Operation::Open => self.complete_open(result),
            Operation::Read => self.complete_read(result),
        }
    }

    #[expect(unsafe_code)]
    fn complete_open(&mut self, descriptor: i32) -> io::Result<()> {
        if descriptor < 0 || self.file.is_some() {
            return Err(invalid_completion());
        }
        // SAFETY: A successful OPENAT CQE returns a newly owned process descriptor. The matching
        // completion is consumed exactly once before the descriptor is transferred into OwnedFd.
        self.file = Some(unsafe { OwnedFd::from_raw_fd(descriptor) });
        Ok(())
    }

    #[expect(unsafe_code)]
    fn complete_read(&mut self, result: i32) -> io::Result<()> {
        let length = usize::try_from(result).map_err(|_| invalid_completion())?;
        if length > READ_SIZE {
            return Err(invalid_completion());
        }
        if length == 0 {
            let contents = mem::take(&mut self.contents);
            self.finish(Ok(Some(contents)));
        } else {
            let buffer = self.buffer.as_ref().ok_or_else(invalid_completion)?;
            // SAFETY: The matching READ CQE has completed this buffer's only in-flight request.
            // The successful result initializes exactly `length` bytes, and no new read is queued
            // until this completion has been consumed. A short positive read is not EOF.
            let bytes = unsafe { &(*buffer.get())[..length] };
            self.contents.extend_from_slice(bytes);
        }
        Ok(())
    }

    fn finish(&mut self, result: ReadResult) {
        self.result = Some(result);
        drop(self.file.take());
    }
}

fn new_buffer() -> ReadBuffer {
    Box::new(UnsafeCell::new([0; READ_SIZE]))
}

fn retryable_control_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Interrupted
        || error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == Some(Errno::BUSY.raw_os_error())
}

#[derive(Default)]
struct Progress {
    previous: Option<(usize, usize)>,
    stalled: usize,
}

impl Progress {
    fn observe(&mut self, completed: usize, unsubmitted: usize) -> io::Result<()> {
        if let Some((previous_completed, previous_unsubmitted)) = self.previous
            && (completed < previous_completed || unsubmitted > previous_unsubmitted)
        {
            return Err(invalid_completion());
        }
        let current = Some((completed, unsubmitted));
        if self.previous == current {
            self.stalled += 1;
            if self.stalled >= MAX_STALLED_ATTEMPTS {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "io_uring small-file requests made no progress",
                ));
            }
        } else {
            self.previous = current;
            self.stalled = 0;
        }
        Ok(())
    }
}

fn invalid_completion() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid io_uring small-file completion",
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::env;
    use std::ffi::OsStr;
    use std::io::{self, Read, Write};
    use std::mem;
    use std::num::NonZeroU32;
    use std::os::fd::{AsRawFd, IntoRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::from_utf8;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use rustix::fs::{Dir, Mode, OFlags, open};
    use rustix::io::Errno;

    use super::{
        MAX_STALLED_ATTEMPTS, Operation, PendingBatch, Progress, READ_SIZE, ReadResult, Reader,
        Request, new_buffer, retryable_control_error,
    };

    fn ordinary(path: &Path) -> ReadResult {
        let mut file = match fs_err::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        Ok(Some(contents))
    }

    fn comparable(result: &ReadResult) -> Result<Option<&[u8]>, (io::ErrorKind, Option<i32>)> {
        match result {
            Ok(contents) => Ok(contents.as_deref()),
            Err(error) => Err((error.kind(), error.raw_os_error())),
        }
    }

    fn assert_results(paths: &[PathBuf], actual: &[ReadResult]) {
        assert_eq!(actual.len(), paths.len());
        for (path, actual) in paths.iter().zip(actual) {
            assert_eq!(comparable(actual), comparable(&ordinary(path)));
        }
    }

    fn open_descriptors() -> io::Result<BTreeSet<i32>> {
        let directory = Dir::new(open(
            "/proc/self/fd",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?)?;
        let observer = directory.fd()?.as_raw_fd();
        let mut descriptors = BTreeSet::new();
        for entry in directory {
            let entry = entry?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            let descriptor = from_utf8(name)
                .map_err(io::Error::other)?
                .parse::<i32>()
                .map_err(io::Error::other)?;
            if descriptor != observer {
                descriptors.insert(descriptor);
            }
        }
        Ok(descriptors)
    }

    #[test]
    fn short_reads_continue_until_eof() -> io::Result<()> {
        let mut request = Request::new(Path::new("file"), new_buffer());
        request.complete(Operation::Open, tempfile::tempfile()?.into_raw_fd())?;
        request
            .buffer
            .as_mut()
            .expect("owned read buffer")
            .get_mut()[..3]
            .copy_from_slice(b"abc");
        request.complete(Operation::Read, 3)?;
        assert!(request.result.is_none());
        assert!(request.file.is_some());
        request.complete(Operation::Read, -Errno::INTR.raw_os_error())?;
        assert!(request.result.is_none());

        request
            .buffer
            .as_mut()
            .expect("owned read buffer")
            .get_mut()[..2]
            .copy_from_slice(b"de");
        request.complete(Operation::Read, 2)?;
        assert!(request.result.is_none());
        request.complete(Operation::Read, 0)?;
        assert_eq!(
            request.result.take().expect("completed read")?,
            Some(b"abcde".to_vec())
        );
        assert!(request.file.is_none());
        Ok(())
    }

    #[test]
    fn only_open_not_found_means_missing() -> io::Result<()> {
        let mut missing = Request::new(Path::new("missing"), new_buffer());
        missing.complete(Operation::Open, -Errno::NOENT.raw_os_error())?;
        assert_eq!(missing.result.take().expect("completed open")?, None);

        let mut unreadable = Request::new(Path::new("unreadable"), new_buffer());
        unreadable.complete(Operation::Open, -Errno::ACCESS.raw_os_error())?;
        assert_eq!(
            unreadable
                .result
                .take()
                .expect("completed open")
                .expect_err("permission errors are not missing files")
                .raw_os_error(),
            Some(Errno::ACCESS.raw_os_error())
        );

        let mut read_failure = Request::new(Path::new("read-failure"), new_buffer());
        read_failure.complete(Operation::Open, tempfile::tempfile()?.into_raw_fd())?;
        read_failure.complete(Operation::Read, -Errno::NOENT.raw_os_error())?;
        assert_eq!(
            read_failure
                .result
                .take()
                .expect("completed read")
                .expect_err("read errors are not missing optional files")
                .raw_os_error(),
            Some(Errno::NOENT.raw_os_error())
        );
        assert!(read_failure.file.is_none());
        Ok(())
    }

    #[test]
    fn invalid_path_has_the_ordinary_error_kind() {
        let path = Path::new(OsStr::from_bytes(b"invalid\0path"));
        let mut request = Request::new(path, new_buffer());
        assert_eq!(
            comparable(&request.result.take().expect("invalid path result")),
            comparable(&ordinary(path))
        );
    }

    #[test]
    fn retirement_discards_only_non_kernel_data() -> io::Result<()> {
        let paths = vec![PathBuf::from("file")];
        let mut batch = PendingBatch::new(&paths, vec![new_buffer()])?;
        let request = &mut batch.requests[0];
        request.file = Some(tempfile::tempfile()?.into());
        request.contents = vec![1; READ_SIZE * 2];
        request.result = Some(Ok(Some(vec![2; READ_SIZE * 2])));
        let path = request.path.as_ref().expect("owned pathname").as_ptr();
        let buffer = request.buffer.as_ref().expect("owned buffer").get();
        let descriptor = request.file.as_ref().map(AsRawFd::as_raw_fd);

        batch.discard_results();
        let request = &batch.requests[0];
        assert_eq!(request.contents.capacity(), 0);
        assert!(request.result.is_none());
        assert_eq!(
            request.path.as_ref().expect("owned pathname").as_ptr(),
            path
        );
        assert_eq!(request.buffer.as_ref().expect("owned buffer").get(), buffer);
        assert_eq!(request.file.as_ref().map(AsRawFd::as_raw_fd), descriptor);
        Ok(())
    }

    #[test]
    fn control_retries_require_progress() -> io::Result<()> {
        let mut progress = Progress::default();
        for _ in 0..MAX_STALLED_ATTEMPTS {
            progress.observe(0, 2)?;
        }
        progress.observe(0, 1)?;
        for _ in 1..MAX_STALLED_ATTEMPTS {
            progress.observe(0, 1)?;
        }
        progress.observe(1, 1)?;
        for _ in 1..MAX_STALLED_ATTEMPTS {
            progress.observe(1, 1)?;
        }
        assert_eq!(
            progress
                .observe(1, 1)
                .expect_err("unchanged control retries must be bounded")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_reader_reuses_buffers_across_batches_and_eof_reads() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let mut paths = Vec::new();
        for length in [
            0,
            1,
            READ_SIZE - 1,
            READ_SIZE,
            READ_SIZE + 1,
            READ_SIZE * 3 + 17,
        ] {
            let path = root.path().join(format!("length-{length}"));
            let contents = (0..length)
                .map(|index| u8::try_from(index % 251).expect("small byte value"))
                .collect::<Vec<_>>();
            fs_err::write(&path, &contents)?;
            paths.push(path);
        }
        let non_utf8 = root.path().join(OsStr::from_bytes(b"file-\xff"));
        fs_err::write(&non_utf8, b"non-UTF-8 filename")?;
        paths.push(non_utf8);
        let symlink = root.path().join("symlink");
        fs_err::os::unix::fs::symlink(&paths[1], &symlink)?;
        paths.push(symlink);
        let dangling = root.path().join("dangling");
        fs_err::os::unix::fs::symlink(root.path().join("absent"), &dangling)?;
        paths.push(dangling);
        paths.push(root.path().join("missing"));
        paths.push(root.path().to_path_buf());

        let mut reader = Reader::new(NonZeroU32::new(4).expect("non-zero queue depth"))?;
        let descriptor = reader.ring.as_ref().map(AsRawFd::as_raw_fd);
        for _ in 0..2 {
            assert_results(&paths, &reader.read(&paths)?);
            assert!(reader.pending.is_none());
            assert_eq!(reader.buffers.len(), reader.capacity);
            assert_eq!(reader.ring.as_ref().map(AsRawFd::as_raw_fd), descriptor);
        }
        assert!(reader.read(&[])?.is_empty());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host with procfs"]
    fn active_reader_keeps_descriptor_count_stable() -> io::Result<()> {
        const CHILD_ENV: &str = "UV_BENCH_SMALL_READ_FD_CHILD";
        let test_name = concat!(
            module_path!(),
            "::active_reader_keeps_descriptor_count_stable"
        )
        .split_once("::")
        .expect("test module path includes its crate")
        .1;
        if env::var(CHILD_ENV).as_deref() != Ok(test_name) {
            // The unavailable-drain test deliberately retains descriptors. Run this one test in
            // a fresh process so neither that test nor parallel libtest activity can affect the
            // descriptor snapshots. The child marker prevents recursive re-execution, while the
            // exact filter keeps all other tests out of the child process.
            let output = Command::new(env::current_exe()?)
                .args([
                    "--exact",
                    test_name,
                    "--ignored",
                    "--test-threads=1",
                    "--format=pretty",
                    "--color=never",
                ])
                .env(CHILD_ENV, test_name)
                .output()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "isolated descriptor test failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                stdout
                    .lines()
                    .filter(|line| *line == "running 1 test")
                    .count(),
                1,
                "isolated process did not execute the exact descriptor test: {stdout}"
            );
            return Ok(());
        }

        let root = tempfile::tempdir()?;
        let paths = (0..37)
            .map(|index| {
                let path = root.path().join(format!("file-{index}"));
                fs_err::write(&path, vec![b'x'; index * 257])?;
                Ok(path)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let error_paths = vec![
            root.path().to_path_buf(),
            root.path().join("missing"),
            paths[0].join("not-a-directory"),
        ];
        assert_eq!(
            ordinary(&error_paths[0])
                .expect_err("reading a directory must fail")
                .raw_os_error(),
            Some(Errno::ISDIR.raw_os_error())
        );
        let without_ring = open_descriptors()?;
        for queue_depth in [1, 4, 16] {
            let mut reader =
                Reader::new(NonZeroU32::new(queue_depth).expect("non-zero queue depth"))?;
            assert_results(&paths, &reader.read(&paths)?);
            assert_results(&error_paths, &reader.read(&error_paths)?);
            let with_ring = open_descriptors()?;
            assert_eq!(with_ring.len(), without_ring.len() + 1);
            for _ in 0..64 {
                assert_results(&paths, &reader.read(&paths)?);
                assert_results(&error_paths, &reader.read(&error_paths)?);
                assert!(reader.pending.is_none());
                assert_eq!(reader.buffers.len(), reader.capacity);
                assert_eq!(open_descriptors()?, with_ring);
            }
            drop(reader);
            assert_eq!(open_descriptors()?, without_ring);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_reader_retries_transient_control_errors() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let paths = vec![root.path().join("file")];
        fs_err::write(&paths[0], b"contents")?;
        let mut reader = Reader::new(NonZeroU32::MIN)?;
        let mut failures = [Errno::INTR, Errno::AGAIN, Errno::BUSY].into_iter();
        let actual = reader.read_with(&paths, &mut |ring, want| {
            if let Some(error) = failures.next() {
                return Err(error.into());
            }
            ring.submit_and_wait(want)
        })?;
        assert_results(&paths, &actual);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_some());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_reader_retires_after_stalled_control_errors() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let paths = vec![root.path().join("file")];
        fs_err::write(&paths[0], b"contents")?;
        let mut reader = Reader::new(NonZeroU32::MIN)?;
        let attempts = Cell::new(0);
        let error = reader
            .read_with(&paths, &mut |_, _| {
                attempts.set(attempts.get() + 1);
                Err(Errno::AGAIN.into())
            })
            .expect_err("a stalled reader must not report a successful read");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(attempts.get(), MAX_STALLED_ATTEMPTS);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_none());
        assert_eq!(
            reader
                .read(&paths)
                .expect_err("a retired reader must not create another ring")
                .kind(),
            io::ErrorKind::Unsupported
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_reader_reclaims_partially_submitted_opens_after_drain() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let paths = (0..4)
            .map(|index| {
                let path = root.path().join(format!("file-{index}"));
                fs_err::write(&path, b"contents")?;
                Ok(path)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut reader = Reader::new(NonZeroU32::new(4).expect("non-zero queue depth"))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(&paths, mem::take(&mut reader.buffers))?;
        batch.drop_counter = Some(Arc::clone(&counter));
        reader.pending = Some(batch);
        let submitted = Cell::new(0);
        let error = reader
            .complete_batch(&mut |ring, _| {
                assert_eq!(counter.load(Ordering::SeqCst), 0);
                let count = loop {
                    // SAFETY: Submit only one valid OPENAT entry without waiting or passing a
                    // signal mask. The reader continues to own all pending request storage.
                    match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                        Ok(count) => break count,
                        Err(error) if retryable_control_error(&error) => {}
                        Err(error) => return Err(error),
                    }
                };
                submitted.set(count);
                Err(Errno::PERM.into())
            })
            .expect_err("injected control error must fail the batch");
        assert_eq!(error.raw_os_error(), Some(Errno::PERM.raw_os_error()));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        reader.retire();
        assert_eq!(submitted.get(), 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_reader_keeps_in_flight_reads_alive_during_retirement() -> io::Result<()> {
        let paths = vec![PathBuf::from("blocked"), PathBuf::from("unsubmitted")];
        let mut reader = Reader::new(NonZeroU32::new(2).expect("non-zero queue depth"))?;
        let (socket, mut writer) = UnixStream::pair()?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(&paths, mem::take(&mut reader.buffers))?;
        batch.requests[0].file = Some(socket.into());
        batch.requests[1].file = Some(tempfile::tempfile()?.into());
        batch.drop_counter = Some(Arc::clone(&counter));
        reader.pending = Some(batch);
        reader.queue()?;

        let (observer, observed) = mpsc::sync_channel(0);
        reader.drain_observer = Some(observer);
        let submitted = Cell::new(0);
        std::thread::scope(|scope| -> io::Result<()> {
            let counter = Arc::clone(&counter);
            let release = scope.spawn(move || -> io::Result<()> {
                let observation = observed.recv_timeout(Duration::from_secs(10));
                let requests_alive = counter.load(Ordering::SeqCst) == 0;
                // Always release the read, even if observing the drain failed, so cleanup cannot
                // leave the test process waiting forever on an unreadable socket.
                let released = writer.write_all(b"x");
                observation.map_err(io::Error::other)?;
                assert!(
                    requests_alive,
                    "request storage was reclaimed before completion"
                );
                released
            });
            let error = reader
                .finish_queued(&mut |ring, _| {
                    let count = loop {
                        // SAFETY: Only the blocked READ is submitted. Its owned socket and fixed
                        // buffer remain in the pending batch until the drain observes its CQE.
                        match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                            Ok(count) => break count,
                            Err(error) if retryable_control_error(&error) => {}
                            Err(error) => return Err(error),
                        }
                    };
                    submitted.set(count);
                    Err(Errno::PERM.into())
                })
                .expect_err("injected control error must fail the read");
            assert_eq!(error.raw_os_error(), Some(Errno::PERM.raw_os_error()));
            reader.retire();
            release.join().expect("read-release thread failed")?;
            Ok(())
        })?;
        assert_eq!(submitted.get(), 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_reader_retains_requests_if_drain_becomes_unavailable() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let paths = vec![root.path().join("file")];
        fs_err::write(&paths[0], b"contents")?;
        let mut reader = Reader::new(NonZeroU32::MIN)?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(&paths, mem::take(&mut reader.buffers))?;
        batch.drop_counter = Some(Arc::clone(&counter));
        reader.pending = Some(batch);
        reader.queue()?;
        let ring = reader.ring.as_ref().expect("initialized ring");
        let submitted = loop {
            // SAFETY: Submit the valid OPENAT without consuming its CQE. All request storage is
            // still owned, and this test deliberately exercises the unavailable-drain branch.
            match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                Ok(count) => break count,
                Err(error) if retryable_control_error(&error) => {}
                Err(error) => return Err(error),
            }
        };
        assert_eq!(submitted, 1);

        // One bounded batch is deliberately retained until this test process exits.
        reader.retire_after_drain(Err(Errno::PERM.into()));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_none());
        assert_eq!(
            reader
                .read(&paths)
                .expect_err("a retired reader must not retain another batch")
                .kind(),
            io::ErrorKind::Unsupported
        );
        Ok(())
    }
}
