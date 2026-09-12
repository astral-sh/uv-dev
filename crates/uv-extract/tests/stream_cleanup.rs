use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use tempfile::TempDir;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::oneshot;
use tokio::time::timeout;

const DEADLINE: Duration = Duration::from_secs(10);

/// A valid ZIP prefix followed by a caller-controlled body error.
struct GatedReader {
    prefix: Vec<u8>,
    position: usize,
    reached: Option<oneshot::Sender<()>>,
    finish: oneshot::Receiver<io::ErrorKind>,
}

impl AsyncRead for GatedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buffer.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.position < self.prefix.len() {
            let count = buffer.remaining().min(self.prefix.len() - self.position);
            buffer.put_slice(&self.prefix[self.position..self.position + count]);
            self.position += count;
            return Poll::Ready(Ok(()));
        }
        if let Some(reached) = self.reached.take() {
            let _ = reached.send(());
        }
        match Pin::new(&mut self.finish).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(kind)) => {
                Poll::Ready(Err(io::Error::new(kind, "injected archive body error")))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(io::Error::other(error))),
        }
    }
}

fn zip_prefix() -> Result<Vec<u8>> {
    futures::executor::block_on(async {
        let mut writer = ZipFileWriter::new(Vec::new());
        writer
            .write_entry_whole(
                ZipEntryBuilder::new("payload.bin".into(), Compression::Stored),
                &vec![b'x'; 1024 * 1024],
            )
            .await?;
        let mut archive = writer.close().await?;
        // Stop inside the file body, before any central-directory validation can finish.
        archive.truncate(archive.len() / 2);
        Ok(archive)
    })
}

fn reader(
    prefix: Vec<u8>,
) -> (
    GatedReader,
    oneshot::Receiver<()>,
    oneshot::Sender<io::ErrorKind>,
) {
    let (reached, progress) = oneshot::channel();
    let (finish, gate) = oneshot::channel();
    (
        GatedReader {
            prefix,
            position: 0,
            reached: Some(reached),
            finish: gate,
        },
        progress,
        finish,
    )
}

async fn extract(
    reader: GatedReader,
    target: TempDir,
    hash_contents: bool,
) -> Result<TempDir, uv_extract::Error> {
    if hash_contents {
        uv_extract::stream::unzip_and_hash(reader, target)
            .await
            .map(|(target, _, _)| target)
    } else {
        uv_extract::stream::unzip(reader, target)
            .await
            .map(|(target, _)| target)
    }
}

/// Runtime shutdown waits for the detached blocking extraction task. The channel bounds the test
/// without asserting cleanup before that task has actually finished.
fn join_runtime(runtime: Runtime) -> Result<()> {
    let (joined, completed) = mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        drop(runtime);
        let _ = joined.send(());
    });
    completed.recv_timeout(DEADLINE)?;
    thread
        .join()
        .map_err(|_| anyhow!("runtime shutdown thread panicked"))?;
    Ok(())
}

#[test]
fn cancelled_streaming_zip_removes_temporary_directory() -> Result<()> {
    for hash_contents in [false, true] {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let target = tempfile::tempdir()?;
        let target_path = target.path().to_path_buf();
        let (reader, progress, finish) = reader(zip_prefix()?);

        let result: Result<()> = runtime.block_on(async {
            let extraction = tokio::spawn(extract(reader, target, hash_contents));
            let progress = timeout(DEADLINE, progress).await;
            extraction.abort();
            let cancelled = extraction.await;
            drop(finish);
            progress??;
            assert!(
                cancelled
                    .expect_err("extraction should be cancelled")
                    .is_cancelled()
            );
            Ok(())
        });

        join_runtime(runtime)?;
        result?;
        assert!(!target_path.exists());
    }
    Ok(())
}

#[test]
fn streaming_zip_preserves_reader_error() -> Result<()> {
    for hash_contents in [false, true] {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let target = tempfile::tempdir()?;
        let target_path = target.path().to_path_buf();
        let (reader, progress, finish) = reader(zip_prefix()?);

        let result: Result<()> = runtime.block_on(async {
            let extraction = tokio::spawn(extract(reader, target, hash_contents));
            timeout(DEADLINE, progress).await??;
            finish
                .send(io::ErrorKind::ConnectionReset)
                .map_err(|_| anyhow!("extraction reader was dropped"))?;
            let error = timeout(DEADLINE, extraction)
                .await??
                .expect_err("injected body error should be returned");
            let uv_extract::Error::Io(error) = error else {
                anyhow::bail!("expected the reader error, got {error:?}");
            };
            assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
            assert_eq!(error.to_string(), "injected archive body error");
            Ok(())
        });

        join_runtime(runtime)?;
        result?;
        assert!(!target_path.exists());
    }
    Ok(())
}
