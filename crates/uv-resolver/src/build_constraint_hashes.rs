//! Trusted build constraints for a resolution spanning multiple marker environments.

use uv_configuration::{Constraints, HashCheckingMode};
use uv_distribution_types::HashCollection;
use uv_pep508::MarkerTree;
use uv_resolver_types::UniversalMarker;
use uv_types::{HashStrategy, HashStrategyError};

/// Hash policies for the disjoint regions of a universal build resolution.
pub struct BuildConstraintHashes {
    domain: MarkerTree,
    regions: Vec<(MarkerTree, HashStrategy)>,
}

impl BuildConstraintHashes {
    /// Keep the original constraint order within each region. In particular, two declarations
    /// for the same version can trust different archives on different targets.
    pub fn from_constraints(
        constraints: &Constraints,
        mode: HashCheckingMode,
        domain: MarkerTree,
    ) -> Result<Self, HashStrategyError> {
        let mut regions = vec![MarkerTree::TRUE];
        for entry in constraints.specifications() {
            let marker = entry.requirement.marker.resolve_extras([]);
            if marker.is_true() || marker.is_disjoint(domain) {
                continue;
            }
            // In require mode a hashless, unpinned declaration can be an error in its region.
            if !mode.is_require()
                && entry.hashes.is_empty()
                && entry.requirement.hashes()?.is_none()
            {
                continue;
            }
            let complement = marker.negate();
            let existing = regions.len();
            for index in 0..existing {
                let region = regions[index];
                let active = region.and(domain);
                if !active.is_disjoint(marker) && !active.is_disjoint(complement) {
                    regions[index] = region.and(marker);
                    regions.push(region.and(complement));
                }
            }
        }
        let regions = regions
            .into_iter()
            .map(|region| {
                let active = region.and(domain);
                let hasher = HashStrategy::from_requirements(
                    std::iter::empty(),
                    constraints
                        .specifications()
                        .filter(|entry| {
                            !entry
                                .requirement
                                .marker
                                .resolve_extras([])
                                .is_disjoint(active)
                        })
                        .map(|entry| (&entry.requirement, entry.hashes.as_slice())),
                    None,
                    mode,
                )?;
                Ok::<_, HashStrategyError>((
                    region,
                    HashStrategy::collect(HashCollection::Url)
                        .with_verification(hasher.verification().clone()),
                ))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { domain, regions })
    }

    /// Environments where both the project and the build owner are supported.
    pub(crate) fn active_domain(&self, scope: UniversalMarker) -> MarkerTree {
        scope.pep508().and(self.domain)
    }

    /// Limit a build owner's scope to regions with a single trusted hash policy.
    pub(crate) fn for_scope(
        &self,
        scope: UniversalMarker,
    ) -> impl Iterator<Item = (UniversalMarker, &HashStrategy)> {
        let active = self.active_domain(scope);
        self.regions.iter().filter_map(move |(region, hasher)| {
            if region.is_disjoint(active) {
                return None;
            }
            let mut scope = scope;
            if !region.negate().is_disjoint(active) {
                scope.and(UniversalMarker::from_combined(*region));
            }
            Some((scope, hasher))
        })
    }
}
