use std::{
    borrow::Cow,
    fmt::{Debug, Formatter},
    sync::Arc,
};

use rustc_hash::FxHashSet;

use uv_normalize::{DefaultExtras, ExtraName};

const EXTRA_INDEX_THRESHOLD: usize = 8;

/// Manager of all extra decisions and settings history.
///
/// This is an Arc mostly just to avoid size bloat on things that contain these.
#[derive(Debug, Default, Clone)]
pub struct ExtrasSpecification(Arc<ExtrasSpecificationInner>);

/// Manager of all dependency-group decisions and settings history.
#[derive(Default, Clone)]
pub struct ExtrasSpecificationInner {
    /// Extras to include.
    include: IncludeExtras,
    /// An optional index for multi-extra includes.
    include_index: Option<FxHashSet<ExtraName>>,
    /// Extras to exclude (always wins over include).
    exclude: Vec<ExtraName>,
    /// An optional index for multi-extra excludes.
    exclude_index: Option<FxHashSet<ExtraName>>,
    /// Whether an `--only` flag was passed.
    ///
    /// If true, users of this API should refrain from looking at packages
    /// that *aren't* specified by the extras. This is exposed
    /// via [`ExtrasSpecificationInner::prod`][].
    only_extras: bool,
    /// The "raw" flags/settings we were passed for diagnostics.
    history: ExtrasSpecificationHistory,
}

impl ExtrasSpecification {
    /// Create from history.
    ///
    /// This is the "real" constructor, it's basically taking raw CLI flags but in
    /// a way that's a bit nicer for other constructors to use.
    fn from_history(history: ExtrasSpecificationHistory) -> Self {
        let ExtrasSpecificationHistory {
            mut extra,
            mut only_extra,
            no_extra,
            all_extras,
            no_default_extras,
            mut defaults,
        } = history.clone();

        // `extra` and `only_extra` actually have the same meanings: packages to include.
        // But if `only_extra` is non-empty then *other* packages should be excluded.
        // So we just record whether it was and then treat the two lists as equivalent.
        let only_extras = !only_extra.is_empty();
        // --only flags imply --no-default-extras
        let default_extras = !no_default_extras && !only_extras;

        let include = if all_extras {
            // If this is set we can ignore extra/only_extra/defaults as irrelevant.
            IncludeExtras::All
        } else {
            // Merge all these lists, they're equivalent now
            extra.append(&mut only_extra);
            // Resolve default extras potentially also setting All
            if default_extras {
                match &mut defaults {
                    DefaultExtras::All => IncludeExtras::All,
                    DefaultExtras::List(defaults) => {
                        extra.append(defaults);
                        IncludeExtras::Some(extra)
                    }
                }
            } else {
                IncludeExtras::Some(extra)
            }
        };

        let include_index = match &include {
            IncludeExtras::Some(extras) if extras.len() > EXTRA_INDEX_THRESHOLD => {
                Some(extras.iter().cloned().collect())
            }
            IncludeExtras::Some(_) | IncludeExtras::All => None,
        };
        let exclude_index = if no_extra.len() > EXTRA_INDEX_THRESHOLD {
            Some(no_extra.iter().cloned().collect())
        } else {
            None
        };

        Self(Arc::new(ExtrasSpecificationInner {
            include,
            include_index,
            exclude: no_extra,
            exclude_index,
            only_extras,
            history,
        }))
    }

    /// Create from raw CLI args
    pub fn from_args(
        extra: Vec<ExtraName>,
        no_extra: Vec<ExtraName>,
        no_default_extras: bool,
        only_extra: Vec<ExtraName>,
        all_extras: bool,
    ) -> Self {
        Self::from_history(ExtrasSpecificationHistory {
            extra,
            only_extra,
            no_extra,
            all_extras,
            no_default_extras,
            // This is unknown at CLI-time, use `.with_defaults(...)` to apply this later!
            defaults: DefaultExtras::default(),
        })
    }

    /// Helper to make a spec from just a --extra
    pub fn from_extra(extra: Vec<ExtraName>) -> Self {
        Self::from_history(ExtrasSpecificationHistory {
            extra,
            ..Default::default()
        })
    }

    /// Helper to make a spec from just --all-extras
    pub fn from_all_extras() -> Self {
        Self::from_history(ExtrasSpecificationHistory {
            all_extras: true,
            ..Default::default()
        })
    }

    /// Apply defaults to a base [`ExtrasSpecification`].
    pub fn with_defaults(&self, defaults: DefaultExtras) -> ExtrasSpecificationWithDefaults {
        if self.0.history.defaults == defaults {
            return ExtrasSpecificationWithDefaults { cur: self.clone() };
        }

        // Explicitly clone the inner history and set the defaults, then remake the result.
        let mut history = self.0.history.clone();
        history.defaults = defaults;

        ExtrasSpecificationWithDefaults {
            cur: Self::from_history(history),
        }
    }
}

impl std::ops::Deref for ExtrasSpecification {
    type Target = ExtrasSpecificationInner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ExtrasSpecificationInner {
    /// Returns `true` if packages other than the ones referenced by these
    /// extras should be considered.
    ///
    /// That is, if I tell you to install a project and this is false,
    /// you should ignore the project itself and all its dependencies,
    /// and instead just install the extras.
    ///
    /// (This is really just asking if an --only flag was passed.)
    fn prod(&self) -> bool {
        !self.only_extras
    }

    /// Returns `true` if the specification includes the given extra.
    pub fn contains(&self, extra: &ExtraName) -> bool {
        // exclude always trumps include
        let excluded = self.exclude_index.as_ref().map_or_else(
            || self.exclude.contains(extra),
            |index| index.contains(extra),
        );
        if excluded {
            return false;
        }

        self.include_index.as_ref().map_or_else(
            || self.include.contains(extra),
            |index| index.contains(extra),
        )
    }

    /// Returns an iterator over all extras that are included in the specification,
    /// assuming `all_names` is an iterator over all extras.
    pub fn extra_names<'a, Names>(
        &'a self,
        all_names: Names,
    ) -> impl Iterator<Item = &'a ExtraName> + 'a
    where
        Names: Iterator<Item = &'a ExtraName> + 'a,
    {
        all_names.filter(move |name| self.contains(name))
    }

    /// Iterate over all groups the user explicitly asked for on the CLI
    pub fn explicit_names(&self) -> impl Iterator<Item = &ExtraName> {
        let ExtrasSpecificationHistory {
            extra,
            only_extra,
            no_extra,
            // These reference no extras explicitly
            all_extras: _,
            no_default_extras: _,
            defaults: _,
        } = self.history();

        extra.iter().chain(no_extra).chain(only_extra)
    }

    /// Returns `true` if the specification will have no effect.
    pub fn is_empty(&self) -> bool {
        self.prod() && self.exclude.is_empty() && self.include.is_empty()
    }

    /// Get the raw history for diagnostics
    pub fn history(&self) -> &ExtrasSpecificationHistory {
        &self.history
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "lookup indexes are implementation details and must not change diagnostics"
)]
impl Debug for ExtrasSpecificationInner {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExtrasSpecificationInner")
            .field("include", &self.include)
            .field("exclude", &self.exclude)
            .field("only_extras", &self.only_extras)
            .field("history", &self.history)
            .finish()
    }
}

/// Context about a [`ExtrasSpecification`][] that we've preserved for diagnostics
#[derive(Debug, Default, Clone)]
pub struct ExtrasSpecificationHistory {
    extra: Vec<ExtraName>,
    only_extra: Vec<ExtraName>,
    no_extra: Vec<ExtraName>,
    all_extras: bool,
    no_default_extras: bool,
    defaults: DefaultExtras,
}

impl ExtrasSpecificationHistory {
    /// Returns all the CLI flags that this represents.
    ///
    /// If a flag was provided multiple times (e.g. `--extra A --extra B`) this will
    /// elide the arguments and just show the flag once (e.g. just yield "--extra").
    ///
    /// Conceptually this being an empty list should be equivalent to
    /// [`ExtrasSpecification::is_empty`][] when there aren't any defaults set.
    /// When there are defaults the two will disagree, and rightfully so!
    pub fn as_flags_pretty(&self) -> Vec<Cow<'_, str>> {
        let Self {
            extra,
            no_extra,
            all_extras,
            only_extra,
            no_default_extras,
            // defaults aren't CLI flags!
            defaults: _,
        } = self;

        let mut flags = vec![];
        if *all_extras {
            flags.push(Cow::Borrowed("--all-extras"));
        }
        if *no_default_extras {
            flags.push(Cow::Borrowed("--no-default-extras"));
        }
        match &**extra {
            [] => {}
            [extra] => flags.push(Cow::Owned(format!("--extra {extra}"))),
            [..] => flags.push(Cow::Borrowed("--extra")),
        }
        match &**only_extra {
            [] => {}
            [extra] => flags.push(Cow::Owned(format!("--only-extra {extra}"))),
            [..] => flags.push(Cow::Borrowed("--only-extra")),
        }
        match &**no_extra {
            [] => {}
            [extra] => flags.push(Cow::Owned(format!("--no-extra {extra}"))),
            [..] => flags.push(Cow::Borrowed("--no-extra")),
        }
        flags
    }
}

/// A trivial newtype wrapped around [`ExtrasSpecification`][] that signifies "defaults applied"
#[derive(Debug, Clone)]
pub struct ExtrasSpecificationWithDefaults {
    /// The active semantics
    cur: ExtrasSpecification,
}

impl std::ops::Deref for ExtrasSpecificationWithDefaults {
    type Target = ExtrasSpecification;
    fn deref(&self) -> &Self::Target {
        &self.cur
    }
}

#[derive(Debug, Clone)]
pub enum IncludeExtras {
    /// Include dependencies from the specified extras.
    Some(Vec<ExtraName>),
    /// A marker indicates including dependencies from all extras.
    All,
}

impl IncludeExtras {
    /// Returns `true` if the specification includes the given extra.
    fn contains(&self, extra: &ExtraName) -> bool {
        match self {
            Self::Some(extras) => extras.contains(extra),
            Self::All => true,
        }
    }

    /// Returns `true` if the specification will have no effect.
    fn is_empty(&self) -> bool {
        match self {
            Self::Some(extras) => extras.is_empty(),
            // Although technically this is a noop if they have no extras,
            // conceptually they're *trying* to have an effect, so treat it as one.
            Self::All => false,
        }
    }
}

impl Default for IncludeExtras {
    fn default() -> Self {
        Self::Some(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_normalize::{DefaultExtras, ExtraName};

    use super::ExtrasSpecification;

    fn extra(name: &str) -> ExtraName {
        ExtraName::from_str(name).expect("valid extra name")
    }

    #[test]
    fn indexed_extras_preserve_precedence_and_history() {
        let extras = ExtrasSpecification::from_args(
            vec![
                extra("Feature_A"),
                extra("feature-a"),
                extra("feature-b"),
                extra("include-0"),
                extra("include-1"),
                extra("include-2"),
                extra("include-3"),
                extra("include-4"),
                extra("include-5"),
                extra("include-6"),
            ],
            vec![
                extra("FEATURE_A"),
                extra("disabled"),
                extra("exclude-0"),
                extra("exclude-1"),
                extra("exclude-2"),
                extra("exclude-3"),
                extra("exclude-4"),
                extra("exclude-5"),
                extra("exclude-6"),
            ],
            false,
            Vec::new(),
            false,
        )
        .with_defaults(DefaultExtras::List(vec![extra("default-c")]));

        assert!(extras.cur.0.include_index.is_some());
        assert!(extras.cur.0.exclude_index.is_some());
        assert!(!extras.contains(&extra("feature-a")));
        assert!(extras.contains(&extra("FEATURE_B")));
        assert!(extras.contains(&extra("default-c")));
        assert!(!extras.contains(&extra("disabled")));
        assert!(!extras.contains(&extra("missing")));
        assert_eq!(
            extras
                .explicit_names()
                .map(ExtraName::as_str)
                .collect::<Vec<_>>(),
            vec![
                "feature-a",
                "feature-a",
                "feature-b",
                "include-0",
                "include-1",
                "include-2",
                "include-3",
                "include-4",
                "include-5",
                "include-6",
                "feature-a",
                "disabled",
                "exclude-0",
                "exclude-1",
                "exclude-2",
                "exclude-3",
                "exclude-4",
                "exclude-5",
                "exclude-6",
            ],
        );
        assert_eq!(
            extras.history().as_flags_pretty(),
            ["--extra", "--no-extra"],
        );
    }

    #[test]
    fn indexed_extras_preserve_all_and_only_semantics() {
        let all = ExtrasSpecification::from_args(
            Vec::new(),
            vec![extra("disabled-a"), extra("disabled-b")],
            false,
            Vec::new(),
            true,
        );
        assert!(all.contains(&extra("other")));
        assert!(!all.contains(&extra("DISABLED_A")));

        let default_all = ExtrasSpecification::default().with_defaults(DefaultExtras::All);
        assert!(default_all.contains(&extra("other")));

        let only = ExtrasSpecification::from_args(
            Vec::new(),
            Vec::new(),
            false,
            vec![extra("only-a"), extra("only-b")],
            false,
        )
        .with_defaults(DefaultExtras::List(vec![extra("default-c")]));
        assert!(only.contains(&extra("ONLY_A")));
        assert!(!only.contains(&extra("default-c")));

        let replaced = default_all.with_defaults(DefaultExtras::default());
        assert!(!replaced.contains(&extra("other")));
    }

    #[test]
    fn extra_index_debug_is_hidden() {
        let extras = ExtrasSpecification::from_args(
            (0..9)
                .map(|index| extra(&format!("included-{index}")))
                .collect(),
            (0..9)
                .map(|index| extra(&format!("excluded-{index}")))
                .collect(),
            false,
            Vec::new(),
            false,
        );
        let debug = format!("{extras:?}");

        assert!(debug.contains("include: Some([ExtraName(\"included-0\")"));
        assert!(debug.contains("ExtraName(\"included-8\")])"));
        assert!(debug.contains("exclude: [ExtraName(\"excluded-0\")"));
        assert!(debug.contains("ExtraName(\"excluded-8\")]"));
        assert!(!debug.contains("include_index"));
        assert!(!debug.contains("exclude_index"));
    }
}
