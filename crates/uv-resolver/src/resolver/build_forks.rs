use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};
use std::thread;

use itertools::Itertools;
use rustc_hash::FxHashMap;
use tracing::debug;

use uv_distribution_types::{DistributionId, HashCollection, Identifier, Name, Requirement};
use uv_pep440::Version;
use uv_resolver_types::{
    BuildResolution, ResolverOutput, SourceBuildRequirements, UniversalMarker,
};
use uv_static::EnvVars;
use uv_types::{BuildContext, HashStrategy, HashVerification, InstalledPackagesProvider};

use super::{
    DefaultResolverProvider, InMemoryIndex, Resolver, ResolverEnvironment, ResolverState, RootFork,
};
use crate::preferences::{Preference, Preferences};
use crate::{AllowedYanks, Manifest, ResolveError};

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

impl<Context: BuildContext, InstalledPackages: InstalledPackagesProvider>
    Resolver<DefaultResolverProvider<'_, Context>, InstalledPackages>
{
    /// Solve runtime and build dependencies on sibling branches of one PubGrub root.
    /// Discovery sees the original runtime graph at every depth for `match-runtime`.
    pub async fn resolve_with_build_forks<E: From<ResolveError>>(
        self,
        strategy: BuildForkStrategy,
        build_hasher: &HashStrategy,
        prepare: impl AsyncFn(
            Vec<Requirement>,
            &ResolverEnvironment,
            &InMemoryIndex,
            &HashStrategy,
        ) -> Result<(Manifest, HashStrategy), E>,
        mut discover: impl AsyncFnMut(
            &ResolverOutput,
            &ResolverOutput,
            UniversalMarker,
            &HashStrategy,
        ) -> Result<Vec<SourceBuildRequirements>, E>,
    ) -> Result<ResolverOutput, E> {
        let state = Arc::new(self.state);
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
        let preferences = Preferences::from_iter(
            Preference::from_runtime(&runtime).map_err(ResolveError::from)?,
            &state.env,
        );
        let roots = discover(&runtime, &runtime, UniversalMarker::TRUE, &state.hasher).await?;
        let mut pending = PendingBuilds::default();
        pending.enqueue(roots, strategy, &runtime.fork_markers);
        let mut builds = Vec::new();
        let hasher = HashStrategy::collect(HashCollection::Url)
            .with_verification(build_hasher.verification().clone());

        let mut solve = async |owners: &[SourceBuildRequirements]| {
            let scope = owners[0].marker;
            let env = ResolverEnvironment::universal(vec![scope.combined()]);
            // A URL can be trusted by one root but carry different hashes in another.
            let index = match hasher.verification() {
                HashVerification::Unspecified => state.index.fork_versions(),
                HashVerification::IfPresent(_) | HashVerification::Required(_) => {
                    InMemoryIndex::default()
                }
            };
            let requirements = owners
                .iter()
                .flat_map(|owner| &owner.requirements)
                .filter(|requirement| !requirement.marker.is_disjoint(scope.pep508()))
                .cloned()
                .collect();
            let (mut manifest, hasher) = prepare(requirements, &env, &index, &hasher).await?;
            provider.set_hasher(hasher.clone());

            manifest.preferences = preferences.clone();
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
                owners = %owners.iter().map(|root| root.owner.name()).join(", "),
                ?scope,
                "Solving build requirements"
            );
            let graph = Arc::new(build).run(&provider, schedule).await;
            Ok::<_, E>(graph.map(|graph| (graph, hasher)))
        };
        while let Some(owners) = pending.groups.pop() {
            for (owners, (graph, hasher)) in resolve_cluster(owners, &mut solve).await? {
                let scope = owners[0].marker;
                debug!(
                    owners = %owners.iter().map(|root| root.owner.name()).join(", "),
                    ?scope,
                    "Resolved build environment"
                );
                let nested = discover(&graph, &runtime, scope, &hasher).await?;
                pending.enqueue(nested, strategy, &runtime.fork_markers);
                builds.push(BuildResolution { owners, graph });
            }
        }
        state.on_complete();
        runtime.builds = builds;
        Ok(runtime)
    }
}

/// Try a single shared graph first. After a conflict, put each owner in the first cluster
/// whose combined requirements resolve. Retain the successful graph so it can be emitted
/// without solving the same combination a second time.
async fn resolve_cluster<T, E: From<ResolveError>>(
    owners: Vec<SourceBuildRequirements>,
    solve: &mut impl AsyncFnMut(&[SourceBuildRequirements]) -> Result<Result<T, ResolveError>, E>,
) -> Result<Vec<(Vec<SourceBuildRequirements>, T)>, E> {
    let count = owners.len();
    match solve(&owners).await? {
        Ok(graph) => return Ok(vec![(owners, graph)]),
        Err(error) if count > 1 && is_cluster_conflict(&error) => {}
        Err(error) => return Err(build_error(&owners, error).into()),
    }

    let mut clusters: Vec<(Vec<SourceBuildRequirements>, Option<T>)> = Vec::new();
    'owner: for owner in owners {
        for (roots, graph) in &mut clusters {
            if roots.len() + 1 == count {
                // The initial solve already rejected the full batch.
                continue;
            }
            roots.push(owner.clone());
            match solve(roots).await? {
                Ok(resolution) => {
                    *graph = Some(resolution);
                    continue 'owner;
                }
                Err(error) if is_cluster_conflict(&error) => {
                    roots.pop();
                }
                Err(error) => return Err(build_error(roots, error).into()),
            }
        }
        clusters.push((vec![owner], None));
    }

    let mut builds = Vec::with_capacity(clusters.len());
    for (owners, graph) in clusters {
        let graph = match graph {
            Some(graph) => graph,
            None => solve(&owners)
                .await?
                .map_err(|error| build_error(&owners, error))?,
        };
        builds.push((owners, graph));
    }
    Ok(builds)
}

fn is_cluster_conflict(error: &ResolveError) -> bool {
    match error {
        ResolveError::Dependencies(source, ..) => is_cluster_conflict(source),
        ResolveError::NoSolution(_)
        | ResolveError::ConflictingUrls { .. }
        | ResolveError::ConflictingIndexesForEnvironment { .. }
        | ResolveError::ConflictingIndexes(..) => true,
        ResolveError::Client(_)
        | ResolveError::Distribution(_)
        | ResolveError::BuildDependencies(..)
        | ResolveError::Preference(_)
        | ResolveError::SpawnThread(_)
        | ResolveError::ChannelClosed
        | ResolveError::UnregisteredTask(_)
        | ResolveError::DisallowedUrl { .. }
        | ResolveError::DistributionType(_)
        | ResolveError::Dist(..)
        | ResolveError::InvalidVersion(_)
        | ResolveError::UnhashedPackage(_)
        | ResolveError::ConflictingDistribution(_)
        | ResolveError::PackageUnavailable(_)
        | ResolveError::ConflictMarker(_)
        | ResolveError::MismatchedPackageName { .. } => false,
    }
}

fn build_error(owners: &[SourceBuildRequirements], error: ResolveError) -> ResolveError {
    let packages = owners
        .iter()
        .map(|root| root.owner.name().clone())
        .collect();
    ResolveError::BuildDependencies(packages, Box::new(error))
}

#[derive(Default)]
struct PendingBuilds {
    /// A source build can recur through another build dependency. Only visit new marker regions.
    covered: FxHashMap<(DistributionId, Option<Version>), UniversalMarker>,
    groups: Vec<Vec<SourceBuildRequirements>>,
}

impl PendingBuilds {
    fn enqueue(
        &mut self,
        mut owners: Vec<SourceBuildRequirements>,
        strategy: BuildForkStrategy,
        runtime_forks: &[UniversalMarker],
    ) {
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        let runtime_forks = if runtime_forks.is_empty() {
            &[UniversalMarker::TRUE][..]
        } else {
            runtime_forks
        };
        owners.sort_by(|left, right| {
            left.owner
                .name()
                .cmp(right.owner.name())
                .then_with(|| left.owner.version().cmp(&right.owner.version()))
                .then_with(|| {
                    left.owner
                        .distribution_id()
                        .cmp(&right.owner.distribution_id())
                })
        });
        for owner in owners {
            let covered = self
                .covered
                .entry((
                    owner.owner.distribution_id(),
                    owner.owner.version().cloned(),
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
                let mut root = owner.clone();
                root.marker = marker;
                match strategy {
                    BuildForkStrategy::Clustered => groups.entry(marker).or_default().push(root),
                    BuildForkStrategy::PerPackage => self.groups.push(vec![root]),
                }
            }
        }
        self.groups.extend(groups.into_values());
    }
}
