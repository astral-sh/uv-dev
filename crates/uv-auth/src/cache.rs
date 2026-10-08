use std::fmt::{self, Display, Formatter};
use std::hash::BuildHasherDefault;
use std::sync::Arc;
use std::sync::RwLock;

use hashbrown::{Equivalent, HashMap};
use rustc_hash::{FxBuildHasher, FxHasher};
use tracing::trace;
use url::Url;

use uv_once_map::OnceMap;
use uv_redacted::DisplaySafeUrl;

use crate::credentials::{Authentication, CredentialsFromUrlError, Username};
use crate::{Credentials, Realm, RealmRef};

type FxOnceMap<K, V> = OnceMap<K, V, BuildHasherDefault<FxHasher>>;

/// Hash fields in the same order as the owned `(Realm, Username)` key.
#[derive(Hash)]
struct RealmKeyRef<'a>(RealmRef<'a>, Option<&'a str>);

impl Equivalent<(Realm, Username)> for RealmKeyRef<'_> {
    fn equivalent(&self, key: &(Realm, Username)) -> bool {
        self.0 == key.0 && self.1 == key.1.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum FetchUrl {
    /// A full index URL
    Index(DisplaySafeUrl),
    /// A realm URL
    Realm(Realm),
}

impl Display for FetchUrl {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            Self::Index(index) => Display::fmt(index, f),
            Self::Realm(realm) => Display::fmt(realm, f),
        }
    }
}

#[derive(Debug)] // All internal types are redacted.
pub struct CredentialsCache {
    /// A cache per realm and username
    realms: RwLock<HashMap<(Realm, Username), Arc<Authentication>, FxBuildHasher>>,
    /// A cache tracking the result of realm or index URL fetches from external services
    pub(crate) fetches: FxOnceMap<(FetchUrl, Username), Option<Arc<Authentication>>>,
    /// A cache per URL, uses a trie for efficient prefix queries.
    urls: RwLock<UrlTrie<Arc<Authentication>>>,
}

impl Default for CredentialsCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialsCache {
    /// Create a new cache.
    pub fn new() -> Self {
        Self {
            fetches: FxOnceMap::default(),
            realms: RwLock::new(HashMap::default()),
            urls: RwLock::new(UrlTrie::new()),
        }
    }

    /// Populate the global authentication store with credentials on a URL, if there are any.
    ///
    /// Returns `true` if the store was updated.
    pub fn store_credentials_from_url(
        &self,
        url: &DisplaySafeUrl,
    ) -> Result<bool, CredentialsFromUrlError> {
        if let Some(credentials) = Credentials::from_url(url)? {
            trace!("Caching credentials for `{url}`");
            self.insert(url, Arc::new(Authentication::from(credentials)));
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Populate the global authentication store with credentials on a URL, if there are any.
    ///
    /// Returns `true` if the store was updated.
    pub fn store_credentials(&self, url: &DisplaySafeUrl, credentials: Credentials) {
        trace!("Caching credentials for `{url}`");
        self.insert(url, Arc::new(Authentication::from(credentials)));
    }

    /// Return the credentials that should be used for a realm and username, if any.
    pub(crate) fn get_realm(
        &self,
        realm: RealmRef<'_>,
        username: &Username,
    ) -> Option<Arc<Authentication>> {
        let realms = self.realms.read().unwrap();
        let given_username = username.is_some();
        let key = RealmKeyRef(realm, username.as_deref());
        let realm_username = fmt::from_fn(|f| {
            if let Some(username) = username.as_deref() {
                write!(f, "{username}@{realm}")
            } else {
                write!(f, "{realm}")
            }
        });

        let Some(credentials) = realms.get(&key).cloned() else {
            trace!("No credentials in cache for realm {realm_username}");
            return None;
        };

        if given_username && credentials.password().is_none() {
            // If given a username, don't return password-less credentials
            trace!("No password in cache for realm {realm_username}");
            return None;
        }

        trace!("Found cached credentials for realm {realm_username}");
        Some(credentials)
    }

    /// Return the cached credentials for a URL and username, if any.
    ///
    /// Note we do not cache per username, but if a username is passed we will confirm that the
    /// cached credentials have a username equal to the provided one — otherwise `None` is returned.
    /// If multiple usernames are used per URL, the realm cache should be queried instead.
    pub(crate) fn get_url(
        &self,
        url: &DisplaySafeUrl,
        username: &Username,
    ) -> Option<Arc<Authentication>> {
        let urls = self.urls.read().unwrap();
        let credentials = urls.get(url);
        if let Some(credentials) = credentials {
            if username.is_none() || username.as_deref() == credentials.username() {
                if username.is_some() && credentials.password().is_none() {
                    // If given a username, don't return password-less credentials
                    trace!("No password in cache for URL `{url}`");
                    return None;
                }
                trace!("Found cached credentials for URL `{url}`");
                return Some(credentials.clone());
            }
        }
        trace!("No credentials in cache for URL `{url}`");
        None
    }

    /// Update the cache with the given credentials.
    pub(crate) fn insert(&self, url: &DisplaySafeUrl, credentials: Arc<Authentication>) {
        // Do not cache empty credentials
        if credentials.is_empty() {
            return;
        }

        // Insert an entry for requests including the username
        let username = credentials.to_username();
        if username.is_some() {
            let realm = (Realm::from(url), username);
            self.insert_realm(realm, &credentials);
        }

        // Insert an entry for requests with no username
        self.insert_realm((Realm::from(url), Username::none()), &credentials);

        // Insert an entry for the URL
        let mut urls = self.urls.write().unwrap();
        urls.insert(url, credentials);
    }

    /// Private interface to update a realm cache entry.
    ///
    /// Returns replaced credentials, if any.
    fn insert_realm(
        &self,
        key: (Realm, Username),
        credentials: &Arc<Authentication>,
    ) -> Option<Arc<Authentication>> {
        // Do not cache empty credentials
        if credentials.is_empty() {
            return None;
        }

        let mut realms = self.realms.write().unwrap();

        // Always replace existing entries if we have a password or token
        if credentials.is_authenticated() {
            return realms.insert(key, credentials.clone());
        }

        // If we only have a username, add a new entry or replace an existing entry if it doesn't have a password
        let existing = realms.get(&key);
        if existing.is_none_or(|credentials| credentials.password().is_none()) {
            return realms.insert(key, credentials.clone());
        }

        None
    }
}

#[derive(Debug)]
struct UrlTrie<T> {
    states: Vec<TrieState<T>>,
}

#[derive(Debug)]
struct TrieState<T> {
    children: Vec<(String, usize)>,
    value: Option<T>,
}

impl<T> Default for TrieState<T> {
    fn default() -> Self {
        Self {
            children: vec![],
            value: None,
        }
    }
}

impl<T> UrlTrie<T> {
    fn new() -> Self {
        let mut trie = Self { states: vec![] };
        trie.alloc();
        trie
    }

    fn get(&self, url: &Url) -> Option<&T> {
        let segments = url.path_segments()?;
        let mut state = 0;
        let realm = Realm::from(url).to_string();
        for component in [realm.as_str()]
            .into_iter()
            .chain(segments.filter(|item| !item.is_empty()))
        {
            state = self.states[state].get(component)?;
            if let Some(ref value) = self.states[state].value {
                return Some(value);
            }
        }
        self.states[state].value.as_ref()
    }

    fn insert(&mut self, url: &Url, value: T) {
        // Opaque URLs have no path hierarchy for prefix matching.
        let Some(segments) = url.path_segments() else {
            return;
        };
        let mut state = 0;
        let realm = Realm::from(url).to_string();
        for component in [realm.as_str()]
            .into_iter()
            .chain(segments.filter(|item| !item.is_empty()))
        {
            match self.states[state].index(component) {
                Ok(i) => state = self.states[state].children[i].1,
                Err(i) => {
                    let new_state = self.alloc();
                    self.states[state]
                        .children
                        .insert(i, (component.to_string(), new_state));
                    state = new_state;
                }
            }
        }
        self.states[state].value = Some(value);
    }

    fn alloc(&mut self) -> usize {
        let id = self.states.len();
        self.states.push(TrieState::default());
        id
    }
}

impl<T> TrieState<T> {
    fn get(&self, component: &str) -> Option<usize> {
        let i = self.index(component).ok()?;
        Some(self.children[i].1)
    }

    fn index(&self, component: &str) -> Result<usize, usize> {
        self.children
            .binary_search_by(|(label, _)| label.as_str().cmp(component))
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::hash::BuildHasher;

    use url::ParseError;

    use crate::Credentials;
    use crate::credentials::Password;

    use super::*;

    #[test]
    fn borrowed_realm_keys_match_owned_hashes() -> Result<(), ParseError> {
        let urls = [
            "https://example.com/simple",
            "https://example.com:443/other",
            "https://example.com:8443/simple",
            "http://example.com/simple",
            "https://other.example.com/simple",
            "https://[::1]:8443/simple",
            "file:///path/to/index",
        ]
        .map(Url::parse)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
        let hasher = FxBuildHasher;
        for url in &urls {
            for username in [None, Some(""), Some("alice"), Some("bob")] {
                let owned = (Realm::from(url), Username::new(username.map(str::to_owned)));
                let borrowed = RealmKeyRef(RealmRef::from(url), owned.1.as_deref());
                assert!(borrowed.equivalent(&owned));
                assert_eq!(hasher.hash_one(&borrowed), hasher.hash_one(&owned));

                for other_url in &urls {
                    for other_username in [None, Some("alice"), Some("bob")] {
                        let other = (
                            Realm::from(other_url),
                            Username::new(other_username.map(str::to_owned)),
                        );
                        assert_eq!(borrowed.equivalent(&other), owned == other);
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn realm_cache_matches_borrowed_keys() -> Result<(), Box<dyn Error>> {
        let cache = CredentialsCache::new();
        let url = DisplaySafeUrl::parse("https://example.com/simple")?;
        let credentials = Arc::new(Authentication::from(Credentials::basic(
            Some("alice".to_owned()),
            Some("password".to_owned()),
        )));
        cache.insert(&url, credentials.clone());

        for url in ["https://example.com/other", "https://example.com:443/other"] {
            let url = Url::parse(url)?;
            for username in [None, Some(""), Some("alice")] {
                let username = Username::new(username.map(str::to_owned));
                assert_eq!(
                    cache.get_realm(RealmRef::from(&url), &username),
                    Some(credentials.clone())
                );
            }
            assert_eq!(
                cache.get_realm(RealmRef::from(&url), &Username::from("bob".to_owned())),
                None
            );
        }
        for url in [
            "http://example.com/simple",
            "https://example.com:8443/simple",
            "https://other.example.com/simple",
            "file:///path/to/index",
        ] {
            let url = Url::parse(url)?;
            for username in [None, Some("alice")] {
                let username = Username::new(username.map(str::to_owned));
                assert_eq!(cache.get_realm(RealmRef::from(&url), &username), None);
            }
        }
        Ok(())
    }

    #[test]
    fn realm_cache_keeps_passwordless_lookup_policy() -> Result<(), Box<dyn Error>> {
        let cache = CredentialsCache::new();
        let url = DisplaySafeUrl::parse("https://example.com/simple")?;
        let realm = RealmRef::from(&*url);
        let username = Username::from("alice".to_owned());
        let empty_username = Username::from(String::new());
        let passwordless = Arc::new(Authentication::from(Credentials::basic(
            Some("alice".to_owned()),
            None,
        )));
        cache.insert(&url, passwordless.clone());
        assert_eq!(
            cache.get_realm(realm, &Username::none()),
            Some(passwordless.clone())
        );
        assert_eq!(
            cache.get_realm(realm, &empty_username),
            Some(passwordless.clone())
        );
        assert_eq!(cache.get_realm(realm, &username), None);

        let authenticated = Arc::new(Authentication::from(Credentials::basic(
            Some("alice".to_owned()),
            Some("password".to_owned()),
        )));
        cache.insert(&url, authenticated.clone());
        cache.insert(&url, passwordless);
        assert_eq!(
            cache.get_realm(realm, &Username::none()),
            Some(authenticated.clone())
        );
        assert_eq!(cache.get_realm(realm, &username), Some(authenticated));
        Ok(())
    }

    #[test]
    fn test_trie() {
        let credentials1 =
            Credentials::basic(Some("username1".to_string()), Some("password1".to_string()));
        let credentials2 =
            Credentials::basic(Some("username2".to_string()), Some("password2".to_string()));
        let credentials3 =
            Credentials::basic(Some("username3".to_string()), Some("password3".to_string()));
        let credentials4 =
            Credentials::basic(Some("username4".to_string()), Some("password4".to_string()));

        let mut trie = UrlTrie::new();
        trie.insert(
            &Url::parse("https://burntsushi.net").unwrap(),
            credentials1.clone(),
        );
        trie.insert(
            &Url::parse("https://astral.sh").unwrap(),
            credentials2.clone(),
        );
        trie.insert(
            &Url::parse("https://example.com/foo").unwrap(),
            credentials3.clone(),
        );
        trie.insert(
            &Url::parse("https://example.com/bar").unwrap(),
            credentials4.clone(),
        );

        let url = Url::parse("https://burntsushi.net/regex-internals").unwrap();
        assert_eq!(trie.get(&url), Some(&credentials1));

        let url = Url::parse("https://burntsushi.net/").unwrap();
        assert_eq!(trie.get(&url), Some(&credentials1));

        let url = Url::parse("https://astral.sh/about").unwrap();
        assert_eq!(trie.get(&url), Some(&credentials2));

        let url = Url::parse("https://example.com/foo").unwrap();
        assert_eq!(trie.get(&url), Some(&credentials3));

        let url = Url::parse("https://example.com/foo/").unwrap();
        assert_eq!(trie.get(&url), Some(&credentials3));

        let url = Url::parse("https://example.com/foo/bar").unwrap();
        assert_eq!(trie.get(&url), Some(&credentials3));

        let url = Url::parse("https://example.com/bar").unwrap();
        assert_eq!(trie.get(&url), Some(&credentials4));

        let url = Url::parse("https://example.com/bar/").unwrap();
        assert_eq!(trie.get(&url), Some(&credentials4));

        let url = Url::parse("https://example.com/bar/foo").unwrap();
        assert_eq!(trie.get(&url), Some(&credentials4));

        let url = Url::parse("https://example.com/about").unwrap();
        assert_eq!(trie.get(&url), None);

        let url = Url::parse("https://example.com/foobar").unwrap();
        assert_eq!(trie.get(&url), None);
    }

    #[test]
    fn test_trie_opaque_url() -> Result<(), ParseError> {
        let mut trie = UrlTrie::new();
        let url = Url::parse("git+https:foo")?;
        let credentials =
            Credentials::basic(Some("username".to_string()), Some("password".to_string()));

        assert_eq!(trie.get(&url), None);
        trie.insert(&url, credentials.clone());
        assert_eq!(trie.get(&url), None);

        // Opaque URLs must not share credentials with hierarchical URLs in the same realm.
        let base_url = Url::parse("git+https:/")?;
        assert_eq!(trie.get(&base_url), None);
        trie.insert(&base_url, credentials.clone());
        assert_eq!(trie.get(&url), None);
        assert_eq!(trie.get(&base_url), Some(&credentials));

        Ok(())
    }

    #[test]
    fn test_url_with_credentials() {
        let username = Username::new(Some(String::from("username")));
        let password = Password::new(String::from("password"));
        let credentials = Arc::new(Authentication::from(Credentials::Basic {
            username: username.clone(),
            password: Some(password),
        }));
        let cache = CredentialsCache::default();
        // Insert with URL with credentials and get with redacted URL.
        let url = DisplaySafeUrl::parse("https://username:password@example.com/foobar").unwrap();
        cache.insert(&url, credentials.clone());
        assert_eq!(cache.get_url(&url, &username), Some(credentials.clone()));
        // Insert with redacted URL and get with URL with credentials.
        let url =
            DisplaySafeUrl::parse("https://username:password@second-example.com/foobar").unwrap();
        cache.insert(&url, credentials.clone());
        assert_eq!(cache.get_url(&url, &username), Some(credentials.clone()));
    }
}
