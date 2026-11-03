use std::collections::BTreeMap;

use itertools::Itertools;
use rustc_hash::FxHashMap;
use tracing::{debug, trace};

use uv_client::RegistryClient;
use uv_configuration::{Concurrency, Constraints, DependencyModifiers};
use uv_dispatch::{BuildDispatch, SourceBuildRequirements};
use uv_distribution::DistributionDatabase;
use uv_distribution_types::{DistributionMetadata, Requirement};
use uv_lock::BuildEncoding;
use uv_pep508::MarkerEnvironment;
use uv_pypi_types::Conflicts;
use uv_requirements::LookaheadResolver;
use uv_resolve_operations::Error;
use uv_resolver::{
    Exclusions, FlatIndex, InMemoryIndex, Manifest, Preference, Preferences, PythonRequirement,
    ResolveError, Resolver, ResolverEnvironment, ResolverOutput, UniversalMarker,
};
use uv_types::{EmptyInstalledPackages, HashStrategy};

use crate::LockError;

#[derive(Clone, Copy)]
pub(crate) enum BuildResolutionStrategy {
    Grouped,
    PerPackage,
}

impl BuildResolutionStrategy {
    pub(crate) fn from_env() -> Result<Self, LockError> {
        match std::env::var("UV_INTERNAL__BUILD_RESOLUTION").as_deref() {
            Ok("separate-grouped") | Err(std::env::VarError::NotPresent) => Ok(Self::Grouped),
            Ok("separate-per-package") => Ok(Self::PerPackage),
            Ok(_) | Err(std::env::VarError::NotUnicode(_)) => Err(anyhow::anyhow!(
                "Expected `separate-grouped` or `separate-per-package` for UV_INTERNAL__BUILD_RESOLUTION"
            ).into()),
        }
    }
}

/// The direct requirements of each owner identify its roots in the shared build graph.
pub(crate) type BuildResolution = (Vec<SourceBuildRequirements>, ResolverOutput);

pub(crate) fn build_encoding() -> Result<BuildEncoding, LockError> {
    match std::env::var("UV_INTERNAL__BUILD_LOCK_ENCODING").as_deref() {
        Ok("dependency") | Err(std::env::VarError::NotPresent) => Ok(BuildEncoding::Dependency),
        Ok("package") => Ok(BuildEncoding::Package),
        Ok("conflict-marker") => Ok(BuildEncoding::ConflictMarker),
        Ok(_) | Err(std::env::VarError::NotUnicode(_)) => Err(anyhow::anyhow!(
            "Expected `dependency`, `package`, or `conflict-marker` for UV_INTERNAL__BUILD_LOCK_ENCODING"
        ).into()),
    }
}

/// Resolve the source builds reachable from a project and from their own build environments.
pub(crate) async fn resolve_builds(
    runtime: &ResolverOutput,
    strategy: BuildResolutionStrategy,
    python_requirement: &PythonRequirement,
    current_environment: &MarkerEnvironment,
    client: &RegistryClient,
    flat_index: &FlatIndex,
    dispatch: &BuildDispatch<'_>,
    constraints: &Constraints,
    runtime_hasher: &HashStrategy,
    build_hasher: &HashStrategy,
    concurrency: &Concurrency,
) -> Result<Vec<BuildResolution>, LockError> {
    let preferences = Preference::from_runtime(runtime).map_err(anyhow::Error::from)?;

    let solve = async |owners: &[SourceBuildRequirements], scope: UniversalMarker| {
        // Each batch shares a scope. Seed the forks to respect runtime preferences and
        // narrow Requires-Python even when a build requirement has no Python marker.
        let forks = if runtime.fork_markers.is_empty() {
            vec![scope.combined()]
        } else {
            runtime
                .fork_markers
                .iter()
                .map(|fork| fork.combined().and(scope.combined()))
                .filter(|fork| !fork.is_false())
                .collect()
        };
        let environment = ResolverEnvironment::universal(forks);
        let active = scope.pep508().and(runtime.requires_python.to_marker_tree());
        let requirements = owners
            .iter()
            .flat_map(|owner| &owner.requirements)
            .filter(|requirement| !requirement.marker.is_disjoint(active))
            .cloned()
            .unique()
            .collect::<Vec<Requirement>>();

        // Metadata from a source archive can verify its direct URL requirements but
        // cannot authorize a new artifact when hashes are required.
        let hasher = build_hasher
            .clone()
            .augment_with_metadata_requirements(requirements.iter())?;
        // A failed solve can leave pending metadata requests. Give each attempt its own
        // index so an individual retry cannot wait on a cancelled fetch from the group.
        let index = InMemoryIndex::default();
        let database =
            || DistributionDatabase::new(client, dispatch, concurrency.downloads_semaphore.clone());
        let modifiers = DependencyModifiers::default();
        let (lookaheads, hasher) = LookaheadResolver::new(
            &requirements,
            constraints,
            &modifiers,
            &hasher,
            &index,
            database(),
        )
        .resolve(&environment)
        .await?;
        let manifest = Manifest::new(
            requirements,
            constraints.clone(),
            modifiers,
            Preferences::from_iter(preferences.iter().cloned(), &environment),
            None,
            BTreeMap::new(),
            Exclusions::default(),
            lookaheads,
        );
        let resolver = Resolver::new(
            manifest,
            runtime.options.clone(),
            python_requirement,
            environment,
            current_environment,
            Conflicts::empty(),
            None,
            flat_index,
            &index,
            &hasher,
            dispatch,
            EmptyInstalledPackages,
            database(),
        )?;
        Ok::<_, Error>((resolver.resolve().await?, hasher))
    };

    let mut builds = Vec::new();
    let mut seen = FxHashMap::default();
    let mut pending = dispatch
        .fork(runtime_hasher)
        .discover_build_requirements(runtime, runtime, UniversalMarker::TRUE)
        .await?;
    while !pending.is_empty() {
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        for mut owner in std::mem::take(&mut pending) {
            let key = (owner.owner.version_id(), owner.owner.index().cloned());
            let covered = seen.entry(key).or_insert(UniversalMarker::FALSE);
            owner.marker = UniversalMarker::from_combined(
                owner.marker.combined().and(covered.combined().negate()),
            );
            if owner.marker.is_false() {
                continue;
            }
            covered.or(owner.marker);
            groups.entry(owner.marker).or_default().push(owner);
        }

        for (scope, mut owners) in groups {
            owners.sort_by_cached_key(|owner| {
                (owner.owner.version_id(), owner.owner.index().cloned())
            });
            let batches =
                resolve_scope(owners, strategy, async |owners| solve(owners, scope).await)
                    .await
                    .map_err(|error| error.with_resolution_context("build"))?;
            for (owners, (graph, hasher)) in batches {
                debug!(
                    "Resolved build requirements for {}: [{}]",
                    owners.iter().map(|owner| &owner.owner).format(", "),
                    graph.base_dists().map(|(_, dist)| dist).format(", ")
                );
                pending.extend(
                    dispatch
                        .fork(&hasher)
                        .discover_build_requirements(&graph, runtime, scope)
                        .await?,
                );
                builds.push((owners, graph));
            }
        }
    }
    Ok(builds)
}

/// Solve a batch optimistically, then partition incompatible owners within this marker domain.
/// Each accepted cluster has passed a single joint solve, including its transitive requirements.
async fn resolve_scope<T>(
    mut owners: Vec<SourceBuildRequirements>,
    strategy: BuildResolutionStrategy,
    mut solve: impl AsyncFnMut(&[SourceBuildRequirements]) -> Result<T, Error>,
) -> Result<Vec<(Vec<SourceBuildRequirements>, T)>, Error> {
    let mut builds = Vec::new();
    let mut attempts = 0;
    match strategy {
        BuildResolutionStrategy::PerPackage => {
            for owner in owners {
                attempts += 1;
                let graph = solve(std::slice::from_ref(&owner)).await?;
                builds.push((vec![owner], graph));
            }
        }
        BuildResolutionStrategy::Grouped => {
            while !owners.is_empty() {
                let mut accepted = Vec::new();
                let mut graph = None;
                let mut pending = vec![(std::mem::take(&mut owners), None)];
                while let Some((mut candidates, failed_size)) = pending.pop() {
                    let previous = accepted.len();
                    accepted.append(&mut candidates);

                    // When the entire left half fit, adding the right half would repeat the
                    // failed parent solve with precisely the same roots.
                    if Some(accepted.len()) != failed_size {
                        attempts += 1;
                        match solve(&accepted).await {
                            Ok(solution) => {
                                graph = Some(solution);
                                continue;
                            }
                            Err(error) if accepted.len() > 1 && is_group_conflict(&error) => {
                                trace!("Splitting incompatible build requirements: {error}");
                            }
                            Err(error) => return Err(error),
                        }
                    }

                    let failed_size = accepted.len();
                    let mut candidates = accepted.split_off(previous);
                    if candidates.len() == 1 {
                        // Start another cluster with the owners that do not fit this one.
                        owners.append(&mut candidates);
                    } else {
                        let right = candidates.split_off(candidates.len().div_ceil(2));
                        pending.push((right, Some(failed_size)));
                        pending.push((candidates, None));
                    }
                }
                if let Some(graph) = graph {
                    builds.push((accepted, graph));
                }
            }
        }
    }
    debug!(
        "Resolved {} build environments in {attempts} solver runs",
        builds.len()
    );
    Ok(builds)
}

fn is_group_conflict(error: &Error) -> bool {
    error.as_no_solution().is_some()
        || matches!(
            error,
            Error::Resolve(
                ResolveError::ConflictingUrls { .. }
                    | ResolveError::ConflictingIndexesForEnvironment { .. }
                    | ResolveError::ConflictingIndexes(..)
            )
        )
}
