use hashbrown::hash_map::EntryRef;
use uv_distribution_types::IndexMetadata;
use uv_normalize::PackageName;

use crate::resolver::ResolverEnvironment;
use crate::{FxHashbrownMap, ResolveError};

/// See [`crate::resolver::ForkState`].
#[derive(Default, Debug, Clone)]
pub(crate) struct ForkIndexes(FxHashbrownMap<PackageName, IndexMetadata>);

impl ForkIndexes {
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
        let previous = match self.0.entry_ref(package_name) {
            EntryRef::Occupied(mut entry) => entry.insert(index.clone()),
            EntryRef::Vacant(entry) => {
                entry.insert(index.clone());
                return Ok(());
            }
        };
        if &previous != index {
            let mut conflicts = vec![previous.url, index.url.clone()];
            conflicts.sort();
            return Err(ResolveError::ConflictingIndexesForEnvironment {
                package_name: package_name.clone(),
                indexes: conflicts,
                env: env.clone(),
            });
        }
        Ok(())
    }
}
