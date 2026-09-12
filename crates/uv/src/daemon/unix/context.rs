//! Process properties that cannot be inferred from the command's arguments or environment.

use std::io;
use std::mem::MaybeUninit;
#[cfg(target_os = "macos")]
use std::mem::size_of;
use std::os::unix::net::UnixStream;

#[cfg(target_os = "macos")]
use anyhow::bail;
use anyhow::{Context, Result, ensure};
use nix::errno::Errno;
use nix::libc;
use nix::sys::signal::{self, SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal};
use nix::sys::socket::{getsockopt, sockopt};
use nix::sys::stat::{Mode, umask};
use nix::unistd::{Pid, getpgid, getpid, getsid, setpgid};
use serde::{Deserialize, Serialize};

#[cfg(all(target_os = "linux", any(target_env = "gnu", target_env = "uclibc")))]
type ResourceId = libc::__rlimit_resource_t;
#[cfg(not(all(target_os = "linux", any(target_env = "gnu", target_env = "uclibc"))))]
type ResourceId = libc::c_int;

#[derive(Serialize, Deserialize)]
pub(super) struct ProcessContext {
    pid: i32,
    process_group: i32,
    session: i32,
    umask: libc::mode_t,
    limits: Vec<ResourceLimit>,
    ignored_signals: Vec<i32>,
    blocked_signals: Vec<i32>,
}

#[derive(Serialize, Deserialize)]
struct ResourceLimit {
    resource: ResourceId,
    soft: libc::rlim_t,
    hard: libc::rlim_t,
}

impl ProcessContext {
    pub(super) fn capture() -> Result<Self> {
        let previous_umask = umask(Mode::empty());
        umask(previous_umask);
        let mut mask = SigSet::empty();
        signal::pthread_sigmask(SigmaskHow::SIG_SETMASK, None, Some(&mut mask))?;
        Ok(Self {
            pid: getpid().as_raw(),
            process_group: getpgid(None)?.as_raw(),
            session: getsid(None)?.as_raw(),
            umask: previous_umask.bits(),
            limits: resource_limits()?,
            ignored_signals: ignored_signals()?,
            blocked_signals: mask.iter().map(|signal| signal as i32).collect(),
        })
    }

    pub(super) fn validate(&self, stream: &UnixStream) -> Result<()> {
        ensure!(
            self.pid == peer_pid(stream)?,
            "Caller PID does not match socket peer"
        );
        let client_pid = Pid::from_raw(self.pid);
        ensure!(
            self.session == getsid(None)?.as_raw()
                && getsid(Some(client_pid))?.as_raw() == self.session
                && getpgid(Some(client_pid))?.as_raw() == self.process_group,
            "Caller process context changed"
        );
        compatible_confinement(self.pid)?;
        ensure!(
            process_priority(self.pid)? == process_priority(0)?,
            "Process priorities differ"
        );
        let current = resource_limits()?;
        ensure!(
            self.limits.len() == current.len(),
            "Resource limit set changed"
        );
        for (requested, current) in self.limits.iter().zip(current) {
            ensure!(
                requested.resource == current.resource && requested.hard <= current.hard,
                "Daemon cannot reproduce the caller's resource limits"
            );
        }
        Ok(())
    }

    pub(super) fn restore(self) -> Result<()> {
        restore_context(self)
    }
}

/// Return the kernel-authenticated process on the other end of a local socket.
pub(super) fn peer_pid(stream: &UnixStream) -> Result<i32> {
    #[cfg(target_os = "macos")]
    return Ok(getsockopt(stream, sockopt::LocalPeerPid)?);
    #[cfg(target_os = "linux")]
    return Ok(getsockopt(stream, sockopt::PeerCredentials)?.pid());
}

#[allow(unsafe_code)]
fn restore_context(context: ProcessContext) -> Result<()> {
    setpgid(Pid::from_raw(0), Pid::from_raw(context.process_group))
        .context("Failed to restore process group")?;
    umask(Mode::from_bits_truncate(context.umask));
    for limit in context.limits {
        let value = libc::rlimit {
            rlim_cur: limit.soft,
            rlim_max: limit.hard,
        };
        // SAFETY: The resource was validated against the platform's own resource list, and
        // value points to an initialized rlimit for the duration of the call.
        if unsafe { libc::setrlimit(limit.resource, &raw const value) } != 0 {
            return Err(io::Error::last_os_error())
                .with_context(|| format!("Failed to restore resource limit {}", limit.resource));
        }
    }
    for signal in Signal::iterator() {
        if matches!(signal, Signal::SIGKILL | Signal::SIGSTOP) {
            continue;
        }
        let handler = if context.ignored_signals.contains(&(signal as i32)) {
            SigHandler::SigIgn
        } else {
            SigHandler::SigDfl
        };
        let action = SigAction::new(handler, SaFlags::empty(), SigSet::empty());
        // SAFETY: Only the default and ignored dispositions are installed. This child has
        // not started uv's runtime or any other threads.
        unsafe { signal::sigaction(signal, &action) }
            .with_context(|| format!("Failed to restore {signal}"))?;
    }
    let mut mask = SigSet::empty();
    for signal in context.blocked_signals {
        mask.add(Signal::try_from(signal)?);
    }
    signal::pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&mask), None)?;
    Ok(())
}

#[allow(unsafe_code)]
fn resource_limits() -> Result<Vec<ResourceLimit>> {
    let mut limits = Vec::new();
    let resources = [
        libc::RLIMIT_CPU,
        libc::RLIMIT_FSIZE,
        libc::RLIMIT_DATA,
        libc::RLIMIT_STACK,
        libc::RLIMIT_CORE,
        libc::RLIMIT_AS,
        libc::RLIMIT_MEMLOCK,
        libc::RLIMIT_NPROC,
        libc::RLIMIT_NOFILE,
        #[cfg(target_os = "linux")]
        libc::RLIMIT_RSS,
        #[cfg(target_os = "linux")]
        libc::RLIMIT_LOCKS,
        #[cfg(target_os = "linux")]
        libc::RLIMIT_SIGPENDING,
        #[cfg(target_os = "linux")]
        libc::RLIMIT_MSGQUEUE,
        #[cfg(target_os = "linux")]
        libc::RLIMIT_NICE,
        #[cfg(target_os = "linux")]
        libc::RLIMIT_RTPRIO,
        #[cfg(target_os = "linux")]
        libc::RLIMIT_RTTIME,
    ];
    for resource in resources {
        let mut value = MaybeUninit::<libc::rlimit>::uninit();
        // SAFETY: value has space for a complete rlimit and is read only after getrlimit succeeds.
        if unsafe { libc::getrlimit(resource, value.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error())
                .with_context(|| format!("Failed to read resource limit {resource}"));
        }
        // SAFETY: getrlimit initialized value.
        let value = unsafe { value.assume_init() };
        limits.push(ResourceLimit {
            resource,
            soft: value.rlim_cur,
            hard: value.rlim_max,
        });
    }
    Ok(limits)
}

#[allow(unsafe_code)]
fn ignored_signals() -> Result<Vec<i32>> {
    let mut ignored = Vec::new();
    for signal in Signal::iterator() {
        if matches!(signal, Signal::SIGKILL | Signal::SIGSTOP) {
            continue;
        }
        let mut action = MaybeUninit::<libc::sigaction>::uninit();
        // SAFETY: A null new action queries the disposition without changing it. action has
        // enough space for the result and is read only on success.
        if unsafe { libc::sigaction(signal as i32, std::ptr::null(), action.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error())
                .with_context(|| format!("Failed to read {signal} disposition"));
        }
        // SAFETY: sigaction initialized action.
        if unsafe { action.assume_init() }.sa_sigaction == libc::SIG_IGN {
            ignored.push(signal as i32);
        }
    }
    Ok(ignored)
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn compatible_confinement(pid: i32) -> Result<()> {
    #[link(name = "sandbox")]
    unsafe extern "C" {
        fn sandbox_check(
            pid: libc::pid_t,
            operation: *const libc::c_char,
            filter: libc::c_int,
            ...
        ) -> libc::c_int;
    }
    // SAFETY: A null operation and filter zero query whether a process has a sandbox. The
    // function does not dereference any Rust memory. Nonzero, including errors, fails closed.
    ensure!(
        unsafe { sandbox_check(pid, std::ptr::null(), 0) } == 0
            && unsafe { sandbox_check(getpid().as_raw(), std::ptr::null(), 0) } == 0,
        "Sandboxed commands must execute in their original process"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn compatible_confinement(pid: i32) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    use fs_err as fs;

    let caller = format!("/proc/{pid}");
    let caller_status = fs::read_to_string(format!("{caller}/status"))?;
    let server_status = fs::read_to_string("/proc/self/status")?;
    let field = |status: &str, name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::trim)
            .map(str::to_owned)
    };
    for name in [
        "NoNewPrivs:",
        "Seccomp:",
        "CapInh:",
        "CapPrm:",
        "CapEff:",
        "CapAmb:",
    ] {
        for status in [&caller_status, &server_status] {
            let value = field(status, name)
                .ok_or_else(|| anyhow::anyhow!("Missing process field {name}"))?;
            ensure!(
                !value.is_empty() && value.bytes().all(|byte| byte == b'0'),
                "Restricted commands must execute in their original process"
            );
        }
    }
    for name in [
        "Uid:",
        "Gid:",
        "Groups:",
        "CapBnd:",
        "Cpus_allowed:",
        "Mems_allowed:",
    ] {
        let caller_field = field(&caller_status, name);
        ensure!(
            caller_field.is_some() && caller_field == field(&server_status, name),
            "Process credentials differ"
        );
    }
    for namespace in fs::read_dir("/proc/self/ns")? {
        let namespace = namespace?.file_name();
        ensure!(
            fs::read_link(std::path::Path::new(&caller).join("ns").join(&namespace))?
                == fs::read_link(std::path::Path::new("/proc/self/ns").join(&namespace))?,
            "Process namespaces differ"
        );
    }
    let caller_root = fs::metadata(format!("{caller}/root"))?;
    let server_root = fs::metadata("/proc/self/root")?;
    ensure!(
        caller_root.dev() == server_root.dev() && caller_root.ino() == server_root.ino(),
        "Process roots differ"
    );
    for path in ["cgroup", "attr/current"] {
        let read = |root: &str| match fs::read(format!("{root}/{path}")) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        };
        ensure!(
            read(&caller)? == read("/proc/self")?,
            "Process confinement differs"
        );
    }
    Ok(())
}

pub(super) fn check_delegation() -> Result<()> {
    compatible_confinement(getpid().as_raw())?;
    ensure!(
        !has_extra_inheritable_descriptors()?,
        "Commands with additional inheritable file descriptors must execute in their original process"
    );
    Ok(())
}

#[allow(unsafe_code)]
fn process_priority(pid: i32) -> Result<i32> {
    Errno::clear();
    // SAFETY: getpriority accepts a numeric process ID and does not access Rust memory.
    let priority = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid.try_into()?) };
    if priority == -1 && Errno::last_raw() != 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(priority)
}

#[allow(unsafe_code)]
fn has_extra_inheritable_descriptors() -> Result<bool> {
    #[cfg(target_os = "linux")]
    let directory = "/proc/self/fd";
    #[cfg(target_os = "macos")]
    let directory = "/dev/fd";
    for entry in fs_err::read_dir(directory)? {
        let Some(descriptor) = entry?
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<i32>().ok())
        else {
            continue;
        };
        if descriptor <= 2 {
            continue;
        }
        // SAFETY: F_GETFD only inspects the numeric descriptor and has no pointer argument.
        let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
        if flags == -1 {
            if Errno::last() == Errno::EBADF {
                continue;
            }
            return Err(io::Error::last_os_error().into());
        }
        if flags & libc::FD_CLOEXEC == 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "linux")]
pub(super) fn ensure_single_threaded() -> Result<()> {
    ensure!(
        fs_err::read_dir("/proc/self/task")?.count() == 1,
        "Daemon parent is not single-threaded"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub(super) fn ensure_single_threaded() -> Result<()> {
    let mut info = MaybeUninit::<libc::proc_taskinfo>::uninit();
    let expected_size = i32::try_from(size_of::<libc::proc_taskinfo>())?;
    // SAFETY: info is a correctly sized output buffer for PROC_PIDTASKINFO, and is read only
    // after the API reports that it initialized the complete structure.
    let size = unsafe {
        libc::proc_pidinfo(
            getpid().as_raw(),
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            expected_size,
        )
    };
    if size != expected_size {
        bail!("Could not inspect daemon threads");
    }
    // SAFETY: proc_pidinfo initialized the complete structure.
    ensure!(
        unsafe { info.assume_init() }.pti_threadnum == 1,
        "Daemon parent is not single-threaded"
    );
    Ok(())
}
