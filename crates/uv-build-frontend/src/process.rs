use std::fmt::Write;
use std::io;
use std::sync::Arc;

use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{OwnedSemaphorePermit, oneshot};
use tracing::Instrument;
use uv_fs::LockedFile;

use crate::{Printer, PythonRunnerOutput};

struct ProtectedHookWorker {
    cancellation: Option<oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ProtectedHookWorker {
    fn drop(&mut self) {
        // The CLI shuts its Tokio runtime down without waiting for background tasks. Cancellation
        // therefore joins this worker synchronously, rather than detaching child cleanup. The
        // worker has its own runtime and thread, so it can kill and reap independently of us.
        drop(self.cancellation.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Keep protected hooks owned through child exit, including cancellation of their async caller.
pub(super) async fn run(
    command: Command,
    source_tree_lock: Option<Arc<LockedFile>>,
    permit: OwnedSemaphorePermit,
    printer: Printer,
) -> io::Result<PythonRunnerOutput> {
    let Some(source_tree_lock) = source_tree_lock else {
        let _permit = permit;
        return run_child(command, printer, None).await;
    };

    // Dropping the caller closes this channel. The worker can still kill and reap its child
    // while the caller's runtime is shutting down.
    let (cancel, cancellation) = oneshot::channel();
    let (completed, completion) = oneshot::channel();
    let command = command.into_std();
    let span = tracing::Span::current();
    let worker = std::thread::Builder::new()
        .name("uv-build-hook".to_owned())
        .spawn(move || {
            let result = {
                let (_source_tree_lock, _permit) = (source_tree_lock, permit);
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .and_then(|runtime| {
                        runtime.block_on(
                            run_child(Command::from(command), printer, Some(cancellation))
                                .instrument(span),
                        )
                    })
            };
            let _ = completed.send(result);
        })?;
    let _worker = ProtectedHookWorker {
        cancellation: Some(cancel),
        worker: Some(worker),
    };
    completion.await.map_err(io::Error::other)?
}

async fn run_child(
    mut command: Command,
    printer: Printer,
    cancellation: Option<oneshot::Receiver<()>>,
) -> io::Result<PythonRunnerOutput> {
    if let Some(mut cancellation) = cancellation {
        // A canceled operation can still be queued in the blocking pool.
        match cancellation.try_recv() {
            Err(oneshot::error::TryRecvError::Empty) => {}
            Ok(()) | Err(oneshot::error::TryRecvError::Closed) => {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Build hook canceled",
                ));
            }
        }
        let mut child = command.kill_on_drop(true).spawn()?;
        let result = tokio::select! {
            biased;
            _ = &mut cancellation => {
                Err(io::Error::new(io::ErrorKind::Interrupted, "Build hook canceled"))
            }
            result = read_and_wait(&mut child, printer) => result,
        };
        if result.is_err() {
            // Output errors and cancellation both leave the child owned until it has exited.
            let kill = child.start_kill();
            child.wait().await?;
            kill?;
        }
        result
    } else {
        let mut child = command.spawn()?;
        read_and_wait(&mut child, printer).await
    }
}

async fn read_and_wait(child: &mut Child, printer: Printer) -> io::Result<PythonRunnerOutput> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Missing build process stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("Missing build process stderr"))?;
    let stdout_reader = tokio::io::BufReader::new(stdout).split(b'\n');
    let stderr_reader = tokio::io::BufReader::new(stderr).split(b'\n');
    let mut stdout = Vec::with_capacity(1024);
    let mut stderr = Vec::with_capacity(1024);
    let result = tokio::join!(
        read_from(stdout_reader, printer, &mut stdout),
        read_from(stderr_reader, printer, &mut stderr),
    );
    match result {
        (Ok(()), Ok(())) => {}
        (Err(err), _) | (_, Err(err)) => return Err(err),
    }
    let status = child.wait().await?;
    Ok(PythonRunnerOutput {
        stdout,
        stderr,
        status,
    })
}

async fn read_from(
    mut reader: tokio::io::Split<tokio::io::BufReader<impl tokio::io::AsyncRead + Unpin>>,
    mut printer: Printer,
    buffer: &mut Vec<String>,
) -> io::Result<()> {
    loop {
        match reader.next_segment().await? {
            Some(line_buf) => {
                let line =
                    String::from_utf8_lossy(line_buf.strip_suffix(b"\r").unwrap_or(&line_buf))
                        .into_owned();
                let _ = write!(printer, "{line}");
                buffer.push(line);
            }
            None => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::process::Stdio;
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::process::Command;
    use tokio::sync::{Semaphore, oneshot};
    use uv_fs::{LockedFile, LockedFileMode};

    use super::run;
    use crate::Printer;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    #[test]
    #[ignore = "child process fixture for hook lifecycle tests"]
    fn protected_hook_fixture() -> TestResult {
        let mut stream = TcpStream::connect(std::env::var("UV_TEST_HOOK_ADDRESS")?)?;
        stream.write_all(b"ready")?;
        let mut release = [0];
        stream.read_exact(&mut release)?;
        assert_eq!(release, [b'S']);
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_hook_reaps_child_and_releases_admission() -> TestResult {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("source.lock");
        let lock = Arc::new(LockedFile::acquire(&path, LockedFileMode::Exclusive, "source").await?);
        let slots = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&slots).acquire_owned().await?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "process::tests::protected_hook_fixture",
                "--ignored",
                "--nocapture",
            ])
            // Panic-abort libtest normally adds a wrapper process. Run the fixture itself so
            // cancellation exercises the direct child owned by the protected hook worker.
            .env(
                "__RUST_TEST_INVOKE",
                "process::tests::protected_hook_fixture",
            )
            .env("UV_TEST_HOOK_ADDRESS", listener.local_addr()?.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let task = tokio::spawn(run(command, Some(lock), permit, Printer::Quiet));
        let (mut gate, _) =
            tokio::time::timeout(Duration::from_secs(30), listener.accept()).await??;
        let mut ready = [0; 5];
        tokio::time::timeout(Duration::from_secs(30), gate.read_exact(&mut ready)).await??;
        assert_eq!(&ready, b"ready");
        assert!(LockedFile::acquire_no_wait(&path, LockedFileMode::Exclusive, "source").is_none());
        task.abort();
        assert!(task.await.expect_err("canceled hook caller").is_cancelled());

        let admission = tokio::time::timeout(
            Duration::from_secs(30),
            LockedFile::acquire(&path, LockedFileMode::Exclusive, "source"),
        )
        .await;
        if admission.is_err() {
            // Let an incorrect natural-exit-only implementation finish before failing.
            gate.write_all(b"S").await?;
        }
        let _next = admission??;
        let mut byte = [0];
        let closed = tokio::time::timeout(Duration::from_secs(30), gate.read(&mut byte)).await?;
        assert!(
            matches!(&closed, Ok(0))
                || matches!(&closed, Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset),
            "the reaped fixture must close its connection: {closed:?}"
        );
        assert_eq!(slots.available_permits(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_shutdown_reaps_protected_hook() -> TestResult {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("source.lock");
        let lock = Arc::new(LockedFile::acquire(&path, LockedFileMode::Exclusive, "source").await?);
        let slots = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&slots).acquire_owned().await?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?.to_string();
        let (shutdown, request_shutdown) = oneshot::channel();
        let (finished, finished_shutdown) = oneshot::channel();
        let thread = std::thread::spawn(move || -> TestResult {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let mut command = Command::new(std::env::current_exe()?);
            command
                .args([
                    "--exact",
                    "process::tests::protected_hook_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env(
                    "__RUST_TEST_INVOKE",
                    "process::tests::protected_hook_fixture",
                )
                .env("UV_TEST_HOOK_ADDRESS", address)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            runtime.spawn(run(command, Some(lock), permit, Printer::Quiet));
            runtime.block_on(request_shutdown)?;
            runtime.shutdown_background();
            let _ = finished.send(());
            Ok(())
        });
        let (mut gate, _) =
            tokio::time::timeout(Duration::from_secs(30), listener.accept()).await??;
        let mut ready = [0; 5];
        tokio::time::timeout(Duration::from_secs(30), gate.read_exact(&mut ready)).await??;
        assert_eq!(&ready, b"ready");
        assert!(LockedFile::acquire_no_wait(&path, LockedFileMode::Exclusive, "source").is_none());
        shutdown
            .send(())
            .map_err(|()| std::io::Error::other("runtime already closed"))?;
        let completed = tokio::time::timeout(Duration::from_secs(30), finished_shutdown).await;
        if completed.is_err() {
            gate.write_all(b"S").await?;
        }
        tokio::task::spawn_blocking(move || thread.join())
            .await?
            .map_err(|_| std::io::Error::other("runtime owner panicked"))??;
        completed??;
        assert!(LockedFile::acquire_no_wait(&path, LockedFileMode::Exclusive, "source").is_some());
        let mut byte = [0];
        let closed = tokio::time::timeout(Duration::from_secs(30), gate.read(&mut byte)).await?;
        assert!(
            matches!(&closed, Ok(0))
                || matches!(&closed, Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset),
            "the reaped fixture must close its connection: {closed:?}"
        );
        assert_eq!(slots.available_permits(), 1);
        Ok(())
    }
}
