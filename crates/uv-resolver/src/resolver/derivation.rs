use pubgrub::{Id, Kind, State};
use rustc_hash::FxHashMap;

use uv_distribution_types::{DerivationChain, DerivationStep};
use uv_pep440::Version;

use crate::dependency_provider::UvDependencyProvider;
use crate::pubgrub::PubGrubPackage;

/// Build a [`DerivationChain`] from the pubgrub state, which is available in `uv-resolver`, but not
/// in `uv-distribution-types`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DerivationChainBuilder;

impl DerivationChainBuilder {
    /// Compute a [`DerivationChain`] from the current PubGrub state.
    ///
    /// This is used to construct a derivation chain upon resolution failure.
    pub(crate) fn from_state(
        id: Id<PubGrubPackage>,
        version: &Version,
        state: &State<UvDependencyProvider>,
    ) -> Option<DerivationChain> {
        /// Find a path from the current package to the root package.
        fn find_path(
            id: Id<PubGrubPackage>,
            version: &Version,
            state: &State<UvDependencyProvider>,
            solution: &FxHashMap<Id<PubGrubPackage>, Version>,
            path: &mut Vec<DerivationStep>,
        ) -> bool {
            // Retrieve the incompatibilities for the current package.
            let Some(incompatibilities) = state.incompatibilities.get(&id) else {
                return false;
            };
            for index in incompatibilities {
                let incompat = &state.incompatibility_store[*index];

                // Find a dependency from a package to the current package.
                let Kind::FromDependencyOf(parent_id, dependency_id) = &incompat.kind else {
                    continue;
                };
                let Some((_, Some(dependency_versions))) = incompat.dependency_version_sets()
                else {
                    continue;
                };
                if id != *dependency_id || !dependency_versions.contains(version) {
                    continue;
                }
                let Some(parent_version) = solution.get(parent_id) else {
                    continue;
                };
                let parent = &state.package_store[*parent_id];
                let dependency = &state.package_store[*dependency_id];
                let parent_name = parent.name_no_root();

                if parent_name == dependency.name_no_root() {
                    // Skip proxied dependencies.
                    if find_path(*parent_id, parent_version, state, solution, path) {
                        return true;
                    }
                    continue;
                }
                let Some(name) = parent_name else {
                    // If we've reached the root, return.
                    return true;
                };

                // Build the path from the root as the successful search unwinds.
                if find_path(*parent_id, parent_version, state, solution, path) {
                    path.push(DerivationStep::new(
                        name.clone(),
                        parent.extra().cloned(),
                        parent.group().cloned(),
                        Some(parent_version.clone()),
                        dependency_versions.encoded_versions().clone(),
                    ));
                    return true;
                }
            }
            false
        }

        let solution: FxHashMap<_, _> = state.partial_solution.extract_solution().collect();
        let mut path = Vec::new();
        if !find_path(id, version, state, &solution, &mut path) {
            return None;
        }
        Some(path.into_iter().collect())
    }
}
