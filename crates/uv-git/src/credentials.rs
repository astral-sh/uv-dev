use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};
use tracing::trace;
use uv_auth::{CredentialsFromUrlError, UrlCredentials};
use uv_cache_key::RepositoryUrl;
use uv_redacted::DisplaySafeUrl;

/// Global authentication cache for a uv invocation.
///
/// This is used to share Git credentials within a single process.
pub(crate) static GIT_STORE: LazyLock<GitStore> = LazyLock::new(GitStore::default);

/// A store for Git credentials.
#[derive(Debug, Default)]
pub(crate) struct GitStore(RwLock<HashMap<RepositoryUrl, Arc<UrlCredentials>>>);

impl GitStore {
    /// Insert [`UrlCredentials`] for the given URL into the store.
    fn insert(&self, url: RepositoryUrl, credentials: UrlCredentials) {
        self.0.write().unwrap().insert(url, Arc::new(credentials));
    }

    /// Get the [`UrlCredentials`] for the given URL, if they exist.
    pub(crate) fn get(&self, url: &RepositoryUrl) -> Option<Arc<UrlCredentials>> {
        self.0.read().unwrap().get(url).cloned()
    }
}

/// Store [`UrlCredentials`] for the given Git repository URL.
pub fn store_credentials(url: RepositoryUrl, credentials: UrlCredentials) {
    GIT_STORE.insert(url, credentials);
}

/// Populate the global authentication store with credentials on a Git URL, if there are any.
///
/// Returns `true` if the store was updated.
pub fn store_credentials_from_url(url: &DisplaySafeUrl) -> Result<bool, CredentialsFromUrlError> {
    if let Some(credentials) = UrlCredentials::from_url(url)? {
        trace!("Caching credentials for `{url}`");
        store_credentials(RepositoryUrl::new(url.clone()), credentials);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, anyhow};
    use uv_cache_key::RepositoryUrl;
    use uv_redacted::DisplaySafeUrl;

    use super::{GIT_STORE, store_credentials_from_url};

    #[test]
    fn ssh_credentials_with_colon_round_trip() -> Result<()> {
        let url =
            DisplaySafeUrl::parse("ssh://user%3Aname:pass%3Aword@example.com/ssh-identity.git")?;
        assert!(store_credentials_from_url(&url)?);
        let credentials = GIT_STORE
            .get(&RepositoryUrl::new(url.clone()))
            .ok_or_else(|| anyhow!("stored Git credentials were not found"))?;
        let remote = DisplaySafeUrl::parse("ssh://example.com/ssh-identity.git")?;
        assert_eq!(credentials.apply(remote), url);
        Ok(())
    }
}
