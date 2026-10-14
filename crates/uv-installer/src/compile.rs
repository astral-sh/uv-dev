use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use std::{env, io};

use async_channel::{Receiver, SendError, Sender};
use tempfile::{TempDir, tempdir_in};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{OnceCell, oneshot};
use tracing::{debug, instrument};
use walkdir::WalkDir;

use uv_configuration::Concurrency;
use uv_fs::Simplified;
use uv_static::EnvVars;
use uv_warnings::warn_user;

const COMPILEALL_SCRIPT: &str = include_str!("pip_compileall.py");
/// This is longer than any compilation should ever take.
const DEFAULT_COMPILE_TIMEOUT: Duration = Duration::from_mins(1);

type WorkerHandle = oneshot::Receiver<Result<(), CompileError>>;

/// Bytecode workers and their input queue.
struct BytecodeWorkers {
    handles: Vec<WorkerHandle>,
    sender: Sender<PathBuf>,
    source_files: usize,
}

/// Immutable settings shared by all bytecode compilation workers.
struct BytecodeWorkerContext {
    directory: PathBuf,
    interpreter: PathBuf,
    script: BytecodeScript,
    timeout: Option<Duration>,
}

/// A compiler script shared by bytecode workers.
///
/// Its path is published after the script is fully written, and its directory remains alive until
/// all workers exit.
struct BytecodeScript {
    directory: TempDir,
    path: OnceCell<PathBuf>,
}

impl BytecodeScript {
    fn new(directory: TempDir) -> Self {
        Self {
            directory,
            path: OnceCell::new(),
        }
    }

    async fn initialize(&self) -> Result<&Path, CompileError> {
        self.path
            .get_or_try_init(|| async {
                let path = self.directory.path().join("pip_compileall.py");
                fs_err::tokio::write(&path, COMPILEALL_SCRIPT)
                    .await
                    .map(|()| path)
            })
            .await
            .map(PathBuf::as_path)
            .map_err(CompileError::TempFile)
    }
}

/// A ready Python bytecode compiler and its communication streams.
struct BytecodeCompiler {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
}

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("Failed to list files in `site-packages`")]
    Walkdir(#[from] walkdir::Error),
    #[error("Failed to send task to worker")]
    WorkerDisappeared(SendError<PathBuf>),
    #[error("Failed to identify Python source files")]
    SourceFiles(#[source] anyhow::Error),
    #[error("The task executor is broken, did some other task panic?")]
    Join,
    #[error("Failed to start Python interpreter to run compile script")]
    PythonSubcommand(#[source] io::Error),
    #[error("Failed to create temporary script file")]
    TempFile(#[source] io::Error),
    #[error(r#"Bytecode compilation failed, expected "{0}", received: "{1}""#)]
    WrongPath(String, String),
    #[error("Failed to write to Python {device}")]
    ChildStdio {
        device: &'static str,
        #[source]
        err: io::Error,
    },
    #[error("Python process stderr:\n{stderr}")]
    ErrorWithStderr {
        stderr: String,
        #[source]
        err: Box<Self>,
    },
    #[error("Bytecode timed out ({}s) compiling file: {}", elapsed.as_secs_f32(), source_file)]
    CompileTimeout {
        elapsed: Duration,
        source_file: String,
    },
    #[error("Python startup timed out ({}s)", _0.as_secs_f32())]
    StartupTimeout(Duration),
    #[error("Got invalid value from environment for {var}: {message}.")]
    EnvironmentError { var: &'static str, message: String },
}

fn compile_timeout() -> Result<Option<Duration>, CompileError> {
    let timeout = match env::var(EnvVars::UV_COMPILE_BYTECODE_TIMEOUT) {
        Ok(value) if value == "0" => None,
        Ok(value) => Some(value.parse::<u64>().map(Duration::from_secs).map_err(|_| {
            CompileError::EnvironmentError {
                var: EnvVars::UV_COMPILE_BYTECODE_TIMEOUT,
                message: format!("Expected an integer number of seconds, got \"{value}\""),
            }
        })?),
        Err(_) => Some(DEFAULT_COMPILE_TIMEOUT),
    };
    if let Some(duration) = timeout {
        debug!(
            "Using bytecode compilation timeout of {}s",
            duration.as_secs()
        );
    } else {
        debug!("Disabling bytecode compilation timeout");
    }
    Ok(timeout)
}

fn spawn_workers(
    dir: &Path,
    python_executable: &Path,
    cache: &Path,
    worker_count: usize,
) -> Result<BytecodeWorkers, CompileError> {
    // A larger buffer is significantly faster than just 1 or the worker count.
    let (sender, receiver) = async_channel::bounded::<PathBuf>(worker_count * 10);

    // Running Python with an actual file will produce better error messages.
    let script = BytecodeScript::new(tempdir_in(cache).map_err(CompileError::TempFile)?);
    let timeout = compile_timeout()?;
    let context = Arc::new(BytecodeWorkerContext {
        directory: dir.to_path_buf(),
        interpreter: python_executable.to_path_buf(),
        script,
        timeout,
    });
    debug!("Starting {} bytecode compilation workers", worker_count);
    let mut worker_handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let (tx, rx) = oneshot::channel();

        let context = Arc::clone(&context);
        let receiver = receiver.clone();

        // Spawn each worker on a dedicated thread.
        std::thread::Builder::new()
            .name("uv-compile".to_owned())
            .spawn(move || {
                let result = run_worker(context, receiver);
                // This may fail if the main thread returned early due to an error.
                let _ = tx.send(result);
            })
            .expect("Failed to start compilation worker");

        worker_handles.push(rx);
    }
    // Make sure the channel gets closed when all workers exit.
    drop(receiver);
    Ok(BytecodeWorkers {
        handles: worker_handles,
        sender,
        source_files: 0,
    })
}

impl BytecodeWorkers {
    /// Submit a source file to a worker.
    async fn submit(&mut self, file: PathBuf) -> Result<(), CompileError> {
        self.sender
            .send(file)
            .await
            .map_err(CompileError::WorkerDisappeared)?;
        self.source_files += 1;
        Ok(())
    }

    /// Submit discovered source files to workers in order.
    async fn submit_files(
        &mut self,
        files: impl IntoIterator<Item = anyhow::Result<PathBuf>>,
    ) -> Result<(), CompileError> {
        for file in files {
            let file = file.map_err(CompileError::SourceFiles)?;
            debug_assert!(
                file.is_absolute(),
                "compileall doesn't work with relative paths: `{}`",
                file.display()
            );
            self.submit(file).await?;
        }
        Ok(())
    }

    /// Wait for all workers to exit, prioritizing worker failures over producer failures.
    async fn finish(
        self,
        producer_result: Result<(), CompileError>,
    ) -> Result<usize, CompileError> {
        let Self {
            handles,
            sender,
            source_files,
        } = self;
        // Closing the channel lets workers exit after consuming the remaining queued files.
        drop(sender);
        for result in futures::future::join_all(handles).await {
            // A disconnected worker panicked or exited without reporting its result.
            result.unwrap_or(Err(CompileError::Join))?;
        }

        producer_result.map(|()| source_files)
    }
}

/// Bytecode compile all file in `dir` using a pool of Python interpreters running a Python script
/// that calls `compileall.compile_file`.
///
/// All compilation errors are muted (like pip). There is a 60s timeout for each file to handle
/// a broken `python`. The timeout can be configured with `UV_COMPILE_BYTECODE_TIMEOUT`; a value of
/// `0` disables the timeout.
///
/// We only compile all files, but we don't update the RECORD, relying on PEP 491:
/// > Uninstallers should be smart enough to remove .pyc even if it is not mentioned in RECORD.
///
/// We've confirmed that both uv and pip (as of 24.0.0) remove the `__pycache__` directory.
#[instrument(skip(python_executable))]
pub async fn compile_tree(
    dir: &Path,
    python_executable: &Path,
    concurrency: &Concurrency,
    cache: &Path,
) -> Result<usize, CompileError> {
    debug_assert!(
        dir.is_absolute(),
        "compileall doesn't work with relative paths: `{}`",
        dir.display()
    );
    let mut workers = spawn_workers(dir, python_executable, cache, concurrency.installs)?;

    // Start the producer, sending all `.py` files to workers.
    let mut producer_result = Ok(());
    let walker = WalkDir::new(dir)
        .into_iter()
        // Otherwise we stumble over temporary files from `compileall`.
        .filter_entry(|dir| dir.file_name() != "__pycache__");
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                if err
                    .io_error()
                    .is_some_and(|err| err.kind() == io::ErrorKind::NotFound)
                {
                    // The directory was removed, just ignore it
                    continue;
                }
                // A walk failure takes precedence, but workers must exit before the tree walk does.
                let _ = workers.finish(Ok(())).await;
                return Err(err.into());
            }
        };
        // https://github.com/pypa/pip/blob/3820b0e52c7fed2b2c43ba731b718f316e6816d1/src/pip/_internal/operations/install/wheel.py#L593-L604
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "py") {
            if let Err(err) = workers.submit(entry.into_path()).await {
                // The workers exited.
                // If e.g. something with the Python interpreter is wrong, the workers have exited
                // with an error. We try to report this informative error and only if that fails,
                // report the send error.
                producer_result = Err(err);
                break;
            }
        }
    }

    workers.finish(producer_result).await
}

/// Bytecode compile the given Python source files using a pool of Python interpreters.
///
/// All paths must be absolute. Compilation errors are muted (like pip), while failures to launch
/// or communicate with the Python workers are returned.
#[instrument(skip(files, python_executable))]
pub async fn compile_files(
    files: impl IntoIterator<Item = anyhow::Result<PathBuf>>,
    python_executable: &Path,
    concurrency: &Concurrency,
    cache: &Path,
) -> Result<usize, CompileError> {
    let mut files = files.into_iter();
    let initial_files = files
        .by_ref()
        .take(concurrency.installs)
        .collect::<Result<Vec<_>, _>>()
        .map_err(CompileError::SourceFiles)?;
    if initial_files.is_empty() {
        return Ok(0);
    }

    let mut workers = spawn_workers(cache, python_executable, cache, initial_files.len())?;

    let producer_result = workers
        .submit_files(initial_files.into_iter().map(Ok).chain(files))
        .await;
    workers.finish(producer_result).await
}

/// Run a worker to completion on its dedicated thread.
fn run_worker(
    context: Arc<BytecodeWorkerContext>,
    receiver: Receiver<PathBuf>,
) -> Result<(), CompileError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build runtime")
        .block_on(worker(context, receiver))
}

async fn worker(
    context: Arc<BytecodeWorkerContext>,
    receiver: Receiver<PathBuf>,
) -> Result<(), CompileError> {
    let pip_compileall_py = context.script.initialize().await?;

    // Sometimes, the first time we read from stdout, we get an empty string back (no newline). If
    // we try to write to stdin, it will often be a broken pipe. In this case, we have to restart
    // the child process
    // https://github.com/astral-sh/uv/issues/2245
    let wait_until_ready = async {
        loop {
            // If the interpreter started successful, return it, else retry.
            if let Some(child) = launch_bytecode_compiler(
                &context.directory,
                &context.interpreter,
                pip_compileall_py,
            )
            .await?
            {
                break Ok::<_, CompileError>(child);
            }
        }
    };

    // Handle a broken `python` by using a timeout, one that's higher than any compilation
    // should ever take.
    let BytecodeCompiler {
        mut child,
        stdin,
        mut stdout,
        mut stderr,
    } = if let Some(duration) = context.timeout {
        tokio::time::timeout(duration, wait_until_ready)
            .await
            .map_err(|_| CompileError::StartupTimeout(duration))??
    } else {
        wait_until_ready.await?
    };

    let stderr_reader = tokio::task::spawn(async move {
        let mut child_stderr_collected: Vec<u8> = Vec::new();
        stderr.read_to_end(&mut child_stderr_collected).await?;
        Ok(child_stderr_collected)
    });

    let result = worker_main_loop(receiver, stdin, &mut stdout, context.timeout).await;
    // Reap the process to avoid zombies.
    let _ = child.kill().await;

    // If there was something printed to stderr (which shouldn't happen, we muted all errors), tell
    // the user, otherwise only forward the result.
    let child_stderr_collected = stderr_reader
        .await
        .map_err(|_| CompileError::Join)?
        .map_err(|err| CompileError::ChildStdio {
            device: "stderr",
            err,
        })?;
    let result = if child_stderr_collected.is_empty() {
        result
    } else {
        let stderr = String::from_utf8_lossy(&child_stderr_collected);
        match result {
            Ok(()) => {
                debug!(
                    "Bytecode compilation `python` at `{}` stderr:\n{}\n---",
                    context.interpreter.user_display(),
                    stderr
                );
                Ok(())
            }
            Err(err) => Err(CompileError::ErrorWithStderr {
                stderr: stderr.trim().to_string(),
                err: Box::new(err),
            }),
        }
    };

    debug!("Bytecode compilation worker exiting: {:?}", result);

    result
}

/// Return a ready compiler, or `None` for a broken interpreter state.
async fn launch_bytecode_compiler(
    dir: &Path,
    interpreter: &Path,
    pip_compileall_py: &Path,
) -> Result<Option<BytecodeCompiler>, CompileError> {
    // We input the paths through stdin and get the successful paths returned through stdout.
    let mut child = Command::new(interpreter)
        .arg(pip_compileall_py)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(dir)
        // Otherwise stdout is buffered and we'll wait forever for a response
        .env(EnvVars::PYTHONUNBUFFERED, "1")
        // Startup can time out before the worker takes ownership of the child.
        .kill_on_drop(true)
        .spawn()
        .map_err(CompileError::PythonSubcommand)?;

    // https://stackoverflow.com/questions/49218599/write-to-child-process-stdin-in-rust/49597789#comment120223107_49597789
    // Unbuffered, we need to write immediately or the python process will get stuck waiting
    let stdin = child.stdin.take().expect("Child must have stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("Child must have stdout"));
    let stderr = child.stderr.take().expect("Child must have stderr");

    // Check if the launch was successful.
    let mut out_line = String::new();
    let read_result = stdout.read_line(&mut out_line).await;
    if read_result.is_ok() && out_line.trim_end() == "Ready" {
        return Ok(Some(BytecodeCompiler {
            child,
            stdin,
            stdout,
            stderr,
        }));
    }

    // Reap a failed startup before retrying or reporting the error.
    let _ = child.kill().await;
    read_result.map_err(|err| CompileError::ChildStdio {
        device: "stdout",
        err,
    })?;
    if out_line.is_empty() {
        // Failed to launch, try again
        Ok(None)
    } else {
        Err(CompileError::WrongPath("Ready".to_string(), out_line))
    }
}

/// We use stdin/stdout as a sort of bounded channel. We write one path to stdin, then wait until
/// we get the same path back from stdout. This way we ensure one worker is only working on one
/// piece of work at the same time.
async fn worker_main_loop(
    receiver: Receiver<PathBuf>,
    mut child_stdin: ChildStdin,
    child_stdout: &mut BufReader<ChildStdout>,
    timeout: Option<Duration>,
) -> Result<(), CompileError> {
    let mut input_line = String::new();
    let mut out_line = String::new();
    while let Ok(source_file) = receiver.recv().await {
        let source_file = source_file.to_string_lossy();
        if source_file.contains(['\r', '\n']) {
            warn_user!("Path contains newline, skipping: {source_file:?}");
            continue;
        }
        // Luckily, LF alone works on windows too
        input_line.clear();
        input_line.push_str(&source_file);
        input_line.push('\n');

        let python_handle = async {
            child_stdin
                .write_all(input_line.as_bytes())
                .await
                .map_err(|err| CompileError::ChildStdio {
                    device: "stdin",
                    err,
                })?;

            out_line.clear();
            child_stdout.read_line(&mut out_line).await.map_err(|err| {
                CompileError::ChildStdio {
                    device: "stdout",
                    err,
                }
            })?;
            Ok::<(), CompileError>(())
        };

        // Handle a broken `python` by using a timeout, one that's higher than any compilation
        // should ever take.
        if let Some(duration) = timeout {
            let Ok(result) = tokio::time::timeout(duration, python_handle).await else {
                return Err(CompileError::CompileTimeout {
                    elapsed: duration,
                    source_file: source_file.into_owned(),
                });
            };
            result?;
        } else {
            python_handle.await?;
        }

        // This is a sanity check, if we don't get the path back something has gone wrong, e.g.
        // we're not actually running a python interpreter.
        let actual = out_line.trim_end_matches(['\n', '\r']);
        if actual != source_file.as_ref() {
            return Err(CompileError::WrongPath(
                source_file.into_owned(),
                actual.to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{ErrorKind, Write};
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;
    use uv_cache::Cache;
    use uv_python_discovery::find_environment;
    use uv_python_types::{EnvironmentPreference, PythonPreference, PythonRequest};

    use super::{BytecodeScript, COMPILEALL_SCRIPT, launch_bytecode_compiler};

    /// Once an interpreter can read the compiler script, later workers must not rewrite it.
    #[tokio::test]
    async fn bytecode_workers_reuse_initialized_script() -> anyhow::Result<()> {
        let script = BytecodeScript::new(tempfile::tempdir()?);
        let path = script.initialize().await?;
        fs_err::OpenOptions::new()
            .append(true)
            .open(path)?
            .write_all(b"\n# initialized\n")?;

        let (first, second) = tokio::join!(script.initialize(), script.initialize());
        assert_eq!(first?, path);
        assert_eq!(second?, path);
        assert_eq!(
            fs_err::read_to_string(path)?,
            format!("{COMPILEALL_SCRIPT}\n# initialized\n")
        );
        Ok(())
    }

    /// Cancelling startup must terminate the interpreter before it reports readiness.
    #[tokio::test]
    async fn cancelled_bytecode_compiler_startup_terminates_child() -> anyhow::Result<()> {
        let _preview = uv_preview::test::with_features(&[]);
        let cache = Cache::temp()?;
        let environment = find_environment(
            &PythonRequest::Any,
            EnvironmentPreference::Any,
            PythonPreference::System,
            None,
            &cache,
        )?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let script = cache.root().join("startup.py");
        fs_err::write(
            &script,
            format!(
                "import socket\nsocket.create_connection(('127.0.0.1', {})).recv(1)\n",
                listener.local_addr()?.port()
            ),
        )?;

        let (mut connection, _) = tokio::select! {
            result = listener.accept() => result?,
            _ = launch_bytecode_compiler(cache.root(), environment.python_executable(), &script) => {
                anyhow::bail!("bytecode compiler exited before startup was cancelled")
            }
            () = tokio::time::sleep(Duration::from_secs(30)) => {
                anyhow::bail!("bytecode compiler did not start")
            }
        };
        match tokio::time::timeout(Duration::from_secs(5), connection.read(&mut [0])).await? {
            Ok(0) => {}
            Err(err) if err.kind() == ErrorKind::ConnectionReset => {}
            result => anyhow::bail!("unexpected compiler connection result: {result:?}"),
        }
        Ok(())
    }
}
