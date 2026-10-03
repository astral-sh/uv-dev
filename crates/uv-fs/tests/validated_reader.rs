use std::io::{self, Cursor, Read};

use uv_fs::ValidatedReader;

struct ErrorOnce(Option<io::ErrorKind>);

impl Read for ErrorOnce {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(kind) = self.0.take() {
            Err(io::Error::new(kind, "injected read error"))
        } else {
            Ok(0)
        }
    }
}

#[test]
fn rejects_incomplete_utf_8_at_eof() -> io::Result<()> {
    for contents in [b"#!\xc3".as_slice(), b"#!\xe2\x82", b"#!\xf0\x9f\xa6"] {
        assert!(
            ValidatedReader::new(Cursor::new(contents))
                .require_prefix("#!")
                .require_utf8()
                .read()?
                .is_none()
        );
    }
    Ok(())
}

#[test]
fn retries_interrupted_reads_with_incomplete_utf_8() -> io::Result<()> {
    let reader = Cursor::new(b"#!\xf0\x9f")
        .chain(ErrorOnce(Some(io::ErrorKind::Interrupted)))
        .chain(Cursor::new(b"\xa6\x80"));
    assert_eq!(
        ValidatedReader::new(reader)
            .require_prefix("#!")
            .require_utf8()
            .read()?,
        Some(b"#!\xf0\x9f\xa6\x80".to_vec())
    );
    Ok(())
}

#[test]
fn propagates_non_interrupted_errors() {
    let reader = Cursor::new(b"#!text").chain(ErrorOnce(Some(io::ErrorKind::PermissionDenied)));
    let error = ValidatedReader::new(reader)
        .require_prefix("#!")
        .require_utf8()
        .read()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "injected read error");
}
