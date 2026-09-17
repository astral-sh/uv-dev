//! Bounded coordination between independently valid resolver forks.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use pubgrub::{Incompatibility, Term};
use rustc_hash::FxHashSet;
use tracing::debug;

use uv_distribution_types::{Identifier, IndexMetadata, IndexUrl, ResourceId};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pep508::{MarkerTree, RequirementOrigin, VerbatimUrl};
use uv_pypi_types::{ConflictItem, ConflictKindRef, VerbatimParsedUrl};
use uv_redacted::DisplaySafeUrl;
use uv_types::InstalledPackagesProvider;

use crate::ResolutionMode;
use crate::error::ResolveError;
use crate::fork_indexes::ForkIndexes;
use crate::fork_urls::ForkUrls;
use crate::preferences::{PreferenceIndex, PreferenceSource, Preferences};
use crate::pubgrub::{PubGrubPackage, PubGrubPackageInner, Range};
use crate::universal_marker::{ConflictMarker, UniversalMarker};

use super::{
    ForkContinuation, ForkMap, ForkOutcome, ForkState, MetadataRequests, RequirementContext,
    RequirementExpander, Resolution, ResolverState, UnavailableReason, VersionsResponse,
};

/// Coordination is an optimization of an already valid resolution, not an unbounded search for a
/// globally minimal solution.
const MAX_COORDINATION_ATTEMPTS: usize = 64;
const MAX_TRIAL_STEPS: usize = 4096;

type Observations = BTreeSet<Observation>;
type Agreements = BTreeMap<RegistryPackage, Version>;

/// Registry identities are canonicalized without credentials before they enter the ledger.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Source {
    Registry(IndexUrl),
    Url(DisplaySafeUrl),
    Other(ResourceId),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Observation {
    name: PackageName,
    source: Source,
    version: Version,
    marker: UniversalMarker,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RegistryPackage {
    name: PackageName,
    index: IndexUrl,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Proposal {
    target: usize,
    package: RegistryPackage,
    version: Version,
}

struct LiveFork {
    state: ForkState,
    observations: Observations,
}

impl LiveFork {
    fn new(state: ForkState) -> Self {
        let observations = observe(&state);
        Self {
            state,
            observations,
        }
    }
}

struct CompletedFork {
    /// No coordination assumptions are present in this checkpoint. Reusing an accepted trial's
    /// PubGrub state would retain clauses learned from agreements that may since have changed.
    checkpoint: ForkState,
    /// The complete, non-overlapping cover currently accepted for the checkpoint's environment.
    states: Vec<ForkState>,
    observations: Observations,
    agreements: Agreements,
}

impl CompletedFork {
    fn new(state: ForkState) -> Self {
        let observations = observe(&state);
        Self {
            checkpoint: state.clone(),
            states: vec![state],
            observations,
            agreements: Agreements::new(),
        }
    }
}

struct AttemptBudget {
    remaining: usize,
    attempted: BTreeSet<Proposal>,
}

impl AttemptBudget {
    fn new(remaining: usize) -> Self {
        Self {
            remaining,
            attempted: BTreeSet::new(),
        }
    }

    fn claim(&mut self, proposal: Proposal) -> bool {
        if self.remaining == 0 || !self.attempted.insert(proposal) {
            return false;
        }
        self.remaining -= 1;
        true
    }
}

enum CoordinationOutcome {
    NoProposal,
    Rejected,
    Accepted,
}

pub(super) fn solve<InstalledPackages: InstalledPackagesProvider>(
    resolver: &ResolverState<InstalledPackages>,
    initial: Vec<ForkState>,
    requests: &MetadataRequests,
    visited: &mut FxHashSet<PackageName>,
) -> Result<Vec<Resolution>, ResolveError> {
    // Fork construction orders states for a stack. Reversing them preserves that priority while
    // giving every live sibling a turn after each resolver decision.
    let mut live: VecDeque<_> = initial.into_iter().rev().map(LiveFork::new).collect();
    let mut completed = Vec::new();
    let mut budget = AttemptBudget::new(MAX_COORDINATION_ATTEMPTS);

    while let Some(mut fork) = live.pop_front() {
        let external = collect_observations(&completed, &live, None);
        let preferences = preferences_for(resolver, &fork.state, &external);
        // Both sibling decisions and newly fetched index metadata can change which live
        // preferences apply. The normal candidate cache does not include either in its key.
        fork.state.selected_versions.clear();
        let yield_decisions = !live.is_empty() || !completed.is_empty();
        match resolver.solve_fork(
            fork.state,
            &preferences,
            visited,
            requests,
            yield_decisions,
            None,
        )? {
            ForkOutcome::Pending(state) => live.push_back(LiveFork::new(state)),
            ForkOutcome::Split(states) => {
                live.extend(states.into_iter().rev().map(LiveFork::new));
            }
            ForkOutcome::Complete(state) => completed.push(CompletedFork::new(state)),
        }

        coordinate_once(
            resolver,
            &mut completed,
            &live,
            &mut budget,
            visited,
            requests,
        );
    }

    // Process remaining proposals as targeted state continuations, without restarting the solve
    // or repeatedly walking a complete resolution pass.
    loop {
        match coordinate_once(
            resolver,
            &mut completed,
            &live,
            &mut budget,
            visited,
            requests,
        ) {
            CoordinationOutcome::NoProposal => break,
            CoordinationOutcome::Rejected | CoordinationOutcome::Accepted => {}
        }
    }

    Ok(completed
        .into_iter()
        .flat_map(|fork| fork.states)
        .map(ForkState::into_resolution)
        .collect())
}

fn credential_free_index(index: &IndexUrl) -> IndexUrl {
    IndexUrl::from(VerbatimUrl::from_url(
        index.without_credentials().into_owned(),
    ))
}

/// Observe only actual decisions, never a candidate that was rejected while adding dependencies.
fn observe(state: &ForkState) -> Observations {
    let marker = state
        .env
        .try_universal_markers()
        .unwrap_or(UniversalMarker::TRUE);
    state
        .pubgrub
        .partial_solution
        .extract_solution()
        .filter_map(|(package, version)| {
            let PubGrubPackageInner::Package {
                name,
                extra: None,
                group: None,
                marker: MarkerTree::TRUE,
            } = &*state.pubgrub.package_store[package]
            else {
                return None;
            };
            let source = if let Some(url) = state.fork_urls.get(name) {
                let mut url = url.verbatim.to_url();
                url.remove_credentials();
                Source::Url(url)
            } else {
                let dist = state.pins.get(name, &version)?;
                if let Some(index) = dist.index() {
                    Source::Registry(credential_free_index(index))
                } else {
                    Source::Other(dist.resource_id())
                }
            };
            Some(Observation {
                name: name.clone(),
                source,
                version,
                marker,
            })
        })
        .collect()
}

fn collect_observations(
    completed: &[CompletedFork],
    live: &VecDeque<LiveFork>,
    skip_completed: Option<usize>,
) -> Observations {
    completed
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != skip_completed)
        .flat_map(|(_, fork)| fork.observations.iter())
        .chain(live.iter().flat_map(|fork| fork.observations.iter()))
        .cloned()
        .collect()
}

fn preferences_for<InstalledPackages: InstalledPackagesProvider>(
    resolver: &ResolverState<InstalledPackages>,
    state: &ForkState,
    observations: &Observations,
) -> Preferences {
    let mut preferences = resolver.preferences.clone();
    for observation in observations {
        let Source::Registry(index) = &observation.source else {
            continue;
        };
        if resolver.workspace_members.contains(&observation.name)
            || !same_registry_source(resolver, state, &observation.name, index)
        {
            continue;
        }
        preferences.insert(
            observation.name.clone(),
            Some(index.clone()),
            observation.marker,
            observation.version.clone(),
            PreferenceSource::Resolver,
        );
    }
    preferences
}

/// Implicit candidate selection does not filter preferences by index. Only publish a preference
/// when the target has one known registry origin; ambiguous or not-yet-loaded origins wait until a
/// later observation. Explicit index constraints can be compared immediately.
fn same_registry_source<InstalledPackages: InstalledPackagesProvider>(
    resolver: &ResolverState<InstalledPackages>,
    state: &ForkState,
    name: &PackageName,
    expected: &IndexUrl,
) -> bool {
    if state.fork_urls.get(name).is_some() {
        return false;
    }
    if let Some(index) = state.fork_indexes.get(name) {
        return credential_free_index(index.url()) == *expected;
    }
    if resolver.urls.any_url(name) {
        return false;
    }
    let Some(response) = resolver.index.implicit().get(name) else {
        return false;
    };
    let VersionsResponse::Found(version_maps) = &*response else {
        return false;
    };
    !version_maps.is_empty()
        && version_maps.iter().all(|versions| {
            versions
                .index()
                .is_some_and(|index| credential_free_index(index) == *expected)
        })
}

/// Count additional versions of a package from the same source, ignoring repeated observations
/// of the same version in several environments.
fn duplicate_count<'a>(observations: impl IntoIterator<Item = &'a Observation>) -> usize {
    let mut versions = BTreeMap::<_, BTreeSet<_>>::new();
    for observation in observations {
        versions
            .entry((&observation.name, &observation.source))
            .or_default()
            .insert(&observation.version);
    }
    versions
        .into_values()
        .map(|versions| versions.len().saturating_sub(1))
        .sum()
}

fn registry_versions(
    observations: &Observations,
    workspace_members: &BTreeSet<PackageName>,
) -> BTreeMap<RegistryPackage, BTreeSet<Version>> {
    let mut versions = BTreeMap::<_, BTreeSet<_>>::new();
    for observation in observations {
        let Source::Registry(index) = &observation.source else {
            continue;
        };
        if !observation.version.is_stable() || workspace_members.contains(&observation.name) {
            continue;
        }
        versions
            .entry(RegistryPackage {
                name: observation.name.clone(),
                index: index.clone(),
            })
            .or_default()
            .insert(observation.version.clone());
    }
    versions
}

/// Whether the preference's index scope includes the distribution actually selected in a fork.
fn preference_index_matches(
    preference: &PreferenceIndex,
    source: &Source,
    has_explicit_index: bool,
) -> bool {
    match source {
        Source::Url(_) => false,
        Source::Registry(index) => match preference {
            PreferenceIndex::Any => true,
            PreferenceIndex::Implicit => !has_explicit_index,
            PreferenceIndex::Explicit(preferred) => credential_free_index(preferred) == *index,
        },
        Source::Other(_) => match preference {
            PreferenceIndex::Any => true,
            PreferenceIndex::Implicit => !has_explicit_index,
            PreferenceIndex::Explicit(_) => false,
        },
    }
}

fn has_selected_preference(
    preferences: &Preferences,
    observation: &Observation,
    has_explicit_index: bool,
) -> bool {
    preferences.get(&observation.name).iter().any(|entry| {
        let external = match entry.source() {
            PreferenceSource::Environment
            | PreferenceSource::Lock
            | PreferenceSource::RequirementsTxt => true,
            PreferenceSource::Resolver => false,
        };
        external
            && entry.pin().version() == &observation.version
            && !entry.marker().is_disjoint(observation.marker)
            && preference_index_matches(entry.index(), &observation.source, has_explicit_index)
    })
}

/// Preserve only preferences that remain selected, so obsolete lockfile pins do not prevent a
/// resolution from adopting a version required by its current dependencies.
fn selected_preferences(preferences: &Preferences, states: &[ForkState]) -> Observations {
    let mut protected = Observations::new();
    for state in states {
        for observation in observe(state) {
            let has_explicit_index = state.fork_indexes.get(&observation.name).is_some();
            if has_selected_preference(preferences, &observation, has_explicit_index) {
                protected.insert(observation);
            }
        }
    }
    protected
}

fn proposal_changes_preference(protected: &Observations, proposal: &Proposal) -> bool {
    protected.iter().any(|observation| {
        if observation.name != proposal.package.name {
            return false;
        }
        let same_source = match &observation.source {
            Source::Registry(index) => *index == proposal.package.index,
            Source::Url(_) | Source::Other(_) => false,
        };
        !same_source || observation.version != proposal.version
    })
}

/// A changed parent may make a package unnecessary, but cannot change an otherwise-valid selected
/// preference in an overlapping environment merely to improve consistency between forks.
fn preferences_preserved(protected: &Observations, observations: &Observations) -> bool {
    protected.iter().all(|preferred| {
        observations
            .iter()
            .filter(|observation| {
                observation.name == preferred.name
                    && !observation.marker.is_disjoint(preferred.marker)
            })
            .all(|observation| {
                observation.source == preferred.source && observation.version == preferred.version
            })
    })
}

fn next_proposal<InstalledPackages: InstalledPackagesProvider>(
    resolver: &ResolverState<InstalledPackages>,
    completed: &[CompletedFork],
    live: &VecDeque<LiveFork>,
    budget: &AttemptBudget,
) -> Option<Proposal> {
    if budget.remaining == 0 {
        return None;
    }
    for (target, fork) in completed.iter().enumerate() {
        let current = registry_versions(&fork.observations, &resolver.workspace_members);
        let protected = selected_preferences(&resolver.preferences, &fork.states);
        let external = registry_versions(
            &collect_observations(completed, live, Some(target)),
            &resolver.workspace_members,
        );
        for (package, current_versions) in current {
            let Some(candidates) = external.get(&package) else {
                continue;
            };
            let mut candidates: Vec<_> = candidates.iter().collect();
            match resolver.options.resolution_mode {
                ResolutionMode::Highest => candidates.reverse(),
                ResolutionMode::Lowest => {}
                ResolutionMode::LowestDirect => return None,
            }
            for candidate in candidates {
                if current_versions.iter().all(|version| version == candidate) {
                    continue;
                }
                let proposal = Proposal {
                    target,
                    package: package.clone(),
                    version: candidate.clone(),
                };
                if proposal_changes_preference(&protected, &proposal) {
                    continue;
                }
                if !budget.attempted.contains(&proposal) {
                    return Some(proposal);
                }
            }
        }
    }
    None
}

fn with_agreement(agreements: &Agreements, proposal: &Proposal) -> Agreements {
    let mut agreements = agreements.clone();
    agreements.retain(|package, _| package.name != proposal.package.name);
    agreements.insert(proposal.package.clone(), proposal.version.clone());
    agreements
}

fn coordinate_once<InstalledPackages: InstalledPackagesProvider>(
    resolver: &ResolverState<InstalledPackages>,
    completed: &mut [CompletedFork],
    live: &VecDeque<LiveFork>,
    budget: &mut AttemptBudget,
    visited: &mut FxHashSet<PackageName>,
    requests: &MetadataRequests,
) -> CoordinationOutcome {
    let Some(proposal) = next_proposal(resolver, completed, live, budget) else {
        return CoordinationOutcome::NoProposal;
    };
    if !budget.claim(proposal.clone()) {
        return CoordinationOutcome::NoProposal;
    }

    let external = collect_observations(completed, live, Some(proposal.target));
    let target = &completed[proposal.target];
    let before = duplicate_count(external.iter().chain(target.observations.iter()));
    let protected = selected_preferences(&resolver.preferences, &target.states);
    let agreements = with_agreement(&target.agreements, &proposal);
    debug!(
        "Trying coordinated backtracking for {}=={} from {} in {}",
        proposal.package.name, proposal.version, proposal.package.index, target.checkpoint.env
    );
    let Some(states) = solve_trial(
        resolver,
        &target.checkpoint,
        &agreements,
        &external,
        visited,
        requests,
    ) else {
        return CoordinationOutcome::Rejected;
    };
    if !sources_preserved(&target.checkpoint, &states) {
        debug!("Rejected coordinated backtracking after a cached package's source changed");
        return CoordinationOutcome::Rejected;
    }
    let observations: Observations = states.iter().flat_map(observe).collect();
    if !preferences_preserved(&protected, &observations) {
        debug!("Rejected coordinated backtracking after a selected external preference changed");
        return CoordinationOutcome::Rejected;
    }
    if !agreements_satisfied(&agreements, &observations) {
        debug!("Rejected coordinated backtracking after its registry source changed");
        return CoordinationOutcome::Rejected;
    }
    let after = duplicate_count(external.iter().chain(observations.iter()));
    if after >= before {
        debug!("Rejected coordinated backtracking: duplicate count {before} -> {after}");
        return CoordinationOutcome::Rejected;
    }

    debug!("Accepted coordinated backtracking: duplicate count {before} -> {after}");
    let target = &mut completed[proposal.target];
    target.states = states;
    target.observations = observations;
    target.agreements = agreements;
    CoordinationOutcome::Accepted
}

fn agreements_satisfied(agreements: &Agreements, observations: &Observations) -> bool {
    agreements.iter().all(|(package, version)| {
        observations
            .iter()
            .filter(|observation| observation.name == package.name)
            .all(|observation| {
                observation.source == Source::Registry(package.index.clone())
                    && observation.version == *version
            })
    })
}

/// Metadata pins and dependency incompatibilities are keyed by package and version, not by
/// source. They remain valid only while each already-cached package has the same source mapping.
fn sources_preserved(checkpoint: &ForkState, states: &[ForkState]) -> bool {
    let names: BTreeSet<_> = checkpoint
        .pins
        .names()
        .cloned()
        .chain(
            checkpoint
                .added_dependencies
                .keys()
                .filter_map(|package| checkpoint.pubgrub.package_store[*package].name().cloned()),
        )
        .collect();
    states.iter().all(|state| {
        cached_sources_match(
            &names,
            &checkpoint.fork_urls,
            &checkpoint.fork_indexes,
            &state.fork_urls,
            &state.fork_indexes,
        )
    })
}

fn cached_sources_match(
    names: &BTreeSet<PackageName>,
    previous_urls: &ForkUrls,
    previous_indexes: &ForkIndexes,
    current_urls: &ForkUrls,
    current_indexes: &ForkIndexes,
) -> bool {
    names.iter().all(|name| {
        previous_urls.get(name) == current_urls.get(name)
            && previous_indexes.get(name) == current_indexes.get(name)
    })
}

/// Source maps survive PubGrub backtracking, so equality with the checkpoint does not prove that
/// a URL or index is still required. Only immutable, environment-covering manifest policy may
/// justify a source-bearing continuation. Parent-introduced source transitions are unsupported.
fn sources_fixed_by_manifest<InstalledPackages: InstalledPackagesProvider>(
    resolver: &ResolverState<InstalledPackages>,
    state: &ForkState,
) -> bool {
    if state.fork_urls.iter().next().is_none() && state.fork_indexes.iter().next().is_none() {
        return true;
    }
    let mut root_urls: ForkMap<VerbatimParsedUrl> = ForkMap::default();
    let expander = RequirementExpander::new(
        &resolver.constraints,
        &resolver.overrides,
        &resolver.excludes,
        &state.env,
        &state.python_requirement,
    );
    for requirement in expander.expand(&resolver.requirements, RequirementContext::Root) {
        // Workspace members have immutable root requirements. Other regular URLs are only an
        // allow-list: their introducing dependency may disappear during backtracking.
        if !resolver.workspace_members.contains(&requirement.name)
            && resolver.project.as_ref() != Some(&requirement.name)
        {
            continue;
        }
        // An override is authoritative only within its own scope. Do not widen that scope by
        // canonicalizing an unconditional root requirement through a partially active override.
        if resolver.urls.has_overrides(&requirement.name)
            || !requirement.groups.is_empty()
            || matches!(
                requirement.origin.as_ref(),
                Some(RequirementOrigin::Group(..))
            )
        {
            continue;
        }
        if resolver
            .conflicts
            .contains(&requirement.name, ConflictKindRef::Project)
        {
            let excluded = UniversalMarker::new(
                MarkerTree::TRUE,
                ConflictMarker::from_conflict_item(&ConflictItem::from(requirement.name.clone()))
                    .negate(),
            );
            if state
                .env
                .try_universal_markers()
                .is_none_or(|environment| !environment.is_disjoint(excluded))
            {
                continue;
            }
        }
        let Some(url) = requirement.source.to_verbatim_parsed_url() else {
            continue;
        };
        let Ok(urls) =
            resolver
                .urls
                .get_url(&state.env, &requirement.name, Some(&url), &resolver.git)
        else {
            return false;
        };
        for url in urls {
            root_urls.add(requirement.as_ref(), url.clone());
        }
    }
    sources_fixed_by_policy(
        &state.fork_urls,
        &state.fork_indexes,
        |name, url| {
            resolver.urls.is_fixed_override(name, &state.env, url)
                || root_urls.is_fixed(name, &state.env, url)
        },
        |name, index| resolver.indexes.is_fixed(name, &state.env, index),
    )
}

fn sources_fixed_by_policy(
    urls: &ForkUrls,
    indexes: &ForkIndexes,
    mut fixed_url: impl FnMut(&PackageName, &VerbatimParsedUrl) -> bool,
    mut fixed_index: impl FnMut(&PackageName, &IndexMetadata) -> bool,
) -> bool {
    urls.iter().all(|(name, url)| fixed_url(name, url))
        && indexes.iter().all(|(name, index)| fixed_index(name, index))
}

/// Add conditional version restrictions: a package may disappear if its parent is backtracked,
/// but if it remains selected, it must have the agreed version. Unary incompatibilities do not
/// introduce synthetic dependency edges into the final resolution graph.
fn prepare_trial(
    checkpoint: &ForkState,
    agreements: &Agreements,
    remaining_steps: &mut usize,
) -> Option<ForkState> {
    let mut state = checkpoint.clone();
    let packages: Vec<_> = agreements
        .iter()
        .map(|(package, version)| {
            let package = state
                .pubgrub
                .package_store
                .alloc(PubGrubPackage::from_package(
                    package.name.clone(),
                    None,
                    None,
                    MarkerTree::TRUE,
                ));
            (package, version.clone())
        })
        .collect();

    // The extracted solution is in decision order. Rewind before the earliest decision that
    // differs, then let propagation discover any earlier parents that also need to change.
    let backtrack =
        state
            .pubgrub
            .partial_solution
            .extract_solution()
            .find_map(|(package, selected)| {
                packages
                    .iter()
                    .any(|(target, version)| *target == package && *version != selected)
                    .then_some(package)
            });
    if let Some(package) = backtrack {
        state.pubgrub.backtrack_package(package);
    }
    for (package, version) in &packages {
        state
            .pubgrub
            .add_incompatibility(Incompatibility::custom_term(
                *package,
                Term::Positive(Range::singleton(version.clone()).complement()),
                UnavailableReason::Coordinated(version.clone()),
            ));
    }
    state.selected_versions.clear();
    state.continuation = ForkContinuation::Propagate;
    state.started_at = None;

    // Propagating a later agreement can backtrack derivations introduced by an earlier one. Run
    // all of them again after any such backjump, with the same finite trial budget.
    loop {
        let decisions = state.pubgrub.partial_solution.extract_solution().count();
        for (package, _) in &packages {
            if *remaining_steps == 0 {
                return None;
            }
            *remaining_steps -= 1;
            state.next = *package;
            let conflicts = state.pubgrub.unit_propagation(*package).ok()?;
            for (affected, incompatibility) in conflicts {
                state.record_conflict(affected, None, incompatibility);
            }
        }
        if state.pubgrub.partial_solution.extract_solution().count() == decisions {
            break;
        }
    }
    Some(state)
}

fn solve_trial<InstalledPackages: InstalledPackagesProvider>(
    resolver: &ResolverState<InstalledPackages>,
    checkpoint: &ForkState,
    agreements: &Agreements,
    external: &Observations,
    visited: &mut FxHashSet<PackageName>,
    requests: &MetadataRequests,
) -> Option<Vec<ForkState>> {
    let trial_requests = requests.speculative();
    let mut remaining_steps = MAX_TRIAL_STEPS;
    let Some(state) = prepare_trial(checkpoint, agreements, &mut remaining_steps) else {
        debug!("Abandoned coordinated backtracking while propagating its agreements");
        return None;
    };
    let mut pending = vec![state];
    let mut completed = Vec::new();
    let mut observations = external.clone();
    while let Some(mut state) = pending.pop() {
        if !sources_fixed_by_manifest(resolver, &state) {
            debug!("Abandoned coordinated backtracking without fixed manifest source policy");
            return None;
        }
        if remaining_steps == 0 {
            debug!("Abandoned coordinated backtracking after its iteration budget was exhausted");
            return None;
        }
        let preferences = preferences_for(resolver, &state, &observations);
        state.selected_versions.clear();
        match resolver.solve_fork(
            state,
            &preferences,
            visited,
            &trial_requests,
            false,
            Some(&mut remaining_steps),
        ) {
            Ok(ForkOutcome::Complete(state)) => {
                if !sources_fixed_by_manifest(resolver, &state) {
                    debug!(
                        "Abandoned coordinated backtracking after an unsupported source transition"
                    );
                    return None;
                }
                debug!("Completed coordinated backtracking fork for {}", state.env);
                observations.extend(observe(&state));
                completed.push(state);
            }
            Ok(ForkOutcome::Split(states)) => {
                debug!("Split coordinated backtracking into {} forks", states.len());
                pending.extend(states);
            }
            Ok(ForkOutcome::Pending(_)) => {
                debug!(
                    "Abandoned coordinated backtracking after its iteration budget was exhausted"
                );
                return None;
            }
            Err(error) => {
                // These failures belong to a speculative, disposable state. They are not learned
                // as package unavailability, and do not replace the known-valid environment.
                if matches!(&error, ResolveError::NoSolution(_)) {
                    debug!(
                        "Abandoned coordinated backtracking after an unsatisfiable fork: {error}"
                    );
                } else {
                    debug!("Abandoned coordinated backtracking: {error}");
                }
                return None;
            }
        }
    }
    (!completed.is_empty()).then_some(completed)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;

    use uv_distribution_types::{IndexMetadata, IndexUrl};
    use uv_normalize::{ExtraName, PackageName};
    use uv_pep440::Version;
    use uv_pep508::{MarkerTree, Pep508Url};
    use uv_pypi_types::{ConflictItem, VerbatimParsedUrl};

    use crate::ResolverEnvironment;
    use crate::fork_indexes::ForkIndexes;
    use crate::fork_urls::ForkUrls;
    use crate::preferences::{PreferenceIndex, PreferenceSource, Preferences};
    use crate::universal_marker::{ConflictMarker, UniversalMarker};

    use super::{
        Agreements, AttemptBudget, Observation, Observations, Proposal, RegistryPackage, Source,
        cached_sources_match, credential_free_index, duplicate_count, has_selected_preference,
        preference_index_matches, preferences_preserved, proposal_changes_preference,
        sources_fixed_by_policy, with_agreement,
    };

    fn observation(name: &str, index: &str, version: u64) -> Result<Observation, Box<dyn Error>> {
        Ok(Observation {
            name: name.parse()?,
            source: Source::Registry(credential_free_index(&IndexUrl::parse(index, None)?)),
            version: Version::new([version]),
            marker: UniversalMarker::TRUE,
        })
    }

    fn proposal(version: u64) -> Result<Proposal, Box<dyn Error>> {
        Ok(Proposal {
            target: 0,
            package: RegistryPackage {
                name: "example".parse()?,
                index: IndexUrl::parse("https://pypi.org/simple", None)?,
            },
            version: Version::new([version]),
        })
    }

    #[test]
    fn registry_identity_ignores_credentials() -> Result<(), Box<dyn Error>> {
        let authenticated = IndexUrl::parse("https://username:password@pypi.org/simple", None)?;
        let anonymous = IndexUrl::parse("https://pypi.org/simple", None)?;
        assert_eq!(
            credential_free_index(&authenticated),
            credential_free_index(&anonymous)
        );
        Ok(())
    }

    #[test]
    fn duplicates_are_counted_per_source() -> Result<(), Box<dyn Error>> {
        let first = observation("example", "https://pypi.org/simple", 1)?;
        let repeated = first.clone();
        let second = observation("example", "https://pypi.org/simple", 2)?;
        let other_index = observation("example", "https://example.org/simple", 3)?;
        assert_eq!(
            duplicate_count([&first, &repeated, &second, &other_index]),
            1
        );
        assert_eq!(duplicate_count([&first, &other_index]), 0);
        Ok(())
    }

    #[test]
    fn selected_preferences_match_version_source_and_full_marker() -> Result<(), Box<dyn Error>> {
        let item = ConflictItem::from((
            "project".parse::<PackageName>()?,
            "feature".parse::<ExtraName>()?,
        ));
        let conflict = ConflictMarker::from_conflict_item(&item);
        let included = UniversalMarker::new(MarkerTree::TRUE, conflict);
        let excluded = UniversalMarker::new(MarkerTree::TRUE, conflict.negate());
        let mut selected = observation("example", "https://pypi.org/simple", 1)?;
        selected.marker = included;

        for source in [
            PreferenceSource::Environment,
            PreferenceSource::Lock,
            PreferenceSource::RequirementsTxt,
        ] {
            let mut preferences = Preferences::default();
            preferences.insert(
                selected.name.clone(),
                Some(IndexUrl::parse(
                    "https://username:password@pypi.org/simple",
                    None,
                )?),
                included,
                selected.version.clone(),
                source,
            );
            assert!(has_selected_preference(&preferences, &selected, true));

            let mut different = selected.clone();
            different.marker = excluded;
            assert!(!has_selected_preference(&preferences, &different, true));
            different.marker = included;
            different.version = Version::new([2]);
            assert!(!has_selected_preference(&preferences, &different, true));
            different.version = selected.version.clone();
            different.source =
                Source::Registry(IndexUrl::parse("https://example.org/simple", None)?);
            assert!(!has_selected_preference(&preferences, &different, true));
        }

        let mut preferences = Preferences::default();
        preferences.insert(
            selected.name.clone(),
            Some(IndexUrl::parse("https://pypi.org/simple", None)?),
            included,
            selected.version.clone(),
            PreferenceSource::Resolver,
        );
        assert!(!has_selected_preference(&preferences, &selected, true));
        assert!(preference_index_matches(
            &PreferenceIndex::Any,
            &selected.source,
            true,
        ));
        assert!(preference_index_matches(
            &PreferenceIndex::Implicit,
            &selected.source,
            false,
        ));
        assert!(!preference_index_matches(
            &PreferenceIndex::Implicit,
            &selected.source,
            true,
        ));
        Ok(())
    }

    #[test]
    fn selected_preferences_allow_removal_but_not_overlapping_changes() -> Result<(), Box<dyn Error>>
    {
        let mut selected = observation("example", "https://pypi.org/simple", 1)?;
        selected.marker = UniversalMarker::from_combined("python_version < '3.12'".parse()?);
        let protected = Observations::from([selected.clone()]);
        assert!(!proposal_changes_preference(&protected, &proposal(1)?));
        assert!(proposal_changes_preference(&protected, &proposal(2)?));
        assert!(preferences_preserved(&protected, &Observations::new()));
        assert!(preferences_preserved(&protected, &protected));

        let mut changed = selected;
        changed.version = Version::new([2]);
        assert!(!preferences_preserved(
            &protected,
            &Observations::from([changed.clone()]),
        ));
        changed.marker = UniversalMarker::from_combined("python_version >= '3.12'".parse()?);
        assert!(preferences_preserved(
            &protected,
            &Observations::from([changed]),
        ));
        Ok(())
    }

    #[test]
    fn cached_sources_reject_changes_but_allow_uncached_names() -> Result<(), Box<dyn Error>> {
        let cached: PackageName = "cached".parse()?;
        let uncached: PackageName = "uncached".parse()?;
        let names = BTreeSet::from([cached.clone()]);
        let env = ResolverEnvironment::universal(Vec::new());
        let previous_urls = ForkUrls::default();
        let previous_indexes = ForkIndexes::default();
        let url = VerbatimParsedUrl::parse_url(
            "https://example.org/package-1.0.0-py3-none-any.whl",
            None,
        )?;
        let index = IndexMetadata::from(IndexUrl::parse("https://example.org/simple", None)?);

        let mut current_urls = ForkUrls::default();
        current_urls.insert(&uncached, &url, &env)?;
        assert!(cached_sources_match(
            &names,
            &previous_urls,
            &previous_indexes,
            &current_urls,
            &previous_indexes,
        ));
        current_urls.insert(&cached, &url, &env)?;
        assert!(!cached_sources_match(
            &names,
            &previous_urls,
            &previous_indexes,
            &current_urls,
            &previous_indexes,
        ));

        let mut current_indexes = ForkIndexes::default();
        current_indexes.insert(&uncached, &index, &env)?;
        assert!(cached_sources_match(
            &names,
            &previous_urls,
            &previous_indexes,
            &previous_urls,
            &current_indexes,
        ));
        current_indexes.insert(&cached, &index, &env)?;
        assert!(!cached_sources_match(
            &names,
            &previous_urls,
            &previous_indexes,
            &previous_urls,
            &current_indexes,
        ));
        Ok(())
    }

    #[test]
    fn unchanged_source_maps_still_require_fixed_policy() -> Result<(), Box<dyn Error>> {
        let package: PackageName = "cached".parse()?;
        let names = BTreeSet::from([package.clone()]);
        let env = ResolverEnvironment::universal(Vec::new());
        let url = VerbatimParsedUrl::parse_url(
            "https://example.org/package-1.0.0-py3-none-any.whl",
            None,
        )?;
        let index = IndexMetadata::from(IndexUrl::parse("https://example.org/simple", None)?);
        let mut urls = ForkUrls::default();
        let mut indexes = ForkIndexes::default();

        // Ordinary registry packages introduce no append-only source mapping.
        assert!(sources_fixed_by_policy(
            &urls,
            &indexes,
            |_, _| false,
            |_, _| false,
        ));

        urls.insert(&package, &url, &env)?;
        assert!(cached_sources_match(
            &names, &urls, &indexes, &urls, &indexes,
        ));
        assert!(!sources_fixed_by_policy(
            &urls,
            &indexes,
            |_, _| false,
            |_, _| true,
        ));
        assert!(sources_fixed_by_policy(
            &urls,
            &indexes,
            |name, source| name == &package && source == &url,
            |_, _| false,
        ));

        indexes.insert(&package, &index, &env)?;
        assert!(!sources_fixed_by_policy(
            &urls,
            &indexes,
            |_, _| true,
            |_, _| false,
        ));
        assert!(sources_fixed_by_policy(
            &urls,
            &indexes,
            |name, source| name == &package && source == &url,
            |name, source| name == &package && source == &index,
        ));
        Ok(())
    }

    #[test]
    fn replacing_an_agreement_keeps_the_checkpoint_unmodified() -> Result<(), Box<dyn Error>> {
        let first = proposal(1)?;
        let second = proposal(2)?;
        let original = with_agreement(&Agreements::new(), &first);
        let replacement = with_agreement(&original, &second);
        assert_eq!(original.get(&first.package), Some(&first.version));
        assert_eq!(replacement.get(&second.package), Some(&second.version));
        assert_eq!(replacement.len(), 1);
        Ok(())
    }

    #[test]
    fn attempted_proposals_do_not_repeat_or_exceed_the_budget() -> Result<(), Box<dyn Error>> {
        let mut budget = AttemptBudget::new(2);
        assert!(budget.claim(proposal(1)?));
        assert!(!budget.claim(proposal(1)?));
        assert!(budget.claim(proposal(2)?));
        assert!(!budget.claim(proposal(3)?));
        assert_eq!(budget.remaining, 0);
        Ok(())
    }
}
