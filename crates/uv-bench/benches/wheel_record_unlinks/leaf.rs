//! The benchmark's all-independent-leaves contract is separate from serial wheel uninstall.

use std::io;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rayon::{ThreadPool, ThreadPoolBuilder};

use super::fixture::OwnedLeafTrial;

pub(super) type Outcomes = Vec<io::Result<()>>;
pub(super) type RawOutcome = Result<(), (io::ErrorKind, Option<i32>)>;

fn require_active(trial: &OwnedLeafTrial) {
    assert!(
        !trial.lease().is_quarantined(),
        "a quarantined mutation fixture cannot be reused"
    );
}

/// Attempt every validated leaf, including leaves after an ordinary filesystem error.
pub(super) fn ordinary(trial: &OwnedLeafTrial) -> Outcomes {
    require_active(trial);
    trial.paths().iter().map(fs_err::remove_file).collect()
}

/// Keep the original operating-system errors for the untimed differential oracle.
#[expect(
    clippy::disallowed_methods,
    reason = "The raw-I/O oracle must retain the original operating-system error"
)]
pub(super) fn raw(trial: &OwnedLeafTrial) -> Outcomes {
    require_active(trial);
    trial.paths().iter().map(std::fs::remove_file).collect()
}

/// The indexed parallel iterator retains the input order while attempting every leaf.
pub(super) fn with_workers(trial: &OwnedLeafTrial, pool: &ThreadPool) -> Outcomes {
    require_active(trial);
    pool.install(|| trial.paths().par_iter().map(fs_err::remove_file).collect())
}

pub(super) fn worker_pool(threads: usize) -> ThreadPool {
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .stack_size(uv_configuration::min_stack_size())
        .build()
        .expect("Failed to create wheel-unlink worker pool")
}

pub(super) fn comparable(result: &io::Result<()>) -> Result<(), io::ErrorKind> {
    comparable_raw(result).map_err(|(kind, _)| kind)
}

pub(super) fn comparable_raw(result: &io::Result<()>) -> RawOutcome {
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err((error.kind(), error.raw_os_error())),
    }
}

pub(super) fn successful(results: &[io::Result<()>]) -> usize {
    results.iter().filter(|result| result.is_ok()).count()
}
