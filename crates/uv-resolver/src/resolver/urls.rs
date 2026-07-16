use std::path::PathBuf;

use either::Either;
use rustc_hash::FxHashMap;
use same_file::is_same_file;
use tracing::debug;

use uv_cache_key::CanonicalUrl;
use uv_git::GitResolver;
use uv_normalize::PackageName;
use uv_pep508::VerbatimUrl;
use uv_pypi_types::{ParsedArchiveUrl, ParsedDirectoryUrl, ParsedUrl, VerbatimParsedUrl};

use crate::resolver::ForkMap;
use crate::{DependencyMode, Manifest, ResolveError, ResolverEnvironment};

/// Number of URLs for a package before archive URLs are indexed by their canonical resource.
const ARCHIVE_INDEX_THRESHOLD: usize = 8;

/// A canonical archive resource, including its normalized subdirectory.
#[derive(Debug, Eq, Hash, PartialEq)]
struct ArchiveResource {
    url: CanonicalUrl,
    subdirectory: Option<PathBuf>,
}

impl ArchiveResource {
    fn new(archive: &ParsedArchiveUrl) -> Self {
        Self {
            url: CanonicalUrl::new(archive.url.clone()),
            subdirectory: archive
                .subdirectory
                .as_deref()
                .map(|subdirectory| uv_fs::normalize_path(subdirectory).into_owned()),
        }
    }
}

/// The regular URLs for a package, with an optional archive-resource index.
#[derive(Debug, Default)]
struct PackageUrls {
    urls: Vec<VerbatimParsedUrl>,
    archives: Option<FxHashMap<ArchiveResource, usize>>,
}

impl PackageUrls {
    fn insert(&mut self, url: VerbatimParsedUrl, git: &GitResolver) {
        if matches!(url.parsed_url, ParsedUrl::Archive(_))
            && self.archives.is_none()
            && self.urls.len() >= ARCHIVE_INDEX_THRESHOLD
        {
            self.archives = Some(
                self.urls
                    .iter()
                    .enumerate()
                    .filter_map(|(index, url)| match &url.parsed_url {
                        ParsedUrl::Archive(archive) => Some((ArchiveResource::new(archive), index)),
                        _ => None,
                    })
                    .collect(),
            );
        }

        let archive = self.archives.as_ref().and_then(|_| match &url.parsed_url {
            ParsedUrl::Archive(archive) => Some(ArchiveResource::new(archive)),
            _ => None,
        });

        let matching_index = if let (Some(archive), Some(archives)) = (&archive, &self.archives) {
            archives.get(archive).copied()
        } else {
            self.urls.iter().position(|package_url| {
                same_resource(&package_url.parsed_url, &url.parsed_url, git)
            })
        };

        if let Some(index) = matching_index {
            let package_url = &mut self.urls[index];

            // Allow editables to override non-editables.
            let previous_editable = package_url.is_editable();
            *package_url = url;
            if previous_editable
                && let VerbatimParsedUrl {
                    parsed_url: ParsedUrl::Directory(ParsedDirectoryUrl { editable, .. }),
                    verbatim: _,
                } = package_url
                && editable.is_none()
            {
                debug!("Allowing an editable variant of {}", &package_url.verbatim);
                *editable = Some(true);
            }
        } else {
            if let (Some(archive), Some(archives)) = (archive, &mut self.archives) {
                archives.insert(archive, self.urls.len());
            }
            self.urls.push(url);
        }
    }
}

/// The URLs that are allowed for packages.
///
/// These are the URLs used in the root package or by other URL dependencies (including path
/// dependencies). They take precedence over requirements by version (except for the special case
/// where we are in a fork that doesn't use any of the URL(s) used in other forks). Each fork may
/// only use a single URL.
///
/// This type contains all URLs without checking, the validation happens in
/// [`crate::fork_urls::ForkUrls`].
#[derive(Debug, Default)]
pub(crate) struct Urls {
    /// URL requirements in overrides. An override URL replaces all requirements and constraints
    /// URLs. There can be multiple URLs for the same package as long as they are in different
    /// forks.
    overrides: ForkMap<VerbatimParsedUrl>,
    /// URLs from regular requirements or from constraints. There can be multiple URLs for the same
    /// package as long as they are in different forks.
    regular: FxHashMap<PackageName, PackageUrls>,
}

impl Urls {
    pub(crate) fn from_manifest(
        manifest: &Manifest,
        env: &ResolverEnvironment,
        git: &GitResolver,
        dependencies: DependencyMode,
    ) -> Self {
        let mut regular: FxHashMap<PackageName, PackageUrls> = FxHashMap::default();
        let mut overrides = ForkMap::default();

        // Add all direct regular requirements and constraints URL.
        for requirement in manifest.requirements_no_overrides(env, dependencies) {
            let Some(url) = requirement.source.to_verbatim_parsed_url() else {
                // Registry requirement
                continue;
            };

            regular
                .entry(requirement.name.clone())
                .or_default()
                .insert(url, git);
        }

        // Add all URLs from overrides. If there is an override URL, all other URLs from
        // requirements and constraints are moot and will be removed.
        for requirement in manifest.overrides(env, dependencies) {
            let Some(url) = requirement.source.to_verbatim_parsed_url() else {
                // Registry requirement
                continue;
            };
            // We only clear for non-URL overrides, since e.g. with an override `anyio==0.0.0` and
            // a requirements.txt entry `./anyio`, we still use the URL. See
            // `allow_recursive_url_local_path_override_constraint`.
            regular.remove(&requirement.name);
            overrides.add(requirement.as_ref(), url);
        }

        Self { overrides, regular }
    }

    /// Return an iterator over the allowed URLs for the given package.
    ///
    /// If we have a URL override, apply it unconditionally for registry and URL requirements.
    /// Otherwise, there are two case: for a URL requirement (`url` isn't `None`), check that the
    /// URL is allowed and return its canonical form.
    ///
    /// For registry requirements, we return an empty iterator.
    pub(crate) fn get_url<'a>(
        &'a self,
        env: &'a ResolverEnvironment,
        name: &'a PackageName,
        url: Option<&'a VerbatimParsedUrl>,
        git: &'a GitResolver,
    ) -> Result<impl Iterator<Item = &'a VerbatimParsedUrl>, ResolveError> {
        if self.overrides.contains_key(name) {
            Ok(Either::Left(Either::Left(
                self.overrides.get(name, env).into_iter(),
            )))
        } else if let Some(url) = url {
            let url =
                self.canonicalize_allowed_url(env, name, git, &url.verbatim, &url.parsed_url)?;
            Ok(Either::Left(Either::Right(std::iter::once(url))))
        } else {
            Ok(Either::Right(std::iter::empty()))
        }
    }

    /// Return `true` if the package has any URL (from overrides or regular requirements).
    pub(crate) fn any_url(&self, name: &PackageName) -> bool {
        self.overrides.contains_key(name) || self.get_regular(name).is_some()
    }

    /// Return the allowed [`VerbatimUrl`]s for given package from regular requirements and
    /// constraints (but not overrides), if any.
    ///
    /// It's more than one more URL if they are in different forks (or conflict after forking).
    fn get_regular(&self, package: &PackageName) -> Option<&PackageUrls> {
        self.regular.get(package)
    }

    /// Check if a URL is allowed (known), and if so, return its canonical form.
    fn canonicalize_allowed_url<'a>(
        &'a self,
        env: &ResolverEnvironment,
        package_name: &'a PackageName,
        git: &'a GitResolver,
        verbatim_url: &'a VerbatimUrl,
        parsed_url: &'a ParsedUrl,
    ) -> Result<&'a VerbatimParsedUrl, ResolveError> {
        let Some(expected) = self.get_regular(package_name) else {
            return Err(ResolveError::DisallowedUrl {
                name: package_name.clone(),
                url: verbatim_url.to_string(),
            });
        };

        if let (ParsedUrl::Archive(archive), Some(archives)) = (parsed_url, &expected.archives) {
            if let Some(index) = archives.get(&ArchiveResource::new(archive)) {
                return Ok(&expected.urls[*index]);
            }

            return Err(ResolveError::ConflictingUrls {
                package_name: package_name.clone(),
                urls: vec![parsed_url.clone()],
                env: env.clone(),
            });
        }

        let matching_urls: Vec<_> = expected
            .urls
            .iter()
            .filter(|requirement| same_resource(&requirement.parsed_url, parsed_url, git))
            .collect();

        let [allowed_url] = matching_urls.as_slice() else {
            let mut conflicting_urls: Vec<_> = matching_urls
                .into_iter()
                .map(|parsed_url| parsed_url.parsed_url.clone())
                .chain(std::iter::once(parsed_url.clone()))
                .collect();
            conflicting_urls.sort();
            return Err(ResolveError::ConflictingUrls {
                package_name: package_name.clone(),
                urls: conflicting_urls,
                env: env.clone(),
            });
        };
        Ok(*allowed_url)
    }
}

/// Returns `true` if the [`ParsedUrl`] instances point to the same resource.
fn same_resource(a: &ParsedUrl, b: &ParsedUrl, git: &GitResolver) -> bool {
    match (a, b) {
        (ParsedUrl::Archive(a), ParsedUrl::Archive(b)) => {
            a.subdirectory.as_deref().map(uv_fs::normalize_path)
                == b.subdirectory.as_deref().map(uv_fs::normalize_path)
                && CanonicalUrl::new(a.url.clone()) == CanonicalUrl::new(b.url.clone())
        }
        (ParsedUrl::GitDirectory(a), ParsedUrl::GitDirectory(b)) => {
            a.subdirectory.as_deref().map(uv_fs::normalize_path)
                == b.subdirectory.as_deref().map(uv_fs::normalize_path)
                && git.same_ref(&a.url, &b.url)
        }
        (ParsedUrl::GitPath(a), ParsedUrl::GitPath(b)) => {
            uv_fs::normalize_path(&a.install_path) == uv_fs::normalize_path(&b.install_path)
                && git.same_ref(&a.url, &b.url)
        }
        (ParsedUrl::Path(a), ParsedUrl::Path(b)) => {
            a.install_path == b.install_path
                || is_same_file(&a.install_path, &b.install_path).unwrap_or(false)
        }
        (ParsedUrl::Directory(a), ParsedUrl::Directory(b)) => {
            (a.install_path == b.install_path
                || is_same_file(&a.install_path, &b.install_path).unwrap_or(false))
                && a.editable.is_none_or(|a| b.editable.is_none_or(|b| a == b))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use uv_pep508::Pep508Url;

    use super::*;

    fn parsed_url(url: &str) -> VerbatimParsedUrl {
        let working_dir = std::env::current_dir().expect("working directory");
        VerbatimParsedUrl::parse_url(url, Some(&working_dir)).expect("valid URL")
    }

    fn archive_resource(url: &VerbatimParsedUrl) -> Option<ArchiveResource> {
        match &url.parsed_url {
            ParsedUrl::Archive(archive) => Some(ArchiveResource::new(archive)),
            _ => None,
        }
    }

    #[test]
    fn archive_resource_uses_canonical_url_and_normalized_subdirectory() {
        let canonical = parsed_url(
            "https://example.com/demo-1.0.0-py3-none-any.whl#subdirectory=packages/demo",
        );
        let equivalent = parsed_url(
            "https://user:password@example.com/demo%2D1.0.0-py3-none-any.whl#subdirectory=packages/./nested/../demo",
        );
        let different = parsed_url(
            "https://example.com/demo-1.0.0-py3-none-any.whl#subdirectory=packages/other",
        );

        assert_eq!(archive_resource(&canonical), archive_resource(&equivalent));
        assert_ne!(archive_resource(&canonical), archive_resource(&different));
    }

    #[test]
    fn archive_index_preserves_first_position_and_last_representative() {
        let git = GitResolver::default();
        let mut package_urls = PackageUrls::default();

        for index in 0..=ARCHIVE_INDEX_THRESHOLD {
            let url = parsed_url(&format!(
                "https://example.com/{index}/demo-1.0.0-py3-none-any.whl#subdirectory=packages/demo"
            ));
            package_urls.insert(url, &git);
        }

        let archives = package_urls.archives.as_ref().expect("archive index");
        assert_eq!(archives.len(), ARCHIVE_INDEX_THRESHOLD + 1);
        assert_eq!(package_urls.urls.len(), ARCHIVE_INDEX_THRESHOLD + 1);

        let replacement = parsed_url(
            "https://user:password@example.com/0/demo%2D1.0.0-py3-none-any.whl#subdirectory=packages/./nested/../demo",
        );
        let replacement_given = replacement.verbatim.given().map(str::to_owned);
        package_urls.insert(replacement.clone(), &git);

        let archives = package_urls.archives.as_ref().expect("archive index");
        assert_eq!(archives.len(), ARCHIVE_INDEX_THRESHOLD + 1);
        assert_eq!(package_urls.urls.len(), ARCHIVE_INDEX_THRESHOLD + 1);
        assert_eq!(
            package_urls.urls[0].verbatim.given(),
            replacement_given.as_deref()
        );
        assert_eq!(
            archives.get(&archive_resource(&replacement).expect("archive resource")),
            Some(&0)
        );
    }

    #[test]
    fn archive_index_keeps_mixed_url_fallback_and_reports_unknown_archive() {
        let git = GitResolver::default();
        let mut package_urls = PackageUrls::default();

        for index in 0..=ARCHIVE_INDEX_THRESHOLD {
            let url = parsed_url(&format!(
                "https://example.com/{index}/demo-1.0.0-py3-none-any.whl#subdirectory=packages/demo"
            ));
            package_urls.insert(url, &git);
        }

        let path = parsed_url("file:///does-not-exist/demo-1.0.0-py3-none-any.whl");
        package_urls.insert(path.clone(), &git);
        package_urls.insert(path, &git);

        let git_url = parsed_url("git+https://example.com/demo.git@v1.0.0");
        package_urls.insert(git_url.clone(), &git);
        package_urls.insert(git_url, &git);

        let mut editable = parsed_url("file:///does-not-exist/demo-editable");
        if let ParsedUrl::Directory(directory) = &mut editable.parsed_url {
            directory.editable = Some(true);
        }
        package_urls.insert(editable, &git);
        package_urls.insert(parsed_url("file:///does-not-exist/demo-editable"), &git);

        assert_eq!(package_urls.urls.len(), ARCHIVE_INDEX_THRESHOLD + 4);
        assert_eq!(
            package_urls.archives.as_ref().map(FxHashMap::len),
            Some(ARCHIVE_INDEX_THRESHOLD + 1)
        );
        assert!(
            package_urls
                .urls
                .last()
                .is_some_and(VerbatimParsedUrl::is_editable)
        );

        let package_name: PackageName = "demo".parse().expect("valid package name");
        let mut regular = FxHashMap::default();
        regular.insert(package_name.clone(), package_urls);
        let urls = Urls {
            overrides: ForkMap::default(),
            regular,
        };
        let env = ResolverEnvironment::universal(Vec::new());

        let known = parsed_url(
            "https://user:password@example.com/0/demo%2D1.0.0-py3-none-any.whl#subdirectory=packages/./nested/../demo",
        );
        let allowed = urls
            .canonicalize_allowed_url(
                &env,
                &package_name,
                &git,
                &known.verbatim,
                &known.parsed_url,
            )
            .expect("known archive");
        assert_eq!(
            allowed.verbatim.given(),
            package_urls_given(&urls, &package_name)
        );

        let unknown = parsed_url(
            "https://example.com/unknown/demo-1.0.0-py3-none-any.whl#subdirectory=packages/demo",
        );
        let error = urls
            .canonicalize_allowed_url(
                &env,
                &package_name,
                &git,
                &unknown.verbatim,
                &unknown.parsed_url,
            )
            .expect_err("unknown archive");
        assert!(matches!(
            error,
            ResolveError::ConflictingUrls { urls, .. } if urls == vec![unknown.parsed_url]
        ));
    }

    fn package_urls_given<'a>(urls: &'a Urls, package_name: &PackageName) -> Option<&'a str> {
        urls.regular
            .get(package_name)
            .and_then(|package_urls| package_urls.urls.first())
            .and_then(|url| url.verbatim.given())
    }
}
