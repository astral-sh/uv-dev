use std::assert_matches;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::ProgressReader;

enum Step {
    Pending,
    Bytes(&'static [u8]),
    Error,
    Eof,
}

struct ScriptedReader {
    steps: VecDeque<Step>,
}

impl AsyncRead for ScriptedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.steps.pop_front().expect("unexpected read") {
            Step::Pending => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Step::Bytes(bytes) => {
                buf.put_slice(bytes);
                Poll::Ready(Ok(()))
            }
            Step::Error => Poll::Ready(Err(io::Error::other("scripted failure"))),
            Step::Eof => Poll::Ready(Ok(())),
        }
    }
}

#[test]
fn progress_reader_preserves_read_contract() {
    let progress = RefCell::new(Vec::new());
    let mut reader = ProgressReader::new(
        ScriptedReader {
            steps: [
                Step::Pending,
                Step::Bytes(b"abc"),
                Step::Error,
                Step::Eof,
                Step::Eof,
            ]
            .into(),
        },
        |bytes| progress.borrow_mut().push(bytes),
    );
    let mut context = Context::from_waker(Waker::noop());
    let mut storage = [0; 16];
    let mut buf = ReadBuf::new(&mut storage);
    buf.put_slice(b"xy");

    assert_matches!(
        Pin::new(&mut reader).poll_read(&mut context, &mut buf),
        Poll::Pending
    );
    assert_eq!(buf.filled(), b"xy");
    assert!(progress.borrow().is_empty());

    assert_matches!(
        Pin::new(&mut reader).poll_read(&mut context, &mut buf),
        Poll::Ready(Ok(()))
    );
    assert_eq!(buf.filled(), b"xyabc");
    assert_eq!(*progress.borrow(), [3]);

    assert_matches!(
        Pin::new(&mut reader).poll_read(&mut context, &mut buf),
        Poll::Ready(Err(err))
            if err.kind() == io::ErrorKind::Other && err.to_string() == "scripted failure"
    );
    assert_eq!(buf.filled(), b"xyabc");
    assert_eq!(*progress.borrow(), [3]);

    // A successful read still reports progress when no bytes were added.
    assert_matches!(
        Pin::new(&mut reader).poll_read(&mut context, &mut buf),
        Poll::Ready(Ok(()))
    );
    assert_eq!(buf.filled(), b"xyabc");
    assert_eq!(*progress.borrow(), [3, 0]);

    let mut empty = [];
    let mut buf = ReadBuf::new(&mut empty);
    assert_matches!(
        Pin::new(&mut reader).poll_read(&mut context, &mut buf),
        Poll::Ready(Ok(()))
    );
    assert_eq!(*progress.borrow(), [3, 0, 0]);
}

#[derive(Default)]
struct PendingOnceWriter {
    pending: bool,
    contents: Vec<u8>,
}

impl AsyncWrite for PendingOnceWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if std::mem::take(&mut self.pending) {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.contents.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[test]
fn progress_reader_counts_copy_buffer_top_ups() {
    let progress = RefCell::new(Vec::new());
    let mut reader = ProgressReader::new(
        ScriptedReader {
            steps: [Step::Bytes(b"ab"), Step::Bytes(b"cd"), Step::Eof].into(),
        },
        |bytes| progress.borrow_mut().push(bytes),
    );
    let mut writer = PendingOnceWriter {
        pending: true,
        ..PendingOnceWriter::default()
    };
    let mut context = Context::from_waker(Waker::noop());

    {
        let mut copy = std::pin::pin!(tokio::io::copy(&mut reader, &mut writer));
        assert_matches!(copy.as_mut().poll(&mut context), Poll::Pending);
        assert_eq!(*progress.borrow(), [2, 2]);
        assert_matches!(copy.as_mut().poll(&mut context), Poll::Ready(Ok(4)));
    }

    assert_eq!(writer.contents.as_slice(), b"abcd");
    assert_eq!(*progress.borrow(), [2, 2, 0]);
}
