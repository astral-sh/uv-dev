//! Optional, process-local storage for context-free project-manifest syntax.

use std::borrow::Borrow;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use rustc_hash::FxHashSet;

use super::PyProjectTomlSource;

const MAX_ENTRIES: usize = 16 * 1024;
const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_ENTRY_BYTES: usize = 256 * 1024;

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
static OBSERVER: OnceLock<fn(&str, bool)> = OnceLock::new();

/// A cache key that borrows its exact text from the retained syntax tree.
#[derive(Clone)]
struct Entry(Arc<PyProjectTomlSource>);

impl Borrow<str> for Entry {
    fn borrow(&self) -> &str {
        self.0.contents()
    }
}

impl Hash for Entry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.contents().hash(state);
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.0.contents() == other.0.contents()
    }
}

impl Eq for Entry {}

struct Cache {
    entries: FxHashSet<Entry>,
    oldest: VecDeque<Entry>,
    source_bytes: usize,
    max_entries: usize,
    max_source_bytes: usize,
    max_entry_bytes: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(MAX_ENTRIES, MAX_SOURCE_BYTES, MAX_ENTRY_BYTES)
    }
}

impl Cache {
    fn new(max_entries: usize, max_source_bytes: usize, max_entry_bytes: usize) -> Self {
        Self {
            entries: FxHashSet::default(),
            oldest: VecDeque::new(),
            source_bytes: 0,
            max_entries,
            max_source_bytes,
            max_entry_bytes,
        }
    }

    fn get(&self, contents: &str) -> Option<Arc<PyProjectTomlSource>> {
        self.entries.get(contents).map(|entry| Arc::clone(&entry.0))
    }

    fn insert(&mut self, source: &Arc<PyProjectTomlSource>) -> bool {
        let contents = source.contents();
        if self.max_entries == 0
            || contents.len() > self.max_entry_bytes
            || contents.len() > self.max_source_bytes
            || self.entries.contains(contents)
        {
            return false;
        }

        while self.entries.len() >= self.max_entries
            || self.source_bytes + contents.len() > self.max_source_bytes
        {
            let Some(entry) = self.oldest.pop_front() else {
                return false;
            };
            self.source_bytes -= entry.0.contents().len();
            self.entries.remove(entry.0.contents());
        }

        self.source_bytes += contents.len();
        let entry = Entry(Arc::clone(source));
        self.oldest.push_back(entry.clone());
        self.entries.insert(entry);
        true
    }
}

pub(super) fn enable() {
    CACHE.get_or_init(Mutex::default);
}

pub(super) fn is_enabled() -> bool {
    CACHE.get().is_some()
}

pub(super) fn accepts(contents: &str) -> bool {
    contents.len() <= MAX_ENTRY_BYTES
}

pub(super) fn observe(observer: fn(&str, bool)) {
    let _ = OBSERVER.set(observer);
}

pub(super) fn get(contents: &str) -> Option<Arc<PyProjectTomlSource>> {
    let source = CACHE.get()?.lock().ok()?.get(contents);
    if source.is_some()
        && let Some(observer) = OBSERVER.get()
    {
        observer(contents, true);
    }
    source
}

pub(super) fn insert(source: &Arc<PyProjectTomlSource>) {
    let inserted = CACHE
        .get()
        .is_some_and(|cache| cache.lock().is_ok_and(|mut cache| cache.insert(source)));
    if inserted && let Some(observer) = OBSERVER.get() {
        observer(source.contents(), false);
    }
}

pub(super) fn warm(contents: String) -> Result<(), toml::de::Error> {
    if !is_enabled() || !accepts(&contents) || get(&contents).is_some() {
        return Ok(());
    }
    let source = Arc::new(PyProjectTomlSource::parse(contents)?);
    insert(&source);
    Ok(())
}

pub(super) fn stats() -> (usize, usize) {
    CACHE
        .get()
        .and_then(|cache| cache.lock().ok())
        .map_or((0, 0), |cache| (cache.entries.len(), cache.source_bytes))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::Cache;
    use crate::pyproject::PyProjectTomlSource;

    fn source(contents: &str) -> Arc<PyProjectTomlSource> {
        Arc::new(PyProjectTomlSource::parse(contents.to_owned()).expect("valid TOML"))
    }

    #[test]
    fn cache_reuses_only_exact_source_text() {
        let mut cache = Cache::new(4, 128, 64);
        let first = source("name = 'first'\n");
        assert!(cache.insert(&first));
        assert!(!cache.insert(&source(first.contents())));
        assert!(Arc::ptr_eq(
            &cache.get(first.contents()).expect("cached source"),
            &first,
        ));
        assert!(cache.get("name = 'other'\n").is_none());
        assert!(cache.get("name = 'first'\n# changed\n").is_none());
        assert_eq!(cache.source_bytes, first.contents().len());
    }

    #[test]
    fn cache_evicts_oldest_source_and_keeps_borrowed_snapshots_alive() {
        let mut cache = Cache::new(2, 128, 64);
        let first = source("name = 'first'\n");
        let second = source("name = 'second'\n");
        let third = source("name = 'third'\n");
        assert!(cache.insert(&first));
        let borrowed = cache.get(first.contents()).expect("cached source");
        assert!(cache.insert(&second));
        assert!(cache.insert(&third));
        assert!(cache.get(first.contents()).is_none());
        assert!(cache.get(second.contents()).is_some());
        assert!(cache.get(third.contents()).is_some());
        assert_eq!(borrowed.contents(), first.contents());
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(
            cache.source_bytes,
            second.contents().len() + third.contents().len()
        );
    }

    #[test]
    fn cache_enforces_entry_and_source_budgets() {
        let mut cache = Cache::new(4, 20, 16);
        let first = source("name = 'a'\n");
        let second = source("name = 'b'\n");
        assert!(cache.insert(&first));
        assert!(!cache.insert(&source("name = 'too long'\n")));
        assert!(cache.get(first.contents()).is_some());
        assert!(cache.insert(&second));
        assert!(cache.get(first.contents()).is_none());
        assert!(cache.get(second.contents()).is_some());
        assert_eq!(cache.source_bytes, second.contents().len());
    }
}
