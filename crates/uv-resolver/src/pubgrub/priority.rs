use std::cmp::Reverse;

use hashbrown::hash_map::EntryRef;
use pubgrub::DependencyProvider;

use uv_normalize::PackageName;
use uv_pep440::Version;

use crate::FxHashbrownMap;
use crate::dependency_provider::UvDependencyProvider;
use crate::fork_urls::ForkUrls;
use crate::pubgrub::{PubGrubPackage, PubGrubPackageInner, PubGrubPython, Range};

/// A prioritization map to guide the PubGrub resolution process.
///
/// During resolution, PubGrub needs to decide which package to consider next. The priorities
/// encoded here are used to guide that decision.
///
/// Like `pip`, we prefer packages that are pinned to direct URLs over packages pinned to a single
/// version over packages that are constrained in some way over packages that are unconstrained.
///
/// See: <https://github.com/pypa/pip/blob/ef78c129b1a966dbbbdb8ebfffc43723e89110d1/src/pip/_internal/resolution/resolvelib/provider.py#L120>
///
/// Our main priority is the package name, the earlier we encounter a package, the higher its
/// priority. This way, all virtual packages of the same name will be applied in a batch. To ensure
/// determinism, we also track the discovery order of virtual packages as secondary order.
#[derive(Clone, Debug, Default)]
pub(crate) struct PubGrubPriorities {
    package_priority: FxHashbrownMap<PackageName, PubGrubPriority>,
    virtual_package_tiebreaker: FxHashbrownMap<PubGrubPackage, PubGrubTiebreaker>,
}

impl PubGrubPriorities {
    /// Add a [`PubGrubPackage`] to the priority map.
    pub(crate) fn insert(
        &mut self,
        package: &PubGrubPackage,
        version: &Range<Version>,
        urls: &ForkUrls,
    ) {
        let len = self.virtual_package_tiebreaker.len();
        self.virtual_package_tiebreaker
            .entry_ref(package)
            .or_insert_with(|| {
                PubGrubTiebreaker::from(u32::try_from(len).expect("Less than 2**32 packages"))
            });

        // The root package and Python constraints have no explicit priority, the root package is
        // always first and the Python version (range) is fixed.
        let Some(name) = package.name_no_root() else {
            return;
        };

        let len = self.package_priority.len();
        let classify = |index| {
            let index = Reverse(index);
            if urls.get(name).is_some() {
                PubGrubPriority::DirectUrl(index)
            } else if version.is_singleton_constraint() {
                PubGrubPriority::Singleton(index)
            } else {
                PubGrubPriority::Unspecified(index)
            }
        };
        match self.package_priority.entry_ref(name) {
            EntryRef::Occupied(mut entry) => {
                // Preserve the original index.
                let index = entry.get().index().unwrap_or(len);
                let priority = classify(index);

                // Retain higher priorities, including conflict priorities, to avoid
                // reprioritization loops.
                if priority > *entry.get() {
                    entry.insert(priority);
                }
            }
            EntryRef::Vacant(entry) => {
                entry.insert(classify(len));
            }
        }
    }

    /// Return the [`PubGrubPriority`] of the given package, if it exists.
    pub(crate) fn get(
        &self,
        package: &PubGrubPackage,
    ) -> <UvDependencyProvider as DependencyProvider>::Priority {
        match &**package {
            // There is only a single root package despite the value. The priorities on root don't
            // matter for the resolution output, since the Pythons don't have dependencies
            // themselves and are only used when the package is incompatible.
            PubGrubPackageInner::Root(_) => (PubGrubPriority::Root, PubGrubTiebreaker::from(0)),
            PubGrubPackageInner::Python(PubGrubPython::Installed) => {
                (PubGrubPriority::Root, PubGrubTiebreaker::from(1))
            }
            PubGrubPackageInner::Python(PubGrubPython::Target) => {
                (PubGrubPriority::Root, PubGrubTiebreaker::from(2))
            }
            PubGrubPackageInner::System(_) => (PubGrubPriority::Root, PubGrubTiebreaker::from(3)),
            PubGrubPackageInner::Marker { name, .. }
            | PubGrubPackageInner::Extra { name, .. }
            | PubGrubPackageInner::Group { name, .. }
            | PubGrubPackageInner::Package { name, .. } => {
                // To ensure deterministic resolution, each (virtual) package needs to be registered
                // on discovery (as dependency of another package), before we query it for
                // prioritization.
                let package_priority = if let Some(priority) = self.package_priority.get(name) {
                    *priority
                } else {
                    debug_assert!(false, "Package not known: `{name}` from `{package}`");
                    PubGrubPriority::Unspecified(Reverse(usize::MAX))
                };

                let package_tiebreaker =
                    if let Some(tiebreaker) = self.virtual_package_tiebreaker.get(package) {
                        *tiebreaker
                    } else {
                        debug_assert!(
                            false,
                            "Package not registered in prioritization: `{package:?}`"
                        );
                        PubGrubTiebreaker(Reverse(u32::MAX))
                    };

                (package_priority, package_tiebreaker)
            }
        }
    }

    /// Set a package's priority to [`PubGrubPriority::ConflictEarly`].
    ///
    /// Returns whether the priority was changed, i.e., it's the first time we hit this condition
    /// for the package.
    pub(crate) fn mark_conflict_early(&mut self, package: &PubGrubPackage) -> bool {
        self.mark_conflict(package, PubGrubPriority::ConflictEarly)
    }

    /// Set a package's priority to [`PubGrubPriority::ConflictLate`] unless it is already marked
    /// as conflicting.
    ///
    /// Returns whether the priority was changed, i.e., it's the first time this package was
    /// marked as conflicting above the threshold.
    pub(crate) fn mark_conflict_late(&mut self, package: &PubGrubPackage) -> bool {
        self.mark_conflict(package, PubGrubPriority::ConflictLate)
    }

    fn mark_conflict(
        &mut self,
        package: &PubGrubPackage,
        make_priority: impl FnOnce(Reverse<usize>) -> PubGrubPriority,
    ) -> bool {
        let Some(name) = package.name_no_root() else {
            // Not a correctness bug
            debug_assert!(
                false,
                "URL packages must not be involved in conflict handling"
            );
            return false;
        };

        let len = self.package_priority.len();
        match self.package_priority.entry_ref(name) {
            EntryRef::Vacant(entry) => {
                entry.insert(make_priority(Reverse(len)));
                true
            }
            EntryRef::Occupied(mut entry) => {
                let current = *entry.get();
                // Keep early conflict priorities stable to avoid reprioritization loops.
                if matches!(current, PubGrubPriority::ConflictEarly(_)) {
                    return false;
                }
                let index = current.index().unwrap_or(len);
                let priority = make_priority(Reverse(index));
                if current == priority {
                    return false;
                }
                entry.insert(priority);
                true
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PubGrubPriority {
    /// The package has no specific priority.
    ///
    /// As such, its priority is based on the order in which the packages were added (FIFO), such
    /// that the first package we visit is prioritized over subsequent packages.
    ///
    /// TODO(charlie): Prefer constrained over unconstrained packages, if they're at the same depth
    /// in the dependency graph.
    Unspecified(Reverse<usize>),

    /// Selected version of this package were often the culprit of rejecting another package, so
    /// it's deprioritized behind `ConflictEarly`. It's still the higher than `Unspecified` to
    /// conflict before selecting unrelated packages.
    ConflictLate(Reverse<usize>),

    /// Selected version of this package were often rejected, so it's prioritized over
    /// `ConflictLate`.
    ConflictEarly(Reverse<usize>),

    /// The version range is constrained to a single version (e.g., with the `==` operator).
    Singleton(Reverse<usize>),

    /// The package was specified via a direct URL.
    ///
    /// N.B.: URLs need to have priority over registry distributions for correctly matching registry
    /// distributions to URLs, see [`PubGrubPackage::from_package`] an
    /// [`ForkUrls`].
    DirectUrl(Reverse<usize>),

    /// The package is the root package.
    Root,
}

impl PubGrubPriority {
    /// Return the package discovery index, if this priority has one.
    fn index(self) -> Option<usize> {
        match self {
            Self::ConflictLate(Reverse(index))
            | Self::Unspecified(Reverse(index))
            | Self::ConflictEarly(Reverse(index))
            | Self::Singleton(Reverse(index))
            | Self::DirectUrl(Reverse(index)) => Some(index),
            Self::Root => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PubGrubTiebreaker(Reverse<u32>);

impl From<u32> for PubGrubTiebreaker {
    fn from(value: u32) -> Self {
        Self(Reverse(value))
    }
}
