use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use uv_extract::dirhash::blake3_copy;

enum ReadStep {
    Bytes(&'static [u8]),
    Error(io::ErrorKind, &'static str),
}

struct ScriptedReader(VecDeque<ReadStep>);

impl AsyncRead for ScriptedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }

        match self.0.pop_front() {
            Some(ReadStep::Bytes(bytes)) => {
                let length = bytes.len().min(buf.remaining());
                buf.put_slice(&bytes[..length]);
                if length < bytes.len() {
                    self.0.push_front(ReadStep::Bytes(&bytes[length..]));
                }
                Poll::Ready(Ok(()))
            }
            Some(ReadStep::Error(kind, message)) => Poll::Ready(Err(io::Error::new(kind, message))),
            None => Poll::Ready(Ok(())),
        }
    }
}

#[derive(Default)]
struct RecordingWriter {
    bytes: Vec<u8>,
    fail_write_after: Option<(usize, io::ErrorKind, &'static str)>,
    fail_flush: Option<(io::ErrorKind, &'static str)>,
    flushed: bool,
}

impl AsyncWrite for RecordingWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let length = if let Some((limit, kind, message)) = self.fail_write_after {
            let remaining = limit.saturating_sub(self.bytes.len());
            if remaining == 0 {
                return Poll::Ready(Err(io::Error::new(kind, message)));
            }
            buf.len().min(remaining)
        } else {
            buf.len()
        };
        self.bytes.extend_from_slice(&buf[..length]);
        Poll::Ready(Ok(length))
    }

    fn poll_flush(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.flushed = true;
        Poll::Ready(match self.fail_flush {
            Some((kind, message)) => Err(io::Error::new(kind, message)),
            None => Ok(()),
        })
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn interrupted_reads_are_retried() -> io::Result<()> {
    let mut reader = ScriptedReader(VecDeque::from([
        ReadStep::Bytes(b"before "),
        ReadStep::Error(io::ErrorKind::Interrupted, "authored interruption"),
        ReadStep::Bytes(b"after"),
    ]));
    let mut writer = RecordingWriter::default();

    let (count, hash) = blake3_copy(&mut reader, &mut writer).await?;

    assert_eq!(writer.bytes, b"before after");
    assert_eq!(count, writer.bytes.len() as u64);
    assert_eq!(hash, blake3::hash(&writer.bytes));
    assert!(reader.0.is_empty());
    assert!(writer.flushed);
    Ok(())
}

#[tokio::test]
async fn read_errors_are_preserved() {
    let reader = ScriptedReader(VecDeque::from([ReadStep::Error(
        io::ErrorKind::PermissionDenied,
        "authored read failure",
    )]));
    let mut writer = RecordingWriter::default();

    let error = blake3_copy(reader, &mut writer)
        .await
        .expect_err("the reader must fail");

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "authored read failure");
    assert!(writer.bytes.is_empty());
    assert!(!writer.flushed);
}

#[tokio::test]
async fn write_errors_are_preserved() {
    let input = b"authored payload";
    let mut writer = RecordingWriter {
        fail_write_after: Some((4, io::ErrorKind::BrokenPipe, "authored write failure")),
        ..RecordingWriter::default()
    };

    let error = blake3_copy(input.as_slice(), &mut writer)
        .await
        .expect_err("the writer must fail");

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(error.to_string(), "authored write failure");
    assert_eq!(writer.bytes, &input[..4]);
    assert!(!writer.flushed);
}

#[tokio::test]
async fn flush_errors_are_preserved() {
    for input in [b"".as_slice(), b"authored payload".as_slice()] {
        let mut writer = RecordingWriter {
            fail_flush: Some((io::ErrorKind::ConnectionReset, "authored flush failure")),
            ..RecordingWriter::default()
        };

        let error = blake3_copy(input, &mut writer)
            .await
            .expect_err("flushing the writer must fail");

        assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
        assert_eq!(error.to_string(), "authored flush failure");
        assert_eq!(writer.bytes, input);
        assert!(writer.flushed);
    }
}
