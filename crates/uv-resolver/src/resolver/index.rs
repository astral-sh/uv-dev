use std::hash::BuildHasherDefault;
use std::sync::Arc;

use rustc_hash::FxHasher;
use uv_distribution_types::{IndexUrl, VariantsJsonContent};
use uv_normalize::PackageName;
use uv_once_map::{RegisteredEntry, RegisteredOnceMap};
use uv_resolver_types::DistributionMetadataIndex;

use crate::prioritized_distribution::RegistryVariantsJson;
use crate::resolver::provider::{MetadataUnavailable, VersionsResponse};

/// In-memory index of package metadata.
#[derive(Default, Clone)]
pub struct InMemoryIndex {
    versions: Arc<PackageVersions>,
    metadata: Arc<SharedMetadata>,
}

#[derive(Default)]
struct PackageVersions {
    /// A map from package name to the metadata for that package and the index where the metadata
    /// came from.
    implicit: FxOnceMap<PackageName, Arc<VersionsResponse>>,

    explicit: FxOnceMap<(PackageName, IndexUrl), Arc<VersionsResponse>>,
}

#[derive(Default)]
struct SharedMetadata {
    /// A map from a concrete distribution to its metadata.
    distributions: DistributionMetadataIndex,

    /// A map from an index variant metadata file to its parsed contents.
    variants_json: FxOnceMap<RegistryVariantsJson, Arc<ParsedVariantsJsonResponse>>,
}

pub(crate) type ParsedVariantsJsonResponse = Result<VariantsJsonContent, MetadataUnavailable>;
pub(crate) type FxOnceMap<K, V> = RegisteredOnceMap<K, V, BuildHasherDefault<FxHasher>>;
pub(crate) type FxRegisteredEntry<'a, K, V> =
    RegisteredEntry<'a, K, V, BuildHasherDefault<FxHasher>>;

impl InMemoryIndex {
    /// Retry with fresh Simple pages while retaining metadata needed by lock validation.
    pub(super) fn fresh_package_versions(&self) -> Self {
        Self {
            versions: Arc::default(),
            metadata: self.metadata.clone(),
        }
    }

    /// Returns a reference to the package metadata map.
    pub(crate) fn implicit(&self) -> &FxOnceMap<PackageName, Arc<VersionsResponse>> {
        &self.versions.implicit
    }

    /// Returns a reference to the package metadata map.
    pub(crate) fn explicit(&self) -> &FxOnceMap<(PackageName, IndexUrl), Arc<VersionsResponse>> {
        &self.versions.explicit
    }

    /// Returns a reference to the distribution metadata map.
    pub fn distributions(&self) -> &DistributionMetadataIndex {
        &self.metadata.distributions
    }

    pub(crate) fn variants_json(
        &self,
    ) -> &FxOnceMap<RegistryVariantsJson, Arc<ParsedVariantsJsonResponse>> {
        &self.metadata.variants_json
    }

    /// Return exclusive access to distribution metadata when no cloned index can use it.
    pub fn distributions_mut(&mut self) -> Option<&mut DistributionMetadataIndex> {
        Arc::get_mut(&mut self.metadata).map(|index| &mut index.distributions)
    }
}
