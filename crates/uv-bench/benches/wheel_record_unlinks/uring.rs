// Benchmark-only, all-independent-leaves `UNLINKAT` driver. This is not wheel uninstall.

use std::cell::Cell;
use std::ffi::CString;
use std::io;
use std::mem;
use std::num::NonZeroU32;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use io_uring::{EnterFlags, IoUring, Probe, opcode, squeue, types};
use rustix::fs::{Mode, OFlags, open};
use rustix::io::Errno;

use super::fixture::{FixtureLease, OwnedLeafTrial};
use super::leaf::{Outcomes, successful};

const MAX_STALLED_ATTEMPTS: usize = 64;

#[derive(Clone, Copy, Default)]
pub(super) struct Activity {
    completed: u64,
    successful: u64,
}

pub(super) struct Unlinker {
    ring: Option<IoUring>,
    capacity: usize,
    pending: Option<PendingBatch>,
    activity: Activity,
    #[cfg(test)]
    drain_observer: Option<std::sync::mpsc::SyncSender<()>>,
}

impl Unlinker {
    pub(super) fn new(queue_depth: NonZeroU32) -> io::Result<Self> {
        // Do not enable SQPOLL: retirement relies on being the only task that can submit SQEs.
        let ring: IoUring = IoUring::builder().dontfork().build(queue_depth.get())?;
        let mut probe = Probe::new();
        ring.submitter().register_probe(&mut probe)?;
        if !probe.is_supported(opcode::UnlinkAt::CODE) {
            return Err(unavailable("io_uring unlinkat is unavailable"));
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
            activity: Activity::default(),
            #[cfg(test)]
            drain_observer: None,
        })
    }

    pub(super) fn activity(&self) -> Activity {
        self.activity
    }

    /// Query the submitting task's effective io-wq limits after a successful real unlink.
    pub(super) fn worker_limits(&self) -> io::Result<Option<[u32; 2]>> {
        if self.activity.successful == 0 || self.pending.is_some() {
            return Err(invalid_completion());
        }
        let ring = self.ring.as_ref().ok_or_else(invalid_completion)?;
        let mut previous = [0, 0];
        match ring.submitter().register_iowq_max_workers(&mut previous) {
            Ok(()) => Ok(Some(previous)),
            Err(error) if unavailable_operation(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Every leaf has one terminal CQE. There is no ordinary-I/O fallback in this driver.
    pub(super) fn assert_activity(&self, before: Activity, expected: usize, removed: usize) {
        assert_eq!(
            self.activity.completed - before.completed,
            u64::try_from(expected).expect("Expected request count should fit in u64"),
            "unexpected number of UNLINKAT completions"
        );
        assert_eq!(
            self.activity.successful - before.successful,
            u64::try_from(removed).expect("Successful request count should fit in u64"),
            "unexpected number of successful UNLINKAT completions"
        );
    }

    /// Attempt all independent leaves, returning one ordered filesystem result per leaf.
    /// A control/retirement error is an outer error and permanently retires the ring.
    pub(super) fn unlink(&mut self, trial: &OwnedLeafTrial) -> io::Result<Outcomes> {
        self.unlink_with(trial, &mut IoUring::submit_and_wait)
    }

    fn unlink_with(
        &mut self,
        trial: &OwnedLeafTrial,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> io::Result<Outcomes> {
        if self.pending.is_some() {
            self.retire();
            return Err(invalid_completion());
        }
        if self.ring.is_none() {
            return Err(unavailable("io_uring unlinker was retired"));
        }
        let lease = trial.lease();
        if lease.is_quarantined() {
            return Err(unavailable("mutation fixture was quarantined"));
        }
        // The opaque trial has already audited the complete tree, all scheme destinations, and
        // symlink ancestors. Reject malformed relative names before submitting any mutation too.
        validate_relative_paths(trial.relative_paths())?;
        let directory = Arc::new(open(
            lease.path(),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?);
        let before = self.activity();
        let mut output = Vec::with_capacity(trial.relative_paths().len());
        for paths in trial.relative_paths().chunks(self.capacity) {
            self.unlink_batch_with(
                Arc::clone(&directory),
                Arc::clone(&lease),
                paths,
                &mut output,
                submit,
            )?;
        }
        self.assert_activity(before, trial.relative_paths().len(), successful(&output));
        Ok(output)
    }

    fn unlink_batch_with(
        &mut self,
        directory: Arc<OwnedFd>,
        lease: Arc<FixtureLease>,
        paths: &[PathBuf],
        output: &mut Outcomes,
        submit: &mut impl FnMut(&IoUring, usize) -> io::Result<usize>,
    ) -> io::Result<()> {
        if self.pending.is_some() {
            self.retire();
            return Err(invalid_completion());
        }
        if self.ring.is_none() {
            return Err(unavailable("io_uring unlinker was retired"));
        }
        self.pending = Some(PendingBatch::new(directory, lease, paths)?);
        if let Err(error) = self.queue() {
            self.retire();
            return Err(error);
        }
        self.complete_queued_batch(submit)?;
        let extracted = self
            .pending
            .as_ref()
            .ok_or_else(invalid_completion)?
            .requests
            .iter()
            .try_for_each(|request| {
                output.push(request.outcome()?);
                Ok::<(), io::Error>(())
            });
        if let Err(error) = extracted {
            self.retire();
            return Err(error);
        }
        drop(self.pending.take());
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
        if !submissions.is_empty() || batch.requests.len() > self.capacity {
            return Err(invalid_completion());
        }
        for (index, request) in batch.requests.iter().enumerate() {
            let entry = request.unlink_entry(&batch.directory, index as u64);
            // SAFETY: The pending batch owns the root FD, each stable CString, and the fixture
            // lease. All three survive every submitted completion, including fatal retirement.
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
            batch.completed += 1;
            self.activity.completed += 1;
            if completion.result() == 0 {
                self.activity.successful += 1;
            }
        }
        Ok(())
    }

    fn retire(&mut self) {
        let drained = self.drain_submitted();
        self.retire_after_drain(&drained);
    }

    fn retire_after_drain(&mut self, drained: &io::Result<()>) {
        if drained.is_err()
            && let Some(batch) = self.pending.take()
        {
            // Ring close/cancellation does not prove that a 5.x io-wq request has stopped using
            // its pathname or resolving its dirfd. Retain this one bounded batch, including the
            // FD and the owning fixture lease, and never recreate its quarantined pathname.
            batch.lease.quarantine();
            mem::forget(batch);
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
            // unsubmitted SQEs untouched; the batch owns all kernel-facing names, FDs, and roots.
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

impl Drop for Unlinker {
    fn drop(&mut self) {
        if self.pending.is_some() {
            self.retire();
        }
    }
}

struct PendingBatch {
    directory: Arc<OwnedFd>,
    lease: Arc<FixtureLease>,
    requests: Box<[Request]>,
    queued: usize,
    completed: usize,
    valid_completions: bool,
    #[cfg(test)]
    drop_counter: Option<Arc<AtomicUsize>>,
}

impl PendingBatch {
    fn new(
        directory: Arc<OwnedFd>,
        lease: Arc<FixtureLease>,
        paths: &[PathBuf],
    ) -> io::Result<Self> {
        Ok(Self {
            directory,
            lease,
            requests: paths
                .iter()
                .map(|path| Request::new(path))
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
    path: CString,
    result: Cell<Option<i32>>,
}

impl Request {
    fn new(path: &Path) -> io::Result<Self> {
        Ok(Self {
            path: CString::new(path.as_os_str().as_bytes())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
            result: Cell::new(None),
        })
    }

    fn unlink_entry(&self, directory: &OwnedFd, user_data: u64) -> squeue::Entry {
        // Flags zero removes a leaf symlink itself and deliberately does not remove directories.
        opcode::UnlinkAt::new(types::Fd(directory.as_raw_fd()), self.path.as_ptr())
            .build()
            .user_data(user_data)
    }

    fn outcome(&self) -> io::Result<io::Result<()>> {
        match self.result.get().ok_or_else(invalid_completion)? {
            0 => Ok(Ok(())),
            result if result < 0 => Ok(Err(io::Error::from_raw_os_error(
                result.checked_neg().ok_or_else(invalid_completion)?,
            ))),
            _ => Err(invalid_completion()),
        }
    }
}

fn validate_relative_paths(paths: &[PathBuf]) -> io::Result<()> {
    for path in paths {
        let mut components = path.components().peekable();
        if components.peek().is_none()
            || !components.all(|component| matches!(component, Component::Normal(_)))
            || path.as_os_str().as_bytes().contains(&0)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unlink leaf must be a nonempty normalized relative path",
            ));
        }
    }
    Ok(())
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
                    "io_uring unlink requests made no progress",
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
        "invalid io_uring wheel-unlink completion",
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
