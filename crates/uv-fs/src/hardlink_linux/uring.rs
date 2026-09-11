use std::cell::{Cell, UnsafeCell};
use std::ffi::{CString, OsStr};
use std::io;
use std::mem::{self, MaybeUninit, align_of, offset_of, size_of};
use std::num::NonZeroU32;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use io_uring::{EnterFlags, IoUring, Probe, opcode, types};
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, Statx, StatxFlags, open};
use rustix::io::Errno;
use tracing::debug;

const MAX_STALLED_ATTEMPTS: usize = 64;

// io_uring::types::statx is only an opaque pointer marker. rustix supplies storage for the Linux
// UAPI's 256-byte struct, including its reserved fields. Check its layout against both the fixed
// UAPI offsets and linux-raw-sys on every enabled architecture before casting that pointer.
const _: () = {
    assert!(size_of::<Statx>() == 256);
    assert!(align_of::<Statx>() == 8);
    assert!(offset_of!(Statx, stx_mask) == 0);
    assert!(offset_of!(Statx, stx_nlink) == 16);
    assert!(offset_of!(Statx, stx_mode) == 28);
    assert!(size_of::<Statx>() == size_of::<linux_raw_sys::general::statx>());
    assert!(align_of::<Statx>() == align_of::<linux_raw_sys::general::statx>());
    assert!(offset_of!(Statx, stx_mask) == offset_of!(linux_raw_sys::general::statx, stx_mask));
    assert!(offset_of!(Statx, stx_mode) == offset_of!(linux_raw_sys::general::statx, stx_mode));
    assert!(offset_of!(Statx, stx_nlink) == offset_of!(linux_raw_sys::general::statx, stx_nlink));
};

pub(crate) struct Scanner {
    queue_depth: NonZeroU32,
    initialized: bool,
    driver: Option<Driver>,
}

impl Scanner {
    pub(crate) fn new(queue_depth: NonZeroU32) -> Self {
        Self {
            queue_depth,
            initialized: false,
            driver: None,
        }
    }

    pub(crate) fn files_with_one_hardlink(
        &mut self,
        path: &Path,
    ) -> io::Result<Option<Vec<PathBuf>>> {
        self.initialize_with(Driver::new);
        let Some(driver) = &mut self.driver else {
            return Ok(None);
        };
        let result = driver.scan_with(path, &mut IoUring::submit_and_wait);
        if driver.ring.is_none() {
            self.driver = None;
        }
        result
    }

    fn initialize_with(&mut self, initialize: impl FnOnce(NonZeroU32) -> io::Result<Driver>) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        match initialize(self.queue_depth) {
            Ok(driver) => self.driver = Some(driver),
            Err(error) => debug!("Falling back to individual hardlink counts: {error}"),
        }
    }
}

struct Driver {
    ring: Option<IoUring>,
    capacity: usize,
    pending: Option<PendingBatch>,
    #[cfg(test)]
    drain_observer: Option<std::sync::mpsc::SyncSender<()>>,
}

impl Driver {
    fn new(queue_depth: NonZeroU32) -> io::Result<Self> {
        let ring: IoUring = IoUring::builder().dontfork().build(queue_depth.get())?;
        let mut probe = Probe::new();
        ring.submitter().register_probe(&mut probe)?;
        if !probe.is_supported(opcode::Statx::CODE) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "io_uring statx is unavailable",
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
            pending: None,
            #[cfg(test)]
            drain_observer: None,
        })
    }

    fn scan_with(
        &mut self,
        path: &Path,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> io::Result<Option<Vec<PathBuf>>> {
        if self.pending.is_some() {
            self.retire();
            return Ok(None);
        }
        if self.ring.is_none() {
            return Ok(None);
        }
        let directory = Arc::new(open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let entries = Dir::read_from(directory.as_ref())?;
        let mut names = Vec::with_capacity(self.capacity);
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            if name.to_bytes().is_empty() || name.to_bytes().contains(&b'/') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid directory entry name",
                ));
            }
            match entry.file_type() {
                FileType::Directory => return Ok(None),
                FileType::RegularFile | FileType::Unknown => names.push(name.to_owned()),
                FileType::Symlink
                | FileType::Fifo
                | FileType::Socket
                | FileType::CharacterDevice
                | FileType::BlockDevice => continue,
            }
            if names.len() == self.capacity {
                let Some(batch) = self.read_batch(path, &directory, names, submit)? else {
                    return Ok(None);
                };
                files.extend(batch);
                names = Vec::with_capacity(self.capacity);
            }
        }
        if !names.is_empty() {
            let Some(batch) = self.read_batch(path, &directory, names, submit)? else {
                return Ok(None);
            };
            files.extend(batch);
        }
        Ok(Some(files))
    }

    fn read_batch(
        &mut self,
        path: &Path,
        directory: &Arc<OwnedFd>,
        names: Vec<CString>,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> io::Result<Option<Vec<PathBuf>>> {
        self.pending = Some(PendingBatch::new(Arc::clone(directory), names));
        if !self.complete_batch(submit) {
            return Ok(None);
        }
        let Some(batch) = self.pending.take() else {
            return Err(invalid_completion());
        };
        collect_candidates(path, &batch.requests)
    }

    fn complete_batch(
        &mut self,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> bool {
        match self.queue() {
            Ok(()) => self.complete_queued_batch(submit),
            Err(error) => self.fail_control(error),
        }
    }

    fn complete_queued_batch(
        &mut self,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> bool {
        match self.finish_batch(submit) {
            Ok(()) => true,
            Err(error) => self.fail_control(error),
        }
    }

    fn fail_control(&mut self, error: io::Error) -> bool {
        self.retire();
        debug!("Falling back to individual hardlink counts: {error}");
        false
    }

    #[expect(unsafe_code)]
    fn queue(&mut self) -> io::Result<()> {
        let (Some(ring), Some(batch)) = (&mut self.ring, &mut self.pending) else {
            return Err(invalid_completion());
        };
        let mut submissions = ring.submission();
        for (index, request) in batch.requests.iter().enumerate() {
            let entry = request.statx_entry(&batch.directory, index as u64);
            // SAFETY: The batch owns the directory descriptor and boxed request storage. Neither
            // the names nor the kernel-layout statx buffers move or become invalid until every
            // queued request has completed, including when submission or cleanup fails.
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
            let request = usize::try_from(completion.user_data())
                .ok()
                .and_then(|index| batch.requests.get(index));
            let Some(request) = request else {
                batch.valid_completions = false;
                return Err(invalid_completion());
            };
            if request.result.replace(Some(completion.result())).is_some() {
                batch.valid_completions = false;
                return Err(invalid_completion());
            }
            batch.completed += 1;
        }
        Ok(())
    }

    /// Retire a failed ring without releasing memory still referenced by the kernel.
    fn retire(&mut self) {
        let drained = self.drain_submitted();
        self.retire_after_drain(drained);
    }

    fn retire_after_drain(&mut self, drained: io::Result<()>) {
        if let Err(error) = drained {
            // Ring shutdown cancels asynchronously, and older kernels do not guarantee that a
            // successful synchronous ANY cancellation has finished running io-wq work. Without
            // the submitted CQEs, retain this one bounded batch so a late statx cannot write into
            // freed memory or use a recycled directory FD.
            mem::forget(self.pending.take());
            debug!("Unable to reclaim pending io_uring metadata requests: {error}");
        }
        // No SQPOLL thread is used, so closing the ring discards any entries that were never
        // submitted. Submitted requests have completed or retained their storage above.
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
            // SAFETY: This uses the ordinary GETEVENTS interface with no signal mask or extended
            // arguments. A zero submission count leaves unsubmitted entries untouched while the
            // batch continues to own every referenced buffer and directory descriptor.
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

impl Drop for Driver {
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
    fn new(directory: Arc<OwnedFd>, names: Vec<CString>) -> Self {
        Self {
            directory,
            requests: names.into_iter().map(Request::new).collect(),
            queued: 0,
            completed: 0,
            valid_completions: true,
            #[cfg(test)]
            drop_counter: None,
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

struct Request {
    name: CString,
    metadata: UnsafeCell<MaybeUninit<Statx>>,
    result: Cell<Option<i32>>,
}

impl Request {
    fn new(name: CString) -> Self {
        Self {
            name,
            metadata: UnsafeCell::new(MaybeUninit::uninit()),
            result: Cell::new(None),
        }
    }

    fn statx_entry(&self, directory: &OwnedFd, user_data: u64) -> io_uring::squeue::Entry {
        opcode::Statx::new(
            types::Fd(directory.as_raw_fd()),
            self.name.as_ptr(),
            self.metadata.get().cast::<types::statx>(),
        )
        .flags(
            (AtFlags::SYMLINK_NOFOLLOW | AtFlags::NO_AUTOMOUNT)
                .bits()
                .cast_signed(),
        )
        .mask((StatxFlags::TYPE | StatxFlags::NLINK).bits())
        .build()
        .user_data(user_data)
    }
}

#[expect(unsafe_code)]
fn collect_candidates(path: &Path, requests: &[Request]) -> io::Result<Option<Vec<PathBuf>>> {
    let mut files = Vec::new();
    for request in requests {
        let Some(result) = request.result.get() else {
            return Err(invalid_completion());
        };
        if result < 0 {
            let error =
                io::Error::from_raw_os_error(result.checked_neg().ok_or_else(invalid_completion)?);
            if error.kind() == io::ErrorKind::NotFound {
                continue;
            }
            if unavailable_metadata(&error) {
                debug!("Falling back to individual hardlink counts: {error}");
                return Ok(None);
            }
            return Err(error);
        }
        if result != 0 {
            return Err(invalid_completion());
        }
        // SAFETY: The successful statx completion initializes the complete kernel-layout buffer,
        // and the driver has observed every CQE before handing these requests to the decoder.
        let metadata = unsafe { (*request.metadata.get()).assume_init_read() };
        match classify_metadata(metadata.stx_mask, metadata.stx_mode, metadata.stx_nlink) {
            Some(true) => files.push(path.join(OsStr::from_bytes(request.name.to_bytes()))),
            Some(false) => {}
            None => return Ok(None),
        }
    }
    Ok(Some(files))
}

fn classify_metadata(mask: u32, mode: u16, link_count: u32) -> Option<bool> {
    if mask & StatxFlags::TYPE.bits() == 0 {
        return None;
    }
    match FileType::from_raw_mode(u32::from(mode)) {
        FileType::Directory | FileType::Unknown => None,
        FileType::RegularFile => {
            if mask & StatxFlags::NLINK.bits() == 0 {
                return None;
            }
            Some(link_count == 1)
        }
        FileType::Symlink
        | FileType::Fifo
        | FileType::Socket
        | FileType::CharacterDevice
        | FileType::BlockDevice => Some(false),
    }
}

fn retryable_control_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Interrupted
        || error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == Some(Errno::BUSY.raw_os_error())
}

/// Bound retryable control errors that do not consume SQEs or produce CQEs.
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
                    "io_uring metadata requests made no progress",
                ));
            }
        } else {
            self.previous = current;
            self.stalled = 0;
        }
        Ok(())
    }
}

fn unavailable_metadata(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code)
            if code == Errno::NOSYS.raw_os_error()
                || code == Errno::OPNOTSUPP.raw_os_error()
                || code == Errno::INVAL.raw_os_error()
    )
}

fn invalid_completion() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid io_uring statx completion",
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::{CString, OsStr};
    use std::io::{self, Write};
    use std::num::NonZeroU32;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use io_uring::{IoUring, opcode, squeue, types};
    use rustix::fs::{FileType, Mode, OFlags, StatxFlags, open};
    use rustix::io::Errno;

    use super::{
        Driver, MAX_STALLED_ATTEMPTS, PendingBatch, Progress, Request, Scanner, classify_metadata,
        collect_candidates, retryable_control_error,
    };

    #[test]
    fn unavailable_ring_requests_fallback_only_once() -> io::Result<()> {
        let attempts = Cell::new(0);
        let mut scanner = Scanner::new(NonZeroU32::MIN);
        for _ in 0..2 {
            scanner.initialize_with(|_| {
                attempts.set(attempts.get() + 1);
                Err(Errno::PERM.into())
            });
        }
        assert_eq!(attempts.get(), 1);
        assert_eq!(
            scanner.files_with_one_hardlink(Path::new("not-read"))?,
            None
        );
        Ok(())
    }

    #[test]
    fn requires_file_type_and_link_count_attributes() {
        let required = (StatxFlags::TYPE | StatxFlags::NLINK).bits();
        let regular = u16::try_from(FileType::RegularFile.as_raw_mode())
            .expect("regular file type fits in statx mode");
        let directory = u16::try_from(FileType::Directory.as_raw_mode())
            .expect("directory type fits in statx mode");
        let symlink = u16::try_from(FileType::Symlink.as_raw_mode())
            .expect("symlink type fits in statx mode");
        assert_eq!(classify_metadata(StatxFlags::TYPE.bits(), regular, 1), None);
        assert_eq!(
            classify_metadata(StatxFlags::NLINK.bits(), regular, 1),
            None
        );
        assert_eq!(classify_metadata(required, regular, 1), Some(true));
        assert_eq!(classify_metadata(required, regular, 2), Some(false));
        assert_eq!(classify_metadata(required, directory, 1), None);
        assert_eq!(classify_metadata(required, symlink, 1), Some(false));
    }

    #[test]
    fn missing_entries_and_unsupported_metadata_are_not_candidates() -> io::Result<()> {
        let request = Request::new(c"removed".to_owned());
        request.result.set(Some(-Errno::NOENT.raw_os_error()));
        assert_eq!(
            collect_candidates(Path::new("files"), &[request])?,
            Some(Vec::new())
        );

        let request = Request::new(c"unsupported".to_owned());
        request.result.set(Some(-Errno::NOSYS.raw_os_error()));
        assert_eq!(collect_candidates(Path::new("files"), &[request])?, None);

        for error in [Errno::IO, Errno::ACCESS, Errno::PERM] {
            let request = Request::new(c"unreadable".to_owned());
            request.result.set(Some(-error.raw_os_error()));
            assert_eq!(
                collect_candidates(Path::new("files"), &[request])
                    .expect_err("path errors must not be treated as unavailable metadata")
                    .raw_os_error(),
                Some(error.raw_os_error())
            );
        }
        Ok(())
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
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_reads_multiple_batches_and_reuses_ring() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let directory = root.path().join("files");
        fs_err::create_dir(&directory)?;
        let mut expected = Vec::new();
        for index in 0..37 {
            let path = directory.join(format!("{index:064x}"));
            fs_err::write(&path, [])?;
            expected.push(path);
        }
        for name in [OsStr::new("file-λ"), OsStr::from_bytes(b"file-\xff")] {
            let path = directory.join(name);
            fs_err::write(&path, [])?;
            expected.push(path);
        }
        let retained = root.path().join("retained");
        fs_err::write(&retained, [])?;
        fs_err::hard_link(&retained, directory.join("shared"))?;
        fs_err::os::unix::fs::symlink(&retained, directory.join("symlink"))?;
        fs_err::os::unix::fs::symlink(root.path().join("missing"), directory.join("dangling"))?;
        let outside = root.path().join("outside");
        fs_err::create_dir(&outside)?;
        fs_err::write(outside.join("unrelated"), [])?;
        fs_err::os::unix::fs::symlink(&outside, directory.join("directory-symlink"))?;

        let mut driver = Driver::new(NonZeroU32::new(4).expect("non-zero queue depth"))?;
        let descriptor = driver.ring.as_ref().map(AsRawFd::as_raw_fd);
        let mut actual = driver.scan_with(&directory, &mut IoUring::submit_and_wait)?;
        if let Some(files) = &mut actual {
            files.sort();
        }
        expected.sort();
        assert_eq!(actual, Some(expected));

        let empty = root.path().join("empty");
        fs_err::create_dir(&empty)?;
        assert_eq!(
            driver.scan_with(&empty, &mut IoUring::submit_and_wait)?,
            Some(Vec::new())
        );
        assert_eq!(driver.ring.as_ref().map(AsRawFd::as_raw_fd), descriptor);

        fs_err::create_dir(directory.join("nested"))?;
        assert_eq!(
            driver.scan_with(&directory, &mut IoUring::submit_and_wait)?,
            None
        );

        let directory_link = root.path().join("directory-link");
        fs_err::os::unix::fs::symlink(&directory, &directory_link)?;
        let error = driver
            .scan_with(&directory_link, &mut IoUring::submit_and_wait)
            .expect_err("the directory itself must not be a symlink");
        assert!(
            error.raw_os_error() == Some(Errno::NOTDIR.raw_os_error())
                || error.raw_os_error() == Some(Errno::LOOP.raw_os_error())
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_retries_unsubmitted_control_failures() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("file");
        fs_err::write(&path, [])?;
        let mut driver = Driver::new(NonZeroU32::MIN)?;
        let mut failures = [Errno::INTR, Errno::AGAIN, Errno::BUSY].into_iter();
        let actual = driver.scan_with(root.path(), &mut |ring, want| {
            if let Some(error) = failures.next() {
                return Err(error.into());
            }
            ring.submit_and_wait(want)
        })?;
        assert_eq!(actual, Some(vec![path]));
        assert!(driver.pending.is_none());
        assert!(driver.ring.is_some());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_falls_back_after_stalled_control_errors() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        fs_err::write(root.path().join("file"), [])?;
        let mut driver = Driver::new(NonZeroU32::MIN)?;
        let attempts = Cell::new(0);
        assert_eq!(
            driver.scan_with(root.path(), &mut |_, _| {
                attempts.set(attempts.get() + 1);
                Err(Errno::AGAIN.into())
            })?,
            None
        );
        assert_eq!(attempts.get(), MAX_STALLED_ATTEMPTS);
        assert!(driver.pending.is_none());
        assert!(driver.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_backend_retires_partial_submission_before_reclaiming_requests() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let names = (0..4)
            .map(|index| {
                let name = format!("file-{index}");
                fs_err::write(root.path().join(&name), [])?;
                CString::new(name).map_err(io::Error::other)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let directory = Arc::new(open(
            root.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(directory, names);
        batch.drop_counter = Some(Arc::clone(&counter));
        let mut driver = Driver::new(NonZeroU32::new(4).expect("non-zero queue depth"))?;
        driver.pending = Some(batch);
        let submitted = Cell::new(0);
        assert!(!driver.complete_batch(&mut |ring, _| {
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            let count = loop {
                // SAFETY: Submit only the first valid statx entry, with no wait, signal mask, or
                // extended arguments. The driver still owns every request in the batch.
                match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                    Ok(count) => break count,
                    Err(error) if retryable_control_error(&error) => {}
                    Err(error) => return Err(error),
                }
            };
            submitted.set(count);
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            Err(Errno::PERM.into())
        }));
        assert_eq!(submitted.get(), 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(driver.pending.is_none());
        assert!(driver.ring.is_none());
        assert_eq!(
            driver.scan_with(&root.path().join("missing"), &mut IoUring::submit_and_wait)?,
            None
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_backend_retains_requests_when_draining_is_unavailable() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        fs_err::write(root.path().join("file"), [])?;
        let directory = Arc::new(open(
            root.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(directory, vec![c"file".to_owned()]);
        batch.drop_counter = Some(Arc::clone(&counter));
        let mut driver = Driver::new(NonZeroU32::MIN)?;
        driver.pending = Some(batch);
        driver.queue()?;
        let ring = driver.ring.as_ref().expect("initialized ring");
        let submitted = loop {
            // SAFETY: The driver owns the queued statx request and its stable storage. Do not
            // wait or consume its CQE, so reclamation still requires the driver's drain barrier.
            match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                Ok(count) => break count,
                Err(error) if retryable_control_error(&error) => {}
                Err(error) => return Err(error),
            }
        };
        assert_eq!(submitted, 1);

        // Simulate completion control becoming inaccessible after a real submission. This
        // deliberately retains one bounded batch until this test process exits.
        driver.retire_after_drain(Err(Errno::PERM.into()));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(driver.pending.is_none());
        assert!(driver.ring.is_none());
        assert_eq!(
            driver.scan_with(&root.path().join("missing"), &mut IoUring::submit_and_wait)?,
            None
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_backend_keeps_in_flight_metadata_alive_during_retirement() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let names = (0..4)
            .map(|index| {
                let name = format!("file-{index}");
                fs_err::write(root.path().join(&name), [])?;
                CString::new(name).map_err(io::Error::other)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let directory = Arc::new(open(
            root.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(directory, names);
        batch.drop_counter = Some(Arc::clone(&counter));
        let mut driver = Driver::new(NonZeroU32::new(4).expect("non-zero queue depth"))?;
        driver.pending = Some(batch);
        let (reader, mut writer) = UnixStream::pair()?;

        // The first request blocks on an unreadable socket. Linking the following statx to it
        // makes that metadata buffer remain in flight until the observer releases the socket.
        // The two remaining statx entries stay in the userspace submission queue.
        {
            let ring = driver.ring.as_mut().expect("initialized ring");
            let batch = driver.pending.as_mut().expect("owned pending batch");
            let poll = opcode::PollAdd::new(
                types::Fd(reader.as_raw_fd()),
                linux_raw_sys::general::POLLIN,
            )
            .build()
            .flags(squeue::Flags::IO_LINK)
            .user_data(0);
            let mut submissions = ring.submission();
            // SAFETY: The socket remains open until the batch is drained. The dummy first
            // request provides completion bookkeeping and has no kernel-accessed metadata.
            unsafe { submissions.push(&poll) }.map_err(io::Error::other)?;
            batch.queued += 1;
            for (index, request) in batch.requests.iter().enumerate().skip(1) {
                let entry = request.statx_entry(&batch.directory, index as u64);
                // SAFETY: The driver owns the same stable request storage used by its ordinary
                // queue path, and retains it until all submitted requests have completed.
                unsafe { submissions.push(&entry) }.map_err(io::Error::other)?;
                batch.queued += 1;
            }
        }

        let (observer, observed) = mpsc::sync_channel(0);
        driver.drain_observer = Some(observer);
        let submitted = Cell::new(0);
        std::thread::scope(|scope| {
            let counter = Arc::clone(&counter);
            let release = scope.spawn(move || -> io::Result<()> {
                let observation = observed.recv_timeout(Duration::from_secs(10));
                let requests_alive = counter.load(Ordering::SeqCst) == 0;
                // Always release the poll, including when the observation fails, so a failing
                // assertion cannot leave the driver's cleanup waiting forever.
                let released = writer.write_all(b"x");
                observation.map_err(io::Error::other)?;
                assert!(
                    requests_alive,
                    "pending metadata was reclaimed before completion"
                );
                released
            });
            let completed = driver.complete_queued_batch(&mut |ring, _| {
                let count = loop {
                    // SAFETY: Submit only the poll/statx pair, with no wait or signal mask. The
                    // driver continues to own the metadata, directory, and unsubmitted entries.
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
            assert!(!completed);
            Ok::<(), io::Error>(())
        })?;
        assert_eq!(submitted.get(), 2);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(driver.pending.is_none());
        assert!(driver.ring.is_none());
        Ok(())
    }
}
