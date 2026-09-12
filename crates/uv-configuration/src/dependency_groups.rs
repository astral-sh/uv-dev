use std::{
    borrow::Cow,
    fmt::{Debug, Formatter},
    sync::Arc,
};

use rustc_hash::FxHashSet;

use uv_normalize::{DEV_DEPENDENCIES, DefaultGroups, GroupName};

const GROUP_INDEX_THRESHOLD: usize = 8;

/// Manager of all dependency-group decisions and settings history.
///
/// This is an Arc mostly just to avoid size bloat on things that contain these.
#[derive(Debug, Default, Clone)]
pub struct DependencyGroups(Arc<DependencyGroupsInner>);

/// Manager of all dependency-group decisions and settings history.
#[derive(Default, Clone)]
pub struct DependencyGroupsInner {
    /// Groups to include.
    include: IncludeGroups,
    /// An optional index for multi-group includes.
    include_index: Option<FxHashSet<GroupName>>,
    /// Groups to exclude (always wins over include).
    exclude: Vec<GroupName>,
    /// An optional index for multi-group excludes.
    exclude_index: Option<FxHashSet<GroupName>>,
    /// Whether an `--only` flag was passed.
    ///
    /// If true, users of this API should refrain from looking at packages
    /// that *aren't* specified by the dependency-groups. This is exposed
    /// via [`DependencyGroupsInner::prod`][].
    only_groups: bool,
    /// The "raw" flags/settings we were passed for diagnostics.
    history: DependencyGroupsHistory,
}

impl DependencyGroups {
    /// Create from history.
    ///
    /// This is the "real" constructor, it's basically taking raw CLI flags but in
    /// a way that's a bit nicer for other constructors to use.
    fn from_history(history: DependencyGroupsHistory) -> Self {
        let DependencyGroupsHistory {
            dev_mode,
            mut group,
            mut only_group,
            mut no_group,
            all_groups,
            no_default_groups,
            mut defaults,
        } = history.clone();

        // First desugar --dev flags
        match dev_mode {
            Some(DevMode::Include) => group.push(DEV_DEPENDENCIES.clone()),
            Some(DevMode::Only) => only_group.push(DEV_DEPENDENCIES.clone()),
            Some(DevMode::Exclude) => no_group.push(DEV_DEPENDENCIES.clone()),
            None => {}
        }

        // `group` and `only_group` actually have the same meanings: packages to include.
        // But if `only_group` is non-empty then *other* packages should be excluded.
        // So we just record whether it was and then treat the two lists as equivalent.
        let only_groups = !only_group.is_empty();
        // --only flags imply --no-default-groups
        let default_groups = !no_default_groups && !only_groups;

        let include = if all_groups {
            // If this is set we can ignore group/only_group/defaults as irrelevant
            // (`--all-groups --only-*` is rejected at the CLI level, don't worry about it).
            IncludeGroups::All
        } else {
            // Merge all these lists, they're equivalent now
            group.append(&mut only_group);
            // Resolve default groups potentially also setting All
            if default_groups {
                match &mut defaults {
                    DefaultGroups::All => IncludeGroups::All,
                    DefaultGroups::List(defaults) => {
                        group.append(defaults);
                        IncludeGroups::Some(group)
                    }
                }
            } else {
                IncludeGroups::Some(group)
            }
        };

        let include_index = match &include {
            IncludeGroups::Some(groups) if groups.len() > GROUP_INDEX_THRESHOLD => {
                Some(groups.iter().cloned().collect())
            }
            IncludeGroups::Some(_) | IncludeGroups::All => None,
        };
        let exclude_index = if no_group.len() > GROUP_INDEX_THRESHOLD {
            Some(no_group.iter().cloned().collect())
        } else {
            None
        };

        Self(Arc::new(DependencyGroupsInner {
            include,
            include_index,
            exclude: no_group,
            exclude_index,
            only_groups,
            history,
        }))
    }

    /// Create from raw CLI args
    pub fn from_args(
        dev_mode: Option<DevMode>,
        group: Vec<GroupName>,
        no_group: Vec<GroupName>,
        no_default_groups: bool,
        only_group: Vec<GroupName>,
        all_groups: bool,
    ) -> Self {
        Self::from_history(DependencyGroupsHistory {
            dev_mode,
            group,
            only_group,
            no_group,
            all_groups,
            no_default_groups,
            // This is unknown at CLI-time, use `.with_defaults(...)` to apply this later!
            defaults: DefaultGroups::default(),
        })
    }

    /// Helper to make a spec from just a --dev flag
    pub fn from_dev_mode(dev_mode: DevMode) -> Self {
        Self::from_history(DependencyGroupsHistory {
            dev_mode: Some(dev_mode),
            ..Default::default()
        })
    }

    /// Helper to make a spec from just a --group
    pub fn from_group(group: GroupName) -> Self {
        Self::from_history(DependencyGroupsHistory {
            group: vec![group],
            ..Default::default()
        })
    }

    /// Helper to make a spec from just --all-groups.
    pub fn from_all_groups() -> Self {
        Self::from_history(DependencyGroupsHistory {
            all_groups: true,
            ..Default::default()
        })
    }

    /// Apply defaults to a base [`DependencyGroups`].
    ///
    /// This is appropriate in projects, where the `dev` group is synced by default.
    pub fn with_defaults(&self, defaults: DefaultGroups) -> DependencyGroupsWithDefaults {
        if self.0.history.defaults == defaults {
            return DependencyGroupsWithDefaults {
                cur: self.clone(),
                prev: self.clone(),
            };
        }

        // Explicitly clone the inner history and set the defaults, then remake the result.
        let mut history = self.0.history.clone();
        history.defaults = defaults;

        DependencyGroupsWithDefaults {
            cur: Self::from_history(history),
            prev: self.clone(),
        }
    }
}

impl std::ops::Deref for DependencyGroups {
    type Target = DependencyGroupsInner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DependencyGroupsInner {
    /// Returns `true` if packages other than the ones referenced by these
    /// dependency-groups should be considered.
    ///
    /// That is, if I tell you to install a project and this is false,
    /// you should ignore the project itself and all its dependencies,
    /// and instead just install the dependency-groups.
    ///
    /// (This is really just asking if an --only flag was passed.)
    pub fn prod(&self) -> bool {
        !self.only_groups
    }

    /// Returns `true` if the specification includes the given group.
    pub fn contains(&self, group: &GroupName) -> bool {
        // exclude always trumps include
        let excluded = self.exclude_index.as_ref().map_or_else(
            || self.exclude.contains(group),
            |index| index.contains(group),
        );
        if excluded {
            return false;
        }

        self.include_index.as_ref().map_or_else(
            || self.include.contains(group),
            |index| index.contains(group),
        )
    }

    /// Returns an iterator over all groups that are included in the specification,
    /// assuming `all_names` is an iterator over all groups.
    pub fn group_names<'a, Names>(
        &'a self,
        all_names: Names,
    ) -> impl Iterator<Item = &'a GroupName> + 'a
    where
        Names: Iterator<Item = &'a GroupName> + 'a,
    {
        all_names.filter(move |name| self.contains(name))
    }

    /// Iterate over all groups the user explicitly asked for on the CLI
    pub fn explicit_names(&self) -> impl Iterator<Item = &GroupName> {
        let DependencyGroupsHistory {
            // Strictly speaking this is an explicit reference to "dev"
            // but we're currently tolerant of dev not existing when referenced with
            // these flags, since it kinda implicitly always exists even if
            // it's not properly defined in a config file.
            dev_mode: _,
            group,
            only_group,
            no_group,
            // These reference no groups explicitly
            all_groups: _,
            no_default_groups: _,
            // This doesn't include defaults because the `dev` group may not be defined
            // but gets implicitly added as a default sometimes!
            defaults: _,
        } = self.history();

        group.iter().chain(no_group).chain(only_group)
    }

    /// Get the raw history for diagnostics
    pub fn history(&self) -> &DependencyGroupsHistory {
        &self.history
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "lookup indexes are implementation details and must not change diagnostics"
)]
impl Debug for DependencyGroupsInner {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DependencyGroupsInner")
            .field("include", &self.include)
            .field("exclude", &self.exclude)
            .field("only_groups", &self.only_groups)
            .field("history", &self.history)
            .finish()
    }
}

/// Context about a [`DependencyGroups`][] that we've preserved for diagnostics
#[derive(Debug, Default, Clone)]
pub struct DependencyGroupsHistory {
    dev_mode: Option<DevMode>,
    group: Vec<GroupName>,
    only_group: Vec<GroupName>,
    no_group: Vec<GroupName>,
    all_groups: bool,
    no_default_groups: bool,
    defaults: DefaultGroups,
}

impl DependencyGroupsHistory {
    /// Returns all the CLI flags that this represents.
    ///
    /// If a flag was provided multiple times (e.g. `--group A --group B`) this will
    /// elide the arguments and just show the flag once (e.g. just yield "--group").
    ///
    /// Conceptually this being an empty list should be equivalent to
    /// [`DependencyGroups::is_empty`][] when there aren't any defaults set.
    /// When there are defaults the two will disagree, and rightfully so!
    pub fn as_flags_pretty(&self) -> Vec<Cow<'_, str>> {
        let Self {
            dev_mode,
            group,
            only_group,
            no_group,
            all_groups,
            no_default_groups,
            // defaults aren't CLI flags!
            defaults: _,
        } = self;

        let mut flags = vec![];
        if *all_groups {
            flags.push(Cow::Borrowed("--all-groups"));
        }
        if *no_default_groups {
            flags.push(Cow::Borrowed("--no-default-groups"));
        }
        if let Some(dev_mode) = dev_mode {
            flags.push(Cow::Borrowed(dev_mode.as_flag()));
        }
        match &**group {
            [] => {}
            [group] => flags.push(Cow::Owned(format!("--group {group}"))),
            [..] => flags.push(Cow::Borrowed("--group")),
        }
        match &**only_group {
            [] => {}
            [group] => flags.push(Cow::Owned(format!("--only-group {group}"))),
            [..] => flags.push(Cow::Borrowed("--only-group")),
        }
        match &**no_group {
            [] => {}
            [group] => flags.push(Cow::Owned(format!("--no-group {group}"))),
            [..] => flags.push(Cow::Borrowed("--no-group")),
        }
        flags
    }
}

/// A trivial newtype wrapped around [`DependencyGroups`][] that signifies "defaults applied"
///
/// It includes a copy of the previous semantics to provide info on if
/// the group being a default actually affected it being enabled, because it's obviously "correct".
/// (These are Arcs so it's ~free to hold onto the previous semantics)
#[derive(Debug, Clone)]
pub struct DependencyGroupsWithDefaults {
    /// The active semantics
    cur: DependencyGroups,
    /// The semantics before defaults were applied
    prev: DependencyGroups,
}

impl DependencyGroupsWithDefaults {
    /// Do not enable any groups
    ///
    /// Many places in the code need to know what dependency-groups are active,
    /// but various commands or subsystems never enable any dependency-groups,
    /// in which case they want this.
    pub fn none() -> Self {
        DependencyGroups::default().with_defaults(DefaultGroups::default())
    }

    /// Returns `true` if the specification was enabled, and *only* because it was a default
    pub fn contains_because_default(&self, group: &GroupName) -> bool {
        self.cur.contains(group) && !self.prev.contains(group)
    }
}
impl std::ops::Deref for DependencyGroupsWithDefaults {
    type Target = DependencyGroups;
    fn deref(&self) -> &Self::Target {
        &self.cur
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DevMode {
    /// Include development dependencies.
    #[default]
    Include,
    /// Exclude development dependencies.
    Exclude,
    /// Only include development dependencies, excluding all other dependencies.
    Only,
}

impl DevMode {
    /// Determine the development dependency mode from the command-line arguments.
    pub fn from_args(dev: bool, no_dev: bool, only_dev: bool) -> Option<Self> {
        // In theory only one of these 3 flags should be set (enforced by CLI),
        // but we explicitly allow `--dev` and `--only-dev` to both be set,
        // and "saturate" that to `--only-dev`.
        if only_dev {
            Some(Self::Only)
        } else if no_dev {
            Some(Self::Exclude)
        } else if dev {
            Some(Self::Include)
        } else {
            None
        }
    }

    /// Returns the flag that was used to request development dependencies.
    fn as_flag(self) -> &'static str {
        match self {
            Self::Exclude => "--no-dev",
            Self::Include => "--dev",
            Self::Only => "--only-dev",
        }
    }
}

#[derive(Debug, Clone)]
pub enum IncludeGroups {
    /// Include dependencies from the specified groups.
    Some(Vec<GroupName>),
    /// A marker indicates including dependencies from all groups.
    All,
}

impl IncludeGroups {
    /// Returns `true` if the specification includes the given group.
    fn contains(&self, group: &GroupName) -> bool {
        match self {
            Self::Some(groups) => groups.contains(group),
            Self::All => true,
        }
    }
}

impl Default for IncludeGroups {
    fn default() -> Self {
        Self::Some(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_normalize::{DefaultGroups, GroupName};

    use super::{DependencyGroups, DevMode};

    fn group(name: &str) -> GroupName {
        GroupName::from_str(name).expect("valid group name")
    }

    #[test]
    fn indexed_groups_preserve_precedence_and_history() {
        let groups = DependencyGroups::from_args(
            None,
            vec![
                group("Feature_A"),
                group("feature-a"),
                group("feature-b"),
                group("include-0"),
                group("include-1"),
                group("include-2"),
                group("include-3"),
                group("include-4"),
                group("include-5"),
                group("include-6"),
            ],
            vec![
                group("FEATURE_A"),
                group("disabled"),
                group("exclude-0"),
                group("exclude-1"),
                group("exclude-2"),
                group("exclude-3"),
                group("exclude-4"),
                group("exclude-5"),
                group("exclude-6"),
            ],
            false,
            Vec::new(),
            false,
        )
        .with_defaults(DefaultGroups::List(vec![group("default-c")]));

        assert!(groups.cur.0.include_index.is_some());
        assert!(groups.cur.0.exclude_index.is_some());
        assert!(!groups.contains(&group("feature-a")));
        assert!(groups.contains(&group("FEATURE_B")));
        assert!(groups.contains(&group("default-c")));
        assert!(!groups.contains(&group("disabled")));
        assert!(!groups.contains(&group("missing")));
        assert!(groups.contains_because_default(&group("default-c")));
        assert!(!groups.contains_because_default(&group("feature-b")));
        assert_eq!(
            groups
                .explicit_names()
                .map(GroupName::as_str)
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
            groups.history().as_flags_pretty(),
            ["--group", "--no-group"],
        );
    }

    #[test]
    fn indexed_groups_preserve_all_only_and_dev_semantics() {
        let all = DependencyGroups::from_args(
            None,
            Vec::new(),
            vec![group("disabled-a"), group("disabled-b")],
            false,
            Vec::new(),
            true,
        )
        .with_defaults(DefaultGroups::default());
        assert!(all.contains(&group("other")));
        assert!(!all.contains(&group("DISABLED_A")));
        assert!(!all.contains_because_default(&group("other")));

        let default_all = DependencyGroups::default().with_defaults(DefaultGroups::All);
        assert!(default_all.contains(&group("other")));
        assert!(default_all.contains_because_default(&group("other")));
        let replaced = default_all.with_defaults(DefaultGroups::default());
        assert!(!replaced.contains(&group("other")));

        let only = DependencyGroups::from_args(
            None,
            Vec::new(),
            Vec::new(),
            false,
            vec![group("only-a"), group("only-b")],
            false,
        )
        .with_defaults(DefaultGroups::List(vec![group("default-c")]));
        assert!(only.contains(&group("ONLY_A")));
        assert!(!only.contains(&group("default-c")));
        assert!(!only.contains_because_default(&group("default-c")));

        let no_dev = DependencyGroups::from_args(
            Some(DevMode::Exclude),
            vec![group("dev"), group("feature")],
            vec![group("other")],
            false,
            Vec::new(),
            false,
        );
        assert!(!no_dev.contains(&group("DEV")));
        assert!(no_dev.contains(&group("feature")));
    }

    #[test]
    fn group_index_debug_is_hidden() {
        let groups = DependencyGroups::from_args(
            None,
            (0..9)
                .map(|index| group(&format!("included-{index}")))
                .collect(),
            (0..9)
                .map(|index| group(&format!("excluded-{index}")))
                .collect(),
            false,
            Vec::new(),
            false,
        );
        let debug = format!("{groups:?}");

        assert!(debug.contains("include: Some([GroupName(\"included-0\")"));
        assert!(debug.contains("GroupName(\"included-8\")])"));
        assert!(debug.contains("exclude: [GroupName(\"excluded-0\")"));
        assert!(debug.contains("GroupName(\"excluded-8\")]"));
        assert!(!debug.contains("include_index"));
        assert!(!debug.contains("exclude_index"));
    }
}
