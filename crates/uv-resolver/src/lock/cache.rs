//! Optional, process-local storage for parsed lockfiles.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use uv_cache_key::hash_digest;

use super::Lock;

const MAX_ENTRIES: usize = 32;
const MAX_SOURCE_BYTES: usize = 128 * 1024 * 1024;

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
static OBSERVER: OnceLock<fn(&str, bool)> = OnceLock::new();

#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
    source_bytes: usize,
}

struct Entry {
    digest: String,
    // Local requirement URLs are still relative to the deserializing process's directory.
    directory: PathBuf,
    contents: Box<str>,
    lock: Lock,
    skip_wheel_filename_check: bool,
}

impl Cache {
    fn get(
        &self,
        contents: &str,
        directory: &Path,
        skip_wheel_filename_check: bool,
    ) -> Option<Lock> {
        let digest = hash_digest(&contents);
        self.entries
            .iter()
            .find(|entry| {
                (skip_wheel_filename_check || !entry.skip_wheel_filename_check)
                    && entry.directory == directory
                    && entry.digest == digest
                    && entry.contents.as_ref() == contents
            })
            .map(|entry| entry.lock.clone())
    }

    fn insert(
        &mut self,
        contents: &str,
        directory: &Path,
        lock: &Lock,
        skip_wheel_filename_check: bool,
    ) -> bool {
        if contents.len() > MAX_SOURCE_BYTES {
            return false;
        }
        let digest = hash_digest(&contents);
        if self.entries.iter().any(|entry| {
            entry.skip_wheel_filename_check == skip_wheel_filename_check
                && entry.directory == directory
                && entry.digest == digest
                && entry.contents.as_ref() == contents
        }) {
            return false;
        }
        while self.entries.len() >= MAX_ENTRIES
            || self.source_bytes + contents.len() > MAX_SOURCE_BYTES
        {
            if let Some(entry) = self.entries.pop_front() {
                self.source_bytes -= entry.contents.len();
            }
        }
        self.source_bytes += contents.len();
        self.entries.push_back(Entry {
            digest,
            directory: directory.to_path_buf(),
            contents: contents.into(),
            lock: lock.clone(),
            skip_wheel_filename_check,
        });
        true
    }
}

pub(super) fn enable() {
    CACHE.get_or_init(Mutex::default);
}

pub(super) fn observe(observer: fn(&str, bool)) {
    let _ = OBSERVER.set(observer);
}

pub(super) fn get(contents: &str) -> Option<Lock> {
    let cache = CACHE.get()?.lock().ok()?;
    let directory = std::env::current_dir().ok()?;
    let skip_wheel_filename_check =
        uv_flags::contains_or_default(uv_flags::EnvironmentFlags::SKIP_WHEEL_FILENAME_CHECK);
    let lock = cache.get(contents, &directory, skip_wheel_filename_check);
    drop(cache);
    if lock.is_some()
        && let Some(observer) = OBSERVER.get()
    {
        observer(contents, true);
    }
    lock
}

pub(super) fn insert(contents: &str, lock: &Lock) {
    let Some(cache) = CACHE.get() else {
        return;
    };
    let Ok(directory) = std::env::current_dir() else {
        return;
    };
    let skip_wheel_filename_check =
        uv_flags::contains_or_default(uv_flags::EnvironmentFlags::SKIP_WHEEL_FILENAME_CHECK);
    let inserted = cache
        .lock()
        .is_ok_and(|mut cache| cache.insert(contents, &directory, lock, skip_wheel_filename_check));
    if inserted && let Some(observer) = OBSERVER.get() {
        observer(contents, false);
    }
}

pub(super) fn stats() -> (usize, usize) {
    CACHE
        .get()
        .and_then(|cache| cache.lock().ok())
        .map_or((0, 0), |cache| (cache.entries.len(), cache.source_bytes))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Cache, MAX_ENTRIES};
    use crate::Lock;

    const SOURCE: &str = "version = 1\nrevision = 3\nrequires-python = \">=3.12\"\n";

    #[test]
    fn cache_identity_includes_contents_directory_and_parser_policy() {
        let lock = Lock::from_toml(SOURCE).expect("valid lock");
        let mut cache = Cache::default();
        let directory = Path::new("first");
        assert!(cache.insert(SOURCE, directory, &lock, true));
        assert_eq!(cache.get(SOURCE, directory, false), None);
        assert_eq!(cache.get(SOURCE, directory, true), Some(lock.clone()));
        assert_eq!(cache.get(SOURCE, Path::new("second"), true), None);
        assert_eq!(
            cache.get(&SOURCE.replace("3.12", "3.13"), directory, true),
            None
        );

        assert!(cache.insert(SOURCE, directory, &lock, false));
        assert_eq!(cache.get(SOURCE, directory, false), Some(lock.clone()));
        assert_eq!(cache.get(SOURCE, directory, true), Some(lock));
    }

    #[test]
    fn cache_evicts_oldest_entries() {
        let lock = Lock::from_toml(SOURCE).expect("valid lock");
        let mut cache = Cache::default();
        let directory = Path::new("first");
        for index in 0..=MAX_ENTRIES {
            assert!(cache.insert(&format!("{SOURCE}# {index}\n"), directory, &lock, false));
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert_eq!(cache.get(&format!("{SOURCE}# 0\n"), directory, false), None);
        assert_eq!(
            cache.get(&format!("{SOURCE}# {MAX_ENTRIES}\n"), directory, false),
            Some(lock)
        );
        assert_eq!(
            cache.source_bytes,
            cache
                .entries
                .iter()
                .map(|entry| entry.contents.len())
                .sum::<usize>()
        );
    }
}
