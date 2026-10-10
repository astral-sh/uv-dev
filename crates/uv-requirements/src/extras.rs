use std::sync::Arc;

use futures::{TryStreamExt, stream::FuturesOrdered};

use uv_distribution::{DistributionDatabase, Reporter};
use uv_distribution_types::Requirement;
use uv_resolver::InMemoryIndex;
use uv_types::{BuildContext, HashStrategy};

use crate::{Error, resolve_requirement_metadata};

/// A resolver to expand the requested extras for a set of requirements to include all defined
/// extras.
pub struct ExtrasResolver<'a, Context: BuildContext> {
    /// Whether to check hashes for distributions.
    hasher: &'a HashStrategy,
    /// The in-memory index for resolving dependencies.
    index: &'a InMemoryIndex,
    /// The database for fetching and building distributions.
    database: DistributionDatabase<'a, Context>,
}

impl<'a, Context: BuildContext> ExtrasResolver<'a, Context> {
    /// Instantiate a new [`ExtrasResolver`] for a given set of requirements.
    pub fn new(
        hasher: &'a HashStrategy,
        index: &'a InMemoryIndex,
        database: DistributionDatabase<'a, Context>,
    ) -> Self {
        Self {
            hasher,
            index,
            database,
        }
    }

    /// Set the [`Reporter`] to use for this resolver.
    #[must_use]
    pub fn with_reporter(self, reporter: Arc<dyn Reporter>) -> Self {
        Self {
            database: self.database.with_reporter(reporter),
            ..self
        }
    }

    /// Expand the set of available extras for a given set of requirements.
    pub async fn resolve(
        self,
        requirements: impl Iterator<Item = Requirement>,
    ) -> Result<Vec<Requirement>, Error> {
        let Self {
            hasher,
            index,
            database,
        } = self;
        requirements
            .map(async |requirement| {
                Box::pin(Self::resolve_requirement(
                    requirement,
                    hasher,
                    index,
                    &database,
                ))
                .await
            })
            .collect::<FuturesOrdered<_>>()
            .try_collect()
            .await
    }

    /// Expand the set of available extras for a given [`Requirement`].
    async fn resolve_requirement(
        requirement: Requirement,
        hasher: &HashStrategy,
        index: &InMemoryIndex,
        database: &DistributionDatabase<'a, Context>,
    ) -> Result<Requirement, Error> {
        let Some(metadata) =
            resolve_requirement_metadata(&requirement, hasher, index, database).await?
        else {
            return Ok(requirement);
        };

        // Sort extras for consistency.
        let extras = {
            let mut extras = metadata.provides_extra.to_vec();
            extras.sort_unstable();
            extras
        };

        Ok(Requirement {
            extras: extras.into_boxed_slice(),
            ..requirement
        })
    }
}
