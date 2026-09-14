#[cfg(test)]
mod tests {
    use std::fs::FileTimes;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::mpsc;

    use globwalk::{FileType as GlobFileType, GlobWalkerBuilder};
    use uv_cache_info::CacheInfo;

    use super::*;

    fn configuration(queue_depth: u32) -> Configuration {
        Configuration {
            queue_depth,
            force_async: false,
            max_workers: None,
        }
    }

    fn entry(path: &Path) -> io::Result<DirEntry> {
        walkdir::WalkDir::new(path)
            .max_depth(0)
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::other("fixture path was not enumerated"))?
            .map_err(io::Error::other)
    }

    fn files(root: &Path, count: usize) -> io::Result<Vec<DirEntry>> {
        (0..count)
            .map(|index| {
                let path = root.join(format!("file-{index}.py"));
                fs_err::write(&path, [])?;
                entry(&path)
            })
            .collect()
    }

    fn ordinary(entries: &[DirEntry]) -> Vec<(PathBuf, Timestamp)> {
        entries
            .iter()
            .cloned()
            .map(|entry| GlobEntryMetadata::read(Ok(entry)))
            .filter_map(GlobEntryMetadata::into_timestamp)
            .collect()
    }

    #[test]
    fn progress_resets_only_after_submission_or_completion() -> io::Result<()> {
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

        let mut progress = Progress::default();
        progress.observe(1, 1)?;
        assert_eq!(
            progress
                .observe(0, 1)
                .expect_err("completion count must not decrease")
                .kind(),
            io::ErrorKind::InvalidData
        );
        let mut progress = Progress::default();
        progress.observe(1, 1)?;
        assert_eq!(
            progress
                .observe(1, 2)
                .expect_err("unsubmitted count must not increase")
                .kind(),
            io::ErrorKind::InvalidData
        );
        Ok(())
    }

    #[test]
    fn completion_indices_and_duplicates_are_checked() -> io::Result<()> {
        assert_eq!(completion_index(1, 0, 2)?, 1);
        for (user_data, flags, request_count) in [(2, 0, 2), (0, 1, 2), (0, 0, 0)] {
            assert_eq!(
                completion_index(user_data, flags, request_count)
                    .expect_err("invalid completion must be rejected")
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
        let result = Cell::new(None);
        record_result(&result, 0)?;
        assert_eq!(result.get(), Some(0));
        assert_eq!(
            record_result(&result, 0)
                .expect_err("a request can complete only once")
                .kind(),
            io::ErrorKind::InvalidData
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_uses_ctime_follows_leaf_symlinks_and_reuses_ring() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("src");
        fs_err::create_dir(&source)?;
        fs_err::write(
            root.path().join("pyproject.toml"),
            "[tool.uv]\ncache-keys = [{ file = \"src/**/*.py\" }]\n",
        )?;
        let regular = source.join("regular.py");
        fs_err::write(&regular, b"regular\n")?;
        let old_mtime = UNIX_EPOCH + Duration::from_hours(262_968);
        fs_err::File::open(&regular)?.set_times(FileTimes::new().set_modified(old_mtime))?;
        let expected_ctime = Timestamp::from_path(&regular)?;
        assert_ne!(expected_ctime, Timestamp::from(old_mtime));
        let outside = root.path().join("outside.py");
        fs_err::write(&outside, b"target\n")?;
        let directory = root.path().join("outside-directory");
        fs_err::create_dir(&directory)?;
        fs_err::write(directory.join("not-traversed.py"), b"outside\n")?;
        fs_err::os::unix::fs::symlink("../outside.py", source.join("link.py"))?;
        fs_err::os::unix::fs::symlink("../outside-directory", source.join("directory.py"))?;
        fs_err::os::unix::fs::symlink("../absent.py", source.join("dangling.py"))?;
        let entries = GlobWalkerBuilder::from_patterns(root.path(), &["src/**/*.py"])
            .file_type(GlobFileType::FILE | GlobFileType::SYMLINK)
            .build()
            .map_err(io::Error::other)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)?;
        let expected = ordinary(&entries);
        assert_eq!(expected.len(), 2);
        assert!(expected.contains(&(regular.clone(), expected_ctime)));
        assert!(expected.contains(&(source.join("link.py"), Timestamp::from_path(&outside)?)));

        let mut scanner = Scanner::new(configuration(2))?;
        let descriptor = scanner.ring.as_ref().map(AsRawFd::as_raw_fd);
        assert_eq!(scanner.probe(&entry(&regular)?)?, expected_ctime);
        assert_eq!(scanner.metadata(&entries)?, expected);
        assert_eq!(
            CacheInfo::from_directory_with_glob_collector(root.path(), &mut scanner)
                .map_err(io::Error::other)?,
            CacheInfo::from_directory(root.path()).map_err(io::Error::other)?
        );

        // Successful symlink-to-directory STATX calls must count, even though they yield no key.
        let successful = entries
            .into_iter()
            .filter(|entry| !entry.path().ends_with("dangling.py"))
            .collect::<Vec<_>>();
        scanner.require_ring_only();
        for _ in 0..2 {
            let before = scanner.activity();
            assert_eq!(scanner.metadata(&successful)?, expected);
            scanner.assert_activity(before, successful.len());
        }
        let before = scanner.activity();
        assert!(scanner.metadata(&[])?.is_empty());
        scanner.assert_activity(before, 0);
        assert_eq!(scanner.ring.as_ref().map(AsRawFd::as_raw_fd), descriptor);
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_handles_missing_after_enumeration_without_timed_fallback() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 1)?;
        fs_err::remove_file(entries[0].path())?;
        let mut scanner = Scanner::new(configuration(1))?;
        assert_eq!(scanner.metadata(&entries)?, ordinary(&entries));
        assert_eq!(scanner.activity.ordinary, 1);

        scanner.require_ring_only();
        let before = scanner.activity();
        assert_eq!(
            scanner
                .metadata(&entries)
                .expect_err("strict timing must not retry missing paths with ordinary metadata")
                .kind(),
            io::ErrorKind::NotFound
        );
        scanner.assert_activity(before, 0);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_some());
        Ok(())
    }

    #[test]
    #[ignore = "requires Linux 5.15+ with io-wq worker-limit registration"]
    fn active_backend_verifies_worker_limits_after_first_submission() -> io::Result<()> {
        // io-wq limits belong to the submitting task on Linux 5.15. Isolate this registration
        // from the test harness's other rings instead of changing a reused test worker's pool.
        std::thread::spawn(|| -> io::Result<()> {
            let root = tempfile::tempdir()?;
            let entries = files(root.path(), 3)?;
            let mut scanner = Scanner::new(Configuration {
                queue_depth: 2,
                force_async: true,
                max_workers: Some(16),
            })?;
            scanner.require_ring_only();
            let before = scanner.activity();
            assert_eq!(scanner.metadata(&entries)?, ordinary(&entries));
            scanner.assert_activity(before, entries.len());
            assert!(scanner.verify_workers.is_none());
            assert_eq!(scanner.worker_limits()?, Some([16, 16]));
            Ok(())
        })
        .join()
        .expect("worker-limit test thread panicked")
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_retries_unsubmitted_control_failures() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 1)?;
        let expected = Timestamp::from_path(entries[0].path())?;
        let mut scanner = Scanner::new(configuration(1))?;
        let mut failures = [Errno::INTR, Errno::AGAIN, Errno::BUSY].into_iter();
        let requests = scanner.read_batch_with(entries, &mut |ring, want| {
            if let Some(error) = failures.next() {
                return Err(error.into());
            }
            ring.submit_and_wait(want)
        })?;
        assert_eq!(requests[0].timestamp()?, Some(expected));
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_some());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_backend_retires_after_stalled_control_errors() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 1)?;
        let mut scanner = Scanner::new(configuration(1))?;
        let attempts = Cell::new(0);
        let result = scanner.read_batch_with(entries, &mut |_, _| {
            attempts.set(attempts.get() + 1);
            Err(Errno::AGAIN.into())
        });
        assert_eq!(
            result
                .map(drop)
                .expect_err("stalled control must fail")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(attempts.get(), MAX_STALLED_ATTEMPTS);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_backend_retires_partial_submission_before_reclaiming_requests() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 4)?;
        let mut scanner = Scanner::new(configuration(4))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(Arc::clone(&scanner.directory), entries)?;
        batch.drop_counter = Some(Arc::clone(&counter));
        scanner.pending = Some(batch);
        scanner.queue()?;
        let submitted = Cell::new(0);
        let result = scanner.complete_queued_batch(&mut |ring, _| {
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            let count = loop {
                // SAFETY: Submit only the first valid statx entry, with no wait, signal mask, or
                // extended arguments. The scanner still owns every request in the batch.
                match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                    Ok(count) => break count,
                    Err(error) if retryable_control_error(&error) => {}
                    Err(error) => return Err(error),
                }
            };
            submitted.set(count);
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            Err(Errno::PERM.into())
        });
        assert_eq!(
            result
                .expect_err("fatal control error must retire the ring")
                .raw_os_error(),
            Some(Errno::PERM.raw_os_error())
        );
        assert_eq!(submitted.get(), 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_backend_retains_requests_when_draining_is_unavailable() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 1)?;
        let mut scanner = Scanner::new(configuration(1))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(Arc::clone(&scanner.directory), entries)?;
        batch.drop_counter = Some(Arc::clone(&counter));
        scanner.pending = Some(batch);
        scanner.queue()?;
        let ring = scanner.ring.as_ref().expect("initialized ring");
        let submitted = loop {
            // SAFETY: The scanner owns the queued statx request and its stable storage. Do not
            // wait or consume its CQE; reclamation still requires the scanner's drain barrier.
            match unsafe { ring.submitter().enter::<()>(1, 0, 0, None) } {
                Ok(count) => break count,
                Err(error) if retryable_control_error(&error) => {}
                Err(error) => return Err(error),
            }
        };
        assert_eq!(submitted, 1);

        // Simulate inaccessible completion control after a real submission. This deliberately
        // retains one bounded batch until the test process exits.
        scanner.retire_after_drain(&Err(Errno::PERM.into()));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_backend_keeps_in_flight_metadata_alive_during_retirement() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let entries = files(root.path(), 4)?;
        let mut scanner = Scanner::new(configuration(4))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut batch = PendingBatch::new(Arc::clone(&scanner.directory), entries)?;
        batch.drop_counter = Some(Arc::clone(&counter));
        scanner.pending = Some(batch);
        let (reader, mut writer) = UnixStream::pair()?;

        // Link statx behind an unreadable socket's poll so its buffer remains in flight until
        // the observer releases the socket. The remaining two SQEs stay unsubmitted.
        {
            let ring = scanner.ring.as_mut().expect("initialized ring");
            let batch = scanner.pending.as_mut().expect("owned pending batch");
            let poll = opcode::PollAdd::new(
                types::Fd(reader.as_raw_fd()),
                linux_raw_sys::general::POLLIN,
            )
            .build()
            .flags(squeue::Flags::IO_LINK)
            .user_data(0);
            let mut submissions = ring.submission();
            // SAFETY: The socket stays open until the batch is drained. The dummy first request
            // provides completion bookkeeping and has no kernel-accessed metadata.
            unsafe { submissions.push(&poll) }.map_err(io::Error::other)?;
            batch.queued += 1;
            for (index, request) in batch.requests.iter().enumerate().skip(1) {
                let entry = request.statx_entry(&batch.directory, index as u64);
                // SAFETY: The scanner owns the same stable request storage as its ordinary
                // queue path and retains it until all submitted requests have completed.
                unsafe { submissions.push(&entry) }.map_err(io::Error::other)?;
                batch.queued += 1;
            }
        }

        let (observer, observed) = mpsc::sync_channel(0);
        scanner.drain_observer = Some(observer);
        let submitted = Cell::new(0);
        std::thread::scope(|scope| {
            let counter = Arc::clone(&counter);
            let release = scope.spawn(move || -> io::Result<()> {
                let observation = observed.recv_timeout(Duration::from_secs(10));
                let requests_alive = counter.load(Ordering::SeqCst) == 0;
                // Always release the poll, including on observation failure, so an assertion
                // cannot leave the scanner's cleanup waiting forever.
                let released = writer.write_all(b"x");
                observation.map_err(io::Error::other)?;
                assert!(
                    requests_alive,
                    "pending metadata was reclaimed before completion"
                );
                released
            });
            let completed = scanner.complete_queued_batch(&mut |ring, _| {
                let count = loop {
                    // SAFETY: Submit only the poll/statx pair, with no wait or signal mask. The
                    // scanner owns the metadata, directory, and unsubmitted entries throughout.
                    match unsafe { ring.submitter().enter::<()>(2, 0, 0, None) } {
                        Ok(count) => break count,
                        Err(error) if retryable_control_error(&error) => {}
                        Err(error) => return Err(error),
                    }
                };
                submitted.set(count);
                Err(Errno::PERM.into())
            });
            release.join().expect("release thread panicked")?;
            assert_eq!(
                completed
                    .expect_err("fatal control error must retire the ring")
                    .raw_os_error(),
                Some(Errno::PERM.raw_os_error())
            );
            Ok::<(), io::Error>(())
        })?;
        assert_eq!(submitted.get(), 2);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(scanner.pending.is_none());
        assert!(scanner.ring.is_none());
        Ok(())
    }
}
