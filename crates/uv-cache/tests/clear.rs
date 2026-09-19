use std::io;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use uv_cache::{Cache, CleanReporter};

struct NoopReporter;

impl CleanReporter for NoopReporter {
    fn on_clean(&self) {}

    fn on_complete(&self) {}
}

fn cache_directory() -> io::Result<(TempDir, PathBuf)> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path().join("cache");
    fs_err::create_dir(&root)?;
    Ok((temporary, root))
}

fn acquire_cache(root: &Path) -> io::Result<Cache> {
    Cache::from_path(root)
        .with_exclusive_lock_no_wait()
        .map_err(|_| io::Error::other("could not acquire the test cache lock"))
}

#[test]
fn clear_empty_cache_without_lock() -> io::Result<()> {
    let (_temporary, root) = cache_directory()?;

    let summary = Cache::from_path(&root).clear(Box::new(NoopReporter))?;

    assert_eq!(summary.num_files, 0);
    assert_eq!(summary.num_dirs, 1);
    assert!(!root.exists());
    Ok(())
}

#[test]
fn clear_populated_cache_without_lock() -> io::Result<()> {
    let (_temporary, root) = cache_directory()?;
    fs_err::write(root.join("payload"), "payload")?;

    let summary = Cache::from_path(&root).clear(Box::new(NoopReporter))?;

    assert_eq!(summary.num_files, 1);
    assert_eq!(summary.num_dirs, 1);
    assert!(!root.exists());
    Ok(())
}

#[test]
fn clear_cache_with_owned_lock() -> io::Result<()> {
    let (_temporary, root) = cache_directory()?;
    fs_err::write(root.join("payload"), "payload")?;
    let cache = acquire_cache(&root)?;
    assert!(root.join(".lock").is_file());

    let summary = cache.clear(Box::new(NoopReporter))?;

    assert_eq!(summary.num_files, 2);
    assert_eq!(summary.num_dirs, 1);
    assert!(!root.exists());
    Ok(())
}

#[test]
fn clear_cache_with_contended_lock() -> io::Result<()> {
    let (_temporary, root) = cache_directory()?;
    fs_err::write(root.join("payload"), "payload")?;
    let holder = acquire_cache(&root)?;

    // This is the same no-wait fallback used by `uv cache clean --force`.
    let Err(cache) = Cache::from_path(&root).with_exclusive_lock_no_wait() else {
        return Err(io::Error::other("the test cache lock was not contended"));
    };
    let summary = cache.clear(Box::new(NoopReporter))?;

    assert_eq!(summary.num_files, 1);
    assert_eq!(summary.num_dirs, 0);
    assert!(!root.join("payload").exists());
    assert!(root.join(".lock").is_file());
    assert!(
        Cache::from_path(&root)
            .with_exclusive_lock_no_wait()
            .is_err()
    );

    drop(holder);
    let _released = acquire_cache(&root)?;
    Ok(())
}
