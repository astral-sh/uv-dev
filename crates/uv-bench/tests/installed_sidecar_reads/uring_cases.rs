#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::env;
    use std::ffi::OsStr;
    use std::io::{self, Write};
    use std::mem;
    use std::num::NonZeroU32;
    use std::os::fd::{AsRawFd, IntoRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::from_utf8;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use rustix::fs::{Dir, Mode, OFlags, open};
    use rustix::io::Errno;

    use crate::read_results::{comparable, comparable_raw, read_file as ordinary, read_raw};

    use super::{
        MAX_STALLED_ATTEMPTS, Operation, PendingBatch, Progress, READ_SIZE, ReadResult, Reader,
        Request, new_buffer, retryable_control_error, unavailable_worker_registration,
    };

    fn assert_results(paths: &[PathBuf], actual: &[ReadResult]) {
        assert_eq!(actual.len(), paths.len());
        for (path, actual) in paths.iter().zip(actual) {
            assert_eq!(comparable(actual), comparable(&ordinary(path)));
            assert_eq!(comparable_raw(actual), comparable_raw(&read_raw(path)));
        }
    }

    fn open_descriptors() -> io::Result<BTreeSet<i32>> {
        let directory = Dir::new(open(
            "/proc/self/fd",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?)?;
        let observer = directory.fd()?.as_raw_fd();
        let mut descriptors = BTreeSet::new();
        for entry in directory {
            let entry = entry?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            let descriptor = from_utf8(name)
                .map_err(io::Error::other)?
                .parse::<i32>()
                .map_err(io::Error::other)?;
            if descriptor != observer {
                descriptors.insert(descriptor);
            }
        }
        Ok(descriptors)
    }

    #[test]
    fn short_reads_continue_until_eof() -> io::Result<()> {
        let mut request = Request::new(Path::new("file"), new_buffer());
        request.complete(Operation::Open, tempfile::tempfile()?.into_raw_fd())?;
        request
            .buffer
            .as_mut()
            .expect("owned read buffer")
            .get_mut()[..3]
            .copy_from_slice(b"abc");
        request.complete(Operation::Read, 3)?;
        assert!(request.result.is_none());
        assert!(request.file.is_some());
        request.complete(Operation::Read, -Errno::INTR.raw_os_error())?;
        assert!(request.result.is_none());

        request
            .buffer
            .as_mut()
            .expect("owned read buffer")
            .get_mut()[..2]
            .copy_from_slice(b"de");
        request.complete(Operation::Read, 2)?;
        assert!(request.result.is_none());
        request.complete(Operation::Read, 0)?;
        assert_eq!(
            request.result.take().expect("completed read")?,
            Some(b"abcde".to_vec())
        );
        assert!(request.file.is_none());
        Ok(())
    }

    #[test]
    fn only_open_not_found_means_missing() -> io::Result<()> {
        let mut missing = Request::new(Path::new("missing"), new_buffer());
        missing.complete(Operation::Open, -Errno::NOENT.raw_os_error())?;
        assert_eq!(missing.result.take().expect("completed open")?, None);

        let mut unreadable = Request::new(Path::new("unreadable"), new_buffer());
        unreadable.complete(Operation::Open, -Errno::ACCESS.raw_os_error())?;
        assert_eq!(
            unreadable
                .result
                .take()
                .expect("completed open")
                .expect_err("permission errors are not missing files")
                .raw_os_error(),
            Some(Errno::ACCESS.raw_os_error())
        );

        let mut read_failure = Request::new(Path::new("read-failure"), new_buffer());
        read_failure.complete(Operation::Open, tempfile::tempfile()?.into_raw_fd())?;
        read_failure.complete(Operation::Read, -Errno::NOENT.raw_os_error())?;
        assert_eq!(
            read_failure
                .result
                .take()
                .expect("completed read")
                .expect_err("read errors are not missing optional files")
                .raw_os_error(),
            Some(Errno::NOENT.raw_os_error())
        );
        assert!(read_failure.file.is_none());
        Ok(())
    }

    #[test]
    fn raw_comparison_keeps_distinct_permission_errors() {
        let permission: ReadResult = Err(Errno::PERM.into());
        let access: ReadResult = Err(Errno::ACCESS.into());
        assert_eq!(comparable(&permission), comparable(&access));
        assert_ne!(comparable_raw(&permission), comparable_raw(&access));
    }

    #[test]
    fn invalid_path_has_the_ordinary_error_kind() {
        let path = Path::new(OsStr::from_bytes(b"invalid\0path"));
        let mut request = Request::new(path, new_buffer());
        let actual = request.result.take().expect("invalid path result");
        assert_eq!(comparable(&actual), comparable(&ordinary(path)));
        assert_eq!(comparable_raw(&actual), comparable_raw(&read_raw(path)));
    }

    #[test]
    fn retirement_discards_only_non_kernel_data() -> io::Result<()> {
        let paths = vec![PathBuf::from("file")];
        let mut batch = PendingBatch::new(&paths, vec![new_buffer()])?;
        let request = &mut batch.requests[0];
        request.file = Some(tempfile::tempfile()?.into());
        request.contents = vec![1; READ_SIZE * 2];
        request.result = Some(Ok(Some(vec![2; READ_SIZE * 2])));
        let path = request.path.as_ref().expect("owned pathname").as_ptr();
        let buffer = request.buffer.as_ref().expect("owned buffer").get();
        let descriptor = request.file.as_ref().map(AsRawFd::as_raw_fd);

        batch.discard_results();
        let request = &batch.requests[0];
        assert_eq!(request.contents.capacity(), 0);
        assert!(request.result.is_none());
        assert_eq!(
            request.path.as_ref().expect("owned pathname").as_ptr(),
            path
        );
        assert_eq!(request.buffer.as_ref().expect("owned buffer").get(), buffer);
        assert_eq!(request.file.as_ref().map(AsRawFd::as_raw_fd), descriptor);
        Ok(())
    }

    #[test]
    fn control_retries_require_progress() -> io::Result<()> {
        let mut progress = Progress::default();
        for _ in 0..MAX_STALLED_ATTEMPTS {
            progress.observe(0, 2)?;
        }
        progress.observe(0, 1)?;
        for _ in 1..MAX_STALLED_ATTEMPTS {
            progress.observe(0, 1)?;
        }
        progress.observe(1, 1)?;
        for _ in 1..MAX_STALLED_ATTEMPTS {
            progress.observe(1, 1)?;
        }
        assert_eq!(
            progress
                .observe(1, 1)
                .expect_err("unchanged control retries must be bounded")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        Ok(())
    }

    #[test]
    fn worker_registration_unavailability_is_distinct_from_other_errors() {
        for error in [Errno::NOSYS, Errno::OPNOTSUPP, Errno::INVAL] {
            assert!(unavailable_worker_registration(&error.into()));
        }
        assert!(unavailable_worker_registration(&io::Error::from(
            io::ErrorKind::Unsupported
        )));
        assert!(!unavailable_worker_registration(&Errno::PERM.into()));
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_reader_reuses_buffers_across_batches_and_eof_reads() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let mut paths = Vec::new();
        for length in [
            0,
            1,
            READ_SIZE - 1,
            READ_SIZE,
            READ_SIZE + 1,
            READ_SIZE * 3 + 17,
        ] {
            let path = root.path().join(format!("length-{length}"));
            let contents = (0..length)
                .map(|index| u8::try_from(index % 251).expect("small byte value"))
                .collect::<Vec<_>>();
            fs_err::write(&path, &contents)?;
            paths.push(path);
        }
        let non_utf8 = root.path().join(OsStr::from_bytes(b"file-\xff"));
        fs_err::write(&non_utf8, b"non-UTF-8 filename")?;
        paths.push(non_utf8);
        let symlink = root.path().join("symlink");
        fs_err::os::unix::fs::symlink(&paths[1], &symlink)?;
        paths.push(symlink);
        let dangling = root.path().join("dangling");
        fs_err::os::unix::fs::symlink(root.path().join("absent"), &dangling)?;
        paths.push(dangling);
        paths.push(root.path().join("missing"));
        paths.push(root.path().to_path_buf());

        let mut reader = Reader::new(NonZeroU32::new(4).expect("non-zero queue depth"))?;
        let descriptor = reader.ring.as_ref().map(AsRawFd::as_raw_fd);
        assert_eq!(
            reader
                .worker_limits()
                .expect_err("Worker limits require an actual completed request")
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_results(&paths, &reader.read(&paths)?);
        let worker_limits = reader.worker_limits()?;
        for _ in 0..2 {
            assert_results(&paths, &reader.read(&paths)?);
            assert_eq!(reader.worker_limits()?, worker_limits);
            assert!(reader.pending.is_none());
            assert_eq!(reader.buffers.len(), reader.capacity);
            assert_eq!(reader.ring.as_ref().map(AsRawFd::as_raw_fd), descriptor);
        }
        assert!(reader.read(&[])?.is_empty());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host with procfs"]
    fn active_reader_keeps_descriptor_count_stable() -> io::Result<()> {
        const CHILD_ENV: &str = "UV_BENCH_SMALL_READ_FD_CHILD";
        let test_name = concat!(
            module_path!(),
            "::active_reader_keeps_descriptor_count_stable"
        )
        .split_once("::")
        .expect("test module path includes its crate")
        .1;
        if env::var(CHILD_ENV).as_deref() != Ok(test_name) {
            // The unavailable-drain test deliberately retains descriptors. Run this one test in
            // a fresh process so neither that test nor parallel libtest activity can affect the
            // descriptor snapshots. The child marker prevents recursive re-execution, while the
            // exact filter keeps all other tests out of the child process.
            let output = Command::new(env::current_exe()?)
                .args([
                    "--exact",
                    test_name,
                    "--ignored",
                    "--test-threads=1",
                    "--format=pretty",
                    "--color=never",
                ])
                .env(CHILD_ENV, test_name)
                .output()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "isolated descriptor test failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                stdout
                    .lines()
                    .filter(|line| *line == "running 1 test")
                    .count(),
                1,
                "isolated process did not execute the exact descriptor test: {stdout}"
            );
            return Ok(());
        }

        let root = tempfile::tempdir()?;
        let paths = (0..37)
            .map(|index| {
                let path = root.path().join(format!("file-{index}"));
                fs_err::write(&path, vec![b'x'; index * 257])?;
                Ok(path)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let error_paths = vec![
            root.path().to_path_buf(),
            root.path().join("missing"),
            paths[0].join("not-a-directory"),
        ];
        assert_eq!(
            read_raw(&error_paths[0])
                .expect_err("reading a directory must fail")
                .raw_os_error(),
            Some(Errno::ISDIR.raw_os_error())
        );
        assert_eq!(
            read_raw(&error_paths[2])
                .expect_err("opening a child of a file must fail")
                .raw_os_error(),
            Some(Errno::NOTDIR.raw_os_error())
        );
        let without_ring = open_descriptors()?;
        for queue_depth in [1, 4, 16] {
            let mut reader =
                Reader::new(NonZeroU32::new(queue_depth).expect("non-zero queue depth"))?;
            assert_results(&paths, &reader.read(&paths)?);
            let errors = reader.read(&error_paths)?;
            assert_results(&error_paths, &errors);
            assert_eq!(
                errors[0]
                    .as_ref()
                    .expect_err("ring directory reads must fail")
                    .raw_os_error(),
                Some(Errno::ISDIR.raw_os_error())
            );
            assert_eq!(
                errors[2]
                    .as_ref()
                    .expect_err("ring opens through a file must fail")
                    .raw_os_error(),
                Some(Errno::NOTDIR.raw_os_error())
            );
            let with_ring = open_descriptors()?;
            assert_eq!(with_ring.len(), without_ring.len() + 1);
            for _ in 0..64 {
                assert_results(&paths, &reader.read(&paths)?);
                assert_results(&error_paths, &reader.read(&error_paths)?);
                assert!(reader.pending.is_none());
                assert_eq!(reader.buffers.len(), reader.capacity);
                assert_eq!(open_descriptors()?, with_ring);
            }
            drop(reader);
            assert_eq!(open_descriptors()?, without_ring);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_reader_retries_transient_control_errors() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let paths = vec![root.path().join("file")];
        fs_err::write(&paths[0], b"contents")?;
        let mut reader = Reader::new(NonZeroU32::MIN)?;
        let mut failures = [Errno::INTR, Errno::AGAIN, Errno::BUSY].into_iter();
        let actual = reader.read_with(&paths, &mut |ring, want| {
            if let Some(error) = failures.next() {
                return Err(error.into());
            }
            ring.submit_and_wait(want)
        })?;
        assert_results(&paths, &actual);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_some());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_reader_retires_after_stalled_control_errors() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let paths = vec![root.path().join("file")];
        fs_err::write(&paths[0], b"contents")?;
        let mut reader = Reader::new(NonZeroU32::MIN)?;
        let attempts = Cell::new(0);
        let error = reader
            .read_with(&paths, &mut |_, _| {
                attempts.set(attempts.get() + 1);
                Err(Errno::AGAIN.into())
            })
            .expect_err("a stalled reader must not report a successful read");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(attempts.get(), MAX_STALLED_ATTEMPTS);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_none());
        assert_eq!(
            reader
                .read(&paths)
                .expect_err("a retired reader must not create another ring")
                .kind(),
            io::ErrorKind::Unsupported
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_reader_reclaims_partially_submitted_opens_after_drain() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let paths = (0..4)
            .map(|index| {
                let path = root.path().join(format!("file-{index}"));
                fs_err::write(&path, b"contents")?;
                Ok(path)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut reader = Reader::new(NonZeroU32::new(4).expect("non-zero queue depth"))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(&paths, mem::take(&mut reader.buffers))?;
        batch.drop_counter = Some(Arc::clone(&counter));
        reader.pending = Some(batch);
        let submitted = Cell::new(0);
        let error = reader
            .complete_batch(&mut |ring, _| {
                assert_eq!(counter.load(Ordering::SeqCst), 0);
                let count = loop {
                    // SAFETY: Submit only one valid OPENAT entry without waiting or passing a
                    // signal mask. The reader continues to own all pending request storage.
                    match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                        Ok(count) => break count,
                        Err(error) if retryable_control_error(&error) => {}
                        Err(error) => return Err(error),
                    }
                };
                submitted.set(count);
                Err(Errno::PERM.into())
            })
            .expect_err("injected control error must fail the batch");
        assert_eq!(error.raw_os_error(), Some(Errno::PERM.raw_os_error()));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        reader.retire();
        assert_eq!(submitted.get(), 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_reader_keeps_in_flight_reads_alive_during_retirement() -> io::Result<()> {
        let paths = vec![PathBuf::from("blocked"), PathBuf::from("unsubmitted")];
        let mut reader = Reader::new(NonZeroU32::new(2).expect("non-zero queue depth"))?;
        let (socket, mut writer) = UnixStream::pair()?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(&paths, mem::take(&mut reader.buffers))?;
        batch.requests[0].file = Some(socket.into());
        batch.requests[1].file = Some(tempfile::tempfile()?.into());
        batch.drop_counter = Some(Arc::clone(&counter));
        reader.pending = Some(batch);
        reader.queue()?;

        let (observer, observed) = mpsc::sync_channel(0);
        reader.drain_observer = Some(observer);
        let submitted = Cell::new(0);
        std::thread::scope(|scope| -> io::Result<()> {
            let counter = Arc::clone(&counter);
            let release = scope.spawn(move || -> io::Result<()> {
                let observation = observed.recv_timeout(Duration::from_secs(10));
                let requests_alive = counter.load(Ordering::SeqCst) == 0;
                // Always release the read, even if observing the drain failed, so cleanup cannot
                // leave the test process waiting forever on an unreadable socket.
                let released = writer.write_all(b"x");
                observation.map_err(io::Error::other)?;
                assert!(
                    requests_alive,
                    "request storage was reclaimed before completion"
                );
                released
            });
            let error = reader
                .finish_queued(&mut |ring, _| {
                    let count = loop {
                        // SAFETY: Only the blocked READ is submitted. Its owned socket and fixed
                        // buffer remain in the pending batch until the drain observes its CQE.
                        match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                            Ok(count) => break count,
                            Err(error) if retryable_control_error(&error) => {}
                            Err(error) => return Err(error),
                        }
                    };
                    submitted.set(count);
                    Err(Errno::PERM.into())
                })
                .expect_err("injected control error must fail the read");
            assert_eq!(error.raw_os_error(), Some(Errno::PERM.raw_os_error()));
            reader.retire();
            release.join().expect("read-release thread failed")?;
            Ok(())
        })?;
        assert_eq!(submitted.get(), 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_reader_retains_requests_if_drain_becomes_unavailable() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let paths = vec![root.path().join("file")];
        fs_err::write(&paths[0], b"contents")?;
        let mut reader = Reader::new(NonZeroU32::MIN)?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(&paths, mem::take(&mut reader.buffers))?;
        batch.drop_counter = Some(Arc::clone(&counter));
        reader.pending = Some(batch);
        reader.queue()?;
        let ring = reader.ring.as_ref().expect("initialized ring");
        let submitted = loop {
            // SAFETY: Submit the valid OPENAT without consuming its CQE. All request storage is
            // still owned, and this test deliberately exercises the unavailable-drain branch.
            match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                Ok(count) => break count,
                Err(error) if retryable_control_error(&error) => {}
                Err(error) => return Err(error),
            }
        };
        assert_eq!(submitted, 1);

        // One bounded batch is deliberately retained until this test process exits.
        reader.retire_after_drain(&Err(Errno::PERM.into()));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(reader.pending.is_none());
        assert!(reader.ring.is_none());
        assert_eq!(
            reader
                .read(&paths)
                .expect_err("a retired reader must not retain another batch")
                .kind(),
            io::ErrorKind::Unsupported
        );
        Ok(())
    }
}
