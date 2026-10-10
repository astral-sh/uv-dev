use std::sync::Arc;

use uv_distribution::{DistributionDatabase, Metadata};
use uv_distribution_types::{Dist, Identifier, Requirement};
use uv_resolver::{InMemoryIndex, MetadataResponse};
use uv_types::{BuildContext, HashStrategy};

use crate::{Error, required_dist};

/// Read full metadata for a direct requirement, reusing the shared distribution cache.
pub async fn resolve_requirement_metadata<Context: BuildContext>(
    requirement: &Requirement,
    hasher: &HashStrategy,
    index: &InMemoryIndex,
    database: &DistributionDatabase<'_, Context>,
) -> Result<Option<Metadata>, Error> {
    // Determine whether the requirement represents a local distribution and convert to a
    // buildable distribution.
    let Some(dist) = required_dist(requirement)? else {
        return Ok(None);
    };

    database.record_metadata(&dist);

    // Fetch the metadata for the distribution.
    let metadata = {
        let id = dist.distribution_id();
        if let Some(metadata) = cached_dist_metadata(&dist, index) {
            metadata
        } else {
            // Run the PEP 517 build process to extract metadata from the source distribution.
            let archive = database
                .get_or_build_wheel_metadata(&dist, hasher.metadata_policy(&dist))
                .await
                .map_err(|err| Error::from_dist(dist, err))?;

            let metadata = archive.metadata.clone();

            // Insert the metadata into the index.
            index
                .distributions()
                .done(id, Arc::new(MetadataResponse::Found(archive)));

            metadata
        }
    };

    Ok(Some(metadata))
}

/// Return cached full metadata for a direct requirement.
pub fn cached_requirement_metadata(
    requirement: &Requirement,
    index: &InMemoryIndex,
) -> Result<Option<Metadata>, Error> {
    Ok(required_dist(requirement)?.and_then(|dist| cached_dist_metadata(&dist, index)))
}

fn cached_dist_metadata(dist: &Dist, index: &InMemoryIndex) -> Option<Metadata> {
    index
        .distributions()
        .get(&dist.distribution_id())
        .and_then(|response| {
            if let MetadataResponse::Found(archive, ..) = response.as_ref() {
                Some(archive.metadata.clone())
            } else {
                None
            }
        })
}
