use std::borrow::Cow;

use either::Either;
use rustc_hash::FxHashMap;

use uv_distribution_types::{
    NameRequirementSpecification, Requirement, RequirementSource, ResolutionRecorder,
};
use uv_normalize::PackageName;

use crate::dependency_modifier::apply_extra_marker;

/// A set of constraints for a set of requirements.
#[derive(Debug, Default, Clone)]
pub struct Constraints {
    recorder: Option<ResolutionRecorder>,
    /// Original declarations, including hashes, for hash verification.
    specifications: Vec<NameRequirementSpecification>,
    /// Constraints grouped by package name.
    requirements: FxHashMap<PackageName, Vec<Requirement>>,
}

impl Constraints {
    /// Record which settings are consulted while resolving runtime dependencies.
    #[must_use]
    pub fn with_recorder(mut self, recorder: Option<ResolutionRecorder>) -> Self {
        self.recorder = recorder;
        self
    }

    /// Create a new set of constraints from a set of requirements.
    pub fn from_requirements(requirements: impl Iterator<Item = Requirement>) -> Self {
        Self::from_specifications(requirements.map(NameRequirementSpecification::from))
    }

    /// Create constraints while retaining their hashes and original declarations.
    pub fn from_specifications(
        specifications: impl IntoIterator<Item = NameRequirementSpecification>,
    ) -> Self {
        let specifications: Vec<_> = specifications.into_iter().collect();
        let mut constraints: FxHashMap<PackageName, Vec<Requirement>> = FxHashMap::default();
        for specification in &specifications {
            let requirement = &specification.requirement;
            // Skip empty constraints.
            if let RequirementSource::Registry { specifier, .. } = &requirement.source
                && specifier.is_empty()
            {
                continue;
            }

            constraints
                .entry(requirement.name.clone())
                .or_default()
                .push(Requirement {
                    // We add and apply constraints independent of their extras.
                    extras: Box::new([]),
                    ..requirement.clone()
                });
        }
        Self {
            recorder: None,
            specifications,
            requirements: constraints,
        }
    }

    /// Return the original declarations, including hashes, in input order.
    pub fn specifications(&self) -> impl Iterator<Item = &NameRequirementSpecification> {
        self.specifications.iter()
    }

    /// Return an iterator over all [`Requirement`]s in the constraint set.
    pub fn requirements(&self) -> impl Iterator<Item = &Requirement> {
        self.requirements.values().flatten()
    }

    /// Get the constraints for a package.
    pub fn get(&self, name: &PackageName) -> Option<&Vec<Requirement>> {
        if let Some(recorder) = &self.recorder {
            recorder.constraint(name);
        }
        self.requirements.get(name)
    }

    /// Apply the constraints to a set of requirements.
    pub fn apply<'a>(
        &'a self,
        requirements: impl IntoIterator<Item = Cow<'a, Requirement>>,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> {
        requirements.into_iter().flat_map(|requirement| {
            let Some(constraints) = self.get(&requirement.name) else {
                return Either::Left(std::iter::once(requirement));
            };

            let constraints = apply_extra_marker(requirement.marker, constraints);
            Either::Right(std::iter::once(requirement).chain(constraints))
        })
    }
}
