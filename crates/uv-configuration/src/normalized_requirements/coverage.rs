//! Track overlapping version exclusions while removing redundant specifiers.

use std::ops::{Bound, Range};

use uv_pep440::Version;
use version_ranges::Ranges;

/// The number of remaining specifiers that exclude each part of the version range.
pub(super) struct VersionRangeCoverage<'a> {
    endpoints: Vec<&'a Version>,
    counts: RangeCounts,
}

impl<'a> VersionRangeCoverage<'a> {
    pub(super) fn new(exclusions: &'a [Ranges<Version>]) -> Self {
        let mut endpoints: Vec<_> = exclusions
            .iter()
            .flat_map(Ranges::iter)
            .flat_map(|(lower, upper)| [lower, upper])
            .filter_map(|bound| match bound {
                Bound::Included(version) | Bound::Excluded(version) => Some(version),
                Bound::Unbounded => None,
            })
            .collect();
        endpoints.sort();
        endpoints.dedup();

        // Each endpoint and each open interval between endpoints is a distinct coordinate.
        let counts = RangeCounts::new(2 * endpoints.len() + 1);
        let mut coverage = Self { endpoints, counts };
        for exclusion in exclusions {
            coverage.update(exclusion, 1);
        }
        coverage
    }

    /// Whether every excluded version is also excluded by another specifier.
    pub(super) fn is_redundant(&self, exclusion: &Ranges<Version>) -> bool {
        exclusion.iter().all(|bounds| {
            let coordinates = self.coordinates(bounds);
            coordinates.is_empty() || self.counts.minimum(&coordinates) >= 2
        })
    }

    pub(super) fn remove(&mut self, exclusion: &Ranges<Version>) {
        self.update(exclusion, -1);
    }

    fn update(&mut self, exclusion: &Ranges<Version>, delta: isize) {
        for bounds in exclusion.iter() {
            let coordinates = self.coordinates(bounds);
            if !coordinates.is_empty() {
                self.counts.add(&coordinates, delta);
            }
        }
    }

    /// Map inclusive and exclusive bounds to a half-open interval of coordinates.
    fn coordinates(&self, (lower, upper): (Bound<&Version>, Bound<&Version>)) -> Range<usize> {
        let endpoint = |version| {
            self.endpoints
                .partition_point(|candidate| *candidate < version)
        };
        let start = match lower {
            Bound::Included(version) => 2 * endpoint(version) + 1,
            Bound::Excluded(version) => 2 * endpoint(version) + 2,
            Bound::Unbounded => 0,
        };
        let end = match upper {
            Bound::Included(version) => 2 * endpoint(version) + 2,
            Bound::Excluded(version) => 2 * endpoint(version) + 1,
            Bound::Unbounded => 2 * self.endpoints.len() + 1,
        };
        start..end
    }
}

/// A segment tree supporting range increments and range-minimum queries.
struct RangeCounts {
    len: usize,
    nodes: Vec<RangeCount>,
}

/// The minimum includes this node's lazy update but excludes its ancestors' updates, permitting
/// updates and queries without pushing lazy values into child nodes.
#[derive(Clone, Default)]
struct RangeCount {
    minimum: isize,
    lazy: isize,
}

impl RangeCounts {
    fn new(len: usize) -> Self {
        Self {
            len,
            nodes: vec![RangeCount::default(); 4 * len],
        }
    }

    fn add(&mut self, target: &Range<usize>, delta: isize) {
        self.add_node(1, 0, self.len, target, delta);
    }

    fn add_node(
        &mut self,
        node: usize,
        left: usize,
        right: usize,
        target: &Range<usize>,
        delta: isize,
    ) {
        if target.end <= left || right <= target.start {
            return;
        }
        if target.start <= left && right <= target.end {
            let node = &mut self.nodes[node];
            node.minimum += delta;
            node.lazy += delta;
            return;
        }
        let middle = left + (right - left) / 2;
        self.add_node(node * 2, left, middle, target, delta);
        self.add_node(node * 2 + 1, middle, right, target, delta);
        let child_minimum = self.nodes[node * 2]
            .minimum
            .min(self.nodes[node * 2 + 1].minimum);
        let node = &mut self.nodes[node];
        node.minimum = node.lazy + child_minimum;
    }

    fn minimum(&self, target: &Range<usize>) -> isize {
        self.minimum_node(1, 0, self.len, target, 0)
    }

    fn minimum_node(
        &self,
        node: usize,
        left: usize,
        right: usize,
        target: &Range<usize>,
        parent: isize,
    ) -> isize {
        if target.end <= left || right <= target.start {
            return isize::MAX;
        }
        if target.start <= left && right <= target.end {
            return parent + self.nodes[node].minimum;
        }
        let middle = left + (right - left) / 2;
        let parent = parent + self.nodes[node].lazy;
        self.minimum_node(node * 2, left, middle, target, parent)
            .min(self.minimum_node(node * 2 + 1, middle, right, target, parent))
    }
}
