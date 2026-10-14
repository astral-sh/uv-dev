use std::hash::BuildHasherDefault;
use std::sync::Arc;

use rustc_hash::FxHasher;
use uv_distribution_types::{IndexMetadata, IndexUrl};
use uv_normalize::PackageName;
use uv_once_map::{RegisteredEntry, RegisteredOnceMap};
use uv_pep440::Version;
use uv_pypi_types::ResolutionMetadata;
use uv_resolver_types::DistributionMetadataIndex;

use crate::resolver::provider::VersionsResponse;

/// In-memory index of package metadata.
#[derive(Default, Clone)]
pub struct InMemoryIndex(Arc<SharedInMemoryIndex>);

#[derive(Default)]
struct SharedInMemoryIndex {
    /// A map from package name to the metadata for that package and the index where the metadata
    /// came from.
    implicit: FxOnceMap<PackageName, Arc<VersionsResponse>>,

    explicit: FxOnceMap<(PackageName, IndexUrl), Arc<VersionsResponse>>,

    /// A map from a concrete distribution to its metadata.
    distributions: DistributionMetadataIndex,
}

pub(crate) type FxOnceMap<K, V> = RegisteredOnceMap<K, V, BuildHasherDefault<FxHasher>>;
pub(crate) type FxRegisteredEntry<'a, K, V> =
    RegisteredEntry<'a, K, V, BuildHasherDefault<FxHasher>>;

impl InMemoryIndex {
    /// Return a completed package-version response for an explicit index or the implicit indexes.
    pub(crate) fn get_versions(
        &self,
        name: &PackageName,
        index: Option<&IndexMetadata>,
    ) -> Option<Arc<VersionsResponse>> {
        match index {
            Some(index) => self.explicit().get(&(name.clone(), index.url().clone())),
            None => self.implicit().get(name),
        }
    }

    /// Find registry-provided metadata, preferring the matching implicit index over the explicit index.
    pub(crate) fn get_registry_metadata(
        &self,
        name: &PackageName,
        version: &Version,
        index: &IndexUrl,
    ) -> Option<ResolutionMetadata> {
        let find = |response: &VersionsResponse, required_index: Option<&IndexUrl>| {
            let VersionsResponse::Found(version_maps) = response else {
                return None;
            };
            version_maps
                .iter()
                .filter(|version_map| {
                    required_index.is_none_or(|index| version_map.index() == Some(index))
                })
                .find_map(|version_map| version_map.get_metadata(version))
        };

        self.implicit()
            .get(name)
            .and_then(|response| find(&response, Some(index)))
            .or_else(|| {
                self.explicit()
                    .get(&(name.clone(), index.clone()))
                    .and_then(|response| find(&response, None))
            })
    }

    /// Returns a reference to the package metadata map.
    pub(crate) fn implicit(&self) -> &FxOnceMap<PackageName, Arc<VersionsResponse>> {
        &self.0.implicit
    }

    /// Returns a reference to the package metadata map.
    pub(crate) fn explicit(&self) -> &FxOnceMap<(PackageName, IndexUrl), Arc<VersionsResponse>> {
        &self.0.explicit
    }

    /// Returns a reference to the distribution metadata map.
    pub fn distributions(&self) -> &DistributionMetadataIndex {
        &self.0.distributions
    }

    /// Return exclusive access to distribution metadata when no cloned index can use it.
    pub fn distributions_mut(&mut self) -> Option<&mut DistributionMetadataIndex> {
        Arc::get_mut(&mut self.0).map(|index| &mut index.distributions)
    }
}
