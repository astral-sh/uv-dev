use std::borrow::Cow;
use std::fmt::{Display, Formatter};
use std::ops::{Deref, Range};
use std::path::Path;
use std::ptr;
use std::str::FromStr;
use std::sync::{Arc, LazyLock, RwLock};

use http::StatusCode;
use itertools::Either;
use rustc_hash::{FxHashMap, FxHashSet};
use thiserror::Error;
use url::ParseError;
use uv_auth::{AuthPolicy, Index as AuthIndex, RealmRef};
use uv_cache_key::CanonicalUrl;
use uv_pep508::{Scheme, VerbatimUrl, VerbatimUrlError, split_scheme};
use uv_pypi_types::HashAlgorithm;
use uv_redacted::SafeUrl;
use uv_redacted::policy::{SerializationPolicy, Stripped};
use uv_warnings::warn_user;

use crate::{
    ExcludeNewerOverride, Index, IndexMetadata, IndexName, IndexStatusCodeStrategy, Origin,
    Verbatim,
};

/// Full index definitions selected by requirements sources, validated before composition.
#[derive(Debug, Default, Clone)]
pub struct SourceIndexes(Vec<Index>);

/// Conflicting source-index definitions would discard a selected policy in index clients.
#[derive(Debug, Error)]
pub enum SourceIndexError {
    #[error("Conflicting definitions for index `{0}` in requirements sources")]
    ConflictingName(IndexName),
    #[error("Multiple default indexes in requirements sources")]
    MultipleDefaults,
    #[error(
        "A default index from requirements sources conflicts with the command-line default index"
    )]
    ConfiguredDefault,
    #[error(
        "Conflicting authentication policies for indexes {first} ({first_policy}) and {second} ({second_policy}): authentication scopes overlap"
    )]
    ConflictingAuthentication {
        first: Box<SourceIndexIdentity>,
        first_policy: AuthPolicy,
        second: Box<SourceIndexIdentity>,
        second_policy: AuthPolicy,
    },
    #[error("Conflicting policies for indexes {first} and {second}")]
    ConflictingUrl {
        first: Box<SourceIndexIdentity>,
        second: Box<SourceIndexIdentity>,
    },
    #[error("Index URL `{0}` pinned by requirements no longer has its configured policy")]
    MissingPinnedIndex(IndexUrl),
}

/// A conflicting index definition with credentials removed from its diagnostic URL.
#[derive(Debug)]
pub struct SourceIndexIdentity {
    pub name: Option<IndexName>,
    pub url: DisplaySafeUrl,
}

impl From<&Index> for SourceIndexIdentity {
    fn from(index: &Index) -> Self {
        Self {
            name: index.name.clone(),
            url: index.url.without_credentials().into_owned(),
        }
    }
}

impl Display for SourceIndexIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(name) = &self.name {
            write!(f, "`{name}` (`{}`)", self.url)
        } else {
            write!(f, "`{}`", self.url)
        }
    }
}

impl SourceIndexes {
    /// Construct a collection while rejecting conflicting selected policies.
    pub fn try_from_iter(
        indexes: impl IntoIterator<Item = Index>,
    ) -> Result<Self, SourceIndexError> {
        let mut result = Self::default();
        result.try_extend(indexes)?;
        Ok(result)
    }

    /// Merge definitions without losing policies to client name or default-index selection.
    pub fn try_extend(
        &mut self,
        indexes: impl IntoIterator<Item = Index>,
    ) -> Result<(), SourceIndexError> {
        for index in indexes {
            if let Some(name) = index.name.as_ref()
                && let Some(existing) = self
                    .0
                    .iter()
                    .find(|existing| existing.name.as_ref() == Some(name))
            {
                if existing != &index {
                    return Err(SourceIndexError::ConflictingName(name.clone()));
                }
                continue;
            }
            for existing in &self.0 {
                validate_source_index_policy(existing, &index)?;
                if index.default && existing.default && !is_same_index(&existing.url, &index.url) {
                    return Err(SourceIndexError::MultipleDefaults);
                }
            }
            self.0.push(index);
        }
        Ok(())
    }

    /// Merge source definitions into index options while retaining command-line precedence.
    ///
    /// Legacy index fields and implicit defaults are not part of this declaration list. Validate
    /// the complete client locations with [`IndexLocations::with_source_indexes`] before persisting
    /// these definitions.
    pub fn merge_index_definitions(self, indexes: Vec<Index>) -> Vec<Index> {
        let (indexes, _) = merge_source_index_definitions(indexes, self);
        let (non_default, default) = indexes
            .into_iter()
            .partition::<Vec<_>, _>(|index| !index.default);
        non_default.into_iter().chain(default).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn iter(&self) -> std::slice::Iter<'_, Index> {
        self.0.iter()
    }
}

/// Merge declarations in command-line, source, then configured order, retaining the first name.
fn merge_source_index_definitions(
    indexes: Vec<Index>,
    sources: SourceIndexes,
) -> (Vec<Index>, Range<usize>) {
    let (mut command_line, mut configured): (Vec<_>, Vec<_>) = indexes
        .into_iter()
        .partition(|index| index.origin == Some(Origin::Cli));
    let mut names = FxHashSet::default();
    let mut keep_first_name = |index: &Index| {
        index
            .name
            .as_ref()
            .is_none_or(|name| names.insert(name.clone()))
    };
    command_line.retain(&mut keep_first_name);
    let sources = sources
        .into_iter()
        .filter(&mut keep_first_name)
        .collect::<Vec<_>>();
    configured.retain(keep_first_name);
    let source_range = command_line.len()..command_line.len() + sources.len();
    let indexes = command_line
        .into_iter()
        .chain(sources)
        .chain(configured)
        .collect();
    (indexes, source_range)
}

/// Authentication scopes and endpoint-based policies must agree across selected definitions.
fn validate_source_index_policy(existing: &Index, index: &Index) -> Result<(), SourceIndexError> {
    let conflicting_authentication = existing.authenticate != index.authenticate
        && AuthIndex::from(existing).has_overlapping_auth_scope(&AuthIndex::from(index));
    if conflicting_authentication {
        return Err(SourceIndexError::ConflictingAuthentication {
            first: Box::new(existing.into()),
            first_policy: existing.authenticate,
            second: Box::new(index.into()),
            second_policy: index.authenticate,
        });
    }
    if is_same_index(&existing.url, &index.url) && !existing.has_same_policy(index) {
        return Err(SourceIndexError::ConflictingUrl {
            first: Box::new(existing.into()),
            second: Box::new(index.into()),
        });
    }
    Ok(())
}

/// Ensure a selected endpoint and its policies remain in the effective client configuration.
fn validate_retained_index_policy(
    index: &Index,
    url: &IndexUrl,
    retained_indexes: &[&Index],
) -> Result<(), SourceIndexError> {
    if !retained_indexes
        .iter()
        .any(|retained| is_same_index(&retained.url, url))
    {
        return Err(SourceIndexError::MissingPinnedIndex(url.clone()));
    }
    for retained in retained_indexes {
        validate_source_index_policy(index, retained)?;
    }
    Ok(())
}

impl<'a> IntoIterator for &'a SourceIndexes {
    type Item = &'a Index;
    type IntoIter = std::slice::Iter<'a, Index>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for SourceIndexes {
    type Item = Index;
    type IntoIter = std::vec::IntoIter<Index>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

pub static PYPI_URL: LazyLock<SafeUrl> =
    LazyLock::new(|| SafeUrl::parse("https://pypi.org/simple").unwrap());

static DEFAULT_INDEX: LazyLock<Index> = LazyLock::new(|| {
    Index::from_index_url(IndexUrl::Pypi(Arc::new(VerbatimUrl::from_url(
        PYPI_URL.clone().with_policy(Stripped),
    ))))
});

/// The URL of an index to use for fetching packages (e.g., PyPI).
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum IndexUrl {
    Pypi(Arc<VerbatimUrl<Stripped>>),
    Url(Arc<VerbatimUrl<Stripped>>),
    Path(Arc<VerbatimUrl<Stripped>>),
}

impl IndexUrl {
    /// Parse an [`IndexUrl`] from a string, relative to an optional root directory.
    ///
    /// If no root directory is provided, relative paths are resolved against the current working
    /// directory.
    pub fn parse(path: &str, root_dir: Option<&Path>) -> Result<Self, IndexUrlError> {
        let url = VerbatimUrl::from_url_or_path(path, root_dir)?;
        Ok(Self::from(url))
    }

    /// Return the root [`SafeUrl`] of the index, if applicable.
    ///
    /// For indexes with a `/simple` endpoint, this is simply the URL with the final segment
    /// removed. This is useful, e.g., for credential propagation to other endpoints on the index.
    pub fn root(&self) -> Option<SafeUrl> {
        let mut segments = self.url().path_segments()?;
        let last = match segments.next_back()? {
            // If the last segment is empty due to a trailing `/`, skip it (as in `pop_if_empty`)
            "" => segments.next_back()?,
            segment => segment,
        };

        // We also handle `/+simple` as it's used in devpi
        if !(last.eq_ignore_ascii_case("simple") || last.eq_ignore_ascii_case("+simple")) {
            return None;
        }

        let mut url = self.url().clone();
        url.path_segments_mut().ok()?.pop_if_empty().pop();
        Some(url)
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for IndexUrl {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("IndexUrl")
    }

    fn json_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "The URL of an index to use for fetching packages (e.g., `https://pypi.org/simple`), or a local path."
        })
    }
}

impl IndexUrl {
    #[inline]
    fn inner(&self) -> &VerbatimUrl<Stripped> {
        match self {
            Self::Pypi(url) | Self::Url(url) | Self::Path(url) => url,
        }
    }

    /// Return the raw URL for the index.
    pub fn url(&self) -> &SafeUrl {
        self.inner().raw().as_url()
    }

    /// Convert the index URL into a [`SafeUrl`].
    pub fn into_url(self) -> SafeUrl<Stripped> {
        match self {
            Self::Pypi(url) | Self::Url(url) | Self::Path(url) => {
                Arc::unwrap_or_clone(url).into_url()
            }
        }
    }

    /// Return the redacted URL for the index, omitting any sensitive credentials.
    pub fn without_credentials(&self) -> Cow<'_, SafeUrl> {
        let url = self.url();
        if url.username().is_empty() && url.password().is_none() {
            Cow::Borrowed(url)
        } else {
            let mut url = url.clone();
            let _ = url.set_username("");
            let _ = url.set_password(None);
            Cow::Owned(url)
        }
    }

    /// Warn user if the given URL was provided as an ambiguous relative path.
    ///
    /// This is a temporary warning. Ambiguous values will not be
    /// accepted in the future.
    pub fn warn_on_disambiguated_relative_path(&self) {
        let Self::Path(verbatim_url) = &self else {
            return;
        };

        if let Some(path) = verbatim_url.given()
            && !is_disambiguated_path(path)
        {
            if cfg!(windows) {
                warn_user!(
                    "Relative paths passed to `--index` or `--default-index` should be disambiguated from index names (use `.\\{path}` or `./{path}`). Support for ambiguous values will be removed in the future"
                );
            } else {
                warn_user!(
                    "Relative paths passed to `--index` or `--default-index` should be disambiguated from index names (use `./{path}`). Support for ambiguous values will be removed in the future"
                );
            }
        }
    }
}

impl Display for IndexUrl {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.inner(), f)
    }
}

impl Verbatim for IndexUrl {
    fn verbatim(&self) -> Cow<'_, str> {
        self.inner().verbatim()
    }
}

/// Checks if a path is disambiguated.
///
/// Disambiguated paths are absolute paths, paths with valid schemes,
/// and paths starting with "./" or "../" on Unix or ".\\", "..\\",
/// "./", or "../" on Windows.
fn is_disambiguated_path(path: &str) -> bool {
    if cfg!(windows) {
        if path.starts_with(".\\") || path.starts_with("..\\") || path.starts_with('/') {
            return true;
        }
    }
    if path.starts_with("./") || path.starts_with("../") || Path::new(path).is_absolute() {
        return true;
    }
    // Check if the path has a scheme (like `file://`)
    if let Some((scheme, _)) = split_scheme(path) {
        return Scheme::parse(scheme).is_some();
    }
    // This is an ambiguous relative path
    false
}

/// An error that can occur when parsing an [`IndexUrl`].
#[derive(Error, Debug)]
pub enum IndexUrlError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Url(#[from] ParseError),
    #[error(transparent)]
    VerbatimUrl(#[from] VerbatimUrlError),
}

impl FromStr for IndexUrl {
    type Err = IndexUrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s, None)
    }
}

impl serde::ser::Serialize for IndexUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        self.inner().serialize(serializer)
    }
}

impl<'de> serde::de::Deserialize<'de> for IndexUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = IndexUrl;

            fn expecting(&self, f: &mut Formatter) -> std::fmt::Result {
                f.write_str("a string")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                IndexUrl::from_str(v).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

impl<P: SerializationPolicy> From<VerbatimUrl<P>> for IndexUrl {
    fn from(url: VerbatimUrl<P>) -> Self {
        let url = url.with_policy(Stripped);
        if url.scheme() == "file" {
            Self::Path(Arc::new(url))
        } else if *url.raw().as_url() == *PYPI_URL {
            Self::Pypi(Arc::new(url))
        } else {
            Self::Url(Arc::new(url))
        }
    }
}

impl From<IndexUrl> for SafeUrl {
    fn from(index: IndexUrl) -> Self {
        index.into_url().into_url()
    }
}

impl Deref for IndexUrl {
    type Target = SafeUrl;

    fn deref(&self) -> &Self::Target {
        self.url()
    }
}

/// The index locations to use for fetching packages. By default, uses the PyPI index.
///
/// This type merges the legacy `--index-url`, `--extra-index-url`, and `--find-links` options,
/// along with the uv-specific `--index` and `--default-index`.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct IndexLocations {
    indexes: Vec<Index>,
    flat_index: Vec<Index>,
    no_index: bool,
}

impl IndexLocations {
    /// Determine the index URLs to use for fetching packages.
    pub fn new(indexes: Vec<Index>, flat_index: Vec<Index>, no_index: bool) -> Self {
        Self {
            indexes,
            flat_index,
            no_index,
        }
    }

    /// Combine a set of index locations.
    ///
    /// If either the current or the other index locations have `no_index` set, the result will
    /// have `no_index` set.
    ///
    /// If the current index location has an `index` set, it will be preserved.
    #[must_use]
    pub fn combine(self, indexes: Vec<Index>, flat_index: Vec<Index>, no_index: bool) -> Self {
        Self {
            indexes: self.indexes.into_iter().chain(indexes).collect(),
            flat_index: self.flat_index.into_iter().chain(flat_index).collect(),
            no_index: self.no_index || no_index,
        }
    }

    /// Ensure persisted index pins retain the policies they selected before settings overrides.
    pub fn validate_pinned_indexes<'index>(
        &self,
        original: &Self,
        pins: impl IntoIterator<Item = &'index IndexMetadata>,
    ) -> Result<(), SourceIndexError> {
        let original_indexes = original.allowed_indexes();
        let retained_indexes = self.allowed_indexes();
        for pin in pins {
            let Some(original) = original_indexes
                .iter()
                .rev()
                .find(|index| is_same_index(&index.url, &pin.url))
            else {
                continue;
            };
            validate_retained_index_policy(original, &pin.url, &retained_indexes)?;
        }
        Ok(())
    }

    /// Validate selected definitions without reordering an already composed client configuration.
    ///
    /// Disabled source indexes do not contribute policies to the client configuration.
    pub fn validate_source_indexes(&self, indexes: &SourceIndexes) -> Result<(), SourceIndexError> {
        if self.no_index || indexes.is_empty() {
            return Ok(());
        }
        let retained_indexes = self.allowed_indexes();
        for index in indexes {
            validate_retained_index_policy(index, &index.url, &retained_indexes)?;
        }
        Ok(())
    }

    /// Add index definitions from sources, retaining command-line precedence.
    pub fn with_source_indexes(mut self, indexes: SourceIndexes) -> Result<Self, SourceIndexError> {
        if indexes.is_empty() {
            return Ok(self);
        }
        let source_defaults = indexes
            .iter()
            .filter(|index| index.default)
            .cloned()
            .collect::<Vec<_>>();
        let (definitions, source_range) = merge_source_index_definitions(self.indexes, indexes);
        self.indexes = definitions;

        let allowed_indexes = self.allowed_indexes();
        // Source defaults must retain a compatible endpoint when a command-line default wins.
        if !self.no_index
            && allowed_indexes
                .iter()
                .any(|index| index.default && index.origin == Some(Origin::Cli))
        {
            for index in &source_defaults {
                if !allowed_indexes
                    .iter()
                    .any(|existing| is_same_index(&existing.url, &index.url))
                {
                    return Err(SourceIndexError::ConfiguredDefault);
                }
                for existing in &allowed_indexes {
                    validate_source_index_policy(existing, index)?;
                }
            }
        }
        // Compare retained source definitions against the complete client set.
        for index in &self.indexes[source_range] {
            if !allowed_indexes
                .iter()
                .any(|existing| ptr::eq(*existing, index))
            {
                continue;
            }
            for existing in &allowed_indexes {
                validate_source_index_policy(existing, index)?;
            }
        }
        Ok(self)
    }

    /// Returns `true` if no index configuration is set, i.e., the [`IndexLocations`] matches the
    /// default configuration.
    pub fn is_none(&self) -> bool {
        *self == Self::default()
    }
}

/// Returns `true` if two [`IndexUrl`]s refer to the same index.
fn is_same_index(a: &IndexUrl, b: &IndexUrl) -> bool {
    RealmRef::from(b.url()) == RealmRef::from(a.url())
        && CanonicalUrl::new(a.url().clone()) == CanonicalUrl::new(b.url().clone())
}

impl<'a> IndexLocations {
    /// Return configured indexes in definition order, keeping the first index for each name.
    fn configured_indexes(&'a self) -> impl Iterator<Item = &'a Index> + 'a {
        let mut seen = FxHashSet::default();
        self.indexes
            .iter()
            .filter(move |index| index.name.as_ref().is_none_or(|name| seen.insert(name)))
    }

    /// Return the default [`Index`] entry.
    ///
    /// If `--no-index` is set, return `None`.
    ///
    /// If no index is provided, use the `PyPI` index.
    pub fn default_index(&'a self) -> Option<&'a Index> {
        if self.no_index {
            None
        } else {
            self.configured_indexes()
                .find(|index| index.default)
                .or_else(|| Some(&DEFAULT_INDEX))
        }
    }

    /// Return an iterator over the implicit [`Index`] entries.
    ///
    /// Default and explicit indexes are excluded.
    pub fn implicit_indexes(&'a self) -> impl Iterator<Item = &'a Index> + 'a {
        if self.no_index {
            Either::Left(std::iter::empty())
        } else {
            Either::Right(
                self.configured_indexes()
                    .filter(|index| !index.default && !index.explicit),
            )
        }
    }

    /// Return an iterator over the explicit [`Index`] entries.
    ///
    /// Explicit indexes are only used when pinned via `tool.uv.sources`.
    pub fn explicit_indexes(&'a self) -> impl Iterator<Item = &'a Index> + 'a {
        if self.no_index {
            Either::Left(std::iter::empty())
        } else {
            Either::Right(self.configured_indexes().filter(|index| index.explicit))
        }
    }

    /// Return an iterator over all [`Index`] entries in order.
    ///
    /// Explicit indexes are excluded.
    ///
    /// Prioritizes the extra indexes over the default index.
    ///
    /// If `no_index` was enabled, then this always returns an empty
    /// iterator.
    pub fn indexes(&'a self) -> impl Iterator<Item = &'a Index> + 'a {
        self.implicit_indexes()
            .chain(self.default_index())
            .filter(|index| !index.explicit)
    }

    /// Return an iterator over all [`Index`] entries to fetch in order.
    ///
    /// Unlike [`IndexLocations::indexes`], indexes with duplicate raw URLs are excluded.
    pub fn fetch_indexes(&'a self) -> impl Iterator<Item = &'a Index> + 'a {
        let mut seen = FxHashSet::default();
        self.indexes()
            .filter(move |index| seen.insert(index.raw_url()))
    }

    /// Return an iterator over all simple [`Index`] entries in order.
    ///
    /// If `no_index` was enabled, then this always returns an empty iterator.
    pub fn simple_indexes(&'a self) -> impl Iterator<Item = &'a Index> + 'a {
        if self.no_index {
            Either::Left(std::iter::empty())
        } else {
            Either::Right(self.configured_indexes())
        }
    }

    /// Return an iterator over the flat [`Index`] entries.
    pub fn flat_indexes(&'a self) -> impl Iterator<Item = &'a Index> + 'a {
        self.flat_index.iter()
    }

    /// Return the `--no-index` flag.
    pub fn no_index(&self) -> bool {
        self.no_index
    }

    /// Return a vector containing all allowed [`Index`] entries.
    ///
    /// This includes explicit indexes, implicit indexes, flat indexes, and the default index.
    ///
    /// The indexes will be returned in the reverse of the order in which they were defined, such
    /// that the last-defined index is the first item in the vector.
    pub fn allowed_indexes(&'a self) -> Vec<&'a Index> {
        let mut indexes = self.effective_indexes().collect::<Vec<_>>();
        indexes.reverse();
        indexes
    }

    /// Return client definitions in declaration order, excluding disabled or shadowed entries.
    fn effective_indexes(&'a self) -> impl Iterator<Item = &'a Index> + 'a {
        if self.no_index {
            Either::Left(self.flat_index.iter())
        } else {
            let mut seen = FxHashSet::default();
            let mut default = false;
            Either::Right(
                self.indexes
                    .iter()
                    .chain(self.flat_index.iter())
                    .chain(std::iter::once(&*DEFAULT_INDEX))
                    .filter(move |index| {
                        if index.name.as_ref().is_some_and(|name| !seen.insert(name)) {
                            return false;
                        }
                        if index.default {
                            if default {
                                return false;
                            }
                            default = true;
                        }
                        true
                    }),
            )
        }
    }

    /// Return a vector containing all known [`Index`] entries.
    ///
    /// This includes explicit indexes, implicit indexes, flat indexes, and default indexes;
    /// in short, it includes all defined indexes, even if they're overridden by some other index
    /// definition.
    ///
    /// The indexes will be returned in the reverse of the order in which they were defined, such
    /// that the last-defined index is the first item in the vector.
    pub fn known_indexes(&'a self) -> impl Iterator<Item = &'a Index> {
        if self.no_index {
            Either::Left(self.flat_index.iter().rev())
        } else {
            Either::Right(
                std::iter::once(&*DEFAULT_INDEX)
                    .chain(self.flat_index.iter().rev())
                    .chain(self.indexes.iter().rev()),
            )
        }
    }

    /// Return an iterator over all user-defined [`Index`] entries in order.
    ///
    /// Prioritizes the `[tool.uv.index]` definitions over the `--extra-index-url` definitions
    /// over the `--index-url` definition.
    ///
    /// Unlike [`IndexLocations::indexes`], this includes explicit indexes and does _not_ insert
    /// PyPI as a fallback default.
    ///
    /// If `no_index` was enabled, then this always returns an empty iterator.
    pub fn defined_indexes(&'a self) -> impl Iterator<Item = &'a Index> + 'a {
        if self.no_index {
            return Either::Left(std::iter::empty());
        }

        let (non_default, default) = self
            .configured_indexes()
            .partition::<Vec<_>, _>(|index| !index.default);

        Either::Right(non_default.into_iter().chain(default))
    }

    /// Return the first effective client definition matching the given URL.
    fn index_for_url(&self, url: &IndexUrl) -> Option<&Index> {
        self.effective_indexes()
            .find(|index| is_same_index(index.url(), url))
    }

    /// Return the [`IndexStatusCodeStrategy`] for an [`IndexUrl`].
    pub fn status_code_strategy_for(&self, url: &IndexUrl) -> IndexStatusCodeStrategy {
        self.index_for_url(url).map_or(
            IndexStatusCodeStrategy::Default,
            Index::status_code_strategy,
        )
    }

    /// Return whether the given status code is explicitly ignored for an [`IndexUrl`].
    pub fn ignores_error_code_for(&self, url: &IndexUrl, status_code: StatusCode) -> bool {
        self.index_for_url(url)
            .is_some_and(|index| index.ignores_error_code(status_code))
    }

    /// Return the Simple API cache control header for an [`IndexUrl`], if configured.
    pub fn simple_api_cache_control_for(&self, url: &IndexUrl) -> Option<http::HeaderValue> {
        self.index_for_url(url)
            .and_then(Index::simple_api_cache_control)
    }

    /// Return the artifact cache control header for an [`IndexUrl`], if configured.
    pub fn artifact_cache_control_for(&self, url: &IndexUrl) -> Option<http::HeaderValue> {
        self.index_for_url(url)
            .and_then(Index::artifact_cache_control)
    }

    /// Return the hash algorithm required for distributions resolved from a given index.
    pub fn hash_algorithm_for(&self, url: &IndexUrl) -> Option<HashAlgorithm> {
        self.index_for_url(url)
            .and_then(|index| index.hash_algorithm.map(HashAlgorithm::from))
    }

    /// Return the `exclude-newer` setting for a given index, if the index is configured.
    pub fn exclude_newer_for(&self, url: &IndexUrl) -> Option<&ExcludeNewerOverride> {
        self.index_for_url(url).and_then(Index::exclude_newer)
    }
}

impl From<&Index> for AuthIndex {
    fn from(index: &Index) -> Self {
        let mut url = index.url().url().clone();
        url.set_username("").ok();
        url.set_password(None).ok();
        let mut root_url = index.url().root().unwrap_or_else(|| url.clone());
        root_url.set_username("").ok();
        root_url.set_password(None).ok();
        Self {
            url,
            root_url,
            auth_policy: index.authenticate,
        }
    }
}

impl From<&IndexLocations> for uv_auth::Indexes {
    fn from(index_locations: &IndexLocations) -> Self {
        Self::from_indexes(
            index_locations
                .allowed_indexes()
                .into_iter()
                .map(AuthIndex::from),
        )
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone)]
    struct Flags: u8 {
        /// Whether the index supports range requests.
        const NO_RANGE_REQUESTS = 1;
        /// Whether the index returned a `401 Unauthorized` status code.
        const UNAUTHORIZED      = 1 << 2;
        /// Whether the index returned a `403 Forbidden` status code.
        const FORBIDDEN         = 1 << 1;
    }
}

/// A map of [`IndexUrl`]s to their capabilities.
///
/// We only store indexes that lack capabilities (i.e., don't support range requests, aren't
/// authorized). The benefit is that the map is almost always empty, so validating capabilities is
/// extremely cheap.
#[derive(Debug, Default, Clone)]
pub struct IndexCapabilities(Arc<RwLock<FxHashMap<IndexUrl, Flags>>>);

impl IndexCapabilities {
    /// Returns `true` if the given [`IndexUrl`] supports range requests.
    pub fn supports_range_requests(&self, index_url: &IndexUrl) -> bool {
        !self
            .0
            .read()
            .unwrap()
            .get(index_url)
            .is_some_and(|flags| flags.intersects(Flags::NO_RANGE_REQUESTS))
    }

    /// Mark an [`IndexUrl`] as not supporting range requests.
    pub fn set_no_range_requests(&self, index_url: IndexUrl) {
        self.0
            .write()
            .unwrap()
            .entry(index_url)
            .or_insert(Flags::empty())
            .insert(Flags::NO_RANGE_REQUESTS);
    }

    /// Returns `true` if the given [`IndexUrl`] returns a `401 Unauthorized` status code.
    pub fn unauthorized(&self, index_url: &IndexUrl) -> bool {
        self.0
            .read()
            .unwrap()
            .get(index_url)
            .is_some_and(|flags| flags.intersects(Flags::UNAUTHORIZED))
    }

    /// Mark an [`IndexUrl`] as returning a `401 Unauthorized` status code.
    pub(crate) fn set_unauthorized(&self, index_url: IndexUrl) {
        self.0
            .write()
            .unwrap()
            .entry(index_url)
            .or_insert(Flags::empty())
            .insert(Flags::UNAUTHORIZED);
    }

    /// Returns `true` if the given [`IndexUrl`] returns a `403 Forbidden` status code.
    pub fn forbidden(&self, index_url: &IndexUrl) -> bool {
        self.0
            .read()
            .unwrap()
            .get(index_url)
            .is_some_and(|flags| flags.intersects(Flags::FORBIDDEN))
    }

    /// Mark an [`IndexUrl`] as returning a `403 Forbidden` status code.
    pub(crate) fn set_forbidden(&self, index_url: IndexUrl) {
        self.0
            .write()
            .unwrap()
            .entry(index_url)
            .or_insert(Flags::empty())
            .insert(Flags::FORBIDDEN);
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::{IndexCacheControl, IndexFormat, IndexHashAlgorithm, IndexName};
    use http::HeaderValue;
    use uv_auth::AuthPolicy;

    fn index_urls<'a>(indexes: impl IntoIterator<Item = &'a Index>) -> Vec<&'a str> {
        indexes
            .into_iter()
            .map(|index| index.url().url().as_str())
            .collect()
    }

    #[test]
    fn selected_indexes_reject_shadowed_default() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://private.example/simple")?;
        selected.default = true;
        selected.exclude_newer = Some(ExcludeNewerOverride::from_str("2023-06-01T00:00:00Z")?);
        let command_line =
            Index::from_index_url(IndexUrl::from_str("https://default.example/simple")?)
                .with_origin(Origin::Cli);
        let locations =
            IndexLocations::new(vec![command_line, selected.clone()], Vec::new(), false);
        let sources = SourceIndexes::try_from_iter([selected])?;

        assert_eq!(
            locations.validate_source_indexes(&sources).err().map(|err| err.to_string()),
            Some("Index URL `https://private.example/simple` pinned by requirements no longer has its configured policy".to_owned())
        );
        Ok(())
    }

    #[test]
    fn selected_indexes_accept_retained_default_alias() -> Result<(), Box<dyn Error>> {
        let cutoff = ExcludeNewerOverride::from_str("2023-06-01T00:00:00Z")?;
        let mut selected = Index::from_str("private=https://private.example/simple")?;
        selected.default = true;
        selected.exclude_newer = Some(cutoff.clone());
        let url = selected.url.clone();
        let mut alias = Index::from_str("mirror=https://private.example/simple")?;
        alias.exclude_newer = Some(cutoff.clone());
        let command_line =
            Index::from_index_url(IndexUrl::from_str("https://default.example/simple")?)
                .with_origin(Origin::Cli);
        let locations = IndexLocations::new(
            vec![command_line, selected.clone(), alias],
            Vec::new(),
            false,
        );
        let sources = SourceIndexes::try_from_iter([selected])?;

        locations.validate_source_indexes(&sources)?;
        assert_eq!(locations.exclude_newer_for(&url), Some(&cutoff));
        Ok(())
    }

    #[test]
    fn selected_indexes_reject_incompatible_retained_alias() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://private.example/simple")?;
        selected.default = true;
        selected.exclude_newer = Some(ExcludeNewerOverride::from_str("2023-06-01T00:00:00Z")?);
        let alias = Index::from_str("mirror=https://private.example/simple")?;
        let command_line =
            Index::from_index_url(IndexUrl::from_str("https://default.example/simple")?)
                .with_origin(Origin::Cli);
        let locations = IndexLocations::new(
            vec![command_line, selected.clone(), alias],
            Vec::new(),
            false,
        );
        let sources = SourceIndexes::try_from_iter([selected])?;

        assert_eq!(
            locations.validate_source_indexes(&sources).err().map(|err| err.to_string()),
            Some("Conflicting policies for indexes `private` (`https://private.example/simple`) and `mirror` (`https://private.example/simple`)".to_owned())
        );
        Ok(())
    }

    #[test]
    fn selected_indexes_ignore_disabled_definitions() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/links")?;
        selected.authenticate = AuthPolicy::Always;
        let flat = Index::from_find_links(selected.url.clone());
        let locations = IndexLocations::new(vec![selected.clone()], vec![flat], true);
        let sources = SourceIndexes::try_from_iter([selected])?;

        locations.validate_source_indexes(&sources)?;
        Ok(())
    }

    #[test]
    fn index_policy_lookup_ignores_shadowed_defaults() -> Result<(), Box<dyn Error>> {
        let command_line =
            Index::from_index_url(IndexUrl::from_str("https://default.example/simple")?)
                .with_origin(Origin::Cli);
        let url = IndexUrl::from_str("https://private.example/simple")?;
        let shadowed = Index::from_index_url(url.clone()).with_origin(Origin::Cli);
        let cutoff = ExcludeNewerOverride::from_str("2023-06-01T00:00:00Z")?;
        let mut selected = Index::from_str("private=https://private.example/simple")?;
        selected.explicit = true;
        selected.exclude_newer = Some(cutoff.clone());
        selected.hash_algorithm = Some(IndexHashAlgorithm::Sha256);
        selected.ignore_error_codes = Some(serde_json::from_str("[403]")?);
        selected.cache_control = Some(IndexCacheControl {
            api: Some(HeaderValue::from_static("max-age=300")),
            files: Some(HeaderValue::from_static("max-age=1800")),
        });

        let locations = IndexLocations::new(vec![command_line, shadowed], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        assert_eq!(locations.exclude_newer_for(&url), Some(&cutoff));
        assert_eq!(
            locations.hash_algorithm_for(&url),
            Some(HashAlgorithm::Sha256)
        );
        assert!(locations.ignores_error_code_for(&url, StatusCode::FORBIDDEN));
        assert_eq!(
            locations.status_code_strategy_for(&url),
            IndexStatusCodeStrategy::IgnoreErrorCodes {
                status_codes: FxHashSet::from_iter([StatusCode::FORBIDDEN]),
            }
        );
        assert_eq!(
            locations.simple_api_cache_control_for(&url),
            Some(HeaderValue::from_static("max-age=300"))
        );
        assert_eq!(
            locations.artifact_cache_control_for(&url),
            Some(HeaderValue::from_static("max-age=1800"))
        );
        Ok(())
    }

    #[test]
    fn index_policy_lookup_retains_first_effective_alias() -> Result<(), Box<dyn Error>> {
        let url = IndexUrl::from_str("https://example.com/simple")?;
        let mut first = Index::from_str("first=https://example.com/simple")?;
        first.cache_control = Some(IndexCacheControl {
            api: Some(HeaderValue::from_static("max-age=300")),
            files: None,
        });
        let mut second = Index::from_str("second=https://example.com/simple")?;
        second.cache_control = Some(IndexCacheControl {
            api: Some(HeaderValue::from_static("max-age=600")),
            files: None,
        });
        let locations = IndexLocations::new(vec![first, second], Vec::new(), false);

        assert_eq!(
            locations.simple_api_cache_control_for(&url),
            Some(HeaderValue::from_static("max-age=300"))
        );
        Ok(())
    }

    #[test]
    fn index_policy_lookup_uses_retained_flat_policy() -> Result<(), Box<dyn Error>> {
        let url = IndexUrl::from_str("https://example.com/links")?;
        let mut flat = Index::from_find_links(url.clone());
        flat.cache_control = Some(IndexCacheControl {
            api: None,
            files: Some(HeaderValue::from_static("max-age=1800")),
        });
        let locations = IndexLocations::new(Vec::new(), vec![flat], false);

        assert_eq!(
            locations.artifact_cache_control_for(&url),
            Some(HeaderValue::from_static("max-age=1800"))
        );
        Ok(())
    }

    #[test]
    fn index_policy_lookup_ignores_disabled_simple_indexes() -> Result<(), Box<dyn Error>> {
        let url = IndexUrl::from_str("https://example.com/links")?;
        let mut disabled = Index::from_str("disabled=https://example.com/links")?;
        disabled.cache_control = Some(IndexCacheControl {
            api: None,
            files: Some(HeaderValue::from_static("max-age=300")),
        });
        let mut flat = Index::from_find_links(url.clone());
        flat.cache_control = Some(IndexCacheControl {
            api: None,
            files: Some(HeaderValue::from_static("max-age=1800")),
        });
        let locations = IndexLocations::new(vec![disabled], vec![flat], true);

        assert_eq!(
            locations.artifact_cache_control_for(&url),
            Some(HeaderValue::from_static("max-age=1800"))
        );
        assert!(
            locations
                .index_for_url(&IndexUrl::from_str("https://pypi.org/simple")?)
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn source_defaults_accept_equivalent_command_line_default() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/simple")?;
        selected.default = true;
        let mut command_line = Index::from_index_url(selected.url.clone());
        command_line.origin = Some(Origin::Cli);

        let locations = IndexLocations::new(vec![command_line], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        assert_eq!(
            index_urls(locations.allowed_indexes()),
            ["https://example.com/simple"]
        );
        Ok(())
    }

    #[test]
    fn source_defaults_retain_endpoint_through_alias() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://private.example.com/simple")?;
        selected.default = true;
        selected.authenticate = AuthPolicy::Always;
        let mut alias = Index::from_str("mirror=https://private.example.com/simple")?;
        alias.authenticate = AuthPolicy::Always;
        alias.origin = Some(Origin::Cli);
        let mut command_line =
            Index::from_index_url(IndexUrl::from_str("https://default.example.com/simple")?);
        command_line.origin = Some(Origin::Cli);

        let locations = IndexLocations::new(vec![command_line, alias], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        assert_eq!(
            index_urls(locations.allowed_indexes()),
            [
                "https://private.example.com/simple",
                "https://default.example.com/simple"
            ]
        );
        Ok(())
    }

    #[test]
    fn source_defaults_reject_incompatible_retained_alias() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://private.example.com/simple")?;
        selected.default = true;
        selected.authenticate = AuthPolicy::Always;
        let mut alias = Index::from_str("mirror=https://private.example.com/simple")?;
        alias.origin = Some(Origin::Cli);
        let mut command_line =
            Index::from_index_url(IndexUrl::from_str("https://default.example.com/simple")?);
        command_line.origin = Some(Origin::Cli);

        let result = IndexLocations::new(vec![command_line, alias], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?);
        assert_eq!(
            result.err().map(|err| err.to_string()),
            Some("Conflicting authentication policies for indexes `mirror` (`https://private.example.com/simple`) (auto) and `private` (`https://private.example.com/simple`) (always): authentication scopes overlap".to_owned())
        );
        Ok(())
    }

    #[test]
    fn source_defaults_ignore_shadowed_command_line_default() -> Result<(), Box<dyn Error>> {
        let mut command_line = Index::from_str("shared=https://example.com/primary")?;
        command_line.origin = Some(Origin::Cli);
        let mut shadowed = Index::from_str("shared=https://example.com/unused")?;
        shadowed.default = true;
        shadowed.origin = Some(Origin::Cli);
        let mut selected = Index::from_str("private=https://example.com/selected")?;
        selected.default = true;

        let locations = IndexLocations::new(vec![command_line, shadowed], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        assert_eq!(
            index_urls(locations.default_index()),
            ["https://example.com/selected"]
        );
        Ok(())
    }

    #[test]
    fn source_defaults_ignore_disabled_command_line_default() -> Result<(), Box<dyn Error>> {
        let mut command_line =
            Index::from_index_url(IndexUrl::from_str("https://example.com/unused")?);
        command_line.origin = Some(Origin::Cli);
        let mut selected = Index::from_str("private=https://example.com/selected")?;
        selected.default = true;
        selected.authenticate = AuthPolicy::Always;

        let locations = IndexLocations::new(vec![command_line], Vec::new(), true)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        assert!(locations.allowed_indexes().is_empty());
        Ok(())
    }

    #[test]
    fn source_index_definitions_preserve_receipt_precedence() -> Result<(), Box<dyn Error>> {
        let mut command_line_default = Index::from_str("cli-default=https://example.com/default")?;
        command_line_default.default = true;
        command_line_default.origin = Some(Origin::Cli);
        let mut command_line = Index::from_str("shared=https://example.com/cli")?;
        command_line.origin = Some(Origin::Cli);
        let configured = Index::from_str("configured=https://example.com/configured")?;
        let mut configured_default =
            Index::from_str("configured-default=https://example.com/unused")?;
        configured_default.default = true;
        let indexes = vec![
            command_line_default,
            configured,
            command_line,
            configured_default,
        ];
        let mut source_default = Index::from_str("source-default=https://example.com/default")?;
        source_default.default = true;
        let sources = SourceIndexes::try_from_iter([
            source_default,
            Index::from_str("source=https://example.com/source")?,
            Index::from_str("shared=https://example.com/shadowed")?,
        ])?;
        let effective = IndexLocations::new(indexes.clone(), Vec::new(), false)
            .with_source_indexes(sources.clone())?;
        let definitions = sources.merge_index_definitions(indexes);

        assert_eq!(
            definitions,
            effective.defined_indexes().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            definitions
                .iter()
                .map(|index| index.name.as_ref().map(ToString::to_string))
                .collect::<Vec<_>>(),
            [
                Some("shared".to_owned()),
                Some("source".to_owned()),
                Some("configured".to_owned()),
                Some("cli-default".to_owned()),
                Some("source-default".to_owned()),
                Some("configured-default".to_owned()),
            ]
        );
        Ok(())
    }

    #[test]
    fn source_indexes_reject_overlapping_auth_scopes() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/simple")?;
        selected.authenticate = AuthPolicy::Always;
        let alias = Index::from_str("mirror=https://example.com/+simple")?;

        let result = IndexLocations::new(vec![alias], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?);
        assert_eq!(
            result.err().map(|err| err.to_string()),
            Some("Conflicting authentication policies for indexes `mirror` (`https://example.com/+simple`) (auto) and `private` (`https://example.com/simple`) (always): authentication scopes overlap".to_owned())
        );
        Ok(())
    }

    #[test]
    fn source_indexes_reject_credentialed_auth_scope_conflict() -> Result<(), Box<dyn Error>> {
        let mut selected =
            Index::from_str("private=https://private-user:private-password@example.com/simple")?;
        selected.authenticate = AuthPolicy::Always;
        let alias =
            Index::from_str("mirror=https://mirror-user:mirror-password@example.com/+simple")?;
        let selected_auth = AuthIndex::from(&selected);
        let alias_auth = AuthIndex::from(&alias);
        assert_eq!(selected_auth.url.as_str(), "https://example.com/simple");
        assert_eq!(selected_auth.root_url.as_str(), "https://example.com/");
        assert_eq!(alias_auth.url.as_str(), "https://example.com/+simple");
        assert_eq!(alias_auth.root_url.as_str(), "https://example.com/");

        let result = IndexLocations::new(vec![alias], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?);
        let error = result.expect_err("overlapping authentication scopes must conflict");
        assert_eq!(
            error.to_string(),
            "Conflicting authentication policies for indexes `mirror` (`https://example.com/+simple`) (auto) and `private` (`https://example.com/simple`) (always): authentication scopes overlap"
        );
        let SourceIndexError::ConflictingAuthentication { first, second, .. } = error else {
            return Err("expected an authentication policy conflict".into());
        };
        assert_eq!(first.name, Some(IndexName::from_str("mirror")?));
        assert_eq!(first.url.as_str(), "https://example.com/+simple");
        assert_eq!(second.name, Some(IndexName::from_str("private")?));
        assert_eq!(second.url.as_str(), "https://example.com/simple");
        Ok(())
    }

    #[test]
    fn source_indexes_allow_disjoint_auth_scopes() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/private/simple")?;
        selected.authenticate = AuthPolicy::Always;
        let public = Index::from_str("public=https://example.com/public/simple")?;

        IndexLocations::new(vec![public], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        Ok(())
    }

    #[test]
    fn source_indexes_keep_endpoint_policies_independent() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/simple")?;
        selected.ignore_error_codes = Some(serde_json::from_str("[403]")?);
        let alias = Index::from_str("mirror=https://example.com/+simple")?;

        IndexLocations::new(vec![alias], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        Ok(())
    }

    #[test]
    fn pinned_indexes_reject_conflicting_retained_alias() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/simple")?;
        selected.authenticate = AuthPolicy::Always;
        let pin = IndexMetadata::from(selected.url.clone());
        let original = IndexLocations::new(vec![selected.clone()], Vec::new(), false);
        let alias = Index::from_str("mirror=https://example.com/simple")?;
        let retained = IndexLocations::new(vec![selected, alias], Vec::new(), false);

        assert_eq!(
            retained.validate_pinned_indexes(&original, [&pin]).err().map(|err| err.to_string()),
            Some("Conflicting authentication policies for indexes `private` (`https://example.com/simple`) (always) and `mirror` (`https://example.com/simple`) (auto): authentication scopes overlap".to_owned())
        );
        Ok(())
    }

    #[test]
    fn pinned_indexes_reject_overlapping_auth_scope() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/simple")?;
        selected.authenticate = AuthPolicy::Always;
        let pin = IndexMetadata::from(selected.url.clone());
        let original = IndexLocations::new(vec![selected.clone()], Vec::new(), false);
        let alias = Index::from_str("mirror=https://example.com/+simple")?;
        let retained = IndexLocations::new(vec![selected, alias], Vec::new(), false);

        assert_eq!(
            retained.validate_pinned_indexes(&original, [&pin]).err().map(|err| err.to_string()),
            Some("Conflicting authentication policies for indexes `private` (`https://example.com/simple`) (always) and `mirror` (`https://example.com/+simple`) (auto): authentication scopes overlap".to_owned())
        );
        Ok(())
    }

    #[test]
    fn pinned_indexes_accept_compatible_alias() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/simple")?;
        selected.authenticate = AuthPolicy::Always;
        let pin = IndexMetadata::from(selected.url.clone());
        let original = IndexLocations::new(vec![selected.clone()], Vec::new(), false);
        let mut alias = Index::from_str("mirror=https://example.com/simple")?;
        alias.authenticate = AuthPolicy::Always;
        let retained = IndexLocations::new(vec![selected, alias], Vec::new(), false);

        retained.validate_pinned_indexes(&original, [&pin])?;
        Ok(())
    }

    #[test]
    fn source_indexes_reject_implicit_artifact_error_policy() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://download.pytorch.org/whl/cpu")?;
        selected.ignore_error_codes = Some(serde_json::from_str("[403]")?);
        let alias = Index::from_str("mirror=https://download.pytorch.org/whl/cpu")?;
        assert_eq!(
            selected.status_code_strategy(),
            alias.status_code_strategy()
        );
        assert!(selected.ignores_error_code(StatusCode::FORBIDDEN));
        assert!(!alias.ignores_error_code(StatusCode::FORBIDDEN));

        let result = IndexLocations::new(vec![alias], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?);
        assert_eq!(
            result.err().map(|err| err.to_string()),
            Some("Conflicting policies for indexes `mirror` (`https://download.pytorch.org/whl/cpu`) and `private` (`https://download.pytorch.org/whl/cpu`)".to_owned())
        );
        Ok(())
    }

    #[test]
    fn source_indexes_accept_reordered_artifact_error_codes() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://download.pytorch.org/whl/cpu")?;
        selected.ignore_error_codes = Some(serde_json::from_str("[403, 404, 403]")?);
        let url = selected.url.clone();
        let mut alias = Index::from_str("mirror=https://download.pytorch.org/whl/cpu")?;
        alias.ignore_error_codes = Some(serde_json::from_str("[404, 403]")?);

        let locations = IndexLocations::new(vec![alias], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        assert!(locations.ignores_error_code_for(&url, StatusCode::FORBIDDEN));
        assert!(locations.ignores_error_code_for(&url, StatusCode::NOT_FOUND));
        Ok(())
    }

    #[test]
    fn source_indexes_reject_conflicting_implicit_default() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://pypi.org/simple")?;
        selected.explicit = true;
        selected.authenticate = AuthPolicy::Always;

        let result = IndexLocations::default()
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?);
        assert_eq!(
            result.err().map(|err| err.to_string()),
            Some("Conflicting authentication policies for indexes `https://pypi.org/simple` (auto) and `private` (`https://pypi.org/simple`) (always): authentication scopes overlap".to_owned())
        );
        Ok(())
    }

    #[test]
    fn source_default_replaces_implicit_default() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://pypi.org/simple")?;
        selected.explicit = true;
        selected.authenticate = AuthPolicy::Always;
        let selected_url = selected.url.clone();
        let default = Index::from_index_url(IndexUrl::from_str("https://example.com/simple")?);

        let locations = IndexLocations::default()
            .with_source_indexes(SourceIndexes::try_from_iter([selected, default])?)?;
        assert_eq!(
            index_urls(locations.default_index()),
            ["https://example.com/simple"]
        );
        let selected_policies = locations
            .allowed_indexes()
            .into_iter()
            .filter(|index| is_same_index(&index.url, &selected_url))
            .map(|index| index.authenticate)
            .collect::<Vec<_>>();
        assert_eq!(selected_policies, [AuthPolicy::Always]);
        Ok(())
    }

    #[test]
    fn source_indexes_reject_retained_flat_policy() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/links")?;
        selected.format = IndexFormat::Flat;
        selected.authenticate = AuthPolicy::Always;
        let flat = Index::from_find_links(selected.url.clone());

        let result = IndexLocations::new(Vec::new(), vec![flat], false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?);
        assert_eq!(
            result.err().map(|err| err.to_string()),
            Some("Conflicting authentication policies for indexes `https://example.com/links` (auto) and `private` (`https://example.com/links`) (always): authentication scopes overlap".to_owned())
        );
        Ok(())
    }

    #[test]
    fn source_indexes_ignore_shadowed_flat_policy() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/links")?;
        selected.format = IndexFormat::Flat;
        selected.authenticate = AuthPolicy::Always;
        let mut flat = Index::from_find_links(selected.url.clone());
        flat.name = selected.name.clone();
        let selected_url = selected.url.clone();

        let locations = IndexLocations::new(Vec::new(), vec![flat], false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        let selected_policies = locations
            .allowed_indexes()
            .into_iter()
            .filter(|index| is_same_index(&index.url, &selected_url))
            .map(|index| index.authenticate)
            .collect::<Vec<_>>();
        assert_eq!(selected_policies, [AuthPolicy::Always]);
        Ok(())
    }

    #[test]
    fn source_indexes_retain_command_line_name_precedence() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://example.com/links")?;
        selected.format = IndexFormat::Flat;
        selected.authenticate = AuthPolicy::Always;
        let mut command_line = selected.clone();
        command_line.authenticate = AuthPolicy::Auto;
        command_line.origin = Some(Origin::Cli);
        let selected_url = selected.url.clone();

        let locations = IndexLocations::new(vec![command_line], Vec::new(), false)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        let selected_policies = locations
            .allowed_indexes()
            .into_iter()
            .filter(|index| is_same_index(&index.url, &selected_url))
            .map(|index| index.authenticate)
            .collect::<Vec<_>>();
        assert_eq!(selected_policies, [AuthPolicy::Auto]);
        Ok(())
    }

    #[test]
    fn source_indexes_have_no_implicit_default_without_index() -> Result<(), Box<dyn Error>> {
        let mut selected = Index::from_str("private=https://pypi.org/simple")?;
        selected.explicit = true;
        selected.authenticate = AuthPolicy::Always;

        let flat = Index::from_find_links(selected.url.clone());
        let locations = IndexLocations::new(Vec::new(), vec![flat], true)
            .with_source_indexes(SourceIndexes::try_from_iter([selected])?)?;
        assert!(locations.default_index().is_none());
        let allowed = locations.allowed_indexes();
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].format, IndexFormat::Flat);
        assert_eq!(allowed[0].authenticate, AuthPolicy::Auto);
        Ok(())
    }

    #[test]
    fn test_index_url_parse_valid_paths() {
        // Absolute path
        assert!(is_disambiguated_path("/absolute/path"));
        // Relative path
        assert!(is_disambiguated_path("./relative/path"));
        assert!(is_disambiguated_path("../../relative/path"));
        if cfg!(windows) {
            // Windows absolute path
            assert!(is_disambiguated_path("C:/absolute/path"));
            // Windows relative path
            assert!(is_disambiguated_path(".\\relative\\path"));
            assert!(is_disambiguated_path("..\\..\\relative\\path"));
        }
    }

    #[test]
    fn test_index_url_parse_ambiguous_paths() {
        // Test single-segment ambiguous path
        assert!(!is_disambiguated_path("index"));
        // Test multi-segment ambiguous path
        assert!(!is_disambiguated_path("relative/path"));
    }

    #[test]
    fn test_index_url_parse_with_schemes() {
        assert!(is_disambiguated_path("file:///absolute/path"));
        assert!(is_disambiguated_path("https://registry.com/simple/"));
        assert!(is_disambiguated_path(
            "git+https://github.com/example/repo.git"
        ));
    }

    #[test]
    fn named_indexes_use_first_definition() -> Result<(), Box<dyn Error>> {
        let first = Index::from_str("shared=https://first.example.com/simple")?;
        let mut shadowed = Index::from_str("shared=https://shadowed.example.com/simple")?;
        shadowed.explicit = true;
        let mut explicit = Index::from_str("explicit=https://explicit.example.com/simple")?;
        explicit.explicit = true;
        let shadowed_implicit =
            Index::from_str("explicit=https://shadowed-implicit.example.com/simple")?;
        let mut default = Index::from_str("default=https://default.example.com/simple")?;
        default.default = true;

        let locations = IndexLocations::new(
            vec![first, shadowed, explicit, shadowed_implicit, default],
            vec![],
            false,
        );

        assert_eq!(
            index_urls(locations.simple_indexes()),
            [
                "https://first.example.com/simple",
                "https://explicit.example.com/simple",
                "https://default.example.com/simple",
            ]
        );
        assert_eq!(
            index_urls(locations.implicit_indexes()),
            ["https://first.example.com/simple"]
        );
        assert_eq!(
            index_urls(locations.explicit_indexes()),
            ["https://explicit.example.com/simple"]
        );
        assert_eq!(
            index_urls(locations.indexes()),
            [
                "https://first.example.com/simple",
                "https://default.example.com/simple",
            ]
        );
        assert_eq!(
            index_urls(locations.defined_indexes()),
            [
                "https://first.example.com/simple",
                "https://explicit.example.com/simple",
                "https://default.example.com/simple",
            ]
        );

        assert!(
            locations
                .index_for_url(&IndexUrl::from_str("https://shadowed.example.com/simple")?)
                .is_none()
        );
        assert!(
            locations
                .index_for_url(&IndexUrl::from_str(
                    "https://shadowed-implicit.example.com/simple"
                )?)
                .is_none()
        );
        assert_eq!(
            index_urls(locations.allowed_indexes()),
            [
                "https://default.example.com/simple",
                "https://explicit.example.com/simple",
                "https://first.example.com/simple"
            ]
        );

        Ok(())
    }

    #[test]
    fn unnamed_indexes_are_not_deduplicated() -> Result<(), Box<dyn Error>> {
        let first = Index::from_str("https://first.example.com/simple")?;
        let repeated = Index::from_str("https://first.example.com/simple")?;
        let last = Index::from_str("https://last.example.com/simple")?;
        let locations = IndexLocations::new(vec![first, repeated, last], vec![], false);
        let expected = [
            "https://first.example.com/simple",
            "https://first.example.com/simple",
            "https://last.example.com/simple",
        ];

        assert_eq!(index_urls(locations.simple_indexes()), expected);
        assert_eq!(index_urls(locations.implicit_indexes()), expected);
        assert_eq!(index_urls(locations.defined_indexes()), expected);
        assert_eq!(
            index_urls(locations.fetch_indexes()),
            [
                "https://first.example.com/simple",
                "https://last.example.com/simple",
                "https://pypi.org/simple",
            ]
        );

        Ok(())
    }

    #[test]
    fn shadowed_default_falls_back_to_pypi() -> Result<(), Box<dyn Error>> {
        let first = Index::from_str("shared=https://first.example.com/simple")?;
        let mut shadowed = Index::from_str("shared=https://shadowed.example.com/simple")?;
        shadowed.default = true;
        let locations = IndexLocations::new(vec![first, shadowed], vec![], false);

        assert_eq!(
            index_urls(locations.default_index()),
            ["https://pypi.org/simple"]
        );
        assert_eq!(
            index_urls(locations.indexes()),
            [
                "https://first.example.com/simple",
                "https://pypi.org/simple",
            ]
        );

        assert!(
            locations
                .index_for_url(&IndexUrl::from_str("https://shadowed.example.com/simple")?)
                .is_none()
        );
        assert!(
            locations
                .index_for_url(&IndexUrl::from_str("https://pypi.org/simple")?)
                .is_some()
        );
        assert_eq!(
            index_urls(locations.allowed_indexes()),
            [
                "https://pypi.org/simple",
                "https://first.example.com/simple"
            ]
        );

        Ok(())
    }

    #[test]
    fn fetch_indexes_deduplicates_raw_urls() {
        let url = IndexUrl::from_str("https://index.example.com/simple").unwrap();
        let mut first = Index::from(url.clone());
        first.name = Some(IndexName::from_str("first").unwrap());
        let mut second = Index::from(url);
        second.name = Some(IndexName::from_str("second").unwrap());
        second.default = true;
        let locations = IndexLocations::new(vec![first, second], Vec::new(), false);

        assert_eq!(locations.indexes().count(), 2);
        assert_eq!(locations.fetch_indexes().count(), 1);
    }

    #[test]
    fn test_cache_control_lookup() {
        use std::str::FromStr;

        use crate::IndexFormat;
        use crate::index_name::IndexName;

        let indexes = vec![
            Index {
                name: Some(IndexName::from_str("index1").unwrap()),
                url: IndexUrl::from_str("https://index1.example.com/simple").unwrap(),
                cache_control: Some(crate::IndexCacheControl {
                    api: Some(HeaderValue::from_static("max-age=300")),
                    files: Some(HeaderValue::from_static("max-age=1800")),
                }),
                explicit: false,
                default: false,
                origin: None,
                format: IndexFormat::Simple,
                publish_url: None,
                authenticate: uv_auth::AuthPolicy::default(),
                ignore_error_codes: None,
                hash_algorithm: None,
                exclude_newer: None,
            },
            Index {
                name: Some(IndexName::from_str("index2").unwrap()),
                url: IndexUrl::from_str("https://index2.example.com/simple").unwrap(),
                cache_control: None,
                explicit: false,
                default: false,
                origin: None,
                format: IndexFormat::Simple,
                publish_url: None,
                authenticate: uv_auth::AuthPolicy::default(),
                ignore_error_codes: None,
                hash_algorithm: None,
                exclude_newer: None,
            },
        ];

        let index_locations = IndexLocations::new(indexes, Vec::new(), false);

        let url1 = IndexUrl::from_str("https://index1.example.com/simple").unwrap();
        assert_eq!(
            index_locations.simple_api_cache_control_for(&url1),
            Some(HeaderValue::from_static("max-age=300"))
        );
        assert_eq!(
            index_locations.artifact_cache_control_for(&url1),
            Some(HeaderValue::from_static("max-age=1800"))
        );

        let url2 = IndexUrl::from_str("https://index2.example.com/simple").unwrap();
        assert_eq!(index_locations.simple_api_cache_control_for(&url2), None);
        assert_eq!(index_locations.artifact_cache_control_for(&url2), None);

        let url3 = IndexUrl::from_str("https://index3.example.com/simple").unwrap();
        assert_eq!(index_locations.simple_api_cache_control_for(&url3), None);
        assert_eq!(index_locations.artifact_cache_control_for(&url3), None);
    }

    #[test]
    fn test_pytorch_default_cache_control() {
        // Test that PyTorch indexes get default cache control from the getter methods
        let indexes = vec![Index {
            name: Some(IndexName::from_str("pytorch").unwrap()),
            url: IndexUrl::from_str("https://download.pytorch.org/whl/cu118").unwrap(),
            cache_control: None, // No explicit cache control
            explicit: false,
            default: false,
            origin: None,
            format: IndexFormat::Simple,
            publish_url: None,
            authenticate: uv_auth::AuthPolicy::default(),
            ignore_error_codes: None,
            hash_algorithm: None,
            exclude_newer: None,
        }];

        let index_locations = IndexLocations::new(indexes, Vec::new(), false);

        let pytorch_url = IndexUrl::from_str("https://download.pytorch.org/whl/cu118").unwrap();

        assert_eq!(
            index_locations.simple_api_cache_control_for(&pytorch_url),
            None
        );
        assert_eq!(
            index_locations.artifact_cache_control_for(&pytorch_url),
            Some(HeaderValue::from_static(
                "max-age=365000000, immutable, public",
            ))
        );
    }

    #[test]
    fn test_pytorch_user_override_cache_control() {
        // Test that user-specified cache control overrides PyTorch defaults
        let indexes = vec![Index {
            name: Some(IndexName::from_str("pytorch").unwrap()),
            url: IndexUrl::from_str("https://download.pytorch.org/whl/cu118").unwrap(),
            cache_control: Some(IndexCacheControl {
                api: Some(HeaderValue::from_static("no-cache")),
                files: Some(HeaderValue::from_static("max-age=3600")),
            }),
            explicit: false,
            default: false,
            origin: None,
            format: IndexFormat::Simple,
            publish_url: None,
            authenticate: uv_auth::AuthPolicy::default(),
            ignore_error_codes: None,
            hash_algorithm: None,
            exclude_newer: None,
        }];

        let index_locations = IndexLocations::new(indexes, Vec::new(), false);

        let pytorch_url = IndexUrl::from_str("https://download.pytorch.org/whl/cu118").unwrap();

        assert_eq!(
            index_locations.simple_api_cache_control_for(&pytorch_url),
            Some(HeaderValue::from_static("no-cache"))
        );
        assert_eq!(
            index_locations.artifact_cache_control_for(&pytorch_url),
            Some(HeaderValue::from_static("max-age=3600"))
        );
    }

    #[test]
    fn test_nvidia_default_cache_control() {
        // Test that NVIDIA indexes get default cache control from the getter methods
        let indexes = vec![Index {
            name: Some(IndexName::from_str("nvidia").unwrap()),
            url: IndexUrl::from_str("https://pypi.nvidia.com").unwrap(),
            cache_control: None, // No explicit cache control
            explicit: false,
            default: false,
            origin: None,
            format: IndexFormat::Simple,
            publish_url: None,
            authenticate: uv_auth::AuthPolicy::default(),
            ignore_error_codes: None,
            hash_algorithm: None,
            exclude_newer: None,
        }];

        let index_locations = IndexLocations::new(indexes, Vec::new(), false);

        let nvidia_url = IndexUrl::from_str("https://pypi.nvidia.com").unwrap();

        assert_eq!(
            index_locations.simple_api_cache_control_for(&nvidia_url),
            None
        );
        assert_eq!(
            index_locations.artifact_cache_control_for(&nvidia_url),
            Some(HeaderValue::from_static(
                "max-age=365000000, immutable, public",
            ))
        );
    }
}
