use rustc_hash::FxHashMap;
use uv_distribution_types::IndexMetadata;
use uv_normalize::PackageName;

use crate::ResolveError;
use crate::resolver::ResolverEnvironment;

/// See [`crate::resolver::ForkState`].
#[derive(Default, Debug, Clone)]
pub(crate) struct ForkIndexes(FxHashMap<PackageName, IndexMetadata>);

impl ForkIndexes {
    /// Return the number of explicit index sources recorded in this fork.
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// Iterate over every index recorded for this fork, including packages no longer selected.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&PackageName, &IndexMetadata)> {
        self.0.iter()
    }

    /// Get the [`Index`] previously used for a package in this fork.
    pub(crate) fn get(&self, package_name: &PackageName) -> Option<&IndexMetadata> {
        self.0.get(package_name)
    }

    /// Check that this is the only [`Index`] used for this package in this fork.
    pub(crate) fn insert(
        &mut self,
        package_name: &PackageName,
        index: &IndexMetadata,
        env: &ResolverEnvironment,
    ) -> Result<(), ResolveError> {
        if let Some(previous) = self.0.insert(package_name.clone(), index.clone()) {
            if &previous != index {
                let mut conflicts = vec![previous.url, index.url.clone()];
                conflicts.sort();
                return Err(ResolveError::ConflictingIndexesForEnvironment {
                    package_name: package_name.clone(),
                    indexes: conflicts,
                    env: env.clone(),
                });
            }
        }
        Ok(())
    }
}
