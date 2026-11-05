use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};
use std::thread;

use itertools::Itertools;
use rustc_hash::FxHashMap;
use tracing::debug;

use uv_distribution_types::{DistributionId, Identifier, IndexUrl, Name, Requirement};
use uv_pep440::Version;
use uv_resolver_types::{
    BuildResolution, ResolverOutput, SourceBuildRequirements, UniversalMarker,
};
use uv_static::EnvVars;
use uv_types::{BuildContext, HashStrategy, InstalledPackagesProvider};

use super::{
    DefaultResolverProvider, InMemoryIndex, ReporterGuard, Resolver, ResolverEnvironment,
    ResolverState, RootFork,
};
use crate::build_clusters::{BuildCluster, BuildClusters};
use crate::preferences::{Preference, Preferences, RuntimePreferenceMode};
use crate::{AllowedYanks, BuildConstraintHashes, Manifest, ResolveError};

/// Which source packages can share an isolated build resolution.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BuildForkStrategy {
    Clustered,
    PerPackage,
}

impl BuildForkStrategy {
    /// Select the strategy used for experimental build dependency locking.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var(EnvVars::UV_INTERNAL__BUILD_FORK_STRATEGY) {
            Ok(value) if value == "per-package" => Ok(Self::PerPackage),
            Ok(value) if value == "clustered" => Ok(Self::Clustered),
            Err(std::env::VarError::NotPresent) => Ok(Self::Clustered),
            Ok(value) => Err(format!(
                "Unknown build fork strategy `{value}` (expected `clustered` or `per-package`)"
            )),
            Err(error) => Err(error.to_string()),
        }
    }
}

impl RuntimePreferenceMode {
    /// Select the runtime preferences used for experimental build dependency locking.
    pub fn from_env() -> Result<Self, &'static str> {
        match std::env::var(EnvVars::UV_INTERNAL__BUILD_RUNTIME_PREFERENCES).as_deref() {
            Ok("all") | Err(std::env::VarError::NotPresent) => Ok(Self::All),
            Ok("first") => Ok(Self::First),
            Ok(_) | Err(std::env::VarError::NotUnicode(_)) => {
                Err("Expected `all` or `first` for UV_INTERNAL__BUILD_RUNTIME_PREFERENCES")
            }
        }
    }
}

impl<Context: BuildContext, InstalledPackages: InstalledPackagesProvider>
    Resolver<DefaultResolverProvider<'_, Context>, InstalledPackages>
{
    /// Solve runtime and build dependencies on sibling branches of one PubGrub root.
    /// Discovery sees the original runtime graph at every depth for `match-runtime`.
    /// Owners can share a build fork when one declaration covers the others in the same
    /// marker, hash and previous-lock context.
    pub async fn resolve_with_build_forks<'pins, P: Ord + 'pins, E: From<ResolveError>>(
        self,
        strategy: BuildForkStrategy,
        runtime_preferences: RuntimePreferenceMode,
        hashes: &BuildConstraintHashes,
        prepare: impl AsyncFn(
            Vec<Requirement>,
            &ResolverEnvironment,
            &InMemoryIndex,
            &HashStrategy,
            Option<&P>,
        ) -> Result<(Manifest, HashStrategy, Vec<Preference>), E>,
        mut discover: impl AsyncFnMut(
            &ResolverOutput,
            &ResolverOutput,
            UniversalMarker,
            &HashStrategy,
        )
            -> Result<Vec<(SourceBuildRequirements, Option<&'pins P>)>, E>,
    ) -> Result<ResolverOutput, E> {
        let state = Arc::new(self.state);
        let _reporter = ReporterGuard(state.reporter.as_deref());
        let mut provider = self.provider;
        let (sender, receiver) = mpsc::channel::<RootFork<InstalledPackages>>();
        let root = state.root();
        thread::Builder::new()
            .name("uv-resolver".into())
            .spawn(move || {
                // The checkpoint precedes the first runtime decision. Even packages selected at
                // decision level zero in the runtime universe can differ in a build environment.
                for fork in receiver {
                    fork.solve(root.clone());
                }
            })
            .map_err(ResolveError::SpawnThread)?;
        let schedule = |fork| sender.send(fork).map_err(|_| ResolveError::ChannelClosed);

        let mut runtime = state.run(&provider, schedule).await?;
        let runtime_preferences = Preference::from_runtime(&runtime, runtime_preferences);
        let preferences = Preferences::from_iter(runtime_preferences.iter().cloned(), &state.env);
        let roots = discover(&runtime, &runtime, UniversalMarker::TRUE, &state.hasher).await?;
        let mut pending = PendingBuilds {
            covered: FxHashMap::default(),
            groups: Vec::new(),
            hashes,
        };
        pending.enqueue(roots, strategy, &runtime.fork_markers);
        let mut builds = BuildClusters::new(strategy == BuildForkStrategy::Clustered);

        let mut solve = async |cluster: &BuildCluster, pins: Option<&P>, hasher: &HashStrategy| {
            let owner = cluster.representative();
            let scope = owner.marker;
            let env = ResolverEnvironment::universal(vec![scope.combined()]);
            // A URL can be trusted by one root but carry different hashes in another.
            let index = InMemoryIndex::default();
            let active = hashes.active_domain(scope);
            let requirements = owner
                .requirements
                .iter()
                .filter(|requirement| !requirement.marker.is_disjoint(active))
                .cloned()
                .collect();
            let (mut manifest, hasher, locked) =
                prepare(requirements, &env, &index, hasher, pins).await?;
            provider.set_hasher(hasher.clone());

            manifest.preferences = if locked.is_empty() {
                preferences.clone()
            } else {
                Preferences::from_iter(
                    locked
                        .into_iter()
                        .chain(runtime_preferences.iter().cloned()),
                    &env,
                )
            };
            provider.set_allowed_yanks(AllowedYanks::from_manifest(
                &manifest,
                &env,
                state.dependency_mode,
            ));
            let mut build = ResolverState::new(
                manifest,
                state.options.clone(),
                &hasher,
                env,
                &state.current_environment,
                state.tags.clone(),
                &state.python_requirement,
                state.conflicts.clone(),
                &index,
                &state.git,
                &state.capabilities,
                &state.locations,
                state.installed_packages.clone(),
            );
            build.reporter = state.reporter.clone();
            debug!(
                owners = %cluster.owners().map(|root| root.owner.name()).join(", "),
                ?scope,
                "Solving build requirements"
            );
            let graph = Arc::new(build).run(&provider, schedule).await;
            Ok::<_, E>(graph.map(|graph| (graph, hasher)))
        };
        while let Some(BuildGroup {
            scope,
            pins,
            owners,
            hasher,
        }) = pending.groups.pop()
        {
            for cluster in builds.plan(scope, pins, hasher, owners) {
                let (graph, build_hasher) = solve(&cluster, pins, hasher)
                    .await?
                    .map_err(|error| build_error(cluster.owners(), error))?;
                debug!(
                    owners = %cluster.owners().map(|root| root.owner.name()).join(", "),
                    ?scope,
                    "Resolved build environment"
                );
                let nested = discover(&graph, &runtime, scope, &build_hasher).await?;
                pending.enqueue(nested, strategy, &runtime.fork_markers);
                builds.record(scope, pins, hasher, cluster, graph);
            }
        }
        runtime.builds = builds
            .into_builds()
            .map(|(owners, graph)| BuildResolution { owners, graph })
            .collect();
        Ok(runtime)
    }
}

fn build_error<'a>(
    owners: impl Iterator<Item = &'a SourceBuildRequirements>,
    error: ResolveError,
) -> ResolveError {
    let packages = owners.map(|root| root.owner.name().clone()).collect();
    ResolveError::BuildDependencies(packages, Box::new(error))
}

struct BuildGroup<'a, P> {
    scope: UniversalMarker,
    pins: Option<&'a P>,
    hasher: &'a HashStrategy,
    owners: Vec<SourceBuildRequirements>,
}

struct PendingBuilds<'a, P> {
    /// A source build can recur through another build dependency. Only visit new marker regions.
    /// Two registries can advertise the same digest but own separate build environments.
    covered: FxHashMap<(DistributionId, Option<Version>, Option<IndexUrl>), UniversalMarker>,
    groups: Vec<BuildGroup<'a, P>>,
    hashes: &'a BuildConstraintHashes,
}

impl<'pins, P: Ord> PendingBuilds<'pins, P> {
    fn enqueue(
        &mut self,
        mut owners: Vec<(SourceBuildRequirements, Option<&'pins P>)>,
        strategy: BuildForkStrategy,
        runtime_forks: &[UniversalMarker],
    ) {
        let mut groups = BTreeMap::new();
        let runtime_forks = if runtime_forks.is_empty() {
            &[UniversalMarker::TRUE][..]
        } else {
            runtime_forks
        };
        owners.sort_by(|(left, _), (right, _)| {
            left.owner
                .name()
                .cmp(right.owner.name())
                .then_with(|| left.owner.version().cmp(&right.owner.version()))
                .then_with(|| {
                    left.owner
                        .distribution_id()
                        .cmp(&right.owner.distribution_id())
                })
                .then_with(|| left.owner.index().cmp(&right.owner.index()))
        });
        for (owner, pins) in owners {
            let covered = self
                .covered
                .entry((
                    owner.owner.distribution_id(),
                    owner.owner.version().cloned(),
                    owner.owner.index().cloned(),
                ))
                .or_insert(UniversalMarker::FALSE);
            for &fork in runtime_forks {
                let mut marker = owner.marker;
                marker.and(fork);
                marker.and(UniversalMarker::from_combined(covered.combined().negate()));
                if marker.is_false() {
                    continue;
                }
                covered.or(marker);
                for (scope, hasher) in self.hashes.for_scope(marker) {
                    let mut root = owner.clone();
                    root.marker = scope;
                    match strategy {
                        BuildForkStrategy::Clustered => {
                            groups
                                .entry((scope, pins))
                                .or_insert_with(|| BuildGroup {
                                    scope,
                                    pins,
                                    hasher,
                                    owners: Vec::new(),
                                })
                                .owners
                                .push(root);
                        }
                        BuildForkStrategy::PerPackage => self.groups.push(BuildGroup {
                            scope,
                            pins,
                            hasher,
                            owners: vec![root],
                        }),
                    }
                }
            }
        }
        self.groups.extend(groups.into_values());
    }
}
