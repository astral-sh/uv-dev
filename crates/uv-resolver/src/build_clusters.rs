use std::collections::BTreeMap;
use std::ptr;

use pubgrub::VersionSet;

use uv_configuration::specifier_opts_into_prereleases;
use uv_distribution_types::{Requirement, RequirementSource};
use uv_pep440::{Operator, Version, VersionSpecifiers};
use uv_resolver_types::{SourceBuildRequirements, UniversalMarker};
use uv_types::HashStrategy;

use crate::pubgrub::Range;

/// Owners whose direct requirements can be solved using one member's declarations.
pub struct BuildCluster {
    representative: SourceBuildRequirements,
    others: Vec<SourceBuildRequirements>,
    by_name: Vec<usize>,
    arbitrary_equality: bool,
}

impl BuildCluster {
    fn new(owner: SourceBuildRequirements) -> Self {
        let mut by_name: Vec<_> = (0..owner.requirements.len()).collect();
        by_name.sort_unstable_by(|&left, &right| {
            owner.requirements[left]
                .name
                .cmp(&owner.requirements[right].name)
        });
        let arbitrary_equality = owner.requirements.iter().any(|requirement| {
            if let RequirementSource::Registry { specifier, .. } = &requirement.source {
                specifier
                    .iter()
                    .any(|specifier| *specifier.operator() == Operator::ExactEqual)
            } else {
                false
            }
        });
        Self {
            representative: owner,
            others: Vec::new(),
            by_name,
            arbitrary_equality,
        }
    }

    pub fn representative(&self) -> &SourceBuildRequirements {
        &self.representative
    }

    pub fn owners(&self) -> impl Iterator<Item = &SourceBuildRequirements> {
        std::iter::once(&self.representative).chain(&self.others)
    }

    fn into_owners(self) -> impl Iterator<Item = SourceBuildRequirements> {
        std::iter::once(self.representative).chain(self.others)
    }

    /// Require the same direct names and the same candidate policy before comparing ranges.
    fn implies(&self, other: &Self) -> bool {
        if self.arbitrary_equality || other.arbitrary_equality {
            return false;
        }
        let requirements = &self.representative.requirements;
        let other_requirements = &other.representative.requirements;
        if requirements.len() != other_requirements.len() {
            return false;
        }
        if requirements
            .iter()
            .zip(other_requirements)
            .all(|(left, right)| same_requirement(left, right))
        {
            return true;
        }
        // Multiple declarations for one name can activate in different environments. Share
        // those only when the declarations agree exactly.
        let unique_names = |requirements: &[Requirement], by_name: &[usize]| {
            by_name
                .windows(2)
                .all(|pair| requirements[pair[0]].name != requirements[pair[1]].name)
        };
        unique_names(requirements, &self.by_name)
            && unique_names(other_requirements, &other.by_name)
            && self
                .by_name
                .iter()
                .zip(&other.by_name)
                .all(|(&left, &right)| {
                    requirement_implies(&requirements[left], &other_requirements[right])
                })
    }
}

fn same_requirement(left: &Requirement, right: &Requirement) -> bool {
    if left != right {
        return false;
    }
    if let (
        RequirementSource::Registry {
            specifier: left, ..
        },
        RequirementSource::Registry {
            specifier: right, ..
        },
    ) = (&left.source, &right.source)
    {
        // PEP 440 version equality ignores trailing zeroes, but prefix matches depend
        // on the number of release segments.
        return left.iter().zip(right.iter()).all(|(left, right)| {
            left.version().release().len() == right.version().release().len()
        });
    }
    true
}

fn requirement_implies(left: &Requirement, right: &Requirement) -> bool {
    if same_requirement(left, right) {
        return true;
    }
    if left.name != right.name
        || left.extras != right.extras
        || left.groups != right.groups
        || left.marker != right.marker
        || left.scope != right.scope
    {
        return false;
    }
    let (
        RequirementSource::Registry {
            specifier: left,
            index: left_index,
            conflict: left_conflict,
        },
        RequirementSource::Registry {
            specifier: right,
            index: right_index,
            conflict: right_conflict,
        },
    ) = (&left.source, &right.source)
    else {
        return false;
    };
    if left_index != right_index
        || left_conflict != right_conflict
        || left.iter().any(specifier_opts_into_prereleases)
            != right.iter().any(specifier_opts_into_prereleases)
        || yanked_pin(left) != yanked_pin(right)
    {
        return false;
    }
    let left = Range::from(left.clone());
    left != Range::empty() && left.subset_of(&Range::from(right.clone()))
}

/// A single exact pin admits yanked versions and can change the available artifacts.
fn yanked_pin(specifiers: &VersionSpecifiers) -> Option<&Version> {
    if let [specifier] = specifiers.as_ref()
        && *specifier.operator() == Operator::Equal
    {
        return Some(specifier.version());
    }
    None
}

/// Plans and retains build graphs for a single lock operation. Only the same marker scope,
/// previous pins and trusted hash policy can reuse a solved representative.
pub struct BuildClusters<'a, P, G> {
    grouped: bool,
    resolved: Vec<(BuildCluster, G)>,
    reusable: BTreeMap<(UniversalMarker, Option<&'a P>), Vec<ReusableBuild<'a>>>,
}

struct ReusableBuild<'a> {
    hasher: &'a HashStrategy,
    index: usize,
}

impl<'a, P: Ord, G> BuildClusters<'a, P, G> {
    pub fn new(grouped: bool) -> Self {
        Self {
            grouped,
            resolved: Vec::new(),
            reusable: BTreeMap::new(),
        }
    }

    pub fn plan(
        &mut self,
        scope: UniversalMarker,
        pins: Option<&'a P>,
        hasher: &'a HashStrategy,
        owners: Vec<SourceBuildRequirements>,
    ) -> Vec<BuildCluster> {
        if !self.grouped {
            return owners.into_iter().map(BuildCluster::new).collect();
        }
        let reusable = self.reusable.get(&(scope, pins)).into_iter().flatten();
        let mut pending: Vec<BuildCluster> = Vec::new();
        for owner in owners {
            let mut candidate = BuildCluster::new(owner);
            if let Some(build) = reusable.clone().find(|build| {
                ptr::eq(build.hasher, hasher) && self.resolved[build.index].0.implies(&candidate)
            }) {
                self.resolved[build.index]
                    .0
                    .others
                    .push(candidate.representative);
            } else if let Some(cluster) = pending
                .iter_mut()
                .find(|cluster| cluster.implies(&candidate))
            {
                cluster.others.push(candidate.representative);
            } else {
                // The pending representatives are incomparable. A stronger declaration can
                // replace any representatives it implies, along with all of their owners.
                candidate.others = pending
                    .extract_if(.., |cluster| candidate.implies(cluster))
                    .flat_map(BuildCluster::into_owners)
                    .collect();
                pending.push(candidate);
            }
        }
        pending
    }

    pub fn record(
        &mut self,
        scope: UniversalMarker,
        pins: Option<&'a P>,
        hasher: &'a HashStrategy,
        cluster: BuildCluster,
        graph: G,
    ) {
        if self.grouped {
            self.reusable
                .entry((scope, pins))
                .or_default()
                .push(ReusableBuild {
                    hasher,
                    index: self.resolved.len(),
                });
        }
        self.resolved.push((cluster, graph));
    }

    pub fn into_builds(self) -> impl Iterator<Item = (Vec<SourceBuildRequirements>, G)> {
        self.resolved
            .into_iter()
            .map(|(cluster, graph)| (cluster.into_owners().collect(), graph))
    }
}
