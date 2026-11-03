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
            // Build requirements are roots of the isolated build environment, so no
            // package extras are active when evaluating the requirement itself.
            let applicable = requirement.marker.resolve_extras([]);
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use petgraph::Graph;
    use uv_configuration::{Constraints, DependencyModifiers};
    use uv_distribution_types::{
        ExtraBuildRequirement, InstalledDistKind, InstalledRegistryDist, Requirement,
        RequiresPython, ResolvedDist,
    };
    use uv_normalize::PackageName;
    use uv_pep440::Version;
    use uv_pep508::{MarkerTree, Requirement as Pep508Requirement};
    use uv_pypi_types::HashDigests;

    use crate::{
        AnnotatedDist, Options, PackageNodeKind, PackageRole, ResolutionGraphNode, ResolverOutput,
        UniversalMarker,
    };

    fn selection(name: &str, version: &str, marker: MarkerTree) -> AnnotatedDist {
        let name: PackageName = name.parse().expect("valid package name");
        let version: Version = version.parse().expect("valid version");
        let installed = InstalledDistKind::Registry(InstalledRegistryDist {
            name: name.clone(),
            version: version.clone(),
            path: PathBuf::new().into_boxed_path(),
            cache_info: None,
            build_info: None,
        });
        AnnotatedDist {
            name,
            version,
            dist: ResolvedDist::Installed {
                dist: Arc::new(installed.into()),
            },
            kind: PackageNodeKind::Base,
            role: PackageRole::Dependency,
            hashes: HashDigests::empty(),
            metadata: None,
            marker: UniversalMarker::from_combined(marker),
        }
    }

    /// Two Python forks select different versions of the build dependency on Linux. It is absent
    /// from the runtime graph on other platforms, where the extra build requirement is inactive.
    #[test]
    fn match_build_requirements_across_python_forks() -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = ResolverOutput {
            builds: Vec::new(),
            graph: Graph::new(),
            requires_python: RequiresPython::from_specifiers(">=3.10".parse()?),
            fork_markers: vec![],
            diagnostics: vec![],
            requirements: vec![],
            constraints: Constraints::default(),
            modifiers: DependencyModifiers::default(),
            options: Options::default(),
        };
        let linux: MarkerTree = "sys_platform == 'linux'".parse()?;
        let older: MarkerTree = "python_version < '3.12'".parse()?;
        let owner = selection("owner", "1", MarkerTree::TRUE);
        runtime.graph.add_node(ResolutionGraphNode::Dist(selection(
            "native",
            "1",
            linux.and(older),
        )));
        let newer_node = runtime.graph.add_node(ResolutionGraphNode::Dist(selection(
            "native",
            "2",
            linux.and(older.negate()),
        )));
        let native = ExtraBuildRequirement {
            requirement: Requirement::from(
                "native; sys_platform == 'linux'".parse::<Pep508Requirement<_>>()?,
            ),
            match_runtime: true,
        };
        let backend = Requirement::from("backend".parse::<Pep508Requirement<_>>()?);

        let sets = runtime.match_build_requirements(
            &owner.dist,
            owner.marker,
            vec![backend.clone()],
            std::slice::from_ref(&native),
        )?;
        let pins: Vec<_> = sets
            .into_iter()
            .map(|set| {
                assert_eq!(set.requirements.first(), Some(&backend));
                (
                    set.marker.combined(),
                    set.requirements.get(1).map(ToString::to_string),
                )
            })
            .collect();
        assert_eq!(
            pins,
            [
                (linux.negate(), None),
                (
                    linux.and(older),
                    Some("native==1 ; sys_platform == 'linux'".to_string())
                ),
                (
                    linux.and(older.negate()),
                    Some("native==2 ; sys_platform == 'linux'".to_string())
                ),
            ]
        );

        runtime.graph.remove_node(newer_node);
        let error = runtime
            .match_build_requirements(&owner.dist, owner.marker, vec![backend], &[native])
            .expect_err("Python 3.12+ is missing a runtime selection on Linux");
        assert_eq!(
            error.to_string(),
            "`native` was declared as an extra build dependency with `match-runtime = true`, but was not found in the resolution"
        );
        Ok(())
    }
}
