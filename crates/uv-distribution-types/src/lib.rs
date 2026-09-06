//! ## Type hierarchy
//!
//! For each `pip sync` requirement, check the installed environment and the wheel cache.
//! [`InstalledDist`] represents an installed package. [`CachedDist`] represents a cached wheel.
//! [`Dist`] represents a package that uv must download, optionally build, and install.
//!
//! ## `Dist`
//! A [`Dist`] represents a built distribution, or wheel, or a source distribution.
//! Check each index to convert a PEP 508 requirement into a [`Dist`]. Requirements can come from
//! `requirements.txt` or `[project] dependencies` in `pyproject.toml`.
//! * [`BuiltDist`]: A wheel with one of four origins:
//!   * [`RegistryBuiltDist`]
//!   * [`DirectUrlBuiltDist`]
//!   * [`PathBuiltDist`]
//!   * [`GitPathBuiltDist`]
//! * [`SourceDist`]: A source distribution with one of six origins:
//!   * [`RegistrySourceDist`]
//!   * [`DirectUrlSourceDist`]
//!   * [`GitDirectorySourceDist`]
//!   * [`GitPathSourceDist`]
//!   * [`PathSourceDist`]
//!   * [`DirectorySourceDist`]
//!
//! ## `CachedDist`
//! A [`CachedDist`] represents a wheel in the local cache. It has one of two origins:
//! * [`CachedRegistryDist`]
//! * [`CachedDirectUrlDist`]
//!
//! ## `InstalledDist`
//! An [`InstalledDist`] represents a distribution in a Python environment.
//! It has one of five types:
//! * [`InstalledRegistryDist`]
//! * [`InstalledDirectUrlDist`]
//! * [`InstalledEggInfoFile`]
//! * [`InstalledEggInfoDirectory`]
//! * [`InstalledLegacyEditable`]
//!
//! [`InstalledDirectUrlDist`] gets direct URL information from
//! [`direct_url.json`](https://packaging.python.org/en/latest/specifications/direct-url-data-structure/)
//! and might not match the original [`Dist`] exactly.
use std::borrow::Cow;
use std::ffi::OsStr;
use std::fmt::Display;
use std::path;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use memchr::memchr3;
use url::Url;

use uv_distribution_filename::{
    DistExtension, SourceDistExtension, SourceDistFilename, WheelFilename,
};
use uv_fs::normalize_absolute_path;
use uv_git_types::GitUrl;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pep508::{Pep508Url, VerbatimUrl};
use uv_pypi_types::{
    ParsedArchiveUrl, ParsedDirectoryUrl, ParsedGitDirectoryUrl, ParsedGitPathUrl, ParsedPathUrl,
    ParsedUrl, VerbatimParsedUrl,
};
use uv_redacted::DisplaySafeUrl;

pub use crate::annotation::*;
pub use crate::any::*;
pub use crate::build_info::*;
pub use crate::build_requires::*;
pub use crate::buildable::*;
pub use crate::cached::*;
pub use crate::config_settings::*;
pub use crate::dependency_metadata::*;
pub use crate::diagnostic::*;
pub use crate::dist_error::*;
pub use crate::error::*;
pub use crate::exclude_newer::*;
pub use crate::file::*;
pub use crate::hash::*;
pub use crate::id::*;
pub use crate::index::*;
pub use crate::index_name::*;
pub use crate::index_url::*;
pub use crate::installed::*;
pub use crate::known_platform::*;
pub use crate::origin::*;
pub use crate::pip_index::*;
pub use crate::prioritized_distribution::*;
pub use crate::requested::*;
pub use crate::requirement::*;
pub use crate::requires_python::*;
pub use crate::resolution::*;
pub use crate::resolved::*;
pub use crate::specified_requirement::*;
pub use crate::status_code_strategy::*;
pub use crate::traits::*;

mod annotation;
mod any;
mod build_info;
mod build_requires;
mod buildable;
mod cached;
mod config_settings;
mod dependency_metadata;
mod diagnostic;
mod dist_error;
mod error;
mod exclude_newer;
mod file;
mod hash;
mod id;
mod index;
mod index_name;
mod index_url;
mod installed;
mod installed_modules;
mod known_platform;
mod origin;
mod pip_index;
mod prioritized_distribution;
mod requested;
mod requirement;
mod requires_python;
mod resolution;
mod resolved;
mod specified_requirement;
mod status_code_strategy;
mod traits;

#[derive(Debug, Clone)]
pub enum VersionOrUrlRef<'a, T: Pep508Url = VerbatimUrl> {
    /// A PEP 440 version specifier that identifies a registry distribution.
    Version(&'a Version),
    /// A URL that identifies a distribution.
    Url(&'a T),
}

impl Verbatim for VersionOrUrlRef<'_> {
    fn verbatim(&self) -> Cow<'_, str> {
        match self {
            Self::Version(version) => Cow::Owned(format!("=={version}")),
            Self::Url(url) => Cow::Owned(format!(" @ {}", url.verbatim())),
        }
    }
}

impl std::fmt::Display for VersionOrUrlRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version(version) => write!(f, "=={version}"),
            Self::Url(url) => write!(f, " @ {url}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InstalledVersion<'a> {
    /// A PEP 440 version specifier that identifies a registry distribution.
    Version(&'a Version),
    /// A distribution URL and its resolved version.
    Url(&'a DisplaySafeUrl, &'a Version),
}

impl<'a> InstalledVersion<'a> {
    /// Return the resolved version.
    pub fn version(&self) -> &'a Version {
        match self {
            Self::Version(version) => version,
            Self::Url(_, version) => version,
        }
    }
}

impl std::fmt::Display for InstalledVersion<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version(version) => write!(f, "=={version}"),
            Self::Url(url, version) => write!(f, "=={version} (from {url})"),
        }
    }
}

/// A built distribution, or wheel, or a source distribution.
///
/// The distribution can come from an index, URL, path, or Git repository.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum Dist {
    Built(BuiltDist),
    Source(SourceDist),
}

/// A reference to a built or source distribution.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum DistRef<'a> {
    Built(&'a BuiltDist),
    Source(&'a SourceDist),
}

impl Display for DistRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Built(built_dist) => Display::fmt(&built_dist, f),
            Self::Source(source_dist) => Display::fmt(&source_dist, f),
        }
    }
}

/// A wheel from an index, URL, path, or Git path.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum BuiltDist {
    Registry(RegistryBuiltDist),
    DirectUrl(DirectUrlBuiltDist),
    Path(PathBuiltDist),
    GitPath(GitPathBuiltDist),
}

/// A source distribution from an index, URL, Git directory, Git path, path, or directory.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum SourceDist {
    Registry(RegistrySourceDist),
    DirectUrl(DirectUrlSourceDist),
    GitDirectory(GitDirectorySourceDist),
    GitPath(GitPathSourceDist),
    Path(PathSourceDist),
    Directory(DirectorySourceDist),
}

/// A wheel in a registry, such as `PyPI`.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct RegistryBuiltWheel {
    pub filename: WheelFilename,
    pub file: Box<File>,
    pub index: IndexUrl,
    /// Whether to validate the recorded size when downloading the wheel.
    pub size_is_authoritative: bool,
}

/// A wheel in a registry, such as `PyPI`.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct RegistryBuiltDist {
    /// All wheels for this distribution. This list always contains at least one wheel.
    pub wheels: Vec<RegistryBuiltWheel>,
    /// The best wheel for the current wheel tag environment.
    ///
    /// This index always points to a valid entry in `wheels`.
    pub best_wheel_index: usize,
    /// The source distribution, if one is available and compatible.
    ///
    /// This value is `None` if no source distribution exists or the user configuration excludes it.
    /// For example, `Requires-Python` or `--exclude-newer` can exclude a source distribution.
    pub sdist: Option<RegistrySourceDist>,
    // Ideally, this type would contain the index URL instead of `RegistryBuiltDist` and
    // `RegistrySourceDist`. However, `--find-links` can give wheels and source distributions
    // different index URLs for the same distribution.
    //
    // A universal lockfile still requires all wheels and source distributions for a distribution
    // to use equivalent index URLs.
}

/// A wheel at a URL.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct DirectUrlBuiltDist {
    /// Wheel URLs must end with the complete wheel filename.
    /// For example: `https://example.org/packages/flask-3.0.0-py3-none-any.whl`.
    pub filename: WheelFilename,
    /// The URL without the subdirectory fragment.
    pub location: Box<DisplaySafeUrl>,
    /// The URL that the user provided.
    pub url: VerbatimUrl,
    /// The archive size from the lockfile, if available.
    pub size: Option<u64>,
}

/// A wheel in a local directory.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct PathBuiltDist {
    pub filename: WheelFilename,
    /// The absolute path to the wheel to install.
    pub install_path: Box<Path>,
    /// The URL that the user provided.
    pub url: VerbatimUrl,
}

/// A wheel in a Git repository.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct GitPathBuiltDist {
    pub filename: WheelFilename,
    /// The URL without the revision and path fragment.
    pub git: Box<GitUrl>,
    /// The path to the distribution to install in the Git repository.
    pub install_path: PathBuf,
    /// The user-provided URL, including the revision and path fragment.
    pub url: VerbatimUrl,
}

/// A source distribution in a registry, such as `PyPI`.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct RegistrySourceDist {
    pub name: PackageName,
    pub version: Version,
    pub file: Box<File>,
    /// The file extension, such as `tar.gz` or `zip`.
    pub ext: SourceDistExtension,
    pub index: IndexUrl,
    /// Available wheels for this source distribution.
    ///
    /// Record these wheels even if none match the current environment. A universal lockfile must
    /// include wheels that are compatible with other environments.
    pub wheels: Vec<RegistryBuiltWheel>,
    /// Whether to validate the recorded size when downloading the source distribution.
    pub size_is_authoritative: bool,
}

/// A source distribution at a URL.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct DirectUrlSourceDist {
    /// Unlike [`DirectUrlBuiltDist`], a source URL does not need a versioned filename.
    /// For example: `foo @ https://github.com/org/repo/archive/master.zip`.
    pub name: PackageName,
    /// The URL without the subdirectory fragment.
    pub location: Box<DisplaySafeUrl>,
    /// The archive subdirectory that contains the source distribution.
    pub subdirectory: Option<Box<Path>>,
    /// The file extension, such as `tar.gz` or `zip`.
    pub ext: SourceDistExtension,
    /// The user-provided URL, including the subdirectory fragment.
    pub url: VerbatimUrl,
    /// The archive size from the lockfile, if available.
    pub size: Option<u64>,
}

/// A source distribution at the root of a Git repository or in a subdirectory.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct GitDirectorySourceDist {
    pub name: PackageName,
    /// The URL without the revision and subdirectory fragment.
    pub git: Box<GitUrl>,
    /// The Git repository subdirectory that contains the source distribution.
    pub subdirectory: Option<Box<Path>>,
    /// The user-provided URL, including the revision and subdirectory fragment.
    pub url: VerbatimUrl,
}

/// A source distribution in an archive, such as a `.tar.gz` file, in a Git repository.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct GitPathSourceDist {
    pub name: PackageName,
    /// The URL without the revision and subdirectory fragment.
    pub git: Box<GitUrl>,
    /// The path to the distribution to install in the Git repository.
    pub install_path: PathBuf,
    /// The file extension, such as `tar.gz` or `zip`.
    pub ext: SourceDistExtension,
    /// The user-provided URL, including the revision and subdirectory fragment.
    pub url: VerbatimUrl,
}

/// A source distribution in a local archive, such as a `.tar.gz` file.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct PathSourceDist {
    pub name: PackageName,
    pub version: Option<Version>,
    /// The absolute path to the distribution to install.
    pub install_path: Box<Path>,
    /// The file extension, such as `tar.gz` or `zip`.
    pub ext: SourceDistExtension,
    /// The URL that the user provided.
    pub url: VerbatimUrl,
}

/// Whether a source distribution is a first-party workspace member.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum FirstParty {
    Yes,
    No,
}

/// A source distribution in a local directory.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct DirectorySourceDist {
    pub name: PackageName,
    /// The absolute path to the distribution to install.
    pub install_path: Box<Path>,
    /// Whether to install the package in editable mode.
    pub editable: Option<bool>,
    /// Whether to build and install the package.
    pub r#virtual: Option<bool>,
    /// Whether the package is a first-party workspace member.
    pub first_party: FirstParty,
    /// The URL that the user provided.
    pub url: VerbatimUrl,
}

impl Dist {
    /// Create a built or source distribution from an `http://` or `https://` URL.
    pub fn from_http_url(
        name: PackageName,
        url: VerbatimUrl,
        location: DisplaySafeUrl,
        subdirectory: Option<Box<Path>>,
        ext: DistExtension,
    ) -> Result<Self, Error> {
        match ext {
            DistExtension::Wheel => {
                // Check that the wheel name matches the requirement name.
                let filename = WheelFilename::from_str(&url.filename()?)?;
                if filename.name != name {
                    return Err(Error::PackageNameMismatch(
                        name,
                        filename.name,
                        url.verbatim().to_string(),
                    ));
                }

                Ok(Self::Built(BuiltDist::DirectUrl(DirectUrlBuiltDist {
                    filename,
                    location: Box::new(location),
                    url,
                    size: None,
                })))
            }
            DistExtension::Source(ext) => {
                if !ext.is_pep625_compliant() {
                    return Err(Error::NotPep625Filename(url.verbatim().to_string()));
                }
                Ok(Self::Source(SourceDist::DirectUrl(DirectUrlSourceDist {
                    name,
                    location: Box::new(location),
                    subdirectory,
                    ext,
                    url,
                    size: None,
                })))
            }
        }
    }

    /// Create a local built or source distribution from a `file://` URL.
    pub fn from_file_url(
        name: PackageName,
        url: VerbatimUrl,
        install_path: &Path,
        ext: DistExtension,
    ) -> Result<Self, Error> {
        // Convert to an absolute path.
        let install_path = path::absolute(install_path)?;

        // Normalize the path.
        let install_path = normalize_absolute_path(&install_path)?;

        // Check that the path exists.
        if !install_path.exists() {
            return Err(Error::NotFound(url.to_url()));
        }

        // Check whether the path represents a built or source distribution.
        match ext {
            DistExtension::Wheel => {
                // Check that the wheel name matches the requirement name.
                let filename = install_path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .ok_or_else(|| Error::MissingWheelFilename(install_path.clone()))?;
                let filename = WheelFilename::from_str(filename)?;
                if filename.name != name {
                    return Err(Error::PackageNameMismatch(
                        name,
                        filename.name,
                        url.verbatim().to_string(),
                    ));
                }
                Ok(Self::Built(BuiltDist::Path(PathBuiltDist {
                    filename,
                    install_path: install_path.into_boxed_path(),
                    url,
                })))
            }
            DistExtension::Source(ext) => {
                if !ext.is_pep625_compliant() {
                    return Err(Error::NotPep625Filename(url.verbatim().to_string()));
                }

                // Record the version in the filename, if present.
                let version = url
                    .filename()
                    .ok()
                    .and_then(|filename| {
                        SourceDistFilename::parse(filename.as_ref(), ext, &name).ok()
                    })
                    .map(|filename| filename.version);

                Ok(Self::Source(SourceDist::Path(PathSourceDist {
                    name,
                    version,
                    install_path: install_path.into_boxed_path(),
                    ext,
                    url,
                })))
            }
        }
    }

    /// Create a local source tree from a `file://` URL.
    pub fn from_directory_url(
        name: PackageName,
        url: VerbatimUrl,
        install_path: &Path,
        editable: Option<bool>,
        r#virtual: Option<bool>,
    ) -> Result<Self, Error> {
        // Convert to an absolute path.
        let install_path = path::absolute(install_path)?;

        // Normalize the path.
        let install_path = normalize_absolute_path(&install_path)?;

        // Check that the path exists.
        if !install_path.exists() {
            return Err(Error::NotFound(url.to_url()));
        }

        // Create a directory source distribution.
        Ok(Self::Source(SourceDist::Directory(DirectorySourceDist {
            name,
            install_path: install_path.into_boxed_path(),
            editable,
            r#virtual,
            first_party: FirstParty::No,
            url,
        })))
    }

    /// Create a [`Dist`] for a source tree in a Git repository.
    ///
    /// The URL can use `git+https://` or `git+ssh://`.
    pub fn from_git_directory_url(
        name: PackageName,
        url: VerbatimUrl,
        git: GitUrl,
        subdirectory: Option<Box<Path>>,
    ) -> Result<Self, Error> {
        Ok(Self::Source(SourceDist::GitDirectory(
            GitDirectorySourceDist {
                name,
                git: Box::new(git),
                subdirectory,
                url,
            },
        )))
    }

    /// Create a [`Dist`] for a source archive in a Git repository.
    ///
    /// The URL can use `git+https://` or `git+ssh://`.
    pub fn from_git_path_url(
        name: PackageName,
        url: VerbatimUrl,
        git: GitUrl,
        install_path: PathBuf,
        ext: DistExtension,
    ) -> Result<Self, Error> {
        match ext {
            DistExtension::Wheel => {
                // Check that the wheel name matches the requirement name.
                let filename = install_path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .ok_or_else(|| Error::MissingWheelFilename(install_path.clone()))?;
                let filename = WheelFilename::from_str(filename)?;
                if filename.name != name {
                    return Err(Error::PackageNameMismatch(
                        name,
                        filename.name,
                        url.verbatim().to_string(),
                    ));
                }

                Ok(Self::Built(BuiltDist::GitPath(GitPathBuiltDist {
                    filename,
                    git: Box::new(git),
                    install_path,
                    url,
                })))
            }
            DistExtension::Source(ext) => {
                Ok(Self::Source(SourceDist::GitPath(GitPathSourceDist {
                    name,
                    git: Box::new(git),
                    install_path,
                    ext,
                    url,
                })))
            }
        }
    }

    /// Create a [`Dist`] for a URL-based distribution.
    pub fn from_url(name: PackageName, url: VerbatimParsedUrl) -> Result<Self, Error> {
        match url.parsed_url {
            ParsedUrl::Archive(archive) => Self::from_http_url(
                name,
                url.verbatim,
                archive.url,
                archive.subdirectory,
                archive.ext,
            ),
            ParsedUrl::Path(file) => {
                Self::from_file_url(name, url.verbatim, &file.install_path, file.ext)
            }
            ParsedUrl::Directory(directory) => Self::from_directory_url(
                name,
                url.verbatim,
                &directory.install_path,
                directory.editable,
                directory.r#virtual,
            ),
            ParsedUrl::GitDirectory(git) => {
                Self::from_git_directory_url(name, url.verbatim, git.url, git.subdirectory)
            }
            ParsedUrl::GitPath(git) => {
                Self::from_git_path_url(name, url.verbatim, git.url, git.install_path, git.ext)
            }
        }
    }

    /// Return `true` if the distribution is editable.
    fn is_editable(&self) -> bool {
        match self {
            Self::Source(dist) => dist.is_editable(),
            Self::Built(_) => false,
        }
    }

    /// Return `true` if the distribution refers to a local file or directory.
    fn is_local(&self) -> bool {
        match self {
            Self::Source(dist) => dist.is_local(),
            Self::Built(dist) => dist.is_local(),
        }
    }

    /// Return the [`IndexUrl`] if the distribution comes from a registry.
    pub fn index(&self) -> Option<&IndexUrl> {
        match self {
            Self::Built(dist) => dist.index(),
            Self::Source(dist) => dist.index(),
        }
    }

    /// Return the [`File`] if the registry supports the simple JSON API.
    pub fn file(&self) -> Option<&File> {
        match self {
            Self::Built(built) => built.file(),
            Self::Source(source) => source.file(),
        }
    }

    /// Return the source tree of the distribution, if available.
    pub fn source_tree(&self) -> Option<&Path> {
        match self {
            Self::Built { .. } => None,
            Self::Source(source) => source.source_tree(),
        }
    }

    /// Return the distribution version, if known.
    pub fn version(&self) -> Option<&Version> {
        match self {
            Self::Built(wheel) => Some(wheel.version()),
            Self::Source(source_dist) => source_dist.version(),
        }
    }
}

impl<'a> From<&'a Dist> for DistRef<'a> {
    fn from(dist: &'a Dist) -> Self {
        match dist {
            Dist::Built(built) => DistRef::Built(built),
            Dist::Source(source) => DistRef::Source(source),
        }
    }
}

impl<'a> From<&'a SourceDist> for DistRef<'a> {
    fn from(dist: &'a SourceDist) -> Self {
        DistRef::Source(dist)
    }
}

impl<'a> From<&'a BuiltDist> for DistRef<'a> {
    fn from(dist: &'a BuiltDist) -> Self {
        DistRef::Built(dist)
    }
}

impl BuiltDist {
    /// Return `true` if the distribution refers to a local file or directory.
    fn is_local(&self) -> bool {
        matches!(self, Self::Path(_))
    }

    /// Return the [`IndexUrl`] if the distribution comes from a registry.
    pub fn index(&self) -> Option<&IndexUrl> {
        match self {
            Self::Registry(registry) => Some(&registry.best_wheel().index),
            Self::DirectUrl(_) => None,
            Self::Path(_) => None,
            Self::GitPath(_) => None,
        }
    }

    /// Return the [`File`] if the distribution comes from a registry.
    fn file(&self) -> Option<&File> {
        match self {
            Self::Registry(registry) => Some(&registry.best_wheel().file),
            Self::DirectUrl(_) | Self::Path(_) | Self::GitPath(_) => None,
        }
    }

    pub fn version(&self) -> &Version {
        match self {
            Self::Registry(wheels) => &wheels.best_wheel().filename.version,
            Self::DirectUrl(wheel) => &wheel.filename.version,
            Self::Path(wheel) => &wheel.filename.version,
            Self::GitPath(wheel) => &wheel.filename.version,
        }
    }
}

impl SourceDist {
    /// Return the [`IndexUrl`] if the distribution comes from a registry.
    fn index(&self) -> Option<&IndexUrl> {
        match self {
            Self::Registry(registry) => Some(&registry.index),
            Self::DirectUrl(_)
            | Self::GitPath(_)
            | Self::GitDirectory(_)
            | Self::Path(_)
            | Self::Directory(_) => None,
        }
    }

    /// Return the [`File`] if the registry supports the simple JSON API.
    fn file(&self) -> Option<&File> {
        match self {
            Self::Registry(registry) => Some(&registry.file),
            Self::DirectUrl(_)
            | Self::GitPath(_)
            | Self::GitDirectory(_)
            | Self::Path(_)
            | Self::Directory(_) => None,
        }
    }

    /// Return the distribution [`Version`], if known.
    pub fn version(&self) -> Option<&Version> {
        match self {
            Self::Registry(source_dist) => Some(&source_dist.version),
            Self::DirectUrl(_)
            | Self::GitPath(_)
            | Self::GitDirectory(_)
            | Self::Path(_)
            | Self::Directory(_) => None,
        }
    }

    /// Return `true` if the distribution is editable.
    pub fn is_editable(&self) -> bool {
        match self {
            Self::Directory(DirectorySourceDist { editable, .. }) => editable.unwrap_or(false),
            _ => false,
        }
    }

    /// Return `true` if the distribution is virtual.
    pub fn is_virtual(&self) -> bool {
        match self {
            Self::Directory(DirectorySourceDist { r#virtual, .. }) => r#virtual.unwrap_or(false),
            _ => false,
        }
    }

    /// Return `true` if the distribution is a first-party workspace member.
    pub fn is_first_party(&self) -> bool {
        match self {
            Self::Directory(DirectorySourceDist {
                first_party: FirstParty::Yes,
                ..
            }) => true,
            Self::Directory(DirectorySourceDist {
                first_party: FirstParty::No,
                ..
            })
            | Self::Registry(_)
            | Self::DirectUrl(_)
            | Self::GitDirectory(_)
            | Self::GitPath(_)
            | Self::Path(_) => false,
        }
    }

    /// Return `true` if the distribution refers to a local file or directory.
    fn is_local(&self) -> bool {
        matches!(self, Self::Directory(_) | Self::Path(_))
    }

    /// Return the path if the source distribution is local.
    pub fn as_path(&self) -> Option<&Path> {
        match self {
            Self::Path(dist) => Some(&dist.install_path),
            Self::Directory(dist) => Some(&dist.install_path),
            _ => None,
        }
    }

    /// Return the source tree of the distribution, if available.
    fn source_tree(&self) -> Option<&Path> {
        match self {
            Self::Directory(dist) => Some(&dist.install_path),
            _ => None,
        }
    }
}

impl RegistryBuiltDist {
    /// Return the most compatible wheel in this distribution.
    pub fn best_wheel(&self) -> &RegistryBuiltWheel {
        &self.wheels[self.best_wheel_index]
    }
}

impl DirectUrlBuiltDist {
    /// Return the [`ParsedUrl`] for the distribution.
    pub fn to_parsed_url(&self) -> ParsedUrl {
        ParsedUrl::Archive(ParsedArchiveUrl::from_source(
            (*self.location).clone(),
            None,
            DistExtension::Wheel,
        ))
    }
}

impl PathBuiltDist {
    /// Return the [`ParsedUrl`] for the distribution.
    pub fn to_parsed_url(&self) -> ParsedUrl {
        ParsedUrl::Path(ParsedPathUrl::from_source(
            self.install_path.clone(),
            DistExtension::Wheel,
            self.url.to_url(),
        ))
    }
}

impl PathSourceDist {
    /// Return the [`ParsedUrl`] for the distribution.
    pub fn to_parsed_url(&self) -> ParsedUrl {
        ParsedUrl::Path(ParsedPathUrl::from_source(
            self.install_path.clone(),
            DistExtension::Source(self.ext),
            self.url.to_url(),
        ))
    }
}

impl DirectUrlSourceDist {
    /// Return the [`ParsedUrl`] for the distribution.
    pub fn to_parsed_url(&self) -> ParsedUrl {
        ParsedUrl::Archive(ParsedArchiveUrl::from_source(
            (*self.location).clone(),
            self.subdirectory.clone(),
            DistExtension::Source(self.ext),
        ))
    }
}

impl GitDirectorySourceDist {
    /// Return the [`ParsedUrl`] for the distribution.
    pub fn to_parsed_url(&self) -> ParsedUrl {
        ParsedUrl::GitDirectory(ParsedGitDirectoryUrl::from_source(
            (*self.git).clone(),
            self.subdirectory.clone(),
        ))
    }
}

impl GitPathBuiltDist {
    /// Return the [`ParsedUrl`] for the distribution.
    pub fn to_parsed_url(&self) -> ParsedUrl {
        ParsedUrl::GitPath(ParsedGitPathUrl::from_source(
            (*self.git).clone(),
            self.install_path.clone(),
            DistExtension::Wheel,
        ))
    }
}

impl GitPathSourceDist {
    /// Return the [`ParsedUrl`] for the distribution.
    pub fn to_parsed_url(&self) -> ParsedUrl {
        ParsedUrl::GitPath(ParsedGitPathUrl::from_source(
            (*self.git).clone(),
            self.install_path.clone(),
            DistExtension::Source(self.ext),
        ))
    }
}

impl DirectorySourceDist {
    /// Return the [`ParsedUrl`] for the distribution.
    pub fn to_parsed_url(&self) -> ParsedUrl {
        ParsedUrl::Directory(ParsedDirectoryUrl::from_source(
            self.install_path.clone(),
            self.editable,
            self.r#virtual,
            self.url.to_url(),
        ))
    }
}

impl Name for RegistryBuiltWheel {
    fn name(&self) -> &PackageName {
        &self.filename.name
    }
}

impl Name for RegistryBuiltDist {
    fn name(&self) -> &PackageName {
        self.best_wheel().name()
    }
}

impl Name for DirectUrlBuiltDist {
    fn name(&self) -> &PackageName {
        &self.filename.name
    }
}

impl Name for PathBuiltDist {
    fn name(&self) -> &PackageName {
        &self.filename.name
    }
}

impl Name for GitPathBuiltDist {
    fn name(&self) -> &PackageName {
        &self.filename.name
    }
}

impl Name for RegistrySourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for DirectUrlSourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for GitPathSourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for GitDirectorySourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for PathSourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for DirectorySourceDist {
    fn name(&self) -> &PackageName {
        &self.name
    }
}

impl Name for SourceDist {
    fn name(&self) -> &PackageName {
        match self {
            Self::Registry(dist) => dist.name(),
            Self::DirectUrl(dist) => dist.name(),
            Self::GitPath(dist) => dist.name(),
            Self::GitDirectory(dist) => dist.name(),
            Self::Path(dist) => dist.name(),
            Self::Directory(dist) => dist.name(),
        }
    }
}

impl Name for BuiltDist {
    fn name(&self) -> &PackageName {
        match self {
            Self::Registry(dist) => dist.name(),
            Self::DirectUrl(dist) => dist.name(),
            Self::Path(dist) => dist.name(),
            Self::GitPath(dist) => dist.name(),
        }
    }
}

impl Name for Dist {
    fn name(&self) -> &PackageName {
        match self {
            Self::Built(dist) => dist.name(),
            Self::Source(dist) => dist.name(),
        }
    }
}

impl Name for CompatibleDist<'_> {
    fn name(&self) -> &PackageName {
        match self {
            Self::InstalledDist(dist) => dist.name(),
            Self::SourceDist {
                sdist,
                prioritized: _,
            } => sdist.name(),
            Self::CompatibleWheel {
                wheel,
                priority: _,
                prioritized: _,
            } => wheel.name(),
            Self::IncompatibleWheel {
                sdist,
                wheel: _,
                prioritized: _,
            } => sdist.name(),
        }
    }
}

impl DistributionMetadata for RegistryBuiltWheel {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Version(&self.filename.version)
    }
}

impl DistributionMetadata for RegistryBuiltDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        self.best_wheel().version_or_url()
    }
}

impl DistributionMetadata for DirectUrlBuiltDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Url(&self.url)
    }

    fn version_id(&self) -> VersionId {
        VersionId::from_archive(self.location.as_ref().clone(), None)
    }
}

impl DistributionMetadata for PathBuiltDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Url(&self.url)
    }

    fn version_id(&self) -> VersionId {
        VersionId::from_path(self.install_path.as_ref())
    }
}

impl DistributionMetadata for GitPathBuiltDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Url(&self.url)
    }
}

impl DistributionMetadata for RegistrySourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Version(&self.version)
    }
}

impl DistributionMetadata for DirectUrlSourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Url(&self.url)
    }

    fn version_id(&self) -> VersionId {
        VersionId::from_archive(
            self.location.as_ref().clone(),
            self.subdirectory.clone().map(Path::into_path_buf),
        )
    }
}

impl DistributionMetadata for GitPathSourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Url(&self.url)
    }

    fn version_id(&self) -> VersionId {
        VersionId::from_git(self.git.as_ref(), Some(&self.install_path))
    }
}

impl DistributionMetadata for GitDirectorySourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Url(&self.url)
    }

    fn version_id(&self) -> VersionId {
        VersionId::from_git(self.git.as_ref(), self.subdirectory.as_deref())
    }
}

impl DistributionMetadata for PathSourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Url(&self.url)
    }

    fn version_id(&self) -> VersionId {
        VersionId::from_path(self.install_path.as_ref())
    }
}

impl DistributionMetadata for DirectorySourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        VersionOrUrlRef::Url(&self.url)
    }

    fn version_id(&self) -> VersionId {
        VersionId::from_directory(self.install_path.as_ref())
    }
}

impl DistributionMetadata for SourceDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        match self {
            Self::Registry(dist) => dist.version_or_url(),
            Self::DirectUrl(dist) => dist.version_or_url(),
            Self::GitPath(dist) => dist.version_or_url(),
            Self::GitDirectory(dist) => dist.version_or_url(),
            Self::Path(dist) => dist.version_or_url(),
            Self::Directory(dist) => dist.version_or_url(),
        }
    }

    fn version_id(&self) -> VersionId {
        match self {
            Self::Registry(dist) => dist.version_id(),
            Self::DirectUrl(dist) => dist.version_id(),
            Self::GitPath(dist) => dist.version_id(),
            Self::GitDirectory(dist) => dist.version_id(),
            Self::Path(dist) => dist.version_id(),
            Self::Directory(dist) => dist.version_id(),
        }
    }
}

impl DistributionMetadata for BuiltDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        match self {
            Self::Registry(dist) => dist.version_or_url(),
            Self::DirectUrl(dist) => dist.version_or_url(),
            Self::Path(dist) => dist.version_or_url(),
            Self::GitPath(dist) => dist.version_or_url(),
        }
    }

    fn version_id(&self) -> VersionId {
        match self {
            Self::Registry(dist) => dist.version_id(),
            Self::DirectUrl(dist) => dist.version_id(),
            Self::Path(dist) => dist.version_id(),
            Self::GitPath(dist) => dist.version_id(),
        }
    }
}

impl DistributionMetadata for Dist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        match self {
            Self::Built(dist) => dist.version_or_url(),
            Self::Source(dist) => dist.version_or_url(),
        }
    }

    fn version_id(&self) -> VersionId {
        match self {
            Self::Built(dist) => dist.version_id(),
            Self::Source(dist) => dist.version_id(),
        }
    }
}

impl RemoteSource for File {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        Ok(Cow::Borrowed(&self.filename))
    }

    fn size(&self) -> Option<u64> {
        self.size
    }
}

impl RemoteSource for Url {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        // Use the last URL segment as the filename.
        let mut path_segments = self
            .path_segments()
            .ok_or_else(|| Error::MissingPathSegments(self.to_string()))?;

        // `Url::path_segments` guarantees that this segment exists.
        let last = path_segments
            .next_back()
            .expect("path segments is non-empty");

        // Decode the filename, which may be percent-encoded.
        let filename = percent_encoding::percent_decode_str(last).decode_utf8()?;

        Ok(filename)
    }

    fn size(&self) -> Option<u64> {
        None
    }
}

impl RemoteSource for UrlString {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        let url = self.as_ref();
        if memchr3(b'?', b'#', b'%', url.as_bytes()).is_none()
            && let Some((_, filename)) = url.rsplit_once('/')
        {
            return Ok(Cow::Borrowed(filename));
        }

        // Get the last segment without the query or fragment.
        let last = self
            .base_str()
            .split('/')
            .next_back()
            .ok_or_else(|| Error::MissingPathSegments(self.to_string()))?;

        // Decode the filename, which may be percent-encoded.
        let filename = percent_encoding::percent_decode_str(last).decode_utf8()?;

        Ok(filename)
    }

    fn size(&self) -> Option<u64> {
        None
    }
}

impl RemoteSource for RegistryBuiltWheel {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.file.filename()
    }

    fn size(&self) -> Option<u64> {
        self.file.size()
    }
}

impl RemoteSource for RegistryBuiltDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.best_wheel().filename()
    }

    fn size(&self) -> Option<u64> {
        self.best_wheel().size()
    }
}

impl RemoteSource for RegistrySourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.file.filename()
    }

    fn size(&self) -> Option<u64> {
        self.file.size()
    }
}

impl RemoteSource for DirectUrlBuiltDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.size
    }
}

impl RemoteSource for DirectUrlSourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.size
    }
}

impl RemoteSource for GitPathSourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        // The filename is the last segment of the URL, before any `@`.
        match self.url.filename()? {
            Cow::Borrowed(filename) if let Some((_, suffix)) = filename.rsplit_once('@') => {
                Ok(Cow::Borrowed(suffix))
            }
            Cow::Owned(ref filename) if let Some((_, suffix)) = filename.rsplit_once('@') => {
                Ok(Cow::Owned(suffix.to_owned()))
            }
            filename => Ok(filename),
        }
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for GitDirectorySourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        // The filename is the last segment of the URL, before any `@`.
        match self.url.filename()? {
            Cow::Borrowed(filename) if let Some((_, suffix)) = filename.rsplit_once('@') => {
                Ok(Cow::Borrowed(suffix))
            }
            Cow::Owned(ref filename) if let Some((_, suffix)) = filename.rsplit_once('@') => {
                Ok(Cow::Owned(suffix.to_owned()))
            }
            filename => Ok(filename),
        }
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for PathBuiltDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for GitPathBuiltDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for PathSourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for DirectorySourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        self.url.filename()
    }

    fn size(&self) -> Option<u64> {
        self.url.size()
    }
}

impl RemoteSource for SourceDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        match self {
            Self::Registry(dist) => dist.filename(),
            Self::DirectUrl(dist) => dist.filename(),
            Self::GitPath(dist) => dist.filename(),
            Self::GitDirectory(dist) => dist.filename(),
            Self::Path(dist) => dist.filename(),
            Self::Directory(dist) => dist.filename(),
        }
    }

    fn size(&self) -> Option<u64> {
        match self {
            Self::Registry(dist) => dist.size(),
            Self::DirectUrl(dist) => dist.size(),
            Self::GitPath(dist) => dist.size(),
            Self::GitDirectory(dist) => dist.size(),
            Self::Path(dist) => dist.size(),
            Self::Directory(dist) => dist.size(),
        }
    }
}

impl RemoteSource for BuiltDist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        match self {
            Self::Registry(dist) => dist.filename(),
            Self::DirectUrl(dist) => dist.filename(),
            Self::Path(dist) => dist.filename(),
            Self::GitPath(dist) => dist.filename(),
        }
    }

    fn size(&self) -> Option<u64> {
        match self {
            Self::Registry(dist) => dist.size(),
            Self::DirectUrl(dist) => dist.size(),
            Self::Path(dist) => dist.size(),
            Self::GitPath(dist) => dist.size(),
        }
    }
}

impl RemoteSource for Dist {
    fn filename(&self) -> Result<Cow<'_, str>, Error> {
        match self {
            Self::Built(dist) => dist.filename(),
            Self::Source(dist) => dist.filename(),
        }
    }

    fn size(&self) -> Option<u64> {
        match self {
            Self::Built(dist) => dist.size(),
            Self::Source(dist) => dist.size(),
        }
    }
}

impl Identifier for DisplaySafeUrl {
    fn distribution_id(&self) -> DistributionId {
        DistributionId::Url(uv_cache_key::CanonicalUrl::new(self.clone()))
    }

    fn resource_id(&self) -> ResourceId {
        ResourceId::Url(uv_cache_key::RepositoryUrl::new(self.clone()))
    }
}

impl Identifier for File {
    fn distribution_id(&self) -> DistributionId {
        self.hashes
            .first()
            .cloned()
            .map(DistributionId::Digest)
            .unwrap_or_else(|| self.url.distribution_id())
    }

    fn resource_id(&self) -> ResourceId {
        self.hashes
            .first()
            .cloned()
            .map(ResourceId::Digest)
            .unwrap_or_else(|| self.url.resource_id())
    }
}

impl Identifier for Path {
    fn distribution_id(&self) -> DistributionId {
        DistributionId::PathBuf(self.to_path_buf())
    }

    fn resource_id(&self) -> ResourceId {
        ResourceId::PathBuf(self.to_path_buf())
    }
}

impl Identifier for FileLocation {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::RelativeUrl(base, url) => {
                DistributionId::RelativeUrl(base.to_string(), url.to_string())
            }
            Self::AbsoluteUrl(url) => DistributionId::AbsoluteUrl(url.to_string()),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::RelativeUrl(base, url) => {
                ResourceId::RelativeUrl(base.to_string(), url.to_string())
            }
            Self::AbsoluteUrl(url) => ResourceId::AbsoluteUrl(url.to_string()),
        }
    }
}

impl Identifier for RegistryBuiltWheel {
    fn distribution_id(&self) -> DistributionId {
        self.file.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.file.resource_id()
    }
}

impl Identifier for RegistryBuiltDist {
    fn distribution_id(&self) -> DistributionId {
        self.best_wheel().distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.best_wheel().resource_id()
    }
}

impl Identifier for RegistrySourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.file.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.file.resource_id()
    }
}

impl Identifier for DirectUrlBuiltDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for DirectUrlSourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for PathBuiltDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for GitPathBuiltDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for PathSourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for DirectorySourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for GitPathSourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for GitDirectorySourceDist {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for SourceDist {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Registry(dist) => dist.distribution_id(),
            Self::DirectUrl(dist) => dist.distribution_id(),
            Self::GitPath(dist) => dist.distribution_id(),
            Self::GitDirectory(dist) => dist.distribution_id(),
            Self::Path(dist) => dist.distribution_id(),
            Self::Directory(dist) => dist.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Registry(dist) => dist.resource_id(),
            Self::DirectUrl(dist) => dist.resource_id(),
            Self::GitPath(dist) => dist.resource_id(),
            Self::GitDirectory(dist) => dist.resource_id(),
            Self::Path(dist) => dist.resource_id(),
            Self::Directory(dist) => dist.resource_id(),
        }
    }
}

impl Identifier for BuiltDist {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Registry(dist) => dist.distribution_id(),
            Self::DirectUrl(dist) => dist.distribution_id(),
            Self::Path(dist) => dist.distribution_id(),
            Self::GitPath(dist) => dist.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Registry(dist) => dist.resource_id(),
            Self::DirectUrl(dist) => dist.resource_id(),
            Self::Path(dist) => dist.resource_id(),
            Self::GitPath(dist) => dist.resource_id(),
        }
    }
}

impl Identifier for InstalledDist {
    fn distribution_id(&self) -> DistributionId {
        self.install_path().distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.install_path().resource_id()
    }
}

impl Identifier for Dist {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Built(dist) => dist.distribution_id(),
            Self::Source(dist) => dist.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Built(dist) => dist.resource_id(),
            Self::Source(dist) => dist.resource_id(),
        }
    }
}

impl Identifier for DirectSourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for GitDirectorySourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for GitPathSourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for PathSourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for DirectorySourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        self.url.distribution_id()
    }

    fn resource_id(&self) -> ResourceId {
        self.url.resource_id()
    }
}

impl Identifier for SourceUrl<'_> {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Direct(url) => url.distribution_id(),
            Self::GitDirectory(url) => url.distribution_id(),
            Self::GitPath(url) => url.distribution_id(),
            Self::Path(url) => url.distribution_id(),
            Self::Directory(url) => url.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Direct(url) => url.resource_id(),
            Self::GitDirectory(url) => url.resource_id(),
            Self::GitPath(url) => url.resource_id(),
            Self::Path(url) => url.resource_id(),
            Self::Directory(url) => url.resource_id(),
        }
    }
}

impl Identifier for BuildableSource<'_> {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Dist(source) => source.distribution_id(),
            Self::Url(source) => source.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Dist(source) => source.resource_id(),
            Self::Url(source) => source.resource_id(),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{BuiltDist, Dist, RemoteSource, SourceDist, UrlString};
    use uv_redacted::DisplaySafeUrl;

    /// Check that the `Dist` types do not grow.
    #[test]
    fn dist_size() {
        assert!(size_of::<Dist>() <= 200, "{}", size_of::<Dist>());
        assert!(size_of::<BuiltDist>() <= 200, "{}", size_of::<BuiltDist>());
        assert!(
            size_of::<SourceDist>() <= 176,
            "{}",
            size_of::<SourceDist>()
        );
    }

    #[test]
    fn remote_source() {
        for url in [
            "https://example.com/foo-0.1.0.tar.gz",
            "https://example.com/foo%2D0.1.0.tar.gz",
            "https://example.com/foo-0.1.0.tar.gz#fragment",
            "https://example.com/foo-0.1.0.tar.gz?query",
            "https://example.com/foo-0.1.0.tar.gz?query#fragment",
            "https://example.com/foo-0.1.0.tar.gz?query=1/2#fragment",
            "https://example.com/foo-0.1.0.tar.gz?query=1/2#fragment/3",
            "https://example.com/foo%2D0.1.0.tar.gz?query=1/2#fragment/3",
        ] {
            let url = DisplaySafeUrl::parse(url).unwrap();
            assert_eq!(url.filename().unwrap(), "foo-0.1.0.tar.gz", "{url}");
            let url = UrlString::from(url.clone());
            assert_eq!(url.filename().unwrap(), "foo-0.1.0.tar.gz", "{url}");
        }
    }
}
