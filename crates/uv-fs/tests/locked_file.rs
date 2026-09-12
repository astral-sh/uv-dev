use std::error::Error;
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::process::Command;
use std::time::Duration;

#[cfg(unix)]
use uv_fs::LockedFileError;
use uv_fs::{LockedFile, LockedFileMode};
#[cfg(unix)]
use uv_static::EnvVars;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[test]
fn cancelled_waiters_do_not_starve_unrelated_locks() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let contended = directory.path().join("contended.lock");
    let unrelated = directory.path().join("unrelated.lock");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .max_blocking_threads(2)
        .build()?;

    let result = runtime.block_on(async {
        let owner = LockedFile::acquire(&contended, LockedFileMode::Exclusive, "contended").await?;
        let result: Result<()> = async {
            // Cancel enough contended acquisitions to occupy the blocking pool if their waiters
            // remain alive after the acquisition futures are dropped.
            for _ in 0..2 {
                assert!(
                    tokio::time::timeout(
                        Duration::from_millis(50),
                        LockedFile::acquire(&contended, LockedFileMode::Exclusive, "contended"),
                    )
                    .await
                    .is_err(),
                    "contended acquisition completed before cancellation"
                );
            }

            let _unrelated = tokio::time::timeout(
                Duration::from_secs(5),
                LockedFile::acquire(&unrelated, LockedFileMode::Exclusive, "unrelated"),
            )
            .await??;
            Ok(())
        }
        .await;
        drop(owner);
        result
    });
    runtime.shutdown_timeout(Duration::from_secs(5));
    result
}

#[cfg(unix)]
#[test]
fn timed_out_waiters_release_file_descriptors() -> Result<()> {
    const CHILD: &str = "UV_FS_TEST_LOCK_TIMEOUT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new("sh")
            .arg("-c")
            .arg("ulimit -S -n 64 && ulimit -H -n 64 && exec \"$@\"")
            .arg("sh")
            .arg(std::env::current_exe()?)
            .args([
                "--exact",
                "timed_out_waiters_release_file_descriptors",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(EnvVars::UV_LOCK_TIMEOUT, "0")
            .output()?;
        assert!(
            output.status.success(),
            "low-descriptor child failed: {output:?}"
        );
        return Ok(());
    }

    let directory = tempfile::tempdir()?;
    let path = directory.path().join("contended.lock");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .max_blocking_threads(256)
        .build()?;
    let result = runtime.block_on(async {
        let owner = LockedFile::acquire(&path, LockedFileMode::Exclusive, "contended").await?;
        let result: Result<()> = async {
            // More attempts than the child can keep open must each reach the configured timeout,
            // not fail while opening another descriptor for the same lock.
            for attempt in 0..128 {
                let outcome =
                    LockedFile::acquire(&path, LockedFileMode::Exclusive, "contended").await;
                if !matches!(outcome, Err(LockedFileError::Timeout { .. })) {
                    return Err(io::Error::other(format!(
                        "unexpected lock result for attempt {attempt}: {outcome:?}"
                    ))
                    .into());
                }
            }
            Ok(())
        }
        .await;
        drop(owner);
        result
    });
    runtime.shutdown_timeout(Duration::from_secs(5));
    result
}
