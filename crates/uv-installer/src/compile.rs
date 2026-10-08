use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use std::{env, io, panic};

use async_channel::{Receiver, SendError};
use tempfile::{TempDir, tempdir_in};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::oneshot;
use tracing::{debug, instrument};
use walkdir::WalkDir;

use uv_configuration::Concurrency;
use uv_fs::Simplified;
use uv_static::EnvVars;
use uv_warnings::warn_user;

const COMPILEALL_SCRIPT: &str = include_str!("pip_compileall.py");
/// This is longer than any compilation should ever take.
const DEFAULT_COMPILE_TIMEOUT: Duration = Duration::from_mins(1);

type WorkerOutcome = std::thread::Result<Result<(), CompileError>>;
type WorkerHandle = oneshot::Receiver<WorkerOutcome>;

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
    #[error("Bytecode timed out ({}s) compiling file: `{}`", elapsed.as_secs_f32(), source_file)]
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
        Ok(value) => match value.as_str() {
            "0" => None,
            _ => match value.parse::<u64>().map(Duration::from_secs) {
                Ok(duration) => Some(duration),
                Err(_) => {
                    return Err(CompileError::EnvironmentError {
                        var: EnvVars::UV_COMPILE_BYTECODE_TIMEOUT,
                        message: format!("Expected an integer number of seconds, got \"{value}\""),
                    });
                }
            },
        },
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

#[derive(Clone)]
struct WorkerResources {
    script: Arc<TempDir>,
    _environment: Option<Arc<TempDir>>,
}

impl WorkerResources {
    fn new(cache: &Path, environment: Option<Arc<TempDir>>) -> Result<Self, CompileError> {
        let script = Arc::new(tempdir_in(cache).map_err(CompileError::TempFile)?);
        fs_err::write(script.path().join("pip_compileall.py"), COMPILEALL_SCRIPT)
            .map_err(CompileError::TempFile)?;
        Ok(Self {
            script,
            _environment: environment,
        })
    }
}

fn spawn_workers(
    dir: &Path,
    python_executable: &Path,
    pip_compileall_py: &Path,
    receiver: &Receiver<PathBuf>,
    worker_count: usize,
    timeout: Option<Duration>,
    resources: &WorkerResources,
    destination: Option<&Path>,
) -> Vec<WorkerHandle> {
    debug!("Starting {} bytecode compilation workers", worker_count);
    let mut worker_handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let (tx, rx) = oneshot::channel();

        let worker = worker(
            dir.to_path_buf(),
            python_executable.to_path_buf(),
            pip_compileall_py.to_path_buf(),
            receiver.clone(),
            timeout,
            destination.map(Path::to_path_buf),
        );
        let resources = resources.clone();

        // Spawn each worker on a dedicated thread.
        std::thread::Builder::new()
            .name("uv-compile".to_owned())
            .spawn(move || {
                // Report panics back to the main thread.
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to build runtime")
                        .block_on(worker)
                }));

                // Actual workers retain both directories after caller cancellation. Release
                // their leases before reporting completion so publication can take ownership.
                drop(resources);
                // This may fail if the main thread returned early due to an error.
                let _ = tx.send(result);
            })
            .expect("Failed to start compilation worker");

        worker_handles.push(rx);
    }
    worker_handles
}

/// Wait for all workers to exit so worker failures are not hidden by channel send errors.
async fn wait_for_workers(
    worker_handles: Vec<WorkerHandle>,
    send_error: Option<SendError<PathBuf>>,
) -> Result<(), CompileError> {
    for result in futures::future::join_all(worker_handles).await {
        match result {
            // A worker thread panicked or exited without reporting its result.
            Err(_) | Ok(Err(_)) => return Err(CompileError::Join),
            Ok(Ok(Err(compile_error))) => return Err(compile_error),
            Ok(Ok(Ok(()))) => {}
        }
    }

    if let Some(send_error) = send_error {
        // This is suspicious: Why did the channel stop working, but all workers exited
        // successfully?
        return Err(CompileError::WorkerDisappeared(send_error));
    }

    Ok(())
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
    compile_tree_inner(dir, python_executable, concurrency, cache, None, None).await
}

/// Compile a staged tree using its published paths in code objects, retaining the environment
/// until every actual worker has exited even when the awaiting caller is cancelled.
///
/// `dir` must be inside `environment`; `destination` is its corresponding published directory.
pub async fn compile_staged_tree(
    dir: &Path,
    destination: &Path,
    python_executable: &Path,
    concurrency: &Concurrency,
    cache: &Path,
    environment: Arc<TempDir>,
) -> Result<usize, CompileError> {
    debug_assert!(dir.starts_with(environment.path()));
    compile_tree_inner(
        dir,
        python_executable,
        concurrency,
        cache,
        Some(destination),
        Some(environment),
    )
    .await
}

async fn compile_tree_inner(
    dir: &Path,
    python_executable: &Path,
    concurrency: &Concurrency,
    cache: &Path,
    destination: Option<&Path>,
    environment: Option<Arc<TempDir>>,
) -> Result<usize, CompileError> {
    debug_assert!(
        dir.is_absolute(),
        "compileall doesn't work with relative paths: `{}`",
        dir.display()
    );
    let worker_count = concurrency.installs;

    // A larger buffer is significantly faster than just 1 or the worker count.
    let (sender, receiver) = async_channel::bounded::<PathBuf>(worker_count * 10);

    // Running Python with an actual file will produce better error messages.
    let resources = WorkerResources::new(cache, environment)?;
    let pip_compileall_py = resources.script.path().join("pip_compileall.py");
    let timeout = compile_timeout()?;
    let worker_handles = spawn_workers(
        dir,
        python_executable,
        &pip_compileall_py,
        &receiver,
        worker_count,
        timeout,
        &resources,
        destination,
    );
    // Make sure the channel gets closed when all workers exit.
    drop(receiver);

    // Start the producer, sending all `.py` files to workers.
    let mut source_files = 0;
    let mut send_error = None;
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
                return Err(err.into());
            }
        };
        // https://github.com/pypa/pip/blob/3820b0e52c7fed2b2c43ba731b718f316e6816d1/src/pip/_internal/operations/install/wheel.py#L593-L604
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "py") {
            source_files += 1;
            if let Err(err) = sender.send(entry.path().to_owned()).await {
                // The workers exited.
                // If e.g. something with the Python interpreter is wrong, the workers have exited
                // with an error. We try to report this informative error and only if that fails,
                // report the send error.
                send_error = Some(err);
                break;
            }
        }
    }

    // All workers will receive an error after the last item. Note that there are still
    // up to worker_count * 10 items in the queue.
    drop(sender);

    wait_for_workers(worker_handles, send_error).await?;

    Ok(source_files)
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
    let mut initial_files = Vec::with_capacity(concurrency.installs);
    for file in files.by_ref().take(concurrency.installs) {
        initial_files.push(file.map_err(CompileError::SourceFiles)?);
    }
    if initial_files.is_empty() {
        return Ok(0);
    }

    let worker_count = initial_files.len();
    let (sender, receiver) = async_channel::bounded::<PathBuf>(worker_count * 10);

    // Running Python with an actual file will produce better error messages.
    let resources = WorkerResources::new(cache, None)?;
    let pip_compileall_py = resources.script.path().join("pip_compileall.py");
    let timeout = compile_timeout()?;
    let worker_handles = spawn_workers(
        cache,
        python_executable,
        &pip_compileall_py,
        &receiver,
        worker_count,
        timeout,
        &resources,
        None,
    );
    drop(receiver);

    let mut send_error = None;
    let mut source_error = None;
    let mut source_files = 0;
    for file in initial_files.into_iter().map(Ok).chain(files) {
        let file = match file {
            Ok(file) => file,
            Err(err) => {
                source_error = Some(err);
                break;
            }
        };
        debug_assert!(
            file.is_absolute(),
            "compileall doesn't work with relative paths: `{}`",
            file.display()
        );
        source_files += 1;
        if let Err(err) = sender.send(file).await {
            send_error = Some(err);
            break;
        }
    }
    drop(sender);

    wait_for_workers(worker_handles, send_error).await?;
    if let Some(source_error) = source_error {
        return Err(CompileError::SourceFiles(source_error));
    }

    Ok(source_files)
}

async fn worker(
    dir: PathBuf,
    interpreter: PathBuf,
    pip_compileall_py: PathBuf,
    receiver: Receiver<PathBuf>,
    timeout: Option<Duration>,
    destination: Option<PathBuf>,
) -> Result<(), CompileError> {
    // Sometimes, the first time we read from stdout, we get an empty string back (no newline). If
    // we try to write to stdin, it will often be a broken pipe. In this case, we have to restart
    // the child process
    // https://github.com/astral-sh/uv/issues/2245
    let startup = timeout.map(|duration| (tokio::time::Instant::now() + duration, duration));
    let (mut bytecode_compiler, child_stdin, mut child_stdout, mut child_stderr) = loop {
        if let Some(child) = launch_bytecode_compiler(
            &dir,
            &interpreter,
            &pip_compileall_py,
            destination.as_deref(),
            startup,
        )
        .await?
        {
            break child;
        }
    };

    let stderr_reader = tokio::task::spawn(async move {
        let mut child_stderr_collected: Vec<u8> = Vec::new();
        child_stderr
            .read_to_end(&mut child_stderr_collected)
            .await?;
        Ok(child_stderr_collected)
    });

    let result = worker_main_loop(receiver, child_stdin, &mut child_stdout, timeout).await;
    // Reap the process to avoid zombies.
    let _ = bytecode_compiler.kill().await;
    let _ = bytecode_compiler.wait().await;

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
                    "Bytecode compilation `python` at {} stderr:\n{}\n---",
                    interpreter.user_display(),
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

/// Returns the child and stdin/stdout/stderr on a successful launch or `None` for a broken interpreter state.
async fn launch_bytecode_compiler(
    dir: &Path,
    interpreter: &Path,
    pip_compileall_py: &Path,
    destination: Option<&Path>,
    startup: Option<(tokio::time::Instant, Duration)>,
) -> Result<
    Option<(
        Child,
        ChildStdin,
        BufReader<ChildStdout>,
        BufReader<ChildStderr>,
    )>,
    CompileError,
> {
    // We input the paths through stdin and get the successful paths returned through stdout.
    let mut command = Command::new(interpreter);
    command.arg(pip_compileall_py);
    if let Some(destination) = destination {
        command.arg(dir).arg(destination);
    }
    let mut bytecode_compiler = command
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(dir)
        // Otherwise stdout is buffered and we'll wait forever for a response
        .env(EnvVars::PYTHONUNBUFFERED, "1")
        .spawn()
        .map_err(CompileError::PythonSubcommand)?;

    // https://stackoverflow.com/questions/49218599/write-to-child-process-stdin-in-rust/49597789#comment120223107_49597789
    // Unbuffered, we need to write immediately or the python process will get stuck waiting
    let child_stdin = bytecode_compiler
        .stdin
        .take()
        .expect("Child must have stdin");
    let mut child_stdout = BufReader::new(
        bytecode_compiler
            .stdout
            .take()
            .expect("Child must have stdout"),
    );
    let child_stderr = BufReader::new(
        bytecode_compiler
            .stderr
            .take()
            .expect("Child must have stderr"),
    );

    // Keep ownership of the child outside the timed read, then reap it before releasing the
    // staging lease on every failed launch. Cancelling the read must not detach the Python writer.
    let mut out_line = String::new();
    let ready = async {
        child_stdout
            .read_line(&mut out_line)
            .await
            .map_err(|err| CompileError::ChildStdio {
                device: "stdout",
                err,
            })
    };
    let result = if let Some((deadline, duration)) = startup {
        tokio::time::timeout_at(deadline, ready)
            .await
            .map_err(|_| CompileError::StartupTimeout(duration))
            .and_then(std::convert::identity)
    } else {
        ready.await
    };
    if result.is_ok() && out_line.trim_end() == "Ready" {
        return Ok(Some((
            bytecode_compiler,
            child_stdin,
            child_stdout,
            child_stderr,
        )));
    }
    let _ = bytecode_compiler.kill().await;
    let _ = bytecode_compiler.wait().await;
    result?;
    if out_line.is_empty() {
        // Failed to launch, try again within the same startup deadline.
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
    let mut out_line = String::new();
    while let Ok(source_file) = receiver.recv().await {
        let source_file = source_file.display().to_string();
        if source_file.contains(['\r', '\n']) {
            warn_user!("Path contains newline, skipping: {source_file:?}");
            continue;
        }
        // Luckily, LF alone works on windows too
        let bytes = format!("{source_file}\n").into_bytes();

        let python_handle = async {
            child_stdin
                .write_all(&bytes)
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
            tokio::time::timeout(duration, python_handle)
                .await
                .map_err(|_| CompileError::CompileTimeout {
                    elapsed: duration,
                    source_file: source_file.clone(),
                })??;
        } else {
            python_handle.await?;
        }

        // This is a sanity check, if we don't get the path back something has gone wrong, e.g.
        // we're not actually running a python interpreter.
        let actual = out_line.trim_end_matches(['\n', '\r']);
        if actual != source_file {
            return Err(CompileError::WrongPath(source_file, actual.to_string()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{Context, Result, bail, ensure};
    use tempfile::TempDir;
    use uv_cache::Cache;
    use uv_configuration::Concurrency;
    use uv_python::{EnvironmentPreference, PythonEnvironment, PythonPreference, PythonRequest};

    use super::compile_staged_tree;
    #[cfg(unix)]
    use super::{CompileError, WorkerResources, spawn_workers, wait_for_workers};

    struct StagedPython {
        directory: Arc<TempDir>,
        environment: PythonEnvironment,
        #[cfg(unix)]
        base_executable: PathBuf,
        cache: Cache,
    }

    impl StagedPython {
        fn new() -> Result<Self> {
            let cache = Cache::temp()?;
            let base = PythonEnvironment::find(
                &PythonRequest::Any,
                EnvironmentPreference::Any,
                PythonPreference::System,
                &cache,
            )?;
            let base_executable = base.python_executable().to_owned();
            let directory = Arc::new(tempfile::tempdir()?);
            let output = Command::new(&base_executable)
                .args(["-m", "venv", "--without-pip"])
                .arg(directory.path())
                .output()?;
            ensure!(output.status.success(), "{output:?}");
            let environment = PythonEnvironment::from_root(directory.path(), &cache)?;
            ensure!(
                environment.root() == directory.path(),
                "unexpected environment root: {:?}",
                environment.root()
            );
            Ok(Self {
                directory,
                environment,
                #[cfg(unix)]
                base_executable,
                cache,
            })
        }

        fn site_packages(&self) -> Result<PathBuf> {
            self.environment
                .site_packages()
                .next()
                .filter(|path| path.starts_with(self.directory.path()))
                .map(|path| path.to_path_buf())
                .context("staged interpreter has no owned site-packages")
        }

        fn block_startup(&self, control: &Path) -> Result<()> {
            let site_packages = self.site_packages()?;
            fs_err::write(
                site_packages.join("control-path"),
                control.to_str().context("control path must be UTF-8")?,
            )?;
            fs_err::write(
                site_packages.join("staging_worker_blocker.py"),
                "import os, pathlib, time\ncontrol = pathlib.Path(pathlib.Path(__file__).with_name('control-path').read_text())\n(control / 'ready').write_text(str(os.getpid()))\nwhile not (control / 'release').exists():\n    time.sleep(0.01)\n(control / 'finished').write_text('finished')\n",
            )?;
            fs_err::write(
                site_packages.join("staging-worker.pth"),
                "import staging_worker_blocker\n",
            )?;
            Ok(())
        }
    }

    fn directories(path: &Path) -> Result<BTreeSet<PathBuf>> {
        fs_err::read_dir(path)?
            .filter_map(|entry| match entry {
                Ok(entry) if entry.path().is_dir() => Some(Ok(entry.path())),
                Ok(_) => None,
                Err(error) => Some(Err(error.into())),
            })
            .collect()
    }

    #[tokio::test]
    async fn staged_bytecode_uses_published_source_paths() -> Result<()> {
        let staged = StagedPython::new()?;
        let source = staged.site_packages()?;
        fs_err::write(source.join("example.py"), "value = 1\n")?;
        let published = staged
            .directory
            .path()
            .with_file_name("published environment");
        let concurrency = Concurrency {
            installs: 1,
            ..Concurrency::default()
        };
        compile_staged_tree(
            &source,
            &published,
            staged.environment.python_executable(),
            &concurrency,
            staged.cache.root(),
            Arc::clone(&staged.directory),
        )
        .await?;
        let output = Command::new(staged.environment.python_executable())
            .args(["-c", "import importlib.util, marshal, sys; f = open(importlib.util.cache_from_source(sys.argv[1]), 'rb'); f.read(16); code = marshal.load(f); assert code.co_filename == sys.argv[2], (code.co_filename, sys.argv[2])"])
            .arg(source.join("example.py"))
            .arg(published.join("example.py"))
            .output()?;
        ensure!(output.status.success(), "{output:?}");
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_compilation_keeps_worker_directories_alive() -> Result<()> {
        let staged = StagedPython::new()?;
        let control = tempfile::tempdir()?;
        staged.block_startup(control.path())?;
        let source = staged.site_packages()?;
        let published = staged
            .directory
            .path()
            .with_file_name("published environment");
        let concurrency = Concurrency {
            installs: 1,
            ..Concurrency::default()
        };
        let previous = directories(staged.cache.root())?;
        let mut compiling = Box::pin(compile_staged_tree(
            &source,
            &published,
            staged.environment.python_executable(),
            &concurrency,
            staged.cache.root(),
            Arc::clone(&staged.directory),
        ));
        tokio::select! {
            result = &mut compiling => bail!("compiler exited before its barrier: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(10), async {
                while !control.path().join("ready").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }) => result.context("compiler did not reach its barrier")?,
        }
        let scripts = directories(staged.cache.root())?
            .difference(&previous)
            .cloned()
            .collect::<Vec<_>>();
        ensure!(
            scripts.len() == 1,
            "expected one compiler script directory: {scripts:?}"
        );
        let environment_path = staged.directory.path().to_owned();
        drop(compiling);
        drop(staged.directory);
        let environment_retained = environment_path.exists();
        let script_retained = scripts[0].join("pip_compileall.py").exists();
        fs_err::write(control.path().join("release"), "release")?;
        tokio::time::timeout(Duration::from_secs(10), async {
            while environment_path.exists() || scripts[0].exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("worker did not release its directories")?;
        ensure!(control.path().join("finished").exists());
        ensure!(
            environment_retained,
            "staging disappeared while its worker was active"
        );
        ensure!(
            script_retained,
            "compiler script disappeared while its worker was active"
        );
        Ok(())
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn startup_timeout_reaps_python_before_releasing_staging() -> Result<()> {
        let staged = StagedPython::new()?;
        let control = tempfile::tempdir()?;
        staged.block_startup(control.path())?;
        let resources =
            WorkerResources::new(staged.cache.root(), Some(Arc::clone(&staged.directory)))?;
        let (sender, receiver) = async_channel::bounded(1);
        let workers = spawn_workers(
            &staged.site_packages()?,
            staged.environment.python_executable(),
            &resources.script.path().join("pip_compileall.py"),
            &receiver,
            1,
            Some(Duration::from_secs(2)),
            &resources,
            None,
        );
        drop(sender);
        drop(receiver);
        let Err(error) = wait_for_workers(workers, None).await else {
            bail!("blocked compiler unexpectedly started");
        };
        let CompileError::StartupTimeout(_) = error else {
            bail!("unexpected compiler error: {error:?}");
        };
        let pid = fs_err::read_to_string(control.path().join("ready"))?;
        let output = Command::new(&staged.base_executable)
            .args(["-c", "import os, sys\ntry:\n    os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError:\n    sys.exit(0)\nsys.exit(1)"])
            .arg(pid)
            .output()?;
        ensure!(
            output.status.success(),
            "compiler process survived its timeout: {output:?}"
        );
        Ok(())
    }
}
