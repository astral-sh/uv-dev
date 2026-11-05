use std::collections::BTreeMap;
use std::path::Path;

use itertools::Itertools;
use rustc_hash::FxHashMap;
use tracing::debug;

use uv_client::RegistryClient;
use uv_configuration::{Concurrency, Constraints, DependencyModifiers, HashCheckingMode, Upgrade};
use uv_dispatch::{BuildDispatch, SourceBuildRequirements};
use uv_distribution::DistributionDatabase;
use uv_distribution_types::{DistributionMetadata, Name, Requirement, ResolutionRecorder};
use uv_lock::{BuildEncoding, BuildLockPins, BuildPins};
use uv_pep508::{MarkerEnvironment, MarkerTree};
use uv_pypi_types::Conflicts;
use uv_requirements::LookaheadResolver;
use uv_resolve_operations::Error;
use uv_resolver::{
    BuildCluster, BuildClusters, BuildConstraintHashes, Exclusions, FlatIndex, InMemoryIndex,
    Manifest, Preference, Preferences, PythonRequirement, Resolver, ResolverEnvironment,
    ResolverOutput, RuntimePreferenceMode, UniversalMarker,
};
use uv_static::EnvVars;
use uv_types::{EmptyInstalledPackages, HashStrategy};

use crate::LockError;

#[derive(Clone, Copy)]
pub(crate) enum BuildResolutionStrategy {
    Grouped,
    PerPackage,
}

impl BuildResolutionStrategy {
    pub(crate) fn from_env() -> Result<Self, LockError> {
        match std::env::var(EnvVars::UV_INTERNAL__BUILD_RESOLUTION).as_deref() {
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

pub(crate) fn build_encoding(existing: Option<BuildEncoding>) -> Result<BuildEncoding, LockError> {
    match std::env::var(EnvVars::UV_INTERNAL__BUILD_LOCK_ENCODING).as_deref() {
        Ok("dependency") => Ok(BuildEncoding::Dependency),
        Err(std::env::VarError::NotPresent) => Ok(existing.unwrap_or(BuildEncoding::Dependency)),
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
    upgrade: &Upgrade,
    runtime_hasher: &HashStrategy,
    build_hash_checking: HashCheckingMode,
    root: &Path,
    project_domain: MarkerTree,
    previous: &BuildLockPins,
    prefer_locked: bool,
    concurrency: &Concurrency,
    recorder: Option<&ResolutionRecorder>,
) -> Result<Vec<BuildResolution>, LockError> {
    let mode = match std::env::var(EnvVars::UV_INTERNAL__BUILD_RUNTIME_PREFERENCES).as_deref() {
        Ok("all") | Err(std::env::VarError::NotPresent) => RuntimePreferenceMode::All,
        Ok("first") => RuntimePreferenceMode::First,
        Ok(_) | Err(std::env::VarError::NotUnicode(_)) => {
            return Err(anyhow::anyhow!(
                "Expected `all` or `first` for UV_INTERNAL__BUILD_RUNTIME_PREFERENCES"
            )
            .into());
        }
    };
    let preferences = Preference::from_runtime(runtime, mode);
    let build_hashes =
        BuildConstraintHashes::from_constraints(constraints, build_hash_checking, project_domain)?;
    let constraints = Constraints::from_specifications(
        constraints
            .specifications()
            .cloned()
            .chain(upgrade.constraints().cloned().map(Into::into)),
    );
    let build_recorder = recorder.map(|_| ResolutionRecorder::default());

    let solve = async |cluster: &BuildCluster,
                       scope: UniversalMarker,
                       pins: Option<&BuildPins>,
                       build_hasher: &HashStrategy| {
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
        let requirements = cluster
            .representative()
            .requirements
            .iter()
            .filter(|requirement| !requirement.marker.is_disjoint(active))
            .cloned()
            .collect::<Vec<Requirement>>();

        // Static build requirements are direct inputs to this isolated environment.
        let hasher = match pins {
            Some(pins) => pins.apply_hashes(build_hasher.clone()),
            None => build_hasher.clone(),
        }
        .augment_with_requirements(requirements.iter())?;
        // Keep metadata fetched under different trust policies in separate indexes.
        let index = InMemoryIndex::default();
        // The fetch policy above belongs to these owners. A distribution that needs a
        // metadata build has its own isolated build environment.
        let database = || {
            DistributionDatabase::new(client, dispatch, concurrency.downloads_semaphore.clone())
                .with_recorder(build_recorder.clone())
        };
        let modifiers = DependencyModifiers::default();
        let (lookaheads, hasher) = LookaheadResolver::new(
            &requirements,
            &constraints,
            &modifiers,
            &hasher,
            &index,
            database(),
        )
        .resolve(&environment)
        .await?;
        let locked = pins
            .filter(|_| prefer_locked)
            .into_iter()
            .flat_map(BuildPins::versions)
            .map(|(name, version, index)| {
                Preference::from_build_lock(name.clone(), version.clone(), index.cloned())
            });
        let manifest = Manifest::new(
            requirements,
            constraints.clone(),
            modifiers,
            Preferences::from_iter(locked.chain(preferences.iter().cloned()), &environment),
            None,
            BTreeMap::new(),
            Exclusions::default(),
            lookaheads,
        )
        .with_recorder(build_recorder.clone());
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
        debug!(
            owners = %cluster.owners().map(|root| root.owner.name()).join(", "),
            ?scope,
            "Solving build requirements"
        );
        Ok::<_, Error>((resolver.resolve().await?, hasher))
    };

    let mut builds = BuildClusters::new(matches!(strategy, BuildResolutionStrategy::Grouped));
    let mut seen = FxHashMap::default();
    let mut pending = dispatch
        .discover_build_requirements(runtime, runtime, UniversalMarker::TRUE, runtime_hasher)
        .await?;
    while !pending.is_empty() {
        let mut groups = BTreeMap::new();
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
            for (scope, pins) in
                previous.for_owner(&owner.owner, root, owner.marker, project_domain)?
            {
                for (scope, hasher) in build_hashes.for_scope(scope) {
                    let mut owner = owner.clone();
                    owner.marker = scope;
                    let (owners, _) = groups
                        .entry((scope, pins))
                        .or_insert_with(|| (Vec::new(), hasher));
                    owners.push(owner);
                }
            }
        }

        for ((scope, pins), (mut owners, hasher)) in groups {
            owners.sort_by_cached_key(|owner| {
                (owner.owner.version_id(), owner.owner.index().cloned())
            });
            for cluster in builds.plan(scope, pins, hasher, owners) {
                let (graph, build_hasher) =
                    solve(&cluster, scope, pins, hasher)
                        .await
                        .map_err(|error| {
                            error.with_resolution_context(format!(
                                "`{}`'s build",
                                cluster.representative().owner
                            ))
                        })?;
                debug!(
                    "Resolved build requirements for {}: [{}]",
                    cluster.owners().map(|owner| &owner.owner).format(", "),
                    graph.base_dists().map(|(_, dist)| dist).format(", ")
                );
                pending.extend(
                    dispatch
                        .discover_build_requirements(&graph, runtime, scope, &build_hasher)
                        .await?,
                );
                builds.record(scope, pins, hasher, cluster, graph);
            }
        }
    }
    if let (Some(recorder), Some(build_recorder)) = (recorder, build_recorder) {
        recorder.record_build_lookups(build_recorder.take());
    }
    Ok(builds.into_builds().collect())
}
