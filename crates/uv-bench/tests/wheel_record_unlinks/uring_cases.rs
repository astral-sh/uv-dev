#[cfg(test)]
mod tests {
    use std::env;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::process::Command;
    use std::sync::{Weak, mpsc};
    use std::time::Duration;

    use anyhow::Result;

    use super::*;
    use crate::leaf;

    fn queue_depth(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("non-zero test queue depth")
    }

    fn files(count: usize) -> Result<OwnedLeafTrial> {
        let root = tempfile::Builder::new()
            .prefix("uv-unlink-driver-test-")
            .tempdir()?;
        let paths = (0..count)
            .map(|index| PathBuf::from(format!("file-{index}")))
            .collect::<Vec<_>>();
        for path in &paths {
            fs_err::write(root.path().join(path), b"file\n")?;
        }
        OwnedLeafTrial::new(root, paths)
    }

    fn error_trial() -> Result<OwnedLeafTrial> {
        let root = tempfile::Builder::new()
            .prefix("uv-unlink-driver-errors-")
            .tempdir()?;
        fs_err::write(root.path().join("regular"), b"regular\n")?;
        fs_err::create_dir_all(root.path().join("targets/directory"))?;
        fs_err::write(root.path().join("targets/file"), b"target\n")?;
        fs_err::write(root.path().join("targets/directory/child"), b"child\n")?;
        fs_err::create_dir(root.path().join("real-directory"))?;
        fs_err::write(root.path().join("real-directory/child"), b"unrelated\n")?;
        fs_err::write(root.path().join("later"), b"later\n")?;
        for (target, name) in [
            ("targets/missing", "dangling-link"),
            ("targets/file", "file-link"),
            ("targets/directory", "directory-link"),
        ] {
            fs_err::os::unix::fs::symlink(target, root.path().join(name))?;
        }
        OwnedLeafTrial::new(
            root,
            [
                "regular",
                "missing",
                "dangling-link",
                "file-link",
                "directory-link",
                "real-directory",
                "later",
            ]
            .map(PathBuf::from)
            .into(),
        )
    }

    fn pending_batch(
        trial: &OwnedLeafTrial,
        counter: Arc<AtomicUsize>,
    ) -> io::Result<(PendingBatch, Weak<OwnedFd>, Weak<FixtureLease>)> {
        let lease = trial.lease();
        let directory = Arc::new(open(
            lease.path(),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?);
        let weak_directory = Arc::downgrade(&directory);
        let weak_lease = Arc::downgrade(&lease);
        let mut batch = PendingBatch::new(directory, lease, trial.relative_paths())?;
        batch.drop_counter = Some(counter);
        Ok((batch, weak_directory, weak_lease))
    }

    #[expect(unsafe_code)]
    fn submit_prefix(ring: &IoUring, count: u32) -> io::Result<usize> {
        for _ in 0..MAX_STALLED_ATTEMPTS {
            // SAFETY: Each caller has queued a valid, owned test batch. This submits only the
            // requested prefix with no wait, signal mask, or extended arguments. The caller's
            // normal completion/retirement path still owns every referenced resource.
            match unsafe { ring.submitter().enter::<()>(count, 0, 0, None) } {
                Ok(submitted) => return Ok(submitted),
                Err(error) if retryable_control_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "test submission control made no progress",
        ))
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
    fn relative_names_are_checked_before_any_submission() -> io::Result<()> {
        validate_relative_paths(&[PathBuf::from("package/file"), PathBuf::from("leaf")])?;
        for path in [
            "",
            ".",
            "../outside",
            "/absolute",
            "parent/../leaf",
            "bad\0name",
        ] {
            assert_eq!(
                validate_relative_paths(&[PathBuf::from(path)])
                    .expect_err("unsafe relative name must be rejected")
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        Ok(())
    }

    #[test]
    fn completion_indices_duplicates_and_result_values_are_checked() -> io::Result<()> {
        assert_eq!(completion_index(1, 0, 2)?, 1);
        for (user_data, flags, count) in [(2, 0, 2), (0, 1, 2), (0, 0, 0)] {
            assert_eq!(
                completion_index(user_data, flags, count)
                    .expect_err("invalid completion must be rejected")
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
        let request = Request::new(Path::new("leaf"))?;
        assert_eq!(
            request
                .outcome()
                .expect_err("an uncompleted request has no result")
                .kind(),
            io::ErrorKind::InvalidData
        );
        record_result(&request.result, 0)?;
        assert!(request.outcome()?.is_ok());
        assert_eq!(
            record_result(&request.result, 0)
                .expect_err("a request can complete only once")
                .kind(),
            io::ErrorKind::InvalidData
        );
        request.result.set(Some(-Errno::NOENT.raw_os_error()));
        assert_eq!(
            request
                .outcome()?
                .expect_err("negative CQE result must retain errno")
                .raw_os_error(),
            Some(Errno::NOENT.raw_os_error())
        );
        for result in [1, i32::MIN] {
            request.result.set(Some(result));
            assert_eq!(
                request
                    .outcome()
                    .expect_err("invalid unlink result must be rejected")
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_unlinker_matches_raw_results_and_reuses_the_ring() -> Result<()> {
        let raw_trial = error_trial()?;
        let expected = leaf::raw(&raw_trial);
        raw_trial.assert_leaf_results(&expected)?;
        assert_eq!(successful(&expected), 5);
        assert_eq!(leaf::comparable(&expected[1]), Err(io::ErrorKind::NotFound));
        assert_eq!(
            leaf::comparable(&expected[5]),
            Err(io::ErrorKind::IsADirectory)
        );
        assert_eq!(leaf::comparable_raw(&expected[6]), Ok(()));

        let mut unlinker = Unlinker::new(queue_depth(2))?;
        let descriptor = unlinker.ring.as_ref().map(AsRawFd::as_raw_fd);
        for _ in 0..3 {
            let trial = error_trial()?;
            let before = unlinker.activity();
            let actual = unlinker.unlink(&trial)?;
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(&expected) {
                assert_eq!(leaf::comparable_raw(actual), leaf::comparable_raw(expected));
            }
            unlinker.assert_activity(before, expected.len(), 5);
            trial.assert_leaf_results(&actual)?;
        }
        let empty = files(0)?;
        let before = unlinker.activity();
        let actual = unlinker.unlink(&empty)?;
        assert!(actual.is_empty());
        unlinker.assert_activity(before, 0, 0);
        empty.assert_leaf_results(&actual)?;
        assert_eq!(unlinker.ring.as_ref().map(AsRawFd::as_raw_fd), descriptor);
        assert!(unlinker.pending.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_unlinker_queries_limits_only_after_a_successful_operation() -> Result<()> {
        std::thread::spawn(|| -> Result<()> {
            let mut unlinker = Unlinker::new(queue_depth(16))?;
            assert_eq!(
                unlinker
                    .worker_limits()
                    .expect_err("a probe bit alone is not an active backend")
                    .kind(),
                io::ErrorKind::InvalidData
            );
            let trial = files(1)?;
            let before = unlinker.activity();
            let actual = unlinker.unlink(&trial)?;
            unlinker.assert_activity(before, 1, 1);
            trial.assert_leaf_results(&actual)?;
            let limits = unlinker.worker_limits()?;
            assert_eq!(unlinker.worker_limits()?, limits);
            Ok(())
        })
        .join()
        .expect("worker-limit test thread panicked")
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_unlinker_retries_unsubmitted_control_errors() -> Result<()> {
        let trial = files(2)?;
        let mut unlinker = Unlinker::new(queue_depth(2))?;
        let mut failures = [Errno::INTR, Errno::AGAIN, Errno::BUSY].into_iter();
        let actual = unlinker.unlink_with(&trial, &mut |ring, want| {
            if let Some(error) = failures.next() {
                return Err(error.into());
            }
            ring.submit_and_wait(want)
        })?;
        assert_eq!(successful(&actual), 2);
        trial.assert_leaf_results(&actual)?;
        assert!(unlinker.pending.is_none());
        assert!(unlinker.ring.is_some());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_unlinker_retires_after_stalled_control_errors() -> Result<()> {
        let trial = files(2)?;
        let mut unlinker = Unlinker::new(queue_depth(2))?;
        let attempts = Cell::new(0);
        let result = unlinker.unlink_with(&trial, &mut |_, _| {
            attempts.set(attempts.get() + 1);
            Err(Errno::AGAIN.into())
        });
        assert_eq!(
            result.expect_err("stalled control must fail").kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(attempts.get(), MAX_STALLED_ATTEMPTS);
        assert!(unlinker.pending.is_none());
        assert!(unlinker.ring.is_none());
        assert!(!trial.lease().is_quarantined());
        assert_eq!(
            unlinker
                .unlink(&trial)
                .expect_err("a retired driver cannot submit another batch")
                .kind(),
            io::ErrorKind::Unsupported
        );
        let remaining = leaf::raw(&trial);
        assert_eq!(successful(&remaining), 2);
        trial.assert_leaf_results(&remaining)?;
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    fn active_unlinker_retires_partial_submission_without_submitting_the_tail() -> Result<()> {
        let trial = files(4)?;
        let mut unlinker = Unlinker::new(queue_depth(4))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let (batch, directory, _) = pending_batch(&trial, Arc::clone(&counter))?;
        unlinker.pending = Some(batch);
        unlinker.queue()?;
        let submitted = Cell::new(0);
        let result = unlinker.complete_queued_batch(&mut |ring, _| {
            assert_eq!(counter.load(Ordering::SeqCst), 0);
            submitted.set(submit_prefix(ring, 1)?);
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
        assert!(directory.upgrade().is_none());
        assert!(unlinker.pending.is_none());
        assert!(unlinker.ring.is_none());
        assert!(!trial.lease().is_quarantined());

        // Only the submitted prefix may have mutated the tree. Once it is retired, the raw
        // oracle can remove the untouched tail and verify the complete resulting tree.
        let remaining = leaf::raw(&trial);
        assert_eq!(
            leaf::comparable(&remaining[0]),
            Err(io::ErrorKind::NotFound)
        );
        assert!(remaining.iter().skip(1).all(Result::is_ok));
        let all_removed = (0..4).map(|_| Ok(())).collect::<Vec<_>>();
        trial.assert_leaf_results(&all_removed)?;
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host"]
    #[expect(unsafe_code)]
    fn active_unlinker_keeps_names_directory_and_lease_alive_during_retirement() -> Result<()> {
        let trial = files(4)?;
        let root = trial.lease().path().to_path_buf();
        let mut unlinker = Unlinker::new(queue_depth(4))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let (batch, directory, lease) = pending_batch(&trial, Arc::clone(&counter))?;
        unlinker.pending = Some(batch);
        let (reader, mut writer) = UnixStream::pair()?;

        // A poll blocks the linked unlink. Two more valid unlink SQEs remain unsubmitted.
        {
            let ring = unlinker.ring.as_mut().expect("initialized ring");
            let batch = unlinker.pending.as_mut().expect("owned pending batch");
            let poll = opcode::PollAdd::new(
                types::Fd(reader.as_raw_fd()),
                linux_raw_sys::general::POLLIN,
            )
            .build()
            .flags(squeue::Flags::IO_LINK)
            .user_data(0);
            let mut submissions = ring.submission();
            // SAFETY: The socket stays open through retirement. Request zero supplies only CQE
            // bookkeeping; the other requests retain their normal owned unlink storage.
            unsafe { submissions.push(&poll) }.map_err(io::Error::other)?;
            batch.queued += 1;
            for (index, request) in batch.requests.iter().enumerate().skip(1) {
                let entry = request.unlink_entry(&batch.directory, index as u64);
                // SAFETY: The pending batch retains the directory, names, and lease until every
                // submitted request has completed, exactly as in the ordinary queue path.
                unsafe { submissions.push(&entry) }.map_err(io::Error::other)?;
                batch.queued += 1;
            }
        }
        drop(trial);
        let (observer, observed) = mpsc::sync_channel(0);
        unlinker.drain_observer = Some(observer);
        let submitted = Cell::new(0);
        std::thread::scope(|scope| {
            let counter = Arc::clone(&counter);
            let directory = directory.clone();
            let lease = lease.clone();
            let root = root.clone();
            let release = scope.spawn(move || -> Result<()> {
                let observation = observed.recv_timeout(Duration::from_secs(10));
                let root_exists = root.try_exists();
                let retained = counter.load(Ordering::SeqCst) == 0
                    && directory.upgrade().is_some()
                    && lease.upgrade().is_some();
                // Always release the poll so assertion failures cannot strand the cleanup wait.
                let released = writer.write_all(b"x");
                observation?;
                assert!(
                    retained && root_exists?,
                    "in-flight unlink ownership was reclaimed early"
                );
                released?;
                Ok(())
            });
            let completed = unlinker.complete_queued_batch(&mut |ring, _| {
                submitted.set(submit_prefix(ring, 2)?);
                Err(Errno::PERM.into())
            });
            release.join().expect("release thread panicked")?;
            assert_eq!(
                completed
                    .expect_err("fatal control error must retire the ring")
                    .raw_os_error(),
                Some(Errno::PERM.raw_os_error())
            );
            Ok::<(), anyhow::Error>(())
        })?;
        assert_eq!(submitted.get(), 2);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(directory.upgrade().is_none());
        assert!(lease.upgrade().is_none());
        assert!(!root.try_exists()?);
        assert!(unlinker.pending.is_none());
        assert!(unlinker.ring.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host; retains one quarantined root until exit"]
    fn active_unlinker_quarantines_all_ownership_when_retirement_is_uncertain() -> Result<()> {
        let trial = files(1)?;
        let root = trial.lease().path().to_path_buf();
        let mut unlinker = Unlinker::new(queue_depth(1))?;
        let counter = Arc::new(AtomicUsize::new(0));
        let (batch, directory, lease) = pending_batch(&trial, Arc::clone(&counter))?;
        unlinker.pending = Some(batch);
        unlinker.queue()?;
        let ring = unlinker.ring.as_ref().expect("initialized ring");
        // Leave the CQE unconsumed to model losing completion control after a real submission.
        let submitted = submit_prefix(ring, 1)?;
        assert_eq!(submitted, 1);
        unlinker.retire_after_drain(&Err(Errno::PERM.into()));
        assert!(unlinker.pending.is_none());
        assert!(unlinker.ring.is_none());
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        drop(trial);
        drop(unlinker);

        let retained_lease = lease.upgrade().expect("quarantine must retain its lease");
        assert!(retained_lease.is_quarantined());
        assert_eq!(retained_lease.path(), root);
        assert!(root.try_exists()?);
        let retained_directory = directory
            .upgrade()
            .expect("quarantine must retain its root descriptor");
        let metadata = rustix::fs::fstat(retained_directory.as_ref())?;
        assert_eq!(
            rustix::fs::FileType::from_raw_mode(metadata.st_mode),
            rustix::fs::FileType::Directory
        );
        assert_eq!(
            fs_err::create_dir(&root)
                .expect_err("normal teardown must not free a quarantined pathname for reuse")
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        let replacement = files(1)?;
        assert_ne!(replacement.lease().path(), root);
        let removed = leaf::ordinary(&replacement);
        replacement.assert_leaf_results(&removed)?;
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        Ok(())
    }

    fn descriptor_count() -> io::Result<usize> {
        Ok(fs_err::read_dir("/proc/self/fd")?
            .collect::<io::Result<Vec<_>>>()?
            .len())
    }

    fn descriptor_count_child() -> Result<()> {
        // Warm up ordinary fixture helpers before taking the process-wide descriptor baseline.
        let warmup = files(1)?;
        warmup.assert_leaf_results(&leaf::ordinary(&warmup))?;
        drop(warmup);
        let before = descriptor_count()?;
        let mut unlinker = Unlinker::new(queue_depth(4))?;
        for _ in 0..4 {
            let trial = error_trial()?;
            let results = unlinker.unlink(&trial)?;
            trial.assert_leaf_results(&results)?;
        }
        let active = descriptor_count()?;
        for _ in 0..64 {
            let trial = error_trial()?;
            let results = unlinker.unlink(&trial)?;
            assert_eq!(successful(&results), 5);
            trial.assert_leaf_results(&results)?;
        }
        assert_eq!(descriptor_count()?, active);
        drop(unlinker);
        assert_eq!(descriptor_count()?, before);
        Ok(())
    }

    #[test]
    #[ignore = "requires an io_uring-capable Linux host and /proc/self/fd"]
    fn active_unlinker_keeps_descriptor_count_stable() -> Result<()> {
        const CHILD: &str = "UV_BENCH_UNLINK_FD_TEST_CHILD";
        const NAME: &str = "uring::tests::active_unlinker_keeps_descriptor_count_stable";
        if matches!(env::var(CHILD).as_deref(), Ok("1")) {
            return descriptor_count_child();
        }
        // A distinct process excludes other active tests, including intentional quarantine leaks.
        let output = Command::new(env::current_exe()?)
            .args([
                "--exact",
                NAME,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()?;
        assert!(
            output.status.success(),
            "descriptor-count child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        Ok(())
    }
}
