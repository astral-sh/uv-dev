//! A session-local, single-threaded fork server.
//!
//! The parent may parse lockfiles and context-free project-manifest syntax, but must never enter
//! uv's command runner, initialize its process-global settings, start a runtime, or create
//! background threads. Each child starts uv exactly once, after restoring the invoking
//! process's execution context.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::DirBuilder;
use std::io::{self, IoSlice, IoSliceMut, IsTerminal, Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use fs_err::os::unix::fs::OpenOptionsExt;
use fs_err::{self as fs, File, OpenOptions};
use nix::errno::Errno;
use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::signal::{self, SaFlags, SigAction, SigHandler, SigSet, Signal};
use nix::sys::socket::{ControlMessage, ControlMessageOwned, MsgFlags, recvmsg, sendmsg};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{
    ForkResult, dup2_stderr, dup2_stdin, dup2_stdout, fchdir, fork, getegid, geteuid, getpid,
    getsid,
};
use serde::{Deserialize, Serialize};

use uv_cache::Cache;
use uv_cache_key::hash_digest;
use uv_cli::Cli;
use uv_resolver::Lock;
use uv_workspace::pyproject::PyProjectToml;

use super::Bootstrap;

mod context;
mod manifests;
mod reaper;
use context::ProcessContext;
use reaper::ChildNotifications;

const SERVER_ARG: &str = "--internal-daemon-server";
const MAX_FRAME: usize = 128 * 1024 * 1024;
const MAX_WORKERS: usize = 64;
const IDLE_TIMEOUT: Duration = Duration::from_mins(15);
static WORKER: AtomicBool = AtomicBool::new(false);
static CACHE_SOCKET: OnceLock<PathBuf> = OnceLock::new();
static FORWARD_PID: AtomicI32 = AtomicI32::new(0);
static INTERACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Deserialize)]
enum Request {
    Ping,
    Run(Invocation),
    Warm {
        contents: String,
        directory: Vec<u8>,
    },
    Hit,
    Stop,
    // Followed by a separately bounded manifest-cache frame.
    Manifests,
}

#[derive(Serialize, Deserialize)]
enum Response {
    Status(Status),
    Accepted(i32),
    Completed(u8),
    Observed,
    RunLocally(String),
    Error(String),
}

#[derive(Serialize, Deserialize)]
struct Invocation {
    args: Vec<Vec<u8>>,
    environment: Vec<(Vec<u8>, Vec<u8>)>,
    context: ProcessContext,
}

#[derive(Serialize, Deserialize)]
struct Status {
    pid: i32,
    session: i32,
    active_workers: usize,
    completed_requests: u64,
    cached_locks: usize,
    cached_source_bytes: usize,
    cache_hits: u64,
    cached_manifests: usize,
    cached_manifest_source_bytes: usize,
    manifest_cache_hits: u64,
    manifest_cache_dropped: u64,
    stopping: bool,
}

struct Paths {
    root: PathBuf,
    enabled: PathBuf,
    socket: PathBuf,
    lock: PathBuf,
    log: PathBuf,
}

impl Paths {
    fn new() -> Result<Self> {
        let executable = std::env::current_exe()?;
        let metadata = fs::metadata(&executable)?;
        let identity = hash_digest(&(
            executable,
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            uv_version::version(),
        ));
        let session = getsid(None)?;
        let root = std::path::absolute(match std::env::var_os("UV_DAEMON_DIR") {
            Some(root) => PathBuf::from(root),
            None => Cache::from_settings(false, None)?.root().join("daemon-v1"),
        })?;
        Ok(Self {
            enabled: root.join(format!("{identity}.enabled")),
            socket: root.join(format!("{identity}-{session}.sock")),
            lock: root.join(format!("{identity}-{session}.lock")),
            log: root.join(format!("{identity}-{session}.log")),
            root,
        })
    }

    fn initialize(&self) -> Result<()> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.root)?;
        let metadata = fs::symlink_metadata(&self.root)?;
        ensure!(
            metadata.is_dir()
                && metadata.uid() == geteuid().as_raw()
                && metadata.mode() & 0o777 == 0o700,
            "Daemon directory must be a private, user-owned directory: {}",
            self.root.display()
        );
        Ok(())
    }

    fn enable(&self) -> Result<()> {
        self.initialize()?;
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.enabled)?;
        Ok(())
    }
}

pub(super) fn bootstrap(args: Vec<OsString>) -> Result<Bootstrap> {
    if args.len() == 3 && args[1] == SERVER_ARG {
        let paths = Paths::new()?;
        ensure!(Path::new(&args[2]) == paths.socket, "Invalid daemon socket");
        return serve(paths);
    }
    if args.len() != 2 {
        return Ok(Bootstrap::Continue(args));
    }
    if args[1] == "--daemon" {
        context::check_delegation().context("The experimental daemon cannot be enabled")?;
        let paths = Paths::new()?;
        paths.enable()?;
        let mut stream = ensure_server(&paths)?;
        write_frame(&mut stream, &Request::Ping)?;
        display_status(read_frame(&mut stream)?)?;
        return Ok(Bootstrap::Exit(ExitCode::SUCCESS));
    }
    if args[1] == "--daemon-status" {
        let paths = Paths::new()?;
        let mut stream = connect(&paths.socket).context("Daemon is not running")?;
        write_frame(&mut stream, &Request::Ping)?;
        display_status(read_frame(&mut stream)?)?;
        return Ok(Bootstrap::Exit(ExitCode::SUCCESS));
    }
    if args[1] == "--no-daemon" {
        let paths = Paths::new()?;
        match fs::remove_file(&paths.enabled) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if let Ok(mut stream) = connect(&paths.socket) {
            stream.set_read_timeout(None)?;
            write_frame(&mut stream, &Request::Stop)?;
            display_status(read_frame(&mut stream)?)?;
        }
        return Ok(Bootstrap::Exit(ExitCode::SUCCESS));
    }
    Ok(Bootstrap::Continue(args))
}

pub(super) fn dispatch(args: &[OsString], cli: &Cli) -> Result<Option<ExitCode>> {
    if WORKER.load(Ordering::Relaxed)
        || cli.top_level.global_args.no_daemon
        || cli.top_level.cache_args.no_cache
    {
        return Ok(None);
    }
    let paths = match Paths::new() {
        Ok(paths) => paths,
        Err(_) if !cli.top_level.global_args.daemon => return Ok(None),
        Err(error) => return Err(error),
    };
    if !cli.top_level.global_args.daemon && !paths.enabled.try_exists().unwrap_or(false) {
        return Ok(None);
    }
    if let Err(error) = context::check_delegation() {
        if cli.top_level.global_args.daemon {
            return Err(error).context("The experimental daemon cannot execute this process");
        }
        return Ok(None);
    }
    if cli.top_level.global_args.daemon {
        paths.enable()?;
    }

    // Failures before submitting a request are safe to run locally. After submission, never
    // replay a command: it may already have changed an environment, lockfile, or remote service.
    let mut stream = match ensure_server(&paths) {
        Ok(stream) => stream,
        Err(_) if !cli.top_level.global_args.daemon => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let invocation = Invocation {
        args: args.iter().map(|arg| arg.as_bytes().to_vec()).collect(),
        environment: std::env::vars_os()
            .map(|(name, value)| (name.as_bytes().to_vec(), value.as_bytes().to_vec()))
            .collect(),
        context: ProcessContext::capture()?,
    };
    let directory = File::open(".")?;
    write_frame(&mut stream, &Request::Run(invocation))?;
    send_fds(&stream, &directory)?;
    stream.set_read_timeout(None)?;
    match read_frame(&mut stream)
        .context("Daemon request outcome is unknown; command was not retried")?
    {
        Response::Accepted(pid) => install_signal_forwarding(pid)?,
        Response::RunLocally(_reason) => {
            return Ok(None);
        }
        Response::Error(error) => bail!("Daemon rejected command: {error}"),
        Response::Status(_) | Response::Completed(_) | Response::Observed => {
            bail!("Invalid daemon response")
        }
    }
    match read_frame(&mut stream)
        .context("Daemon request outcome is unknown; command was not retried")?
    {
        Response::Completed(code) => Ok(Some(ExitCode::from(code))),
        Response::Error(error) => bail!("Daemon request failed: {error}"),
        Response::Status(_)
        | Response::Accepted(_)
        | Response::RunLocally(_)
        | Response::Observed => {
            bail!("Invalid daemon response")
        }
    }
}

fn display_status(response: Response) -> Result<()> {
    match response {
        Response::Status(status) => {
            serde_json::to_writer(io::stdout().lock(), &status)?;
            writeln!(io::stdout())?;
            Ok(())
        }
        Response::Error(error) => Err(anyhow!(error)),
        Response::Accepted(_)
        | Response::Completed(_)
        | Response::RunLocally(_)
        | Response::Observed => {
            bail!("Invalid daemon status response")
        }
    }
}

fn connect(path: &Path) -> Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    verify_peer(&stream)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    Ok(stream)
}

fn ensure_server(paths: &Paths) -> Result<UnixStream> {
    paths.initialize()?;
    if let Ok(stream) = connect(&paths.socket) {
        return Ok(stream);
    }
    let log = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&paths.log)?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg(SERVER_ARG)
        .arg(&paths.socket)
        // jemalloc must not start a background thread in the fork-server parent.
        .env("_RJEM_MALLOC_CONF", "background_thread:false")
        .env("MALLOC_CONF", "background_thread:false")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log.into_file()))
        .process_group(0)
        .spawn()?;
    for _ in 0..500 {
        if let Ok(stream) = connect(&paths.socket) {
            return Ok(stream);
        }
        if let Some(status) = child.try_wait()? {
            let mut stderr = String::new();
            File::open(&paths.log)?
                .take(64 * 1024)
                .read_to_string(&mut stderr)?;
            // Another starter may have won the lock and still be binding its socket.
            if !status.success() {
                bail!("Daemon exited with {status}: {}", stderr.trim());
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    bail!("Timed out starting uv daemon")
}

fn status(
    children: &BTreeMap<i32, UnixStream>,
    completed: u64,
    hits: u64,
    manifests: &manifests::Counters,
    stopping: bool,
) -> Result<Status> {
    let (cached_locks, cached_source_bytes) = Lock::process_cache_stats();
    let (cached_manifests, cached_manifest_source_bytes) = PyProjectToml::process_cache_stats();
    Ok(Status {
        pid: getpid().as_raw(),
        session: getsid(None)?.as_raw(),
        active_workers: children.len(),
        completed_requests: completed,
        cached_locks,
        cached_source_bytes,
        cache_hits: hits,
        cached_manifests,
        cached_manifest_source_bytes,
        manifest_cache_hits: manifests.hits,
        manifest_cache_dropped: manifests.dropped,
        stopping,
    })
}

#[allow(unsafe_code)]
fn serve(paths: Paths) -> Result<Bootstrap> {
    paths.initialize()?;
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&paths.lock)?;
    if lock_file.try_lock().is_err() {
        return Ok(Bootstrap::Exit(ExitCode::SUCCESS));
    }
    if let Ok(metadata) = fs::symlink_metadata(&paths.socket) {
        ensure!(
            metadata.file_type().is_socket() && metadata.uid() == geteuid().as_raw(),
            "Refusing to replace an unexpected daemon socket"
        );
        fs::remove_file(&paths.socket)?;
    }
    let listener = UnixListener::bind(&paths.socket)?;
    listener.set_nonblocking(true)?;
    let notifications = ChildNotifications::install()?;
    Lock::enable_process_cache();
    PyProjectToml::enable_process_cache();
    let mut children = BTreeMap::new();
    let mut stop_waiters = Vec::new();
    let mut completed = 0;
    let mut hits = 0;
    let mut manifest_counters = manifests::Counters::default();
    let mut last_activity = Instant::now();
    loop {
        // Drain before reaping: a child that exits after waitpid reports StillAlive must leave
        // a readable notification for the following poll, even if SIGCHLDs were coalesced.
        notifications.drain()?;
        loop {
            let (pid, code) = match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(pid, code)) => (pid, u8::try_from(code).unwrap_or(1)),
                Ok(WaitStatus::Signaled(pid, signal, _)) => (pid, 128 + signal as u8),
                Ok(WaitStatus::StillAlive) | Err(Errno::ECHILD) => break,
                Ok(_) => continue,
                Err(Errno::EINTR) => continue,
                Err(error) => return Err(error).context("Failed to reap daemon worker"),
            };
            if let Some(mut stream) = children.remove(&pid.as_raw()) {
                let _ = write_frame(&mut stream, &Response::Completed(code));
                completed += 1;
                last_activity = Instant::now();
            }
        }
        if children.is_empty()
            && (!stop_waiters.is_empty() || last_activity.elapsed() >= IDLE_TIMEOUT)
        {
            let response = Response::Status(status(
                &children,
                completed,
                hits,
                &manifest_counters,
                true,
            )?);
            for mut stream in stop_waiters {
                let _ = write_frame(&mut stream, &response);
            }
            fs::remove_file(&paths.socket)?;
            return Ok(Bootstrap::Exit(ExitCode::SUCCESS));
        }
        let timeout = if children.is_empty() {
            // Round up so a final fractional millisecond cannot become a zero-timeout loop.
            PollTimeout::try_from(
                IDLE_TIMEOUT
                    .saturating_sub(last_activity.elapsed())
                    .as_millis()
                    .saturating_add(1),
            )?
        } else {
            PollTimeout::NONE
        };
        let mut poll_fds = [
            PollFd::new(listener.as_fd(), PollFlags::POLLIN),
            PollFd::new(notifications.as_fd(), PollFlags::POLLIN),
        ];
        if let Err(error) = poll(&mut poll_fds, timeout) {
            if error == Errno::EINTR {
                continue;
            }
            return Err(error).context("Failed polling daemon events");
        }
        let [listener_poll, notification_poll] = poll_fds;
        let listener_events = listener_poll
            .revents()
            .context("Unknown daemon listener poll event")?;
        let notification_events = notification_poll
            .revents()
            .context("Unknown child-notification poll event")?;
        let failed = PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL;
        ensure!(
            !listener_events.intersects(failed),
            "Daemon listener failed"
        );
        ensure!(
            !notification_events.intersects(failed),
            "Daemon child-notification pipe failed"
        );
        if notification_events.contains(PollFlags::POLLIN)
            || !listener_events.contains(PollFlags::POLLIN)
        {
            continue;
        }
        let (mut stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).context("Failed accepting daemon connection"),
        };
        last_activity = Instant::now();
        // BSD sockets inherit the listener's nonblocking mode across accept.
        if stream.set_nonblocking(false).is_err()
            || verify_peer(&stream).is_err()
            || stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .is_err()
            || stream
                .set_write_timeout(Some(Duration::from_secs(10)))
                .is_err()
        {
            continue;
        }
        let Ok(request) = read_frame::<Request>(&mut stream) else {
            continue;
        };
        match request {
            Request::Ping => {
                let response = Response::Status(status(
                    &children,
                    completed,
                    hits,
                    &manifest_counters,
                    !stop_waiters.is_empty(),
                )?);
                let _ = write_frame(&mut stream, &response);
            }
            Request::Stop => stop_waiters.push(stream),
            Request::Warm {
                contents,
                directory,
            } => {
                // Only the parser's default, strict policy is cached by the parent. A worker
                // using a more permissive policy may miss this cache, but cannot weaken it.
                let directory = PathBuf::from(OsString::from_vec(directory));
                if directory.is_absolute() && context::ensure_single_threaded().is_ok() {
                    let previous = File::open(".")?;
                    if std::env::set_current_dir(directory).is_ok() {
                        let _ = Lock::from_toml_shared(&contents);
                        fchdir(previous).context("Failed to restore daemon working directory")?;
                    }
                }
                let _ = write_frame(&mut stream, &Response::Observed);
            }
            Request::Hit => {
                hits += 1;
                let _ = write_frame(&mut stream, &Response::Observed);
            }
            Request::Manifests => {
                // A batch can only originate from a live worker forked by this server. The
                // syntax itself is portable across invocation contexts; lowering stays local.
                if context::peer_pid(&stream).is_ok_and(|pid| children.contains_key(&pid)) {
                    let _ = manifests::accept(&mut stream, &mut manifest_counters);
                }
            }
            Request::Run(invocation) => {
                let descriptors = match receive_fds(&stream) {
                    Ok(descriptors) => descriptors,
                    Err(error) => {
                        let _ = write_frame(&mut stream, &Response::Error(error.to_string()));
                        continue;
                    }
                };
                if children.len() >= MAX_WORKERS || !stop_waiters.is_empty() {
                    let _ = write_frame(
                        &mut stream,
                        &Response::RunLocally("Daemon is busy or stopping".into()),
                    );
                    continue;
                }
                if let Err(error) = invocation.context.validate(&stream) {
                    let _ = write_frame(&mut stream, &Response::RunLocally(error.to_string()));
                    continue;
                }
                if let Err(error) = context::ensure_single_threaded() {
                    let _ = write_frame(&mut stream, &Response::RunLocally(error.to_string()));
                    continue;
                }
                // SAFETY: This process has never started uv's runtime, initialized its command
                // globals, or created a thread. Its only application work is synchronous socket
                // handling and source parsing, and no mutex guard is held across this call.
                match unsafe { fork() } {
                    Ok(ForkResult::Parent { child }) => {
                        let _ = write_frame(&mut stream, &Response::Accepted(child.as_raw()));
                        children.insert(child.as_raw(), stream);
                    }
                    Ok(ForkResult::Child) => {
                        // Disarm the copied handler before its pipe descriptors can be reused.
                        // prepare_worker then restores the requesting client's signal state.
                        drop(notifications);
                        // Closing these inherited descriptors must not explicitly unlock the
                        // shared lock-file description or unlink the parent's socket.
                        drop(children);
                        drop(stop_waiters);
                        drop(listener);
                        drop(lock_file);
                        drop(stream);
                        return prepare_worker(invocation, descriptors, paths.socket);
                    }
                    Err(error) => {
                        let _ = write_frame(
                            &mut stream,
                            &Response::Error(format!("Failed to fork daemon worker: {error}")),
                        );
                    }
                }
            }
        }
    }
}

#[allow(unsafe_code)]
fn prepare_worker(
    invocation: Invocation,
    descriptors: [OwnedFd; 4],
    socket: PathBuf,
) -> Result<Bootstrap> {
    let [stdin, stdout, stderr, directory] = descriptors;
    dup2_stdin(stdin)?;
    dup2_stdout(stdout)?;
    dup2_stderr(stderr)?;
    fchdir(directory)?;
    invocation.context.restore()?;
    // SAFETY: The fork-server parent was single-threaded and this child has not started any
    // threads. Replace the environment before invoking uv's normal process initialization.
    unsafe {
        for (name, _) in std::env::vars_os() {
            std::env::remove_var(name);
        }
        for (name, value) in invocation.environment {
            std::env::set_var(OsString::from_vec(name), OsString::from_vec(value));
        }
    }
    WORKER.store(true, Ordering::Relaxed);
    let _ = CACHE_SOCKET.set(socket);
    Lock::observe_process_cache(observe_lock_cache);
    manifests::initialize_worker();
    Ok(Bootstrap::Continue(
        invocation
            .args
            .into_iter()
            .map(OsString::from_vec)
            .collect(),
    ))
}

pub(super) fn flush_process_caches(close: bool) {
    manifests::flush(close);
}

fn observe_lock_cache(contents: &str, hit: bool) {
    let Some(socket) = CACHE_SOCKET.get() else {
        return;
    };
    let Ok(mut stream) = connect(socket) else {
        return;
    };
    let request = if hit {
        Request::Hit
    } else {
        let Ok(directory) = std::env::current_dir() else {
            return;
        };
        Request::Warm {
            contents: contents.to_owned(),
            directory: directory.as_os_str().as_bytes().to_vec(),
        }
    };
    if write_frame(&mut stream, &request).is_ok() {
        let _ = read_frame::<Response>(&mut stream);
    }
}

fn verify_peer(stream: &UnixStream) -> Result<()> {
    #[cfg(target_os = "macos")]
    let (uid, gid) = nix::unistd::getpeereid(stream)?;
    #[cfg(target_os = "linux")]
    let (uid, gid) = {
        let credentials =
            nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)?;
        (
            nix::unistd::Uid::from_raw(credentials.uid()),
            nix::unistd::Gid::from_raw(credentials.gid()),
        )
    };
    ensure!(
        uid == geteuid() && gid == getegid(),
        "Daemon peer has a different identity"
    );
    Ok(())
}

fn write_frame<T: Serialize>(stream: &mut UnixStream, message: &T) -> Result<()> {
    let encoded = rmp_serde::to_vec(message)?;
    ensure!(
        encoded.len() <= MAX_FRAME,
        "Daemon message exceeds size limit"
    );
    stream.write_all(&u32::try_from(encoded.len())?.to_be_bytes())?;
    stream.write_all(&encoded)?;
    Ok(())
}

fn read_frame<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    ensure!(length <= MAX_FRAME, "Daemon message exceeds size limit");
    let mut encoded = vec![0; length];
    stream.read_exact(&mut encoded)?;
    Ok(rmp_serde::from_slice(&encoded)?)
}

fn send_fds(stream: &UnixStream, directory: &File) -> Result<()> {
    let descriptors = [0, 1, 2, directory.as_raw_fd()];
    ensure!(
        sendmsg::<()>(
            stream.as_raw_fd(),
            &[IoSlice::new(&[0])],
            &[ControlMessage::ScmRights(&descriptors)],
            MsgFlags::empty(),
            None
        )? == 1,
        "Failed sending daemon descriptors"
    );
    Ok(())
}

#[allow(unsafe_code)]
fn receive_fds(stream: &UnixStream) -> Result<[OwnedFd; 4]> {
    let mut byte = [0];
    let mut ancillary = nix::cmsg_space!([i32; 4]);
    let mut buffers = [IoSliceMut::new(&mut byte)];
    let message = recvmsg::<()>(
        stream.as_raw_fd(),
        &mut buffers,
        Some(&mut ancillary),
        MsgFlags::empty(),
    )?;
    let mut descriptors = Vec::new();
    for control in message.cmsgs()? {
        if let ControlMessageOwned::ScmRights(received) = control {
            for descriptor in received {
                // SAFETY: SCM_RIGHTS returns new, owned descriptors. Every received descriptor
                // is immediately placed in an OwnedFd, including malformed requests' descriptors.
                descriptors.push(unsafe { OwnedFd::from_raw_fd(descriptor) });
            }
        }
    }
    ensure!(
        !message.flags.contains(MsgFlags::MSG_CTRUNC),
        "Truncated daemon descriptors"
    );
    descriptors
        .try_into()
        .map_err(|_| anyhow!("Expected four daemon descriptors"))
}

#[allow(unsafe_code)]
extern "C" fn forward_signal(signal: libc::c_int) {
    let pid = FORWARD_PID.load(Ordering::Relaxed);
    if pid > 0 && !(signal == libc::SIGINT && INTERACTIVE.load(Ordering::Relaxed)) {
        // SAFETY: kill is async-signal-safe; the handler otherwise only reads lock-free atomics.
        let _ = unsafe { libc::kill(pid, signal) };
    }
}

#[allow(unsafe_code)]
fn install_signal_forwarding(pid: i32) -> Result<()> {
    FORWARD_PID.store(pid, Ordering::Relaxed);
    INTERACTIVE.store(io::stdin().is_terminal(), Ordering::Relaxed);
    let action = SigAction::new(
        SigHandler::Handler(forward_signal),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    for signal in [
        Signal::SIGINT,
        Signal::SIGTERM,
        Signal::SIGHUP,
        Signal::SIGQUIT,
        Signal::SIGUSR1,
        Signal::SIGUSR2,
        Signal::SIGALRM,
    ] {
        // SAFETY: The installed handler only uses async-signal-safe operations. This foreground
        // client exits immediately after the worker, so these handlers need not be restored.
        unsafe { signal::sigaction(signal, &action) }?;
    }
    Ok(())
}
