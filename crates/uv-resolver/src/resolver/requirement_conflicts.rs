//! Classify independently selectable roots in exhaustive version partitions.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use itertools::Itertools;

use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictSet, Conflicts, RequirementConflict};

use crate::manifest::WorkspaceRootConflicts;
use crate::pubgrub::Range;
use crate::universal_marker::{ConflictMarker, UniversalMarker};

/// One native resolver state used to determine whether a selection can use a version partition.
#[derive(Clone, Debug)]
pub(super) struct RequirementFork {
    pub(super) selection: ConflictItem,
    partitions: Arc<[usize]>,
    pub(super) requirements: Arc<[RequirementConflict]>,
}

/// Successful environments for every independently selectable root and version partition.
///
/// A failed state contributes no environments. It does not remove any dependency from another
/// state, and its failure is not itself permission to split two selections.
pub(super) struct RequirementForks {
    selections: Vec<ConflictItem>,
    partitions: Vec<Arc<[RequirementConflict]>>,
    environments: Vec<Vec<MarkerTree>>,
}

impl RequirementForks {
    pub(super) fn new(
        selections: BTreeSet<ConflictItem>,
        allowed: &WorkspaceRootConflicts,
    ) -> Option<Self> {
        if selections.is_empty() || allowed.requirements.is_empty() {
            return None;
        }
        let partitions: Vec<Arc<[RequirementConflict]>> = allowed
            .requirements
            .iter()
            .map(|set| set.iter())
            .multi_cartesian_product()
            // Several declarations for the same package may have an empty intersection.
            .filter(|partition| {
                let mut ranges = BTreeMap::<&PackageName, Range<Version>>::new();
                for requirement in partition {
                    let range = ranges
                        .entry(requirement.package())
                        .or_insert_with(Range::full);
                    *range = range.intersection(&Range::from(requirement.version_range()));
                    if *range == Range::empty() {
                        return false;
                    }
                }
                true
            })
            .map(|partition| partition.into_iter().cloned().collect())
            .collect();
        let environments = vec![vec![MarkerTree::FALSE; partitions.len()]; selections.len()];
        Some(Self {
            selections: selections.into_iter().collect(),
            partitions,
            environments,
        })
    }

    pub(super) fn forks(&self) -> impl Iterator<Item = Arc<RequirementFork>> + '_ {
        let partitions: Arc<[usize]> = (0..self.partitions.len()).collect();
        self.selections.iter().map(move |selection| {
            Arc::new(RequirementFork {
                selection: selection.clone(),
                partitions: Arc::clone(&partitions),
                requirements: Arc::new([]),
            })
        })
    }

    /// Split immediately before deciding the first version of a declared package. Dimensions
    /// that never become dependencies remain unassigned and do not create redundant solves.
    pub(super) fn split(
        &self,
        fork: &RequirementFork,
        package: &PackageName,
    ) -> Option<Vec<Arc<RequirementFork>>> {
        if fork
            .requirements
            .iter()
            .any(|requirement| requirement.package() == package)
        {
            return None;
        }
        let mut alternatives = BTreeMap::<Vec<RequirementConflict>, Vec<usize>>::new();
        for &partition in fork.partitions.iter() {
            let requirements: Vec<_> = self.partitions[partition]
                .iter()
                .filter(|requirement| requirement.package() == package)
                .cloned()
                .collect();
            if requirements.is_empty() {
                return None;
            }
            alternatives
                .entry(requirements)
                .or_default()
                .push(partition);
        }
        Some(
            alternatives
                .into_iter()
                .map(|(requirements, partitions)| {
                    Arc::new(RequirementFork {
                        selection: fork.selection.clone(),
                        partitions: partitions.into(),
                        requirements: fork
                            .requirements
                            .iter()
                            .cloned()
                            .chain(requirements)
                            .collect(),
                    })
                })
                .collect(),
        )
    }

    pub(super) fn record(&mut self, fork: &RequirementFork, env: UniversalMarker) {
        let selection = self
            .selections
            .binary_search(&fork.selection)
            .expect("requirement fork has a registered selection");
        for &partition in fork.partitions.iter() {
            let marker = &mut self.environments[selection][partition];
            *marker = marker.or(env.combined());
        }
    }

    /// Lower a pair when, in some supported environment, both selections are independently usable
    /// but no declared partition admits both. Selection conflicts apply to the entire lockfile.
    /// The ordinary combined solve remains responsible for every other incompatibility.
    pub(super) fn conflicts(self, existing: &Conflicts) -> Vec<ConflictSet> {
        let world =
            UniversalMarker::new(MarkerTree::TRUE, ConflictMarker::from_conflicts(existing))
                .combined();
        let mut conflicts = Vec::new();
        for (left_index, right_index) in (0..self.selections.len()).tuple_combinations() {
            let left = &self.selections[left_index];
            let right = &self.selections[right_index];
            if existing.iter().any(|set| {
                set.contains(left.package(), left.kind().as_ref())
                    && set.contains(right.package(), right.kind().as_ref())
            }) {
                continue;
            }
            let left_environments = &self.environments[left_index];
            let right_environments = &self.environments[right_index];
            if left_environments.iter().all(|marker| marker.is_false())
                || right_environments.iter().all(|marker| marker.is_false())
            {
                continue;
            }
            let left_any = left_environments
                .iter()
                .fold(MarkerTree::FALSE, |all, marker| all.or(*marker));
            let right_any = right_environments
                .iter()
                .fold(MarkerTree::FALSE, |all, marker| all.or(*marker));
            let common = left_environments
                .iter()
                .zip(right_environments)
                .fold(MarkerTree::FALSE, |all, (left, right)| {
                    all.or(left.and(*right))
                });
            if !left_any
                .and(right_any)
                .and(world)
                .and(common.negate())
                .is_false()
            {
                conflicts.push(
                    ConflictSet::try_from(vec![left.clone(), right.clone()])
                        .expect("two distinct selections form a conflict set"),
                );
            }
        }
        conflicts
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::str::FromStr;

    use uv_normalize::PackageName;
    use uv_pep508::MarkerTree;
    use uv_pypi_types::{ConflictItem, Conflicts, RequirementConflict};

    use super::RequirementForks;
    use crate::manifest::WorkspaceRootConflicts;
    use crate::universal_marker::{ConflictMarker, UniversalMarker};

    #[test]
    fn overlapping_declarations_split_only_when_the_package_is_relevant() {
        let selection = ConflictItem::from(PackageName::from_str("root").unwrap());
        let allowed = WorkspaceRootConflicts::new(
            [selection.package().clone()].into_iter().collect(),
            [["leaf<2", "leaf>=2"], ["leaf<3", "leaf>=3"]]
                .map(|set| {
                    set.map(|requirement| {
                        RequirementConflict::try_from(requirement.to_owned()).unwrap()
                    })
                    .into_iter()
                    .collect()
                })
                .into_iter()
                .collect(),
        );
        let classification =
            RequirementForks::new([selection].into_iter().collect(), &allowed).unwrap();
        let forks = classification.forks().collect::<Vec<_>>();
        assert_eq!(forks.len(), 1);
        assert_eq!(forks[0].partitions.len(), 3);
        assert!(
            classification
                .split(&forks[0], &PackageName::from_str("unrelated").unwrap())
                .is_none()
        );
        let leaf = PackageName::from_str("leaf").unwrap();
        let forks = classification.split(&forks[0], &leaf).unwrap();
        assert_eq!(forks.len(), 3);
        assert!(
            forks
                .iter()
                .all(|fork| classification.split(fork, &leaf).is_none())
        );
    }

    #[test]
    fn disjoint_partitions_do_not_conflict_with_broad_or_invalid_roots() {
        let selections: BTreeSet<_> = ["root-a", "root-b", "root-c", "root-invalid"]
            .map(|name| ConflictItem::from(PackageName::from_str(name).unwrap()))
            .into_iter()
            .collect();
        let allowed = WorkspaceRootConflicts::new(
            selections
                .iter()
                .map(|item| item.package().clone())
                .collect(),
            vec![vec![
                RequirementConflict::try_from("leaf<2".to_owned()).unwrap(),
                RequirementConflict::try_from("leaf>=2".to_owned()).unwrap(),
            ]],
        );
        let mut classification = RequirementForks::new(selections, &allowed).unwrap();
        let leaf = PackageName::from_str("leaf").unwrap();
        let forks = classification
            .forks()
            .flat_map(|fork| classification.split(&fork, &leaf).unwrap())
            .collect::<Vec<_>>();
        for fork in forks {
            let compatible = match fork.selection.package().as_str() {
                "root-a" => fork.partitions.as_ref() == [0],
                "root-b" => fork.partitions.as_ref() == [1],
                "root-c" => true,
                "root-invalid" => false,
                _ => unreachable!(),
            };
            if compatible {
                classification.record(
                    &fork,
                    UniversalMarker::new(
                        MarkerTree::TRUE,
                        ConflictMarker::from_conflict_item(&fork.selection),
                    ),
                );
            }
        }
        let conflicts = classification.conflicts(&Conflicts::empty());
        assert_eq!(conflicts.len(), 1);
        assert_eq!(
            conflicts[0]
                .iter()
                .map(|item| item.package().as_str())
                .collect::<Vec<_>>(),
            ["root-a", "root-b"]
        );
    }
}
