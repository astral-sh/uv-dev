use std::collections::hash_map::Entry;
use std::sync::Arc;

use pubgrub::{DerivationTree, External};
use rustc_hash::{FxHashMap, FxHashSet};

use uv_distribution_types::RequirementProvenance;
use uv_normalize::PackageName;
use uv_pep440::Version;

use crate::error::ErrorTree;
use crate::pubgrub::{
    DependencySource, PubGrubDependency, PubGrubPackage, PubGrubPackageInner, Range,
};
use crate::python_requirement::PythonRequirement;
use crate::resolver::ResolverEnvironment;

/// The complete dependency insertion ledger for an unnamed root.
///
/// PubGrub does not retain dependency occurrence IDs in its error tree. An unnamed root has one
/// version, so a source can still be identified when every inserted clause for a dependency name
/// is the same clause and comes from one authored occurrence. Any distinct or unlocated clause
/// makes that name permanently ambiguous. This also prevents display-time marker or range
/// simplification from making one declaration look like another.
#[derive(Clone, Default)]
pub(crate) struct RootDependencyLedger {
    by_name: Arc<FxHashMap<PackageName, Option<RootDependencyWitness>>>,
}

/// An unambiguous insertion that also occurs in the original PubGrub proof.
#[derive(Default)]
pub(crate) struct RootDependencyProof {
    by_name: FxHashMap<PackageName, RootDependencyWitness>,
}

#[derive(Clone)]
struct RootDependencyWitness {
    clause: RootDependencyClause,
    source: DependencySource,
    provenance: RequirementProvenance,
}

#[derive(Clone, Eq, PartialEq)]
struct RootDependencyClause {
    package: PubGrubPackage,
    versions: Range<Version>,
    dependency: PubGrubPackage,
    dependency_versions: Range<Version>,
}

impl RootDependencyLedger {
    /// Record the exact edges passed to PubGrub after applicability and fork filtering.
    pub(crate) fn record(
        &mut self,
        package: &PubGrubPackage,
        versions: &Range<Version>,
        dependencies: &[PubGrubDependency],
    ) {
        let PubGrubPackageInner::Root(None) = &**package else {
            return;
        };

        for dependency in dependencies {
            let Some(name) = dependency.package.name_no_root() else {
                continue;
            };
            let witness = dependency
                .provenance
                .as_ref()
                .map(|provenance| RootDependencyWitness {
                    clause: RootDependencyClause {
                        package: package.clone(),
                        versions: versions.clone(),
                        dependency: dependency.package.clone(),
                        dependency_versions: dependency.version.clone(),
                    },
                    source: dependency.source.clone(),
                    provenance: provenance.clone(),
                });

            match Arc::make_mut(&mut self.by_name).entry(name.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(witness);
                }
                Entry::Occupied(mut entry) => {
                    let merged = entry
                        .get_mut()
                        .take()
                        .zip(witness)
                        .and_then(|(existing, witness)| existing.unambiguous_with(&witness));
                    entry.insert(merged);
                }
            }
        }
    }

    /// Retain only exact, applicable root clauses used by the original proof.
    pub(crate) fn into_proof(
        self,
        tree: &ErrorTree,
        env: &ResolverEnvironment,
        python_requirement: &PythonRequirement,
    ) -> RootDependencyProof {
        let mut proof = RootDependencyProof::default();
        visit_root_dependencies(
            tree,
            |package, versions, dependency, dependency_versions| {
                let Some(name) = dependency.name_no_root() else {
                    return;
                };
                let Some(Some(witness)) = self.by_name.get(name) else {
                    return;
                };
                if witness
                    .clause
                    .matches(package, versions, dependency, dependency_versions)
                    && witness.is_applicable(env, python_requirement)
                {
                    proof.by_name.insert(name.clone(), witness.clone());
                }
            },
        );
        proof
    }
}

impl RootDependencyProof {
    /// Return sources only for exact clauses that remain in the reduced report.
    pub(crate) fn sources(&self, tree: &ErrorTree) -> Vec<RequirementProvenance> {
        let mut sources = Vec::new();
        let mut seen = FxHashSet::default();
        visit_root_dependencies(
            tree,
            |package, versions, dependency, dependency_versions| {
                let Some(name) = dependency.name_no_root() else {
                    return;
                };
                let Some(witness) = self.by_name.get(name) else {
                    return;
                };
                if witness
                    .clause
                    .matches(package, versions, dependency, dependency_versions)
                    && seen.insert(name.clone())
                {
                    sources.push(witness.provenance.clone());
                }
            },
        );
        sources
    }
}

impl RootDependencyWitness {
    fn unambiguous_with(self, other: &Self) -> Option<Self> {
        if self.clause != other.clause || self.source != other.source {
            return None;
        }
        self.provenance
            .unambiguous_with(&other.provenance)
            .map(|provenance| Self { provenance, ..self })
    }

    fn is_applicable(
        &self,
        env: &ResolverEnvironment,
        python_requirement: &PythonRequirement,
    ) -> bool {
        let marker = self.clause.dependency.marker();
        marker.evaluate_optional_environment(env.marker_environment(), &[])
            && !python_requirement.to_marker_tree().is_disjoint(marker)
            && env.included_by_marker(marker)
            && self
                .clause
                .dependency
                .conflicting_item()
                .is_none_or(|item| env.included_by_group(item))
    }
}

impl RootDependencyClause {
    fn matches(
        &self,
        package: &PubGrubPackage,
        versions: &Range<Version>,
        dependency: &PubGrubPackage,
        dependency_versions: &Range<Version>,
    ) -> bool {
        self.package == *package
            && self.versions == *versions
            && self.dependency == *dependency
            && self.dependency_versions == *dependency_versions
    }
}

fn visit_root_dependencies(
    tree: &ErrorTree,
    mut visit: impl FnMut(&PubGrubPackage, &Range<Version>, &PubGrubPackage, &Range<Version>),
) {
    let mut trees = vec![tree];
    let mut seen = FxHashSet::default();
    while let Some(tree) = trees.pop() {
        if !seen.insert(std::ptr::from_ref(tree)) {
            continue;
        }
        match tree {
            DerivationTree::Derived(derived) => {
                trees.push(&derived.cause2);
                trees.push(&derived.cause1);
            }
            DerivationTree::External(External::FromDependencyOf(
                package,
                versions,
                dependency,
                dependency_versions,
            )) => {
                if let PubGrubPackageInner::Root(None) = &**package {
                    visit(package, versions, dependency, dependency_versions);
                }
            }
            DerivationTree::External(
                External::NotRoot(..) | External::NoVersions(..) | External::Custom(..),
            ) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error as StdError;
    use std::io;

    use uv_distribution_types::{Requirement, RequiresPython};
    use uv_errors::SourceFile;
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};
    use uv_pypi_types::{Conflicts, VerbatimParsedUrl};

    use crate::resolver::environment::fork_version_by_marker;
    use crate::resolver::{Dependencies, ForkedDependencies};

    use super::*;

    fn root() -> PubGrubPackage {
        PubGrubPackage::from(PubGrubPackageInner::Root(None))
    }

    fn root_versions() -> Range<Version> {
        Range::singleton(Version::new([0]))
    }

    fn source(name: &str, contents: &str) -> RequirementProvenance {
        RequirementProvenance::new(
            SourceFile::new(name, contents),
            0..contents.trim_end().len(),
        )
        .with_source_text()
    }

    fn dependency(
        requirement: &str,
        provenance: Option<RequirementProvenance>,
    ) -> Result<PubGrubDependency, Box<dyn StdError>> {
        let requirement = Requirement {
            provenance,
            ..Requirement::from(requirement.parse::<uv_pep508::Requirement<VerbatimParsedUrl>>()?)
        };
        PubGrubDependency::from_requirements(
            &Conflicts::default(),
            [Cow::Owned(requirement)],
            None,
            Some(&root()),
        )
        .map_err(|error| io::Error::other(error.to_string()))?
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::other("expected a dependency").into())
    }

    fn tree(dependency: &PubGrubDependency) -> ErrorTree {
        ErrorTree::External(External::FromDependencyOf(
            root(),
            root_versions(),
            dependency.package.clone(),
            dependency.version.clone(),
        ))
    }

    fn python_requirement() -> Result<PythonRequirement, Box<dyn StdError>> {
        let marker_env = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: "3.12.0",
            os_name: "posix",
            platform_machine: "arm64",
            platform_python_implementation: "CPython",
            platform_release: "1",
            platform_system: "Darwin",
            platform_version: "1",
            python_full_version: "3.12.0",
            python_version: "3.12",
            sys_platform: "darwin",
        })?;
        Ok(PythonRequirement::from_marker_environment(
            &marker_env,
            RequiresPython::greater_than_equal_version(&Version::new([3, 12])),
        ))
    }

    #[test]
    fn provenance_is_not_dependency_identity() -> Result<(), Box<dyn StdError>> {
        let plain = dependency("pypyp>=1", None)?;
        let first = dependency("pypyp>=1", Some(source("first.in", "pypyp>=1\n")))?;
        let second = dependency("pypyp>=1", Some(source("second.in", "pypyp>=1\n")))?;
        assert_eq!(plain, first);
        assert_eq!(first, second);
        assert!(first.provenance.is_some());
        Ok(())
    }

    #[test]
    fn root_dependency_requires_complete_insertion_identity() -> Result<(), Box<dyn StdError>> {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement()?;
        let occurrence = source("requirements.in", "pypyp>=1\n");
        let first = dependency("pypyp>=1", Some(occurrence.clone()))?;
        let original = tree(&first);
        let mut outcomes = Vec::new();
        for (name, other) in [
            ("same occurrence", first.clone()),
            (
                "independent parse",
                dependency("pypyp>=1", Some(source("requirements.in", "pypyp>=1\n")))?,
            ),
            ("missing occurrence", dependency("pypyp>=1", None)?),
            (
                "different range",
                dependency("pypyp>=2", Some(occurrence.clone()))?,
            ),
            (
                "different marker",
                dependency(
                    "pypyp>=1; sys_platform == 'linux'",
                    Some(occurrence.clone()),
                )?,
            ),
        ] {
            let mut ledger = RootDependencyLedger::default();
            ledger.record(&root(), &root_versions(), &[first.clone(), other]);
            let proof = ledger.into_proof(&original, &env, &python_requirement);
            outcomes.push((name, proof.sources(&original).len()));
        }
        insta::assert_debug_snapshot!(outcomes, @r#"
        [
            (
                "same occurrence",
                1,
            ),
            (
                "independent parse",
                0,
            ),
            (
                "missing occurrence",
                0,
            ),
            (
                "different range",
                0,
            ),
            (
                "different marker",
                0,
            ),
        ]
        "#);
        Ok(())
    }

    #[test]
    fn root_dependency_must_survive_the_reduced_proof() -> Result<(), Box<dyn StdError>> {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement()?;
        let first = dependency("pypyp>=1", Some(source("requirements.in", "pypyp>=1\n")))?;
        let other = dependency("pypyp>=2", None)?;
        let original = tree(&first);
        let reduced = tree(&other);
        let mut ledger = RootDependencyLedger::default();
        ledger.record(&root(), &root_versions(), std::slice::from_ref(&first));

        let unrelated_proof = ledger
            .clone()
            .into_proof(&reduced, &env, &python_requirement);
        assert!(unrelated_proof.sources(&original).is_empty());

        let proof = ledger.into_proof(&original, &env, &python_requirement);
        assert_eq!(proof.sources(&original).len(), 1);
        assert!(proof.sources(&reduced).is_empty());
        assert!(
            proof
                .sources(&ErrorTree::External(External::NoVersions(
                    first.package,
                    first.version,
                )))
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn root_dependency_source_context_must_be_unique() -> Result<(), Box<dyn StdError>> {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement()?;
        let occurrence = source("requirements.in", "pypyp\n");
        let registry = dependency("pypyp", Some(occurrence.clone()))?;
        let url = dependency(
            "pypyp @ https://example.com/pypyp-1.0-py3-none-any.whl",
            Some(occurrence),
        )?;
        assert_eq!(registry.package, url.package);
        assert_eq!(registry.version, url.version);
        assert_ne!(registry.source, url.source);

        let original = tree(&registry);
        let mut ledger = RootDependencyLedger::default();
        ledger.record(&root(), &root_versions(), &[registry, url]);
        assert!(
            ledger
                .into_proof(&original, &env, &python_requirement)
                .sources(&original)
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn root_dependency_ignores_named_projects() -> Result<(), Box<dyn StdError>> {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement()?;
        let dependency = dependency("pypyp>=1", Some(source("requirements.in", "pypyp>=1\n")))?;
        let project = PubGrubPackage::from(PubGrubPackageInner::Root(Some("project".parse()?)));
        let original = ErrorTree::External(External::FromDependencyOf(
            project.clone(),
            root_versions(),
            dependency.package.clone(),
            dependency.version.clone(),
        ));
        let mut ledger = RootDependencyLedger::default();
        ledger.record(&project, &root_versions(), &[dependency]);
        assert!(
            ledger
                .into_proof(&original, &env, &python_requirement)
                .sources(&original)
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn root_dependency_ledger_is_fork_local() -> Result<(), Box<dyn StdError>> {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement()?;
        let first = dependency("pypyp>=1", Some(source("first.in", "pypyp>=1\n")))?;
        let other = dependency("pypyp>=1", Some(source("second.in", "pypyp>=1\n")))?;
        let original = tree(&first);
        let mut parent = RootDependencyLedger::default();
        parent.record(&root(), &root_versions(), std::slice::from_ref(&first));
        let mut child = parent.clone();
        child.record(&root(), &root_versions(), &[other]);

        assert_eq!(
            parent
                .into_proof(&original, &env, &python_requirement)
                .sources(&original)
                .len(),
            1
        );
        assert!(
            child
                .into_proof(&original, &env, &python_requirement)
                .sources(&original)
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn root_dependency_rechecks_fork_applicability() -> Result<(), Box<dyn StdError>> {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement()?;
        let dependency = dependency(
            "pypyp>=1; sys_platform == 'linux'",
            Some(source(
                "requirements.in",
                "pypyp>=1; sys_platform == 'linux'\n",
            )),
        )?;
        let (included, excluded) = fork_version_by_marker(&env, dependency.package.marker())
            .ok_or_else(|| io::Error::other("expected a marker fork"))?;
        let original = tree(&dependency);
        let mut ledger = RootDependencyLedger::default();
        ledger.record(&root(), &root_versions(), &[dependency]);

        assert_eq!(
            ledger
                .clone()
                .into_proof(&original, &included, &python_requirement)
                .sources(&original)
                .len(),
            1
        );
        assert!(
            ledger
                .into_proof(&original, &excluded, &python_requirement)
                .sources(&original)
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn root_dependency_records_only_each_forks_inserted_edges() -> Result<(), Box<dyn StdError>> {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement()?;
        let first = dependency(
            "pypyp>=1; sys_platform == 'linux'",
            Some(source("linux.in", "pypyp>=1; sys_platform == 'linux'\n")),
        )?;
        let other = dependency(
            "pypyp>=2; sys_platform != 'linux'",
            Some(source("other.in", "pypyp>=2; sys_platform != 'linux'\n")),
        )?;
        let ForkedDependencies::Forked { forks, .. } =
            ForkedDependencies::from_dependencies_universal(
                Dependencies::Available(vec![first.clone(), other.clone()]),
                &env,
                &python_requirement,
                &Conflicts::default(),
            )
        else {
            return Err(io::Error::other("expected disjoint dependency forks").into());
        };

        assert_eq!(forks.len(), 2);
        for fork in forks {
            let [selected] = fork.dependencies.as_slice() else {
                return Err(io::Error::other("expected one dependency per fork").into());
            };
            let original = tree(selected);
            let mut ledger = RootDependencyLedger::default();
            ledger.record(&root(), &root_versions(), &fork.dependencies);
            let proof = ledger.into_proof(&original, &fork.env, &python_requirement);
            let sources = proof.sources(&original);
            let [source] = sources.as_slice() else {
                return Err(io::Error::other("expected the selected occurrence").into());
            };
            assert!(
                selected
                    .provenance
                    .as_ref()
                    .and_then(|selected| source.unambiguous_with(selected))
                    .is_some()
            );
            let unselected = if selected == &first { &other } else { &first };
            assert!(proof.sources(&tree(unselected)).is_empty());
        }
        Ok(())
    }

    #[test]
    fn root_dependency_does_not_choose_an_overlapping_fork_occurrence()
    -> Result<(), Box<dyn StdError>> {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement()?;
        let first = dependency(
            "pypyp>=1; sys_platform == 'linux'",
            Some(source("first.in", "pypyp>=1; sys_platform == 'linux'\n")),
        )?;
        let other = dependency(
            "pypyp>=2; sys_platform != 'win32'",
            Some(source("second.in", "pypyp>=2; sys_platform != 'win32'\n")),
        )?;
        let ForkedDependencies::Forked { forks, .. } =
            ForkedDependencies::from_dependencies_universal(
                Dependencies::Available(vec![first.clone(), other]),
                &env,
                &python_requirement,
                &Conflicts::default(),
            )
        else {
            return Err(io::Error::other("expected overlapping dependency forks").into());
        };
        let fork = forks
            .into_iter()
            .find(|fork| fork.dependencies.len() == 2)
            .ok_or_else(|| io::Error::other("expected a fork containing both declarations"))?;
        let original = tree(&first);
        let mut ledger = RootDependencyLedger::default();
        ledger.record(&root(), &root_versions(), &fork.dependencies);
        assert!(
            ledger
                .into_proof(&original, &fork.env, &python_requirement)
                .sources(&original)
                .is_empty()
        );
        Ok(())
    }
}
