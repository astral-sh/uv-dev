//! Classify independently selectable roots in exhaustive version partitions.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ptr;
use std::sync::Arc;

use itertools::Itertools;
use rustc_hash::FxHashMap;

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
    assignment: Arc<[(usize, usize)]>,
    pub(super) requirements: Arc<[RequirementConflict]>,
}

/// The nonempty intersections of declarations for a single package.
struct RequirementPartitions {
    index: usize,
    alternatives: Vec<Arc<[RequirementConflict]>>,
}

/// Successful environments and partial version assignments for each selectable root.
///
/// A failed state contributes no environments. It does not remove any dependency from another
/// state, and its failure is not itself permission to split two selections.
pub(super) struct RequirementForks {
    selections: Vec<ConflictItem>,
    partitions: BTreeMap<PackageName, RequirementPartitions>,
    counts: Vec<usize>,
    successes: Vec<RequirementSuccesses>,
}

impl RequirementForks {
    pub(super) fn new(
        selections: BTreeSet<ConflictItem>,
        allowed: &WorkspaceRootConflicts,
    ) -> Option<Self> {
        if selections.is_empty() || allowed.requirements.is_empty() {
            return None;
        }
        let mut declarations = BTreeMap::<_, Vec<_>>::new();
        for set in &allowed.requirements {
            if let Some(requirement) = set.first() {
                declarations
                    .entry(requirement.package().clone())
                    .or_default()
                    .push(set);
            }
        }

        let mut counts = Vec::new();
        let partitions = declarations
            .into_iter()
            .enumerate()
            .map(|(index, (package, sets))| {
                // Refine only declarations for the same package. Intersections across different
                // packages are represented symbolically and never need an upfront product.
                let mut alternatives = vec![(Range::<Version>::full(), Vec::new())];
                for set in sets {
                    let mut refined = Vec::new();
                    for (range, requirements) in alternatives {
                        for requirement in set {
                            let intersection =
                                range.intersection(&Range::from(requirement.version_range()));
                            if intersection != Range::empty() {
                                let mut requirements = requirements.clone();
                                requirements.push(requirement.clone());
                                refined.push((intersection, requirements));
                            }
                        }
                    }
                    alternatives = refined;
                }

                counts.push(alternatives.len());
                let alternatives = alternatives
                    .into_iter()
                    .map(|(_, requirements)| requirements.into())
                    .collect();
                (
                    package,
                    RequirementPartitions {
                        index,
                        alternatives,
                    },
                )
            })
            .collect();
        let successes = vec![RequirementSuccesses::empty(); selections.len()];
        Some(Self {
            selections: selections.into_iter().collect(),
            partitions,
            counts,
            successes,
        })
    }

    pub(super) fn forks(&self) -> impl Iterator<Item = Arc<RequirementFork>> + '_ {
        self.selections.iter().map(move |selection| {
            Arc::new(RequirementFork {
                selection: selection.clone(),
                assignment: Arc::new([]),
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
        let partitions = self.partitions.get(package)?;
        let Err(position) = fork
            .assignment
            .binary_search_by_key(&partitions.index, |(index, _)| *index)
        else {
            return None;
        };
        Some(
            partitions
                .alternatives
                .iter()
                .enumerate()
                .map(|(alternative, requirements)| {
                    let mut assignment = fork.assignment.to_vec();
                    assignment.insert(position, (partitions.index, alternative));
                    Arc::new(RequirementFork {
                        selection: fork.selection.clone(),
                        assignment: assignment.into(),
                        requirements: fork
                            .requirements
                            .iter()
                            .chain(requirements.iter())
                            .cloned()
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
        self.successes[selection].record(&fork.assignment, env.combined(), &self.counts);
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
            let left_successes = &self.successes[left_index];
            let right_successes = &self.successes[right_index];
            let left_any = left_successes.environments;
            let right_any = right_successes.environments;
            let joint = left_any.and(right_any).and(world);
            if joint.is_false() {
                continue;
            }
            let common =
                left_successes.common_environments(right_successes, &mut FxHashMap::default());
            if !joint.and(common.negate()).is_false() {
                conflicts.push(
                    ConflictSet::try_from(vec![left.clone(), right.clone()])
                        .expect("two distinct selections form a conflict set"),
                );
            }
        }
        conflicts
    }
}

/// A reduced, ordered decision tree over package partitions. A leaf applies to every remaining
/// package; equal alternatives collapse back into a leaf or shared subtree. Reference counting
/// releases completed intermediate trees immediately.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RequirementSuccesses {
    /// The environments in which any assignment below this node succeeds.
    environments: MarkerTree,
    branches: Option<RequirementBranches>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RequirementBranches {
    index: usize,
    alternatives: Arc<[RequirementSuccesses]>,
}

type RequirementComparisonCache =
    FxHashMap<(*const RequirementSuccesses, *const RequirementSuccesses), MarkerTree>;

impl RequirementSuccesses {
    fn empty() -> Self {
        Self {
            environments: MarkerTree::FALSE,
            branches: None,
        }
    }

    /// Add a successful partial assignment. Skipped package indices are unconstrained.
    fn record(&mut self, assignment: &[(usize, usize)], env: MarkerTree, counts: &[usize]) {
        if env.is_false()
            || (self.branches.is_none() && env.and(self.environments.negate()).is_false())
        {
            return;
        }
        let Some((&(index, alternative), remaining)) = assignment.split_first() else {
            if self.environments.and(env.negate()).is_false() {
                self.environments = env;
                self.branches = None;
                return;
            }
            if let Some(branches) = &mut self.branches {
                for child in Arc::make_mut(&mut branches.alternatives) {
                    child.record(assignment, env, counts);
                }
            }
            self.environments = self.environments.or(env);
            self.reduce();
            return;
        };

        match &mut self.branches {
            Some(branches) if branches.index < index => {
                for child in Arc::make_mut(&mut branches.alternatives) {
                    child.record(assignment, env, counts);
                }
            }
            Some(branches) if branches.index == index => {
                Arc::make_mut(&mut branches.alternatives)[alternative]
                    .record(remaining, env, counts);
            }
            Some(_) | None => {
                let mut alternatives = vec![self.clone(); counts[index]];
                alternatives[alternative].record(remaining, env, counts);
                self.branches = Some(RequirementBranches {
                    index,
                    alternatives: alternatives.into(),
                });
            }
        }
        self.environments = self.environments.or(env);
        self.reduce();
    }

    fn reduce(&mut self) {
        if let Some(branches) = &self.branches
            && let Some(first) = branches.alternatives.first()
            && branches.alternatives.iter().all(|child| child == first)
        {
            *self = first.clone();
        }
    }

    /// Find environments with a shared assignment without enumerating the assignment product.
    fn common_environments(
        &self,
        other: &Self,
        cache: &mut RequirementComparisonCache,
    ) -> MarkerTree {
        let joint = self.environments.and(other.environments);
        if joint.is_false() || ptr::eq(self, other) {
            return joint;
        }
        let (Some(left), Some(right)) = (&self.branches, &other.branches) else {
            return joint;
        };
        // The trees are immutable for the duration of a comparison. Shared subtrees can therefore
        // use their addresses as cache keys without keeping intermediate trees alive.
        let key = (ptr::from_ref(self), ptr::from_ref(other));
        if let Some(common) = cache.get(&key) {
            return *common;
        }
        let mut common = MarkerTree::FALSE;
        match left.index.cmp(&right.index) {
            Ordering::Less => {
                for child in left.alternatives.iter() {
                    common = common.or(child.common_environments(other, cache));
                    if common == joint {
                        break;
                    }
                }
            }
            Ordering::Equal => {
                for (left, right) in left.alternatives.iter().zip(right.alternatives.iter()) {
                    common = common.or(left.common_environments(right, cache));
                    if common == joint {
                        break;
                    }
                }
            }
            Ordering::Greater => {
                for child in right.alternatives.iter() {
                    common = common.or(self.common_environments(child, cache));
                    if common == joint {
                        break;
                    }
                }
            }
        }
        cache.insert(key, common);
        common
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::str::FromStr;

    use rustc_hash::FxHashMap;

    use uv_normalize::PackageName;
    use uv_pep508::MarkerTree;
    use uv_pypi_types::{ConflictItem, Conflicts, RequirementConflict};

    use super::{RequirementForks, RequirementSuccesses};
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
        assert!(forks[0].assignment.is_empty());
        assert_eq!(classification.partitions.len(), 1);
        assert_eq!(classification.counts, [3]);
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
                "root-a" => fork.assignment.as_ref() == [(0, 0)],
                "root-b" => fork.assignment.as_ref() == [(0, 1)],
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

    #[test]
    fn independent_packages_do_not_expand_the_partition_product() {
        let selection = ConflictItem::from(PackageName::from_str("root").unwrap());
        let allowed = WorkspaceRootConflicts::new(
            [selection.package().clone()].into_iter().collect(),
            (0..32)
                .map(|index| {
                    ["<2", ">=2"]
                        .map(|range| {
                            RequirementConflict::try_from(format!("leaf-{index}{range}")).unwrap()
                        })
                        .into_iter()
                        .collect()
                })
                .collect(),
        );
        let mut classification =
            RequirementForks::new([selection].into_iter().collect(), &allowed).unwrap();
        assert_eq!(classification.partitions.len(), 32);
        assert_eq!(classification.counts, [2; 32]);
        assert!(
            classification
                .partitions
                .values()
                .all(|partitions| partitions.alternatives.len() == 2)
        );
        let fork = classification.forks().next().unwrap();
        assert!(fork.assignment.is_empty());
        assert!(fork.requirements.is_empty());
        let environment = UniversalMarker::new(
            MarkerTree::from_str("sys_platform == 'linux'").unwrap(),
            ConflictMarker::from_conflict_item(&fork.selection),
        );
        classification.record(&fork, environment);
        assert_eq!(
            classification.successes[0].environments,
            environment.combined()
        );
        assert!(classification.successes[0].branches.is_none());
    }

    #[test]
    fn partial_assignments_overlap_across_three_way_partitions() {
        let selections: BTreeSet<_> = ["root-a", "root-b", "root-c"]
            .map(|name| ConflictItem::from(PackageName::from_str(name).unwrap()))
            .into_iter()
            .collect();
        let allowed = WorkspaceRootConflicts::new(
            selections
                .iter()
                .map(|item| item.package().clone())
                .collect(),
            ["first-leaf", "second-leaf"]
                .map(|package| {
                    ["<2", ">=2,<3", ">=3"]
                        .map(|range| {
                            RequirementConflict::try_from(format!("{package}{range}")).unwrap()
                        })
                        .into_iter()
                        .collect()
                })
                .into_iter()
                .collect(),
        );
        let mut classification = RequirementForks::new(selections, &allowed).unwrap();
        let first = PackageName::from_str("first-leaf").unwrap();
        let second = PackageName::from_str("second-leaf").unwrap();
        let roots = classification.forks().collect::<Vec<_>>();
        let first_low = classification.split(&roots[0], &first).unwrap()[0].clone();
        let second_high = classification.split(&roots[1], &second).unwrap()[2].clone();
        let first_high = classification.split(&roots[2], &first).unwrap()[2].clone();
        for fork in [first_low, second_high, first_high] {
            classification.record(
                &fork,
                UniversalMarker::new(
                    MarkerTree::TRUE,
                    ConflictMarker::from_conflict_item(&fork.selection),
                ),
            );
        }
        let conflicts = classification.conflicts(&Conflicts::empty());
        assert_eq!(conflicts.len(), 1);
        assert_eq!(
            conflicts[0]
                .iter()
                .map(|item| item.package().as_str())
                .collect::<Vec<_>>(),
            ["root-a", "root-c"]
        );
    }

    #[test]
    fn partition_overlap_retains_environment_correlations() {
        let selections: BTreeSet<_> = ["root-a", "root-b", "root-c"]
            .map(|name| ConflictItem::from(PackageName::from_str(name).unwrap()))
            .into_iter()
            .collect();
        let allowed = WorkspaceRootConflicts::new(
            selections
                .iter()
                .map(|item| item.package().clone())
                .collect(),
            ["first-leaf", "second-leaf"]
                .map(|package| {
                    ["<2", ">=2,<3", ">=3"]
                        .map(|range| {
                            RequirementConflict::try_from(format!("{package}{range}")).unwrap()
                        })
                        .into_iter()
                        .collect()
                })
                .into_iter()
                .collect(),
        );
        let mut classification = RequirementForks::new(selections, &allowed).unwrap();
        let first = PackageName::from_str("first-leaf").unwrap();
        let second = PackageName::from_str("second-leaf").unwrap();
        let linux = MarkerTree::from_str("sys_platform == 'linux'").unwrap();
        let windows = MarkerTree::from_str("sys_platform == 'win32'").unwrap();
        let roots = classification.forks().collect::<Vec<_>>();
        for (root, first_partition, second_partition, environment) in [
            (0, Some(0), None, linux),
            (0, None, Some(2), windows),
            (1, Some(2), None, linux),
            (1, None, Some(0), windows),
            (2, Some(0), Some(0), linux),
            (2, Some(2), Some(2), windows),
        ] {
            let mut fork = roots[root].clone();
            if let Some(partition) = first_partition {
                fork = classification.split(&fork, &first).unwrap()[partition].clone();
            }
            if let Some(partition) = second_partition {
                fork = classification.split(&fork, &second).unwrap()[partition].clone();
            }
            classification.record(
                &fork,
                UniversalMarker::new(
                    environment,
                    ConflictMarker::from_conflict_item(&fork.selection),
                ),
            );
        }
        for (root, successes) in roots.iter().zip(&classification.successes) {
            assert_eq!(
                successes.environments,
                UniversalMarker::new(
                    linux.or(windows),
                    ConflictMarker::from_conflict_item(&root.selection),
                )
                .combined()
            );
        }
        let conflicts = classification.conflicts(&Conflicts::empty());
        assert_eq!(
            conflicts
                .iter()
                .map(|conflict| conflict
                    .iter()
                    .map(|item| item.package().as_str())
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [["root-a", "root-b"], ["root-b", "root-c"]]
        );
    }

    #[test]
    fn completed_partitions_reduce_to_an_unconstrained_environment() {
        let linux = MarkerTree::from_str("sys_platform == 'linux'").unwrap();
        let windows = MarkerTree::from_str("sys_platform == 'win32'").unwrap();
        let counts = [2, 3, 2];
        let mut successes = RequirementSuccesses::empty();
        successes.record(&[(1, 1)], linux, &counts);
        for index in (0..12).map(|index| index * 5 % 12) {
            let assignment = [(0, index / 6), (1, index / 2 % 3), (2, index % 2)];
            successes.record(&assignment, linux, &counts);
        }
        assert_eq!(successes.environments, linux);
        assert!(successes.branches.is_none());
        successes.record(&[], windows, &counts);
        assert_eq!(successes.environments, linux.or(windows));
        assert!(successes.branches.is_none());
    }

    #[test]
    fn decision_trees_match_an_exhaustive_partition_table() {
        let counts = [2, 3, 2];
        let environments = [
            MarkerTree::from_str("sys_platform == 'linux'").unwrap(),
            MarkerTree::from_str("sys_platform == 'win32'").unwrap(),
            MarkerTree::from_str("python_version < '3.12'").unwrap(),
        ];
        let mut trees = [RequirementSuccesses::empty(), RequirementSuccesses::empty()];
        let mut tables = [[MarkerTree::FALSE; 12]; 2];
        for tree in 0..2 {
            for index in (1_usize..36).map(|index| index * 5 % 36) {
                if (index + tree) % 3 == 0 {
                    continue;
                }
                let choices = [index % 3, index / 3 % 4, index / 12 % 3];
                let assignment = choices
                    .into_iter()
                    .enumerate()
                    .filter_map(|(package, choice)| {
                        choice.checked_sub(1).map(|choice| (package, choice))
                    })
                    .collect::<Vec<_>>();
                let env = environments[(index + tree) % environments.len()];
                trees[tree].record(&assignment, env, &counts);
                for (cell, marker) in tables[tree].iter_mut().enumerate() {
                    let point = [(0, cell / 6), (1, cell / 2 % 3), (2, cell % 2)];
                    if assignment
                        .iter()
                        .all(|(package, choice)| point[*package].1 == *choice)
                    {
                        *marker = marker.or(env);
                    }
                    let mut selected = RequirementSuccesses::empty();
                    selected.record(&point, MarkerTree::TRUE, &counts);
                    assert_eq!(
                        trees[tree].common_environments(&selected, &mut FxHashMap::default()),
                        *marker
                    );
                }
            }
            assert_eq!(
                trees[tree].environments,
                tables[tree]
                    .iter()
                    .fold(MarkerTree::FALSE, |all, marker| all.or(*marker))
            );
        }
        assert_eq!(
            trees[0].common_environments(&trees[1], &mut FxHashMap::default()),
            tables[0]
                .iter()
                .zip(tables[1])
                .fold(MarkerTree::FALSE, |all, (left, right)| all
                    .or(left.and(right)))
        );
    }
}
