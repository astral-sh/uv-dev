use serde::{Deserialize, Serialize};
use uv_pep440::{EncodedVersion, EncodedVersionRanges};

use super::wire::{CaptureReason, CaptureStatus};

/// Hard limits for the additional work and storage used by an internal capture.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureLimits {
    pub derivation_nodes: usize,
    pub packages: usize,
    pub terms: usize,
    pub intervals: usize,
    pub marker_nodes: usize,
    pub marker_edges: usize,
    pub availability_entries: usize,
    pub version_components: usize,
    pub atom_bytes: usize,
    pub text_bytes: usize,
    pub work: usize,
    pub json_bytes: usize,
}

impl CaptureLimits {
    /// The largest limits accepted by version 1 of the internal protocol.
    pub const V1: Self = Self {
        derivation_nodes: 16_384,
        packages: 4_096,
        terms: 65_536,
        intervals: EncodedVersionRanges::MAX_INTERVALS,
        marker_nodes: 16_384,
        marker_edges: 65_536,
        availability_entries: 65_536,
        version_components: EncodedVersion::MAX_COMPONENTS,
        atom_bytes: EncodedVersion::MAX_LOCAL_SEGMENT_BYTES,
        text_bytes: 2 * 1024 * 1024,
        work: 1_000_000,
        json_bytes: 8 * 1024 * 1024,
    };

    pub(super) fn is_supported(self) -> bool {
        let hard = Self::V1;
        self.derivation_nodes <= hard.derivation_nodes
            && self.packages <= hard.packages
            && self.terms <= hard.terms
            && self.intervals <= hard.intervals
            && self.marker_nodes <= hard.marker_nodes
            && self.marker_edges <= hard.marker_edges
            && self.availability_entries <= hard.availability_entries
            && self.version_components <= hard.version_components
            && self.atom_bytes <= hard.atom_bytes
            && self.text_bytes <= hard.text_bytes
            && self.work <= hard.work
            && self.json_bytes <= hard.json_bytes
    }
}

/// Counters collected without retaining a partial proof after a limit is reached.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureUsage {
    pub derivation_nodes: usize,
    pub packages: usize,
    pub terms: usize,
    pub intervals: usize,
    pub marker_nodes: usize,
    pub marker_edges: usize,
    pub availability_entries: usize,
    pub max_version_components: usize,
    pub max_atom_bytes: usize,
    pub text_bytes: usize,
    pub work: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Resource {
    DerivationNodes,
    Packages,
    Terms,
    Intervals,
    MarkerNodes,
    MarkerEdges,
    AvailabilityEntries,
    TextBytes,
    Work,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Stop {
    pub status: CaptureStatus,
    pub reason: CaptureReason,
}

impl Stop {
    pub const fn unsupported(reason: CaptureReason) -> Self {
        Self {
            status: CaptureStatus::Unsupported,
            reason,
        }
    }

    pub const fn truncated(reason: CaptureReason) -> Self {
        Self {
            status: CaptureStatus::Truncated,
            reason,
        }
    }
}

pub(super) struct Budget {
    pub limits: CaptureLimits,
    pub usage: CaptureUsage,
}

impl Budget {
    pub const fn new(limits: CaptureLimits) -> Self {
        Self {
            limits,
            usage: CaptureUsage {
                derivation_nodes: 0,
                packages: 0,
                terms: 0,
                intervals: 0,
                marker_nodes: 0,
                marker_edges: 0,
                availability_entries: 0,
                max_version_components: 0,
                max_atom_bytes: 0,
                text_bytes: 0,
                work: 0,
            },
        }
    }

    pub fn charge(&mut self, resource: Resource, amount: usize) -> Result<(), Stop> {
        let (counter, limit, reason) = match resource {
            Resource::DerivationNodes => (
                &mut self.usage.derivation_nodes,
                self.limits.derivation_nodes,
                CaptureReason::DerivationNodes,
            ),
            Resource::Packages => (
                &mut self.usage.packages,
                self.limits.packages,
                CaptureReason::Packages,
            ),
            Resource::Terms => (
                &mut self.usage.terms,
                self.limits.terms,
                CaptureReason::Terms,
            ),
            Resource::Intervals => (
                &mut self.usage.intervals,
                self.limits.intervals,
                CaptureReason::Intervals,
            ),
            Resource::MarkerNodes => (
                &mut self.usage.marker_nodes,
                self.limits.marker_nodes,
                CaptureReason::MarkerNodes,
            ),
            Resource::MarkerEdges => (
                &mut self.usage.marker_edges,
                self.limits.marker_edges,
                CaptureReason::MarkerEdges,
            ),
            Resource::AvailabilityEntries => (
                &mut self.usage.availability_entries,
                self.limits.availability_entries,
                CaptureReason::AvailabilityEntries,
            ),
            Resource::TextBytes => (
                &mut self.usage.text_bytes,
                self.limits.text_bytes,
                CaptureReason::TextBytes,
            ),
            Resource::Work => (&mut self.usage.work, self.limits.work, CaptureReason::Work),
        };
        let next = counter.saturating_add(amount);
        *counter = next.min(limit.saturating_add(1));
        if next > limit {
            Err(Stop::truncated(reason))
        } else {
            Ok(())
        }
    }

    pub fn work(&mut self, amount: usize) -> Result<(), Stop> {
        self.charge(Resource::Work, amount)
    }

    pub fn components(&mut self, count: usize) -> Result<(), Stop> {
        self.check_components(count)?;
        self.work(count)
    }

    pub fn check_components(&mut self, count: usize) -> Result<(), Stop> {
        self.usage.max_version_components = self
            .usage
            .max_version_components
            .max(count.min(self.limits.version_components.saturating_add(1)));
        if count > self.limits.version_components {
            return Err(Stop::truncated(CaptureReason::VersionComponents));
        }
        Ok(())
    }

    pub fn check_atom(&mut self, bytes: usize) -> Result<(), Stop> {
        self.usage.max_atom_bytes = self
            .usage
            .max_atom_bytes
            .max(bytes.min(self.limits.atom_bytes.saturating_add(1)));
        if bytes > self.limits.atom_bytes {
            Err(Stop::truncated(CaptureReason::AtomBytes))
        } else {
            Ok(())
        }
    }

    pub fn atom(&mut self, bytes: usize) -> Result<(), Stop> {
        self.check_atom(bytes)?;
        self.charge(Resource::TextBytes, bytes)?;
        self.work(1)
    }

    pub fn string(&mut self, value: &str) -> Result<String, Stop> {
        self.atom(value.len())?;
        Ok(value.to_owned())
    }

    pub fn decimal(&mut self, value: u64) -> Result<(), Stop> {
        let digits = value.checked_ilog10().map_or(1, |value| value as usize + 1);
        self.atom(digits)
    }
}
