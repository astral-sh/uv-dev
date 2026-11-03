use uv_distribution_types::{
    ExtraBuildRequirement, ExtraBuildRequiresError, Requirement, RequirementSource, ResolvedDist,
};

use crate::{ResolverOutput, UniversalMarker};

/// The static requirements to build one distribution throughout the given marker environment.
#[derive(Clone, Debug)]
pub struct SourceBuildRequirements {
    pub owner: ResolvedDist,
    pub marker: UniversalMarker,
    pub requirements: Vec<Requirement>,
}

impl ResolverOutput {
    /// Combine the backend's requirements with the owner's configured extra build dependencies.
    /// A runtime-matched requirement can select different distributions in different forks.
    pub fn match_build_requirements(
        &self,
        owner: &ResolvedDist,
        marker: UniversalMarker,
        backend: Vec<Requirement>,
        extras: &[ExtraBuildRequirement],
    ) -> Result<Vec<SourceBuildRequirements>, ExtraBuildRequiresError> {
        let mut sets = vec![SourceBuildRequirements {
            owner: owner.clone(),
            marker,
            requirements: backend,
        }];
        for extra in extras {
            if !extra.match_runtime {
                for set in &mut sets {
                    set.requirements.push(extra.requirement.clone());
                }
                continue;
            }

            extra.validate_runtime_source()?;
            let requirement = &extra.requirement;
            let applicable = requirement.marker;
            let candidates: Vec<_> = self
                .base_dists()
                .map(|(_, dist)| dist)
                .filter(|dist| dist.name == requirement.name)
                .collect();
            let mut next = Vec::new();
            for set in sets {
                let active = set.marker.combined().and(applicable);
                let inactive = set.marker.combined().and(applicable.negate());
                if !inactive.is_false() {
                    next.push(SourceBuildRequirements {
                        marker: UniversalMarker::from_combined(inactive),
                        ..set.clone()
                    });
                }

                let mut missing = active;
                for candidate in &candidates {
                    let intersection = active.and(candidate.marker.combined());
                    if intersection.is_false() {
                        continue;
                    }
                    let mut pinned = set.clone();
                    pinned.marker = UniversalMarker::from_combined(intersection);
                    pinned.requirements.push(Requirement {
                        source: RequirementSource::from(&candidate.dist),
                        ..requirement.clone()
                    });
                    next.push(pinned);
                    missing = missing.and(candidate.marker.combined().negate());
                }
                if !self.requires_python.simplify_markers(missing).is_false() {
                    return Err(ExtraBuildRequiresError::NotFound(requirement.name.clone()));
                }
            }
            sets = next;
        }
        Ok(sets)
    }
}
