use std::assert_matches;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use tokio::io::{AsyncRead, ReadBuf};

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
    assert_eq!(*progress.borrow(), [5]);

    assert_matches!(
        Pin::new(&mut reader).poll_read(&mut context, &mut buf),
        Poll::Ready(Err(err))
            if err.kind() == io::ErrorKind::Other && err.to_string() == "scripted failure"
    );
    assert_eq!(buf.filled(), b"xyabc");
    assert_eq!(*progress.borrow(), [5]);

    // A successful read reports the whole filled buffer, even if no bytes were added.
    assert_matches!(
        Pin::new(&mut reader).poll_read(&mut context, &mut buf),
        Poll::Ready(Ok(()))
    );
    assert_eq!(buf.filled(), b"xyabc");
    assert_eq!(*progress.borrow(), [5, 5]);

    let mut empty = [];
    let mut buf = ReadBuf::new(&mut empty);
    assert_matches!(
        Pin::new(&mut reader).poll_read(&mut context, &mut buf),
        Poll::Ready(Ok(()))
    );
    assert_eq!(*progress.borrow(), [5, 5, 0]);
}
