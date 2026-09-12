//! Wake the single-threaded fork server when a child exits.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result, bail, ensure};
use nix::errno::Errno;
use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
use nix::libc;
use nix::sys::signal::{self, SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal};
use nix::unistd::{pipe, read};

use super::context;

static NOTIFY_FD: AtomicI32 = AtomicI32::new(-1);

struct NotificationPipe {
    reader: OwnedFd,
    writer: OwnedFd,
}

impl NotificationPipe {
    fn new() -> Result<Self> {
        // macOS does not provide pipe2. The fork server has no other threads, so setting these
        // flags immediately after creation cannot race another thread's fork or exec.
        let (reader, writer) = pipe()?;
        for descriptor in [&reader, &writer] {
            let descriptor_flags =
                FdFlag::from_bits_truncate(fcntl(descriptor, FcntlArg::F_GETFD)?);
            fcntl(
                descriptor,
                FcntlArg::F_SETFD(descriptor_flags | FdFlag::FD_CLOEXEC),
            )?;
            let status_flags = OFlag::from_bits_truncate(fcntl(descriptor, FcntlArg::F_GETFL)?);
            fcntl(
                descriptor,
                FcntlArg::F_SETFL(status_flags | OFlag::O_NONBLOCK),
            )?;
        }
        Ok(Self { reader, writer })
    }

    fn drain(&self) -> Result<()> {
        let mut buffer = [0; 256];
        loop {
            match read(&self.reader, &mut buffer) {
                Ok(0) => bail!("Daemon child-notification pipe closed"),
                Ok(_) => {}
                Err(Errno::EAGAIN) => return Ok(()),
                Err(Errno::EINTR) => {}
                Err(error) => return Err(error).context("Failed to drain child notifications"),
            }
        }
    }
}

impl AsFd for NotificationPipe {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.reader.as_fd()
    }
}

/// A SIGCHLD handler and its nonblocking notification pipe.
///
/// Notifications are only wakeup hints. Drain the pipe before reaping all exited children so an
/// exit between the reap loop and poll leaves a readable notification behind.
pub(super) struct ChildNotifications {
    pipe: NotificationPipe,
    previous_action: SigAction,
    previous_mask: SigSet,
}

impl ChildNotifications {
    #[allow(unsafe_code)]
    pub(super) fn install() -> Result<Self> {
        context::ensure_single_threaded()?;
        ensure!(
            NOTIFY_FD.load(Ordering::Relaxed) == -1,
            "Daemon child notifications are already installed"
        );
        let pipe = NotificationPipe::new()?;
        let child_signal = SigSet::from(Signal::SIGCHLD);
        let previous_mask = child_signal.thread_swap_mask(SigmaskHow::SIG_BLOCK)?;
        let action = SigAction::new(
            SigHandler::Handler(notify_child_exit),
            SaFlags::SA_RESTART | SaFlags::SA_NOCLDSTOP,
            SigSet::empty(),
        );
        // SAFETY: SIGCHLD is blocked while the handler and its descriptor are installed. The
        // handler only performs an async-signal-safe write and accesses lock-free atomics/errno.
        let previous_action = match unsafe { signal::sigaction(Signal::SIGCHLD, &action) } {
            Ok(previous_action) => previous_action,
            Err(error) => {
                previous_mask.thread_set_mask()?;
                return Err(error).context("Failed to install child notifications");
            }
        };
        let notifications = Self {
            pipe,
            previous_action,
            previous_mask,
        };
        NOTIFY_FD.store(notifications.pipe.writer.as_raw_fd(), Ordering::Relaxed);
        // Both SIG_IGN and a blocked SIGCHLD mask survive exec. The server owns its children and
        // must receive their exits; each worker restores the requesting process's signal state.
        child_signal.thread_unblock()?;
        Ok(notifications)
    }

    pub(super) fn drain(&self) -> Result<()> {
        self.pipe.drain()
    }
}

impl AsFd for ChildNotifications {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.pipe.as_fd()
    }
}

impl Drop for ChildNotifications {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        let _ = SigSet::from(Signal::SIGCHLD).thread_block();
        NOTIFY_FD.store(-1, Ordering::Relaxed);
        // SAFETY: The saved action came from sigaction. The single-threaded process blocks
        // SIGCHLD and disarms the handler before the owned pipe descriptors can be closed.
        let _ = unsafe { signal::sigaction(Signal::SIGCHLD, &self.previous_action) };
        let _ = self.previous_mask.thread_set_mask();
    }
}

extern "C" fn notify_child_exit(_signal: libc::c_int) {
    let descriptor = NOTIFY_FD.load(Ordering::Relaxed);
    if descriptor >= 0 {
        write_notification(descriptor);
    }
}

#[allow(unsafe_code)]
fn write_notification(descriptor: RawFd) {
    let previous_errno = Errno::last_raw();
    let byte = 1_u8;
    loop {
        // SAFETY: The handler's descriptor is a live nonblocking pipe writer until it is
        // disarmed. write is async-signal-safe, and byte remains valid for this one-byte write.
        let written = unsafe { libc::write(descriptor, (&raw const byte).cast(), 1) };
        if written != -1 || Errno::last_raw() != libc::EINTR {
            // EAGAIN means a previous notification already makes the pipe readable.
            break;
        }
    }
    Errno::set_raw(previous_errno);
}

#[cfg(test)]
mod tests {
    use std::os::fd::{AsFd, AsRawFd};

    use anyhow::{Result, bail};
    use nix::errno::Errno;
    use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use nix::unistd::write;

    use super::{NotificationPipe, write_notification};

    #[test]
    fn notification_pipe_preserves_queued_wakeups() -> Result<()> {
        let pipe = NotificationPipe::new()?;
        for descriptor in [&pipe.reader, &pipe.writer] {
            assert!(
                FdFlag::from_bits_truncate(fcntl(descriptor, FcntlArg::F_GETFD)?)
                    .contains(FdFlag::FD_CLOEXEC)
            );
            assert!(
                OFlag::from_bits_truncate(fcntl(descriptor, FcntlArg::F_GETFL)?)
                    .contains(OFlag::O_NONBLOCK)
            );
        }

        Errno::EDOM.set();
        write_notification(pipe.writer.as_raw_fd());
        assert_eq!(Errno::last(), Errno::EDOM);
        let mut descriptors = [PollFd::new(pipe.as_fd(), PollFlags::POLLIN)];
        assert_eq!(poll(&mut descriptors, PollTimeout::ZERO)?, 1);
        pipe.drain()?;
        assert_eq!(poll(&mut descriptors, PollTimeout::ZERO)?, 0);

        loop {
            match write(&pipe.writer, &[1; 4096]) {
                Ok(0) => bail!("notification pipe accepted no bytes"),
                Ok(_) => {}
                Err(Errno::EAGAIN) => break,
                Err(Errno::EINTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
        loop {
            match write(&pipe.writer, &[1]) {
                Ok(0) => bail!("notification pipe accepted no bytes"),
                Ok(_) => {}
                Err(Errno::EAGAIN) => break,
                Err(Errno::EINTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Errno::EDOM.set();
        write_notification(pipe.writer.as_raw_fd());
        assert_eq!(Errno::last(), Errno::EDOM);
        assert_eq!(poll(&mut descriptors, PollTimeout::ZERO)?, 1);
        pipe.drain()?;
        assert_eq!(poll(&mut descriptors, PollTimeout::ZERO)?, 0);
        Ok(())
    }
}
