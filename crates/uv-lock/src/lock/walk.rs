//! Shared mechanics for traversing an immutable lock.
//!
//! [`LockWalker`] visits each package/extra pair once. [`MarkerReachabilityWalker`] uses the same
//! states and dependency expansion, but revisits a state when its reachability marker grows.
//! Roots, dependency groups, marker evaluation, pruning, and output remain caller policy.
//!
//! Not every operation on a lock is a package/extra walk. Validation can revisit a package after
//! discovering additional metadata obligations. Tree rendering tracks path/depth and inversion
//! context, and export's conflict inference tracks per-node conflict maps as well as reachability.
//! Those algorithms retain their richer state instead of reducing it to a package/extra pair.

use rustc_hash::FxHashMap;
use uv_normalize::ExtraName;
use uv_types::OnceQueue;

use super::{Dependency, Lock, Package, PackageIndex};
use crate::UniversalMarker;
use crate::graph_ops::{Boolean, MarkerReachability};

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
pub(super) struct MarkerReachabilityWalker<'lock, Marker = UniversalMarker> {
    lock: &'lock Lock,
    reachability: MarkerReachability<LockTraversalState<'lock>, Marker>,
}

impl<'lock, Marker: Boolean + Copy + PartialEq> MarkerReachabilityWalker<'lock, Marker> {
    /// Create an empty traversal over `lock`.
    pub(super) fn new(lock: &'lock Lock) -> Self {
        Self {
            lock,
            reachability: MarkerReachability::with_capacity(lock.len()),
        }
    }

    /// Propagate `marker` to a package state, queuing it when reachability expands.
    pub(super) fn push(
        &mut self,
        index: PackageIndex,
        extra: Option<&'lock ExtraName>,
        marker: Marker,
    ) {
        self.reachability.push((index, extra), marker);
    }

    /// Propagate `marker` to a package's base dependencies and activated extras.
    pub(super) fn push_package(
        &mut self,
        index: PackageIndex,
        extras: impl IntoIterator<Item = &'lock ExtraName>,
        marker: Marker,
    ) {
        self.push(index, None, marker);
        for extra in extras {
            self.push(index, Some(extra), marker);
        }
    }

    /// Propagate `marker` to the package and extras selected by a dependency edge.
    pub(super) fn push_dependency(&mut self, dependency: &'lock Dependency, marker: Marker) {
        self.push_package(dependency.index, &dependency.extra, marker);
    }

    /// Pop the next expanded package-extra state and its accumulated reachability.
    pub(super) fn pop(&mut self) -> Option<(LockVisit<'lock>, Marker)> {
        let ((index, extra), marker) = self.reachability.pop()?;
        Some((LockVisit::new(self.lock, index, extra), marker))
    }

    /// Return the accumulated reachability of every discovered package-extra state.
    pub(super) fn into_markers(self) -> FxHashMap<LockTraversalState<'lock>, Marker> {
        self.reachability.into_markers()
    }
}

/// A package-extra state yielded by either lock walker.
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
