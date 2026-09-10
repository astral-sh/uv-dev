use std::assert_matches;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Result;
use sha2::{Digest, Sha256, Sha512};
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use uv_extract::hash::{HashReader, Hasher};
use uv_pypi_types::{HashAlgorithm, HashDigest};

#[derive(Debug, PartialEq, Eq)]
enum ReadEvent {
    Pending(usize),
    Read(usize, usize),
    Error(usize, io::ErrorKind),
}

struct ObservedReader<'a> {
    remaining: &'a [u8],
    bytes_read: usize,
    max_chunk: usize,
    failure: Option<(usize, io::ErrorKind)>,
    pending: bool,
    events: Vec<ReadEvent>,
}

impl<'a> ObservedReader<'a> {
    fn new(contents: &'a [u8], max_chunk: usize, failure: Option<(usize, io::ErrorKind)>) -> Self {
        Self {
            remaining: contents,
            bytes_read: 0,
            max_chunk,
            failure,
            pending: true,
            events: Vec::new(),
        }
    }
}

impl AsyncRead for ObservedReader<'_> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let reader = self.get_mut();
        let capacity = buffer.remaining();
        if reader.pending {
            reader.pending = false;
            reader.events.push(ReadEvent::Pending(capacity));
            context.waker().wake_by_ref();
            return Poll::Pending;
        }
        if let Some((position, kind)) = reader.failure
            && reader.bytes_read == position
        {
            reader.events.push(ReadEvent::Error(capacity, kind));
            return Poll::Ready(Err(io::Error::new(kind, "authored read failure")));
        }

        let until_failure = reader
            .failure
            .map_or(usize::MAX, |(position, _)| position - reader.bytes_read);
        let read = reader
            .remaining
            .len()
            .min(capacity)
            .min(reader.max_chunk)
            .min(until_failure);
        buffer.put_slice(&reader.remaining[..read]);
        reader.remaining = &reader.remaining[read..];
        reader.bytes_read += read;
        reader.events.push(ReadEvent::Read(capacity, read));
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn finish_hashes_the_remaining_bytes() -> Result<()> {
    let contents = (0u8..=250).cycle().take(20_000).collect::<Vec<_>>();
    let mut source = ObservedReader::new(&contents, 997, None);
    let mut hashers = [
        Hasher::from(HashAlgorithm::Sha256),
        Hasher::from(HashAlgorithm::Sha512),
    ];
    let mut prefix = [0; 13];
    {
        let mut reader = HashReader::new(&mut source, &mut hashers);
        reader.read_exact(&mut prefix).await?;
        assert_eq!(prefix, contents[..prefix.len()]);
        reader.finish().await?;
        assert_eq!(reader.bytes_read(), contents.len() as u64);
        reader.finish().await?;
        assert_eq!(reader.bytes_read(), contents.len() as u64);
    }

    let mut expected = vec![ReadEvent::Pending(13), ReadEvent::Read(13, 13)];
    let mut remaining = contents.len() - prefix.len();
    while remaining > 0 {
        let read = remaining.min(997);
        expected.push(ReadEvent::Read(8192, read));
        remaining -= read;
    }
    expected.extend([ReadEvent::Read(8192, 0), ReadEvent::Read(8192, 0)]);
    assert_eq!(source.events, expected);
    assert_eq!(
        hashers.map(|hasher| HashDigest::from(hasher).to_string()),
        [
            format!("sha256:{}", hex::encode(Sha256::digest(&contents))),
            format!("sha512:{}", hex::encode(Sha512::digest(&contents))),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn finish_preserves_terminal_read_errors() {
    let contents = b"abcdefghijklmnopqrstuvwxyz";
    for kind in [io::ErrorKind::Interrupted, io::ErrorKind::Other] {
        let mut source = ObservedReader::new(contents, 7, Some((19, kind)));
        let mut hashers = [Hasher::from(HashAlgorithm::Sha256)];
        {
            let mut reader = HashReader::new(&mut source, &mut hashers);
            for _ in 0..2 {
                assert_matches!(
                    reader.finish().await,
                    Err(error) if error.kind() == kind && error.to_string() == "authored read failure"
                );
                assert_eq!(reader.bytes_read(), 19);
            }
        }
        assert_eq!(
            source.events,
            [
                ReadEvent::Pending(8192),
                ReadEvent::Read(8192, 7),
                ReadEvent::Read(8192, 7),
                ReadEvent::Read(8192, 5),
                ReadEvent::Error(8192, kind),
                ReadEvent::Error(8192, kind),
            ]
        );
        assert_eq!(
            hashers.map(|hasher| HashDigest::from(hasher).to_string()),
            [format!(
                "sha256:{}",
                hex::encode(Sha256::digest(&contents[..19]))
            )]
        );
    }
}

#[tokio::test]
async fn finish_accepts_empty_input_without_hashers() -> Result<()> {
    let mut source = ObservedReader::new(b"", 7, None);
    let mut hashers = [];
    {
        let mut reader = HashReader::new(&mut source, &mut hashers);
        reader.finish().await?;
        reader.finish().await?;
        assert_eq!(reader.bytes_read(), 0);
    }
    assert_eq!(
        source.events,
        [
            ReadEvent::Pending(8192),
            ReadEvent::Read(8192, 0),
            ReadEvent::Read(8192, 0),
        ]
    );
    Ok(())
}
