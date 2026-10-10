//! Ordered raw and prepared filters for command snapshots.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, LazyLock, OnceLock};

use regex::Regex;

use crate::INSTA_FILTERS;

const INVALID_FILTER: &str = "Do you need to regex::escape your filter?";

static STANDARD_FILTERS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    INSTA_FILTERS
        .iter()
        .map(|(matcher, replacement)| (Regex::new(matcher).expect(INVALID_FILTER), *replacement))
        .collect()
});

thread_local! {
    /// Reuse compiled patterns across snapshots and contexts on the same test thread.
    static SNAPSHOT_FILTER_CACHE: RefCell<HashMap<String, Regex>> = RefCell::new(HashMap::new());
}

/// Ordered filters accepted by the command snapshot helpers.
pub trait SnapshotFilterSource<T: AsRef<str>> {
    fn apply(self, snapshot: String) -> String;
}

impl<T: AsRef<str>, F: AsRef<[(T, T)]>> SnapshotFilterSource<T> for F {
    fn apply(self, mut snapshot: String) -> String {
        let filters = self.as_ref();
        // Release the thread-local borrow before converting the caller's filters.
        let mut compiled_filters = SNAPSHOT_FILTER_CACHE.take();
        for (matcher, replacement) in filters {
            let matcher = matcher.as_ref();
            if !compiled_filters.contains_key(matcher) {
                compiled_filters.insert(
                    matcher.to_owned(),
                    Regex::new(matcher).expect(INVALID_FILTER),
                );
            }
            let regex = &compiled_filters[matcher];
            if regex.is_match(&snapshot) {
                snapshot = regex
                    .replace_all(&snapshot, replacement.as_ref())
                    .to_string();
            }
        }
        SNAPSHOT_FILTER_CACHE.set(compiled_filters);
        snapshot
    }
}

/// An immutable view of compiled context filters followed by the standard snapshot filters.
#[derive(Debug, Clone)]
pub struct PreparedFilters {
    context: Arc<[(Regex, String)]>,
}

impl PreparedFilters {
    fn new(filters: &[(String, String)]) -> Self {
        let context = filters
            .iter()
            .map(|(matcher, replacement)| {
                (
                    Regex::new(matcher).expect(INVALID_FILTER),
                    replacement.clone(),
                )
            })
            .collect();
        LazyLock::force(&STANDARD_FILTERS);
        Self { context }
    }

    fn apply_prepared(&self, mut snapshot: String) -> String {
        for (regex, replacement) in self
            .context
            .iter()
            .map(|(regex, replacement)| (regex, replacement.as_str()))
            .chain(
                STANDARD_FILTERS
                    .iter()
                    .map(|(regex, replacement)| (regex, *replacement)),
            )
        {
            if regex.is_match(&snapshot) {
                snapshot = regex.replace_all(&snapshot, replacement).to_string();
            }
        }
        snapshot
    }
}

impl SnapshotFilterSource<String> for PreparedFilters {
    fn apply(self, snapshot: String) -> String {
        self.apply_prepared(snapshot)
    }
}

impl SnapshotFilterSource<String> for &PreparedFilters {
    fn apply(self, snapshot: String) -> String {
        self.apply_prepared(snapshot)
    }
}

/// Context patterns retain their raw representation for Insta and invalidate preparation on edits.
pub(super) struct ContextFilters {
    patterns: Vec<(String, String)>,
    prepared: OnceLock<PreparedFilters>,
}

impl ContextFilters {
    pub(super) fn prepared(&self) -> PreparedFilters {
        self.prepared
            .get_or_init(|| PreparedFilters::new(&self.patterns))
            .clone()
    }
}

impl From<Vec<(String, String)>> for ContextFilters {
    fn from(patterns: Vec<(String, String)>) -> Self {
        Self {
            patterns,
            prepared: OnceLock::new(),
        }
    }
}

impl Deref for ContextFilters {
    type Target = Vec<(String, String)>;

    fn deref(&self) -> &Self::Target {
        &self.patterns
    }
}

impl DerefMut for ContextFilters {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.prepared = OnceLock::new();
        &mut self.patterns
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::ContextFilters;
    use crate::{INSTA_FILTERS, apply_filters};

    #[test]
    fn prepared_filters_retain_order_and_capture_replacements() {
        let filters = ContextFilters::from(vec![
            (r"/fixture/root".to_string(), "[ROOT]".to_string()),
            (r"\[ROOT\]/(\w+)".to_string(), "[PACKAGE:$1]".to_string()),
            (
                r"C:\\Windows\\fixture".to_string(),
                "[WINDOWS_ROOT]".to_string(),
            ),
        ]);
        let input = indoc! {r#"
            ----- stdout -----
            {"path":"/fixture/root/example","version":"1.2.3"}
            source = { path = "/fixture/root/example" }
            ----- stderr -----
            error: missing /fixture/root/example/metadata.json (os error 2)
            Resolved 21 packages in 12.5ms
            C:\Windows\fixture\file.py
        "#};
        let raw: Vec<_> = filters
            .iter()
            .map(|(matcher, replacement)| (matcher.as_str(), replacement.as_str()))
            .chain(INSTA_FILTERS.iter().copied())
            .collect();
        let prepared = apply_filters(input.to_string(), filters.prepared());
        assert_eq!(prepared, apply_filters(input.to_string(), raw));
        insta::assert_snapshot!(prepared, @r#"
        ----- stdout -----
        {"path":"[PACKAGE:example]","version":"1.2.3"}
        source = { path = "[PACKAGE:example]" }
        ----- stderr -----
        error: missing [PACKAGE:example]/metadata.json (os error 2)
        Resolved 21 packages in [TIME]
        [WINDOWS_ROOT]/file.py
        "#);
    }

    #[test]
    fn changing_patterns_invalidates_preparation() {
        let mut filters = ContextFilters::from(vec![("first".to_string(), "second".to_string())]);
        let original = filters.prepared();
        assert_eq!(apply_filters("first".to_string(), &original), "second");

        filters.push(("second".to_string(), "third".to_string()));
        assert_eq!(
            apply_filters("first".to_string(), filters.prepared()),
            "third"
        );
        // A previously acquired view keeps the filter order it was prepared with.
        assert_eq!(apply_filters("first".to_string(), original), "second");
    }

    #[test]
    #[should_panic(expected = "Do you need to regex::escape your filter?")]
    fn invalid_patterns_keep_the_filter_diagnostic() {
        let filters = ContextFilters::from(vec![("(".to_string(), String::new())]);
        filters.prepared();
    }
}
