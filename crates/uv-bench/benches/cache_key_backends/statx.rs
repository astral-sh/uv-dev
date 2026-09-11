//! Benchmark-only `STATX` collection. No production cache-key backend is selected here.

use std::cell::{Cell, UnsafeCell};
use std::ffi::CString;
use std::io;
use std::mem::{self, MaybeUninit, align_of, offset_of, size_of};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use globwalk::GlobWalker;
use io_uring::{EnterFlags, IoUring, Probe, opcode, squeue, types};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, Statx, StatxFlags, StatxTimestamp, open};
use rustix::io::Errno;
use uv_cache_info::{GlobEntryMetadata, GlobMetadataCollector, Timestamp};
use walkdir::DirEntry;

const MAX_STALLED_ATTEMPTS: usize = 64;

// The io-uring crate uses an opaque statx pointer. Check the actual kernel buffer and every field
// read below against the UAPI before casting the rustix storage to that pointer type.
const _: () = {
    assert!(size_of::<Statx>() == 256);
    assert!(align_of::<Statx>() == 8);
    assert!(offset_of!(Statx, stx_mask) == 0);
    assert!(offset_of!(Statx, stx_mode) == 28);
    assert!(offset_of!(Statx, stx_ctime) == 96);
    assert!(size_of::<Statx>() == size_of::<linux_raw_sys::general::statx>());
    assert!(align_of::<Statx>() == align_of::<linux_raw_sys::general::statx>());
    assert!(offset_of!(Statx, stx_mask) == offset_of!(linux_raw_sys::general::statx, stx_mask));
    assert!(offset_of!(Statx, stx_mode) == offset_of!(linux_raw_sys::general::statx, stx_mode));
    assert!(offset_of!(Statx, stx_ctime) == offset_of!(linux_raw_sys::general::statx, stx_ctime));
    assert!(size_of::<StatxTimestamp>() == 16);
    assert!(align_of::<StatxTimestamp>() == 8);
    assert!(offset_of!(StatxTimestamp, tv_sec) == 0);
    assert!(offset_of!(StatxTimestamp, tv_nsec) == 8);
    assert!(size_of::<StatxTimestamp>() == size_of::<linux_raw_sys::general::statx_timestamp>());
};

#[derive(Clone, Copy)]
pub(super) struct Configuration {
    pub(super) queue_depth: u32,
    pub(super) force_async: bool,
    pub(super) max_workers: Option<u32>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Activity {
    successful: u64,
    ordinary: u64,
}

pub(super) struct Scanner {
    ring: Option<IoUring>,
    directory: Arc<OwnedFd>,
    capacity: usize,
    flags: squeue::Flags,
    pending: Option<PendingBatch>,
    activity: Activity,
    ring_only: bool,
    configured_workers: Option<u32>,
    verify_workers: Option<u32>,
    #[cfg(test)]
    drain_observer: Option<std::sync::mpsc::SyncSender<()>>,
}

impl Scanner {
    pub(super) fn new(configuration: Configuration) -> io::Result<Self> {
        let ring: IoUring = IoUring::builder()
            .dontfork()
            .build(configuration.queue_depth)?;
        let mut probe = Probe::new();
        ring.submitter().register_probe(&mut probe)?;
        if !probe.is_supported(opcode::Statx::CODE) {
            return Err(unavailable("io_uring statx is unavailable"));
        }
        if let Some(max_workers) = configuration.max_workers {
            ring.submitter()
                .register_iowq_max_workers(&mut [max_workers, max_workers])?;
        }
        let capacity = usize::try_from(configuration.queue_depth.min(ring.params().sq_entries()))
            .map_err(|_| invalid_completion())?;
        if capacity == 0 {
            return Err(invalid_completion());
        }
        let directory = Arc::new(open(
            ".",
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        Ok(Self {
            ring: Some(ring),
            directory,
            capacity,
            flags: if configuration.force_async {
                squeue::Flags::ASYNC
            } else {
                squeue::Flags::empty()
            },
            pending: None,
            activity: Activity::default(),
            ring_only: false,
            configured_workers: configuration.max_workers,
            verify_workers: configuration.max_workers,
            #[cfg(test)]
            drain_observer: None,
        })
    }

    /// Reject per-file ordinary-I/O retries when measuring the active ring backend.
    pub(super) fn require_ring_only(&mut self) {
        self.ring_only = true;
    }

    pub(super) fn activity(&self) -> Activity {
        self.activity
    }

    /// Read the submitting task's effective io-wq limits after an actual request.
    pub(super) fn worker_limits(&self) -> io::Result<Option<[u32; 2]>> {
        if self.activity.successful == 0 || self.pending.is_some() {
            return Err(invalid_completion());
        }
        let ring = self.ring.as_ref().ok_or_else(invalid_completion)?;
        // Reapply an explicit cap rather than replacing the ring's saved limit with zeros.
        let mut previous = [self.configured_workers.unwrap_or(0); 2];
        match ring.submitter().register_iowq_max_workers(&mut previous) {
            Ok(()) => Ok(Some(previous)),
            Err(error) if unavailable_operation(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Verify that an entire metadata or CacheInfo invocation used the expected ring operations.
    pub(super) fn assert_activity(&self, before: Activity, expected: usize) {
        assert_eq!(
            self.activity.successful - before.successful,
            u64::try_from(expected).expect("Expected request count should fit in u64"),
            "unexpected number of successful STATX completions"
        );
        assert_eq!(
            self.activity.ordinary, before.ordinary,
            "ordinary metadata must not be included in an io_uring timing"
        );
    }

    /// Require an actual successful `STATX`, rather than accepting the per-file error fallback.
    pub(super) fn probe(&mut self, entry: &DirEntry) -> io::Result<Timestamp> {
        let mut requests = self.read_batch(vec![entry.clone()])?.into_vec().into_iter();
        let request = requests.next().ok_or_else(invalid_completion)?;
        request
            .timestamp()?
            .ok_or_else(|| unavailable("statx probe did not identify a regular file"))
    }

    pub(super) fn metadata(
        &mut self,
        entries: &[DirEntry],
    ) -> io::Result<Vec<(PathBuf, Timestamp)>> {
        Ok(self
            .read_entries(entries.to_vec())?
            .into_iter()
            .filter_map(GlobEntryMetadata::into_timestamp)
            .collect())
    }

    fn read_entries(&mut self, entries: Vec<DirEntry>) -> io::Result<Vec<GlobEntryMetadata>> {
        let mut output = Vec::with_capacity(entries.len());
        let mut batch = Vec::with_capacity(self.capacity);
        for entry in entries {
            batch.push(entry);
            if batch.len() == self.capacity {
                self.flush(&mut batch, &mut output)?;
            }
        }
        self.flush(&mut batch, &mut output)?;
        Ok(output)
    }

    fn flush(
        &mut self,
        entries: &mut Vec<DirEntry>,
        output: &mut Vec<GlobEntryMetadata>,
    ) -> io::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let entries = mem::replace(entries, Vec::with_capacity(self.capacity));
        for request in self.read_batch(entries)?.into_vec() {
            let timestamp = match request.timestamp() {
                Ok(timestamp) => timestamp,
                Err(error) if unavailable_operation(&error) => return Err(error),
                Err(error) if error.kind() == io::ErrorKind::InvalidData => return Err(error),
                Err(error) if self.ring_only => return Err(error),
                Err(_) => {
                    // Obtain the ordinary metadata error's original path context. The shared
                    // ordered consumer then emits exactly the normal cache-key diagnostic.
                    self.activity.ordinary += 1;
                    output.push(GlobEntryMetadata::read(Ok(request.entry)));
                    continue;
                }
            };
            output.push(GlobEntryMetadata::from_timestamp(request.entry, timestamp));
        }
        Ok(())
    }

    fn read_batch(&mut self, entries: Vec<DirEntry>) -> io::Result<Box<[Request]>> {
        self.read_batch_with(entries, &mut IoUring::submit_and_wait)
    }

    fn read_batch_with(
        &mut self,
        entries: Vec<DirEntry>,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> io::Result<Box<[Request]>> {
        if self.pending.is_some() {
            self.retire();
            return Err(invalid_completion());
        }
        if self.ring.is_none() {
            return Err(unavailable("io_uring statx reader was retired"));
        }
        self.pending = Some(PendingBatch::new(Arc::clone(&self.directory), entries)?);
        if let Err(error) = self.queue() {
            self.retire();
            return Err(error);
        }
        self.complete_queued_batch(submit)?;
        if let Err(error) = self.verify_worker_limit() {
            self.retire();
            return Err(error);
        }
        let mut batch = self.pending.take().ok_or_else(invalid_completion)?;
        Ok(mem::take(&mut batch.requests))
    }

    fn verify_worker_limit(&mut self) -> io::Result<()> {
        let Some(max_workers) = self.verify_workers else {
            return Ok(());
        };
        let ring = self.ring.as_ref().ok_or_else(invalid_completion)?;
        // Linux 5.15 creates io-wq for the submitting task on its first request. Reapply the
        // limit only after that request completes, and check the previous effective values.
        // Unlike a [0, 0] query, this also keeps the ring's saved limits set for future users.
        let mut previous = [max_workers, max_workers];
        ring.submitter().register_iowq_max_workers(&mut previous)?;
        if previous != [max_workers, max_workers] {
            return Err(unavailable("requested io-wq worker limits are unavailable"));
        }
        self.verify_workers = None;
        Ok(())
    }

    fn complete_queued_batch(
        &mut self,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> io::Result<()> {
        if let Err(error) = self.finish_batch(submit) {
            self.retire();
            return Err(error);
        }
        Ok(())
    }

    #[expect(unsafe_code)]
    fn queue(&mut self) -> io::Result<()> {
        let (Some(ring), Some(batch)) = (&mut self.ring, &mut self.pending) else {
            return Err(invalid_completion());
        };
        let mut submissions = ring.submission();
        for (index, request) in batch.requests.iter().enumerate() {
            let entry = request
                .statx_entry(&batch.directory, index as u64)
                .flags(self.flags);
            // SAFETY: The pending batch owns the directory FD, C strings, and boxed statx
            // buffers. They remain valid through all submitted completions, including retirement.
            unsafe { submissions.push(&entry) }.map_err(|_| invalid_completion())?;
            batch.queued += 1;
        }
        Ok(())
    }

    fn finish_batch(
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
            let recorded = completion_index(
                completion.user_data(),
                completion.flags(),
                batch.requests.len(),
            )
            .and_then(|index| record_result(&batch.requests[index].result, completion.result()));
            if let Err(error) = recorded {
                batch.valid_completions = false;
                return Err(error);
            }
            if completion.result() == 0 {
                self.activity.successful += 1;
            }
            batch.completed += 1;
        }
        Ok(())
    }

    fn retire(&mut self) {
        let drained = self.drain_submitted();
        self.retire_after_drain(drained);
    }

    fn retire_after_drain(&mut self, drained: io::Result<()>) {
        if drained.is_err() {
            // Closing a ring cancels asynchronously. On supported 5.x kernels neither close nor
            // broad cancellation proves that io-wq has stopped using the buffers. Retain this
            // one bounded batch if the submitted CQEs cannot be accounted for.
            mem::forget(self.pending.take());
        }
        // No SQPOLL thread can submit the remaining SQEs after this ring is closed.
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
            // SAFETY: GETEVENTS has no optional arguments here. The zero submission count leaves
            // unsubmitted SQEs untouched, and the batch still owns all kernel-facing storage.
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

impl GlobMetadataCollector for Scanner {
    fn collect(
        &mut self,
        walker: GlobWalker,
    ) -> io::Result<impl Iterator<Item = GlobEntryMetadata>> {
        let mut output = Vec::new();
        let mut batch = Vec::with_capacity(self.capacity);
        for entry in walker {
            match entry {
                Ok(entry) => batch.push(entry),
                Err(error) => {
                    self.flush(&mut batch, &mut output)?;
                    output.push(GlobEntryMetadata::read(Err(error)));
                }
            }
            if batch.len() == self.capacity {
                self.flush(&mut batch, &mut output)?;
            }
        }
        self.flush(&mut batch, &mut output)?;
        Ok(output.into_iter())
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        if self.pending.is_some() {
            self.retire();
        }
    }
}

struct PendingBatch {
    directory: Arc<OwnedFd>,
    requests: Box<[Request]>,
    queued: usize,
    completed: usize,
    valid_completions: bool,
    #[cfg(test)]
    drop_counter: Option<Arc<AtomicUsize>>,
}

impl PendingBatch {
    fn new(directory: Arc<OwnedFd>, entries: Vec<DirEntry>) -> io::Result<Self> {
        Ok(Self {
            directory,
            requests: entries
                .into_iter()
                .map(Request::new)
                .collect::<io::Result<_>>()?,
            queued: 0,
            completed: 0,
            valid_completions: true,
            #[cfg(test)]
            drop_counter: None,
        })
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

struct Request {
    entry: DirEntry,
    path: CString,
    metadata: UnsafeCell<MaybeUninit<Statx>>,
    result: Cell<Option<i32>>,
}

impl Request {
    fn new(entry: DirEntry) -> io::Result<Self> {
        let path = CString::new(entry.path().as_os_str().as_bytes())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        Ok(Self {
            entry,
            path,
            metadata: UnsafeCell::new(MaybeUninit::uninit()),
            result: Cell::new(None),
        })
    }

    fn statx_entry(&self, directory: &OwnedFd, user_data: u64) -> squeue::Entry {
        let flags = if self.entry.path_is_symlink() {
            AtFlags::NO_AUTOMOUNT
        } else {
            AtFlags::SYMLINK_NOFOLLOW | AtFlags::NO_AUTOMOUNT
        };
        opcode::Statx::new(
            types::Fd(directory.as_raw_fd()),
            self.path.as_ptr(),
            self.metadata.get().cast::<types::statx>(),
        )
        .flags(flags.bits().cast_signed())
        .mask((StatxFlags::TYPE | StatxFlags::CTIME).bits())
        .build()
        .user_data(user_data)
    }

    #[expect(unsafe_code)]
    fn timestamp(&self) -> io::Result<Option<Timestamp>> {
        let result = self.result.get().ok_or_else(invalid_completion)?;
        if result < 0 {
            return Err(io::Error::from_raw_os_error(
                result.checked_neg().ok_or_else(invalid_completion)?,
            ));
        }
        if result != 0 {
            return Err(invalid_completion());
        }
        // SAFETY: A successful statx completion initializes the complete kernel-layout buffer.
        // The scanner only returns requests after accounting for every submitted completion.
        let metadata = unsafe { (*self.metadata.get()).assume_init_read() };
        if metadata.stx_mask & StatxFlags::TYPE.bits() == 0 {
            return Err(unavailable("statx did not return the file type"));
        }
        if FileType::from_raw_mode(u32::from(metadata.stx_mode)) != FileType::RegularFile {
            return Ok(None);
        }
        if metadata.stx_mask & StatxFlags::CTIME.bits() == 0 {
            return Err(unavailable("statx did not return ctime"));
        }
        // Unix Timestamp uses ctime, not mtime. Use the same seconds/nanoseconds representation.
        let seconds = u64::try_from(metadata.stx_ctime.tv_sec).map_err(|_| invalid_completion())?;
        if metadata.stx_ctime.tv_nsec >= 1_000_000_000 {
            return Err(invalid_completion());
        }
        let duration = Duration::new(seconds, metadata.stx_ctime.tv_nsec);
        let timestamp = UNIX_EPOCH
            .checked_add(duration)
            .ok_or_else(invalid_completion)?;
        Ok(Some(Timestamp::from(timestamp)))
    }
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
                    "io_uring cache-key requests made no progress",
                ));
            }
        } else {
            self.previous = current;
            self.stalled = 0;
        }
        Ok(())
    }
}

fn retryable_control_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Interrupted
        || error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == Some(Errno::BUSY.raw_os_error())
}

pub(super) fn unavailable_operation(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Unsupported
        || matches!(
            error.raw_os_error(),
            Some(code)
                if code == Errno::NOSYS.raw_os_error()
                    || code == Errno::OPNOTSUPP.raw_os_error()
                    || code == Errno::INVAL.raw_os_error()
        )
}

fn unavailable(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message)
}

fn invalid_completion() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid io_uring cache-key statx completion",
    )
}

fn completion_index(user_data: u64, flags: u32, request_count: usize) -> io::Result<usize> {
    if flags != 0 {
        return Err(invalid_completion());
    }
    usize::try_from(user_data)
        .ok()
        .filter(|index| *index < request_count)
        .ok_or_else(invalid_completion)
}

fn record_result(result: &Cell<Option<i32>>, value: i32) -> io::Result<()> {
    if result.replace(Some(value)).is_some() {
        return Err(invalid_completion());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::FileTimes;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::mpsc;

    use globwalk::{FileType as GlobFileType, GlobWalkerBuilder};
    use uv_cache_info::CacheInfo;

    use super::*;

    fn configuration(queue_depth: u32) -> Configuration {
        Configuration {
            queue_depth,
            force_async: false,
            max_workers: None,
        }
    }

    fn entry(path: &Path) -> io::Result<DirEntry> {
        walkdir::WalkDir::new(path)
            .max_depth(0)
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::other("fixture path was not enumerated"))?
            .map_err(io::Error::other)
    }

    fn files(root: &Path, count: usize) -> io::Result<Vec<DirEntry>> {
        (0..count)
            .map(|index| {
                let path = root.join(format!("file-{index}.py"));
                fs_err::write(&path, [])?;
                entry(&path)
            })
            .collect()
    }

    fn ordinary(entries: &[DirEntry]) -> Vec<(PathBuf, Timestamp)> {
        entries
            .iter()
            .cloned()
            .map(|entry| GlobEntryMetadata::read(Ok(entry)))
            .filter_map(GlobEntryMetadata::into_timestamp)
            .collect()
    }

    #[test]
    fn progress_resets_only_after_submission_or_completion() -> io::Result<()> {
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

        let mut progress = Progress::default();
        progress.observe(1, 1)?;
        assert_eq!(
            progress
                .observe(0, 1)
                .expect_err("completion count must not decrease")
                .kind(),
            io::ErrorKind::InvalidData
        );
        let mut progress = Progress::default();
        progress.observe(1, 1)?;
        assert_eq!(
            progress
                .observe(1, 2)
                .expect_err("unsubmitted count must not increase")
                .kind(),
            io::ErrorKind::InvalidData
        );
        Ok(())
    }

    #[test]
    fn completion_indices_and_duplicates_are_checked() -> io::Result<()> {
        assert_eq!(completion_index(1, 0, 2)?, 1);
        for (user_data, flags, request_count) in [(2, 0, 2), (0, 1, 2), (0, 0, 0)] {
            assert_eq!(
                completion_index(user_data, flags, request_count)
                    .expect_err("invalid completion must be rejected")
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
        let result = Cell::new(None);
        record_result(&result, 0)?;
        assert_eq!(result.get(), Some(0));
        assert_eq!(
            record_result(&result, 0)
                .expect_err("a request can complete only once")
                .kind(),
            io::ErrorKind::InvalidData
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_uses_ctime_follows_leaf_symlinks_and_reuses_ring() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("src");
        fs_err::create_dir(&source)?;
        fs_err::write(
            root.path().join("pyproject.toml"),
            "[tool.uv]\ncache-keys = [{ file = \"src/**/*.py\" }]\n",
        )?;
        let regular = source.join("regular.py");
        fs_err::write(&regular, b"regular\n")?;
        let old_mtime = UNIX_EPOCH + Duration::from_secs(946_684_800);
        fs_err::File::open(&regular)?.set_times(FileTimes::new().set_modified(old_mtime))?;
        let expected_ctime = Timestamp::from_path(&regular)?;
        assert_ne!(expected_ctime, Timestamp::from(old_mtime));
        let outside = root.path().join("outside.py");
        fs_err::write(&outside, b"target\n")?;
        let directory = root.path().join("outside-directory");
        fs_err::create_dir(&directory)?;
        fs_err::write(directory.join("not-traversed.py"), b"outside\n")?;
        fs_err::os::unix::fs::symlink("../outside.py", source.join("link.py"))?;
        fs_err::os::unix::fs::symlink("../outside-directory", source.join("directory.py"))?;
        fs_err::os::unix::fs::symlink("../absent.py", source.join("dangling.py"))?;
        let entries = GlobWalkerBuilder::from_patterns(root.path(), &["src/**/*.py"])
            .file_type(GlobFileType::FILE | GlobFileType::SYMLINK)
            .build()
            .map_err(io::Error::other)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)?;
        let expected = ordinary(&entries);
        assert_eq!(expected.len(), 2);
        assert!(expected.contains(&(regular.clone(), expected_ctime)));
        assert!(expected.contains(&(source.join("link.py"), Timestamp::from_path(&outside)?)));

        let mut scanner = Scanner::new(configuration(2))?;
        let descriptor = scanner.ring.as_ref().map(AsRawFd::as_raw_fd);
        assert_eq!(scanner.probe(&entry(&regular)?)?, expected_ctime);
        assert_eq!(scanner.metadata(&entries)?, expected);
        assert_eq!(
            CacheInfo::from_directory_with_glob_collector(root.path(), &mut scanner)
                .map_err(io::Error::other)?,
            CacheInfo::from_directory(root.path()).map_err(io::Error::other)?
        );

        // Successful symlink-to-directory STATX calls must count, even though they yield no key.
        let successful = entries
            .into_iter()
            .filter(|entry| !entry.path().ends_with("dangling.py"))
            .collect::<Vec<_>>();
        scanner.require_ring_only();
        for _ in 0..2 {
            let before = scanner.activity();
            assert_eq!(scanner.metadata(&successful)?, expected);
            scanner.assert_activity(before, successful.len());
        }
        let before = scanner.activity();
        assert!(scanner.metadata(&[])?.is_empty());
        scanner.assert_activity(before, 0);
        assert_eq!(scanner.ring.as_ref().map(AsRawFd::as_raw_fd), descriptor);
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_handles_missing_after_enumeration_without_timed_fallback() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 1)?;
        fs_err::remove_file(entries[0].path())?;
        let mut scanner = Scanner::new(configuration(1))?;
        assert_eq!(scanner.metadata(&entries)?, ordinary(&entries));
        assert_eq!(scanner.activity.ordinary, 1);

        scanner.require_ring_only();
        let before = scanner.activity();
        assert_eq!(
            scanner
                .metadata(&entries)
                .expect_err("strict timing must not retry missing paths with ordinary metadata")
                .kind(),
            io::ErrorKind::NotFound
        );
        scanner.assert_activity(before, 0);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_some());
        Ok(())
    }

    #[test]
    #[ignore = "requires Linux 5.15+ with io-wq worker-limit registration"]
    fn active_backend_verifies_worker_limits_after_first_submission() -> io::Result<()> {
        // io-wq limits belong to the submitting task on Linux 5.15. Isolate this registration
        // from the test harness's other rings instead of changing a reused test worker's pool.
        std::thread::spawn(|| -> io::Result<()> {
            let root = tempfile::tempdir()?;
            let entries = files(root.path(), 3)?;
            let mut scanner = Scanner::new(Configuration {
                queue_depth: 2,
                force_async: true,
                max_workers: Some(16),
            })?;
            scanner.require_ring_only();
            let before = scanner.activity();
            assert_eq!(scanner.metadata(&entries)?, ordinary(&entries));
            scanner.assert_activity(before, entries.len());
            assert!(scanner.verify_workers.is_none());
            assert_eq!(scanner.worker_limits()?, Some([16, 16]));
            Ok(())
        })
        .join()
        .expect("worker-limit test thread panicked")
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_retries_unsubmitted_control_failures() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 1)?;
        let expected = Timestamp::from_path(entries[0].path())?;
        let mut scanner = Scanner::new(configuration(1))?;
        let mut failures = [Errno::INTR, Errno::AGAIN, Errno::BUSY].into_iter();
        let requests = scanner.read_batch_with(entries, &mut |ring, want| {
            if let Some(error) = failures.next() {
                return Err(error.into());
            }
            ring.submit_and_wait(want)
        })?;
        assert_eq!(requests[0].timestamp()?, Some(expected));
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_some());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_retires_after_stalled_control_errors() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 1)?;
        let mut scanner = Scanner::new(configuration(1))?;
        let attempts = Cell::new(0);
        let result = scanner.read_batch_with(entries, &mut |_, _| {
            attempts.set(attempts.get() + 1);
            Err(Errno::AGAIN.into())
        });
        assert_eq!(
            result.err().expect("stalled control must fail").kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(attempts.get(), MAX_STALLED_ATTEMPTS);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_backend_retires_partial_submission_before_reclaiming_requests() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 4)?;
        let mut scanner = Scanner::new(configuration(4))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(Arc::clone(&scanner.directory), entries)?;
        batch.drop_counter = Some(Arc::clone(&counter));
        scanner.pending = Some(batch);
        scanner.queue()?;
        let submitted = Cell::new(0);
        let result = scanner.complete_queued_batch(&mut |ring, _| {
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            let count = loop {
                // SAFETY: Submit only the first valid statx entry, with no wait, signal mask, or
                // extended arguments. The scanner still owns every request in the batch.
                match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                    Ok(count) => break count,
                    Err(error) if retryable_control_error(&error) => {}
                    Err(error) => return Err(error),
                }
            };
            submitted.set(count);
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            Err(Errno::PERM.into())
        });
        assert_eq!(
            result
                .expect_err("fatal control error must retire the ring")
                .raw_os_error(),
            Some(Errno::PERM.raw_os_error())
        );
        assert_eq!(submitted.get(), 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_backend_retains_requests_when_draining_is_unavailable() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 1)?;
        let mut scanner = Scanner::new(configuration(1))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(Arc::clone(&scanner.directory), entries)?;
        batch.drop_counter = Some(Arc::clone(&counter));
        scanner.pending = Some(batch);
        scanner.queue()?;
        let ring = scanner.ring.as_ref().expect("initialized ring");
        let submitted = loop {
            // SAFETY: The scanner owns the queued statx request and its stable storage. Do not
            // wait or consume its CQE; reclamation still requires the scanner's drain barrier.
            match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                Ok(count) => break count,
                Err(error) if retryable_control_error(&error) => {}
                Err(error) => return Err(error),
            }
        };
        assert_eq!(submitted, 1);

        // Simulate inaccessible completion control after a real submission. This deliberately
        // retains one bounded batch until the test process exits.
        scanner.retire_after_drain(Err(Errno::PERM.into()));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_backend_keeps_in_flight_metadata_alive_during_retirement() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 4)?;
        let mut scanner = Scanner::new(configuration(4))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(Arc::clone(&scanner.directory), entries)?;
        batch.drop_counter = Some(Arc::clone(&counter));
        scanner.pending = Some(batch);
        let (reader, mut writer) = UnixStream::pair()?;

        // Link statx behind an unreadable socket's poll so its buffer remains in flight until
        // the observer releases the socket. The remaining two SQEs stay unsubmitted.
        {
            let ring = scanner.ring.as_mut().expect("initialized ring");
            let batch = scanner.pending.as_mut().expect("owned pending batch");
            let poll = opcode::PollAdd::new(
                types::Fd(reader.as_raw_fd()),
                linux_raw_sys::general::POLLIN,
            )
            .build()
            .flags(squeue::Flags::IO_LINK)
            .user_data(0);
            let mut submissions = ring.submission();
            // SAFETY: The socket stays open until the batch is drained. The dummy first request
            // provides completion bookkeeping and has no kernel-accessed metadata.
            unsafe { submissions.push(&poll) }.map_err(io::Error::other)?;
            batch.queued += 1;
            for (index, request) in batch.requests.iter().enumerate().skip(1) {
                let entry = request.statx_entry(&batch.directory, index as u64);
                // SAFETY: The scanner owns the same stable request storage as its ordinary
                // queue path and retains it until all submitted requests have completed.
                unsafe { submissions.push(&entry) }.map_err(io::Error::other)?;
                batch.queued += 1;
            }
        }

        let (observer, observed) = mpsc::sync_channel(0);
        scanner.drain_observer = Some(observer);
        let submitted = Cell::new(0);
        std::thread::scope(|scope| {
            let counter = Arc::clone(&counter);
            let release = scope.spawn(move || -> io::Result<()> {
                let observation = observed.recv_timeout(Duration::from_secs(10));
                let requests_alive = counter.load(Ordering::SeqCst) == 0;
                // Always release the poll, including on observation failure, so an assertion
                // cannot leave the scanner's cleanup waiting forever.
                let released = writer.write_all(b"x");
                observation.map_err(io::Error::other)?;
                assert!(
                    requests_alive,
                    "pending metadata was reclaimed before completion"
                );
                released
            });
            let completed = scanner.complete_queued_batch(&mut |ring, _| {
                let count = loop {
                    // SAFETY: Submit only the poll/statx pair, with no wait or signal mask. The
                    // scanner owns the metadata, directory, and unsubmitted entries throughout.
                    match unsafe { ring.submitter().enter::<()>(2, 0, 0, None) } {
                        Ok(count) => break count,
                        Err(error) if retryable_control_error(&error) => {}
                        Err(error) => return Err(error),
                    }
                };
                submitted.set(count);
                Err(Errno::PERM.into())
            });
            release.join().expect("release thread panicked")?;
            assert_eq!(
                completed
                    .expect_err("fatal control error must retire the ring")
                    .raw_os_error(),
                Some(Errno::PERM.raw_os_error())
            );
            Ok::<(), io::Error>(())
        })?;
        assert_eq!(submitted.get(), 2);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_none());
        Ok(())
    }
}
