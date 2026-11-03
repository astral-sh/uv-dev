use std::borrow::Cow;

use anyhow::{Context, Result};
use futures::{StreamExt, TryStreamExt, stream};
use uv_build_frontend::SourceBuild;
use uv_configuration::NoSources;
use uv_distribution::DistributionDatabase;
use uv_distribution_types::{BuiltDist, Dist, ResolvedDist, SourceDist};
use uv_resolver::{ResolverOutput, UniversalMarker};

use crate::{BuildDispatch, SourceBuildRequirements};

impl BuildDispatch<'_> {
    /// Discover requirements for every installable source in `graph`, including releases where
    /// the resolver used a wheel for metadata. Match extra build requirements against `runtime`.
    /// For recursive discovery, pass the marker of the parent build and the original runtime graph.
    pub async fn discover_build_requirements(
        &self,
        graph: &ResolverOutput,
        runtime: &ResolverOutput,
        scope: UniversalMarker,
    ) -> Result<Vec<SourceBuildRequirements>> {
        let database = DistributionDatabase::new(
            self.client,
            self,
            self.concurrency.downloads_semaphore.clone(),
        );
        let owners = graph.base_dists().filter_map(|(_, owner)| {
            let ResolvedDist::Installable { dist, .. } = &owner.dist else {
                return None;
            };
            let source = match dist.as_ref() {
                Dist::Source(source) if source.is_virtual() => None,
                Dist::Source(source) => Some(Cow::Borrowed(source)),
                Dist::Built(BuiltDist::Registry(wheels)) => wheels
                    .sdist
                    .as_ref()
                    .map(|source| Cow::Owned(SourceDist::Registry(source.clone()))),
                Dist::Built(
                    BuiltDist::DirectUrl(_) | BuiltDist::Path(_) | BuiltDist::GitPath(_),
                ) => None,
            }?;
            let mut marker = scope;
            marker.and(owner.marker);
            (!marker.is_false()).then_some((owner, source, marker))
        });
        let sets = stream::iter(owners)
            .map(|(owner, source, marker)| {
                let database = &database;
                async move {
                    let hashes = self.base_hasher.archive_policy(source.as_ref());
                    let tree = database
                        .build_source_tree(&source, hashes)
                        .await
                        .with_context(|| format!("Failed to read build source for `{owner}`"))?;
                    // Only source trees can refer to other projects in their workspace. Registry and
                    // archive requirements are lowered as published PEP 508 requirements.
                    let sources = match source.as_ref() {
                        SourceDist::Directory(_) | SourceDist::GitDirectory(_) => &self.sources,
                        SourceDist::Registry(_)
                        | SourceDist::DirectUrl(_)
                        | SourceDist::Path(_)
                        | SourceDist::GitPath(_) => &NoSources::All,
                    };
                    let backend = SourceBuild::static_requirements(
                        &tree,
                        &owner.name,
                        self.index_locations,
                        sources,
                        self.cache,
                        &self.workspace_cache,
                        self.client.credentials_cache(),
                    )
                    .await
                    .with_context(|| format!("Failed to read build requirements for `{owner}`"))?;
                    runtime
                        .match_build_requirements(
                            &owner.dist,
                            marker,
                            backend,
                            self.extra_build_requires
                                .get(&owner.name)
                                .map_or(&[], Vec::as_slice),
                        )
                        .with_context(|| {
                            format!("Failed to match extra build requirements for `{owner}`")
                        })
                }
            })
            .buffered(self.concurrency.downloads)
            .try_collect::<Vec<_>>()
            .await?;
        Ok(sets.into_iter().flatten().collect())
    }
}
