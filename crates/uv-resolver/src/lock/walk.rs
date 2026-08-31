use std::collections::VecDeque;
use std::collections::hash_map::Entry;

use rustc_hash::{FxHashMap, FxHashSet};
use uv_normalize::ExtraName;
use uv_types::OnceQueue;

use super::{Dependency, Lock, Package, PackageIndex};
use crate::UniversalMarker;

/// A package and optional activated extra in a lock traversal.
pub(super) type LockTraversalState<'lock> = (PackageIndex, Option<&'lock ExtraName>);

/// A breadth-first lock traversal that visits each package-extra state once.
pub(super) struct LockWalker<'lock> {
    lock: &'lock Lock,
    queue: OnceQueue<LockTraversalState<'lock>>,
}

impl<'lock> LockWalker<'lock> {
    /// Create an empty traversal over `lock`.
    pub(super) fn new(lock: &'lock Lock) -> Self {
        Self {
            lock,
            queue: OnceQueue::default(),
        }
    }

    /// Queue a package state if it has not already been visited or queued.
    pub(super) fn push(&mut self, index: PackageIndex, extra: Option<&'lock ExtraName>) -> bool {
        self.queue.push((index, extra))
    }

    /// Queue a package's base dependencies and explicitly activated extras.
    pub(super) fn push_package(
        &mut self,
        index: PackageIndex,
        extras: impl IntoIterator<Item = &'lock ExtraName>,
    ) {
        self.push(index, None);
        for extra in extras {
            self.push(index, Some(extra));
        }
    }

    /// Queue the package and extras selected by a dependency edge.
    pub(super) fn push_dependency(&mut self, dependency: &'lock Dependency) {
        self.push_package(dependency.index, &dependency.extra);
    }

    /// Pop the next package-extra state and its contributed dependencies.
    pub(super) fn pop(&mut self) -> Option<LockVisit<'lock>> {
        let (index, extra) = self.queue.pop()?;
        Some(LockVisit::new(self.lock, index, extra))
    }
}

/// A fixed-point lock traversal that revisits states when their marker reachability changes.
pub(super) struct MarkerReachabilityWalker<'lock> {
    lock: &'lock Lock,
    queue: VecDeque<LockTraversalState<'lock>>,
    queued: FxHashSet<LockTraversalState<'lock>>,
    reachability: FxHashMap<LockTraversalState<'lock>, UniversalMarker>,
}

impl<'lock> MarkerReachabilityWalker<'lock> {
    /// Create an empty traversal over `lock`.
    pub(super) fn new(lock: &'lock Lock) -> Self {
        Self {
            lock,
            queue: VecDeque::new(),
            queued: FxHashSet::default(),
            reachability: FxHashMap::default(),
        }
    }

    /// Propagate `marker` to a package state, queuing it when reachability expands.
    pub(super) fn push(
        &mut self,
        index: PackageIndex,
        extra: Option<&'lock ExtraName>,
        marker: UniversalMarker,
    ) {
        let state = (index, extra);
        let changed = match self.reachability.entry(state) {
            Entry::Occupied(mut entry) => {
                let mut combined = *entry.get();
                combined.or(marker);
                if combined == *entry.get() {
                    false
                } else {
                    entry.insert(combined);
                    true
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(marker);
                true
            }
        };
        if changed && self.queued.insert(state) {
            self.queue.push_back(state);
        }
    }

    /// Propagate `marker` to a package's base dependencies and activated extras.
    pub(super) fn push_package(
        &mut self,
        index: PackageIndex,
        extras: impl IntoIterator<Item = &'lock ExtraName>,
        marker: UniversalMarker,
    ) {
        self.push(index, None, marker);
        for extra in extras {
            self.push(index, Some(extra), marker);
        }
    }

    /// Propagate `marker` to the package and extras selected by a dependency edge.
    pub(super) fn push_dependency(
        &mut self,
        dependency: &'lock Dependency,
        marker: UniversalMarker,
    ) {
        self.push_package(dependency.index, &dependency.extra, marker);
    }

    /// Pop the next expanded package-extra state and its accumulated reachability.
    pub(super) fn pop(&mut self) -> Option<(LockVisit<'lock>, UniversalMarker)> {
        let (index, extra) = self.queue.pop_front()?;
        self.queued.remove(&(index, extra));
        let marker = *self.reachability.get(&(index, extra))?;
        Some((LockVisit::new(self.lock, index, extra), marker))
    }
}

/// A package-extra state yielded by [`LockWalker`].
pub(super) struct LockVisit<'lock> {
    pub(super) index: PackageIndex,
    pub(super) extra: Option<&'lock ExtraName>,
    pub(super) package: &'lock Package,
    pub(super) dependencies: &'lock [Dependency],
}

impl<'lock> LockVisit<'lock> {
    fn new(lock: &'lock Lock, index: PackageIndex, extra: Option<&'lock ExtraName>) -> Self {
        let package = lock.package(index);
        Self {
            index,
            extra,
            package,
            dependencies: package.dependencies_for_extra(extra),
        }
    }
}
