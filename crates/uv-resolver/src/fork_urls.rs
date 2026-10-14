use hashbrown::hash_map::EntryRef;

use uv_normalize::PackageName;
use uv_pypi_types::VerbatimParsedUrl;

use crate::resolver::ResolverEnvironment;
use crate::{FxHashbrownMap, ResolveError};

/// See [`crate::resolver::ForkState`].
#[derive(Default, Debug, Clone)]
pub(crate) struct ForkUrls(FxHashbrownMap<PackageName, VerbatimParsedUrl>);

impl ForkUrls {
    /// Get the URL previously used for a package in this fork.
    pub(crate) fn get(&self, package_name: &PackageName) -> Option<&VerbatimParsedUrl> {
        self.0.get(package_name)
    }

    /// Whether we use a URL for this package.
    pub(crate) fn contains_key(&self, package_name: &PackageName) -> bool {
        self.0.contains_key(package_name)
    }

    /// Check that this is the only URL used for this package in this fork.
    pub(crate) fn insert(
        &mut self,
        package_name: &PackageName,
        url: &VerbatimParsedUrl,
        env: &ResolverEnvironment,
    ) -> Result<(), ResolveError> {
        match self.0.entry_ref(package_name) {
            EntryRef::Occupied(previous) => {
                if previous.get() != url {
                    let mut conflicting_url =
                        vec![previous.get().parsed_url.clone(), url.parsed_url.clone()];
                    conflicting_url.sort();
                    return Err(ResolveError::ConflictingUrls {
                        package_name: package_name.clone(),
                        urls: conflicting_url,
                        env: env.clone(),
                    });
                }
            }
            EntryRef::Vacant(vacant) => {
                vacant.insert(url.clone());
            }
        }
        Ok(())
    }
}
