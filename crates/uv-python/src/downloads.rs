use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Display;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::str::FromStr;
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTimeError};
use std::{env, io};

use futures::{StreamExt, TryStreamExt};
use indexmap::{IndexMap, map::Entry};
use itertools::Itertools;
use owo_colors::OwoColorize;
use reqwest::Response;
use reqwest_retry::RetryError;
use reqwest_retry::policies::ExponentialBackoff;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWriteExt, BufWriter, ReadBuf};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tokio_util::either::Either;
use tracing::{debug, instrument};
use url::Url;

use uv_cache::{Cache, CacheBucket, CacheEntry};
use uv_cache_key::cache_digest;
use uv_client::{
    BaseClient, BaseClientBuilder, CacheControl, CachedClient, CachedClientError, ClientBuildError,
    Connectivity, RetriableError, WrappedReqwestError, fetch_with_url_fallback,
    retryable_on_request_failure,
};
use uv_distribution_filename::{ExtensionError, SourceDistExtension};
use uv_extract::hash::Hasher;
use uv_fs::{Simplified, rename_with_retry, write_atomic};
use uv_platform::{self as platform, Arch, Libc, Os, Platform};
use uv_preview::PreviewFeature;
use uv_pypi_types::{HashAlgorithm, HashDigest};
use uv_redacted::{DisplaySafeUrl, DisplaySafeUrlError};
use uv_static::{
    EnvVars, astral_mirror_base_url, astral_mirror_url_from_env, custom_astral_mirror_url,
};

use crate::PythonVariant;
use crate::implementation::{
    Error as ImplementationError, ImplementationName, LenientImplementationName,
};
use crate::installation::PythonInstallationKey;
use crate::managed::ManagedPythonInstallation;
use crate::python_version::{BuildVersionError, python_build_version_from_env};
use crate::{Interpreter, PythonRequest, PythonVersion, VersionRequest};

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    ImplementationError(#[from] ImplementationError),
    #[error("Expected download URL (`{0}`) to end in a supported file extension: {1}")]
    MissingExtension(String, ExtensionError),
    #[error("Invalid Python version: {0}")]
    InvalidPythonVersion(String),
    #[error("Invalid request key (empty request)")]
    EmptyRequest,
    #[error("Invalid request key (too many parts): {0}")]
    TooManyParts(String),
    #[error("Failed to download {0}")]
    NetworkError(DisplaySafeUrl, #[source] WrappedReqwestError),
    #[error(
        "Request failed after {retries} {subject} in {duration:.1}s",
        subject = if *retries > 1 { "retries" } else { "retry" },
        duration = duration.as_secs_f32()
    )]
    NetworkErrorWithRetries {
        #[source]
        err: Box<Self>,
        retries: u32,
        duration: Duration,
    },
    #[error("Failed to download {0}")]
    NetworkMiddlewareError(DisplaySafeUrl, #[source] anyhow::Error),
    #[error("Failed to extract archive: {0}")]
    ExtractError(String, #[source] uv_extract::Error),
    #[error("Failed to hash installation")]
    HashExhaustion(#[source] io::Error),
    #[error("Hash mismatch for `{installation}`\n\nExpected:\n{expected}\n\nComputed:\n{actual}")]
    HashMismatch {
        installation: String,
        expected: String,
        actual: String,
    },
    #[error("Invalid download URL")]
    InvalidUrl(#[from] DisplaySafeUrlError),
    #[error("Invalid download URL: {0}")]
    InvalidUrlFormat(DisplaySafeUrl),
    #[error("Invalid path in file URL: `{0}`")]
    InvalidFileUrl(String),
    #[error("Failed to create download directory")]
    DownloadDirError(#[source] io::Error),
    #[error("Failed to copy to: {0}", to.user_display())]
    CopyError {
        to: PathBuf,
        #[source]
        err: io::Error,
    },
    #[error("Failed to read managed Python installation directory: {0}", dir.user_display())]
    ReadError {
        dir: PathBuf,
        #[source]
        err: io::Error,
    },
    #[error("Failed to parse request part")]
    InvalidRequestPlatform(#[from] platform::Error),
    #[error("No download found for request: {}", _0.green())]
    NoDownloadFound(PythonDownloadRequest),
    #[error("A mirror was provided via `{0}`, but the URL does not match the expected format: {0}")]
    Mirror(&'static str, String),
    #[error("Failed to determine the libc used on the current platform")]
    LibcDetection(#[from] platform::LibcDetectionError),
    #[error("Unable to parse the JSON Python download list at {0}")]
    InvalidPythonDownloadsJSON(String, #[source] serde_json::Error),
    #[error("This version of uv is too old to support the JSON Python download list at {0}")]
    UnsupportedPythonDownloadsJSON(String),
    #[error("Error while fetching remote python downloads json from '{0}'")]
    FetchingPythonDownloadsJSONError(String, #[source] Box<Self>),
    #[error(transparent)]
    RemotePythonDownloadsJSONClient(Box<uv_client::Error>),
    #[error(transparent)]
    ClientBuild(Box<ClientBuildError>),
    #[error("Unable to parse NDJSON line at {0}")]
    InvalidPythonDownloadsNdjsonLine(String, #[source] serde_json::Error),
    #[error("Error while fetching remote python downloads NDJSON from '{0}'")]
    FetchingPythonDownloadsNdjsonError(String, #[source] Box<Self>),
    #[error("An offline Python installation was requested, but {file} (from {url}) is missing in {}", python_builds_dir.user_display())]
    OfflinePythonMissing {
        file: Box<PythonInstallationKey>,
        url: Box<DisplaySafeUrl>,
        python_builds_dir: PathBuf,
    },
    #[error(transparent)]
    BuildVersion(#[from] BuildVersionError),
    #[error("No download URL found for Python")]
    NoPythonDownloadUrlFound,
    #[error(transparent)]
    SystemTime(#[from] SystemTimeError),
}

impl RetriableError for Error {
    // Return the number of retries that were made to complete this request before this error was
    // returned.
    //
    // Note that e.g. 3 retries equates to 4 attempts.
    fn retries(&self) -> u32 {
        // Unfortunately different variants of `Error` track retry counts in different ways. We
        // could consider unifying the variants we handle here in `Error::from_reqwest_middleware`
        // instead, but both approaches will be fragile as new variants get added over time.
        if let Self::NetworkErrorWithRetries { retries, .. } = self {
            return *retries;
        }
        if let Self::NetworkMiddlewareError(_, anyhow_error) = self
            && let Some(RetryError::WithRetries { retries, .. }) =
                anyhow_error.downcast_ref::<RetryError>()
        {
            return *retries;
        }
        0
    }

    /// Returns `true` if trying an alternative URL makes sense after this error.
    ///
    /// HTTP-level failures (4xx, 5xx) and connection-level failures return `true`.
    /// Hash mismatches, extraction failures, and similar post-download errors return `false`
    /// because switching to a different host would not fix them.
    fn should_try_next_url(&self) -> bool {
        match self {
            // There are two primary reasons to try an alternative URL:
            // - HTTP/DNS/TCP/etc errors due to a mirror being blocked at various layers
            // - HTTP 404s from the mirror, which may mean the next URL still works
            // So we catch all network-level errors here.
            Self::NetworkError(..)
            | Self::NetworkMiddlewareError(..)
            | Self::NetworkErrorWithRetries { .. } => true,
            // `Io` uses `#[error(transparent)]`, so `source()` delegates to the inner error's
            // own source rather than returning the `io::Error` itself. We must unwrap it
            // explicitly so that `retryable_on_request_failure` can inspect the io error kind.
            Self::Io(err) => retryable_on_request_failure(err).is_some(),
            _ => false,
        }
    }

    fn into_retried(self, retries: u32, duration: Duration) -> Self {
        Self::NetworkErrorWithRetries {
            err: Box::new(self),
            retries,
            duration,
        }
    }
}

/// The URL prefix used by `python-build-standalone` releases on GitHub.
const CPYTHON_DOWNLOADS_URL_PREFIX: &str =
    "https://github.com/astral-sh/python-build-standalone/releases/download/";

/// The suffix appended to the Astral mirror base for `python-build-standalone` releases.
const CPYTHON_MIRROR_SUFFIX: &str = "/github/python-build-standalone/releases/download/";

/// Return the Astral mirror base URL for CPython downloads.
fn effective_cpython_mirror(astral_mirror_url: Option<&str>) -> String {
    format!(
        "{}{CPYTHON_MIRROR_SUFFIX}",
        astral_mirror_base_url(astral_mirror_url)
    )
}

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct ManagedPythonDownload {
    key: PythonInstallationKey,
    url: Cow<'static, str>,
    sha256: Option<Cow<'static, str>>,
    build: Option<&'static str>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Hash)]
pub struct PythonDownloadRequest {
    pub(crate) version: Option<VersionRequest>,
    pub(crate) implementation: Option<ImplementationName>,
    pub(crate) arch: Option<ArchRequest>,
    pub(crate) os: Option<Os>,
    pub(crate) libc: Option<Libc>,
    pub(crate) build: Option<String>,

    /// Whether to allow pre-releases or not. If not set, defaults to true if [`Self::version`] is
    /// not None, and false otherwise.
    pub(crate) prereleases: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchRequest {
    Explicit(Arch),
    Environment(Arch),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlatformRequest {
    os: Option<Os>,
    arch: Option<ArchRequest>,
    libc: Option<Libc>,
}

impl PlatformRequest {
    /// Check if this platform request is satisfied by a platform.
    pub(crate) fn matches(&self, platform: &Platform) -> bool {
        if let Some(os) = self.os
            && !platform.os.supports(os)
        {
            return false;
        }

        if let Some(arch) = self.arch
            && !arch.satisfied_by(platform)
        {
            return false;
        }

        if let Some(libc) = self.libc
            && platform.libc != libc
        {
            return false;
        }

        true
    }
}

impl Display for PlatformRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if let Some(os) = &self.os {
            parts.push(os.to_string());
        }
        if let Some(arch) = &self.arch {
            parts.push(arch.to_string());
        }
        if let Some(libc) = &self.libc {
            parts.push(libc.to_string());
        }
        write!(f, "{}", parts.join("-"))
    }
}

impl Display for ArchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Explicit(arch) | Self::Environment(arch) => write!(f, "{arch}"),
        }
    }
}

impl ArchRequest {
    fn satisfied_by(self, platform: &Platform) -> bool {
        match self {
            Self::Explicit(request) => request == platform.arch,
            Self::Environment(env) => {
                // Check if the environment's platform can run the target platform
                let env_platform = Platform::new(platform.os, env, platform.libc);
                env_platform.supports(platform)
            }
        }
    }

    pub fn inner(&self) -> Arch {
        match self {
            Self::Explicit(arch) | Self::Environment(arch) => *arch,
        }
    }
}

impl PythonDownloadRequest {
    fn new(
        version: Option<VersionRequest>,
        implementation: Option<ImplementationName>,
        arch: Option<ArchRequest>,
        os: Option<Os>,
        libc: Option<Libc>,
        prereleases: Option<bool>,
    ) -> Self {
        Self {
            version,
            implementation,
            arch,
            os,
            libc,
            build: None,
            prereleases,
        }
    }

    #[must_use]
    fn with_implementation(mut self, implementation: ImplementationName) -> Self {
        match implementation {
            // Pyodide is actually CPython with an Emscripten OS, we paper over that for usability
            ImplementationName::Pyodide => {
                self = self.with_os(Os::new(target_lexicon::OperatingSystem::Emscripten));
                self = self.with_arch(Arch::new(target_lexicon::Architecture::Wasm32, None));
                self = self.with_libc(Libc::Some(target_lexicon::Environment::Musl));
            }
            _ => {
                self.implementation = Some(implementation);
            }
        }
        self
    }

    #[must_use]
    pub fn with_version(mut self, version: VersionRequest) -> Self {
        self.version = Some(version);
        self
    }

    #[must_use]
    pub fn with_arch(mut self, arch: Arch) -> Self {
        self.arch = Some(ArchRequest::Explicit(arch));
        self
    }

    #[must_use]
    pub fn with_any_arch(mut self) -> Self {
        self.arch = None;
        self
    }

    #[must_use]
    fn with_os(mut self, os: Os) -> Self {
        self.os = Some(os);
        self
    }

    #[must_use]
    fn with_libc(mut self, libc: Libc) -> Self {
        self.libc = Some(libc);
        self
    }

    #[must_use]
    pub fn with_prereleases(mut self, prereleases: bool) -> Self {
        self.prereleases = Some(prereleases);
        self
    }

    /// Construct a new [`PythonDownloadRequest`] from a [`PythonRequest`] if possible.
    ///
    /// Returns [`None`] if the request kind is not compatible with a download, e.g., it is
    /// a request for a specific directory or executable name.
    pub fn from_request(request: &PythonRequest) -> Option<Self> {
        match request {
            PythonRequest::Version(version) => Some(Self::default().with_version(version.clone())),
            PythonRequest::Implementation(implementation) => {
                Some(Self::default().with_implementation(*implementation))
            }
            PythonRequest::ImplementationVersion(implementation, version) => Some(
                Self::default()
                    .with_implementation(*implementation)
                    .with_version(version.clone()),
            ),
            PythonRequest::Key(request) => Some(request.clone()),
            PythonRequest::Any => Some(Self {
                prereleases: Some(true), // Explicitly allow pre-releases for PythonRequest::Any
                ..Self::default()
            }),
            PythonRequest::Default => Some(Self::default()),
            // We can't download a managed installation for these request kinds
            PythonRequest::Directory(_)
            | PythonRequest::ExecutableName(_)
            | PythonRequest::File(_) => None,
        }
    }

    /// Fill empty entries with default values.
    ///
    /// Platform information is pulled from the environment.
    pub fn fill_platform(mut self) -> Result<Self, Error> {
        let platform = Platform::from_env().map_err(|err| match err {
            platform::Error::LibcDetectionError(err) => Error::LibcDetection(err),
            err => Error::InvalidRequestPlatform(err),
        })?;
        if self.arch.is_none() {
            self.arch = Some(ArchRequest::Environment(platform.arch));
        }
        if self.os.is_none() {
            self.os = Some(platform.os);
        }
        if self.libc.is_none() {
            self.libc = Some(platform.libc);
        }
        Ok(self)
    }

    /// Fill the build field from the environment variable relevant for the [`ImplementationName`].
    fn fill_build_from_env(mut self) -> Result<Self, Error> {
        if self.build.is_some() {
            return Ok(self);
        }
        let Some(implementation) = self.implementation else {
            return Ok(self);
        };

        self.build = python_build_version_from_env(implementation)?;
        Ok(self)
    }

    pub fn fill(mut self) -> Result<Self, Error> {
        if self.implementation.is_none() {
            self.implementation = Some(ImplementationName::CPython);
        }
        self = self.fill_platform()?;
        self = self.fill_build_from_env()?;
        Ok(self)
    }

    pub(crate) fn implementation(&self) -> Option<&ImplementationName> {
        self.implementation.as_ref()
    }

    pub(crate) fn version(&self) -> Option<&VersionRequest> {
        self.version.as_ref()
    }

    pub fn arch(&self) -> Option<&ArchRequest> {
        self.arch.as_ref()
    }

    pub fn libc(&self) -> Option<&Libc> {
        self.libc.as_ref()
    }

    pub fn take_version(&mut self) -> Option<VersionRequest> {
        self.version.take()
    }

    /// Remove default implementation and platform details so the request only contains
    /// explicitly user-specified segments.
    #[must_use]
    pub(crate) fn unset_defaults(self) -> Self {
        let request = self.unset_non_platform_defaults();

        if let Ok(host) = Platform::from_env() {
            request.unset_platform_defaults(&host)
        } else {
            request
        }
    }

    fn unset_non_platform_defaults(mut self) -> Self {
        self.implementation = self
            .implementation
            .filter(|implementation_name| *implementation_name != ImplementationName::default());

        self.version = self
            .version
            .filter(|version| !matches!(version, VersionRequest::Any | VersionRequest::Default));

        // Drop implicit architecture derived from environment so only user overrides remain.
        self.arch = self
            .arch
            .filter(|arch| !matches!(arch, ArchRequest::Environment(_)));

        self
    }

    #[cfg(test)]
    fn unset_defaults_for_host(self, host: &Platform) -> Self {
        self.unset_non_platform_defaults()
            .unset_platform_defaults(host)
    }

    fn unset_platform_defaults(mut self, host: &Platform) -> Self {
        self.os = self.os.filter(|os| *os != host.os);

        self.libc = self.libc.filter(|libc| *libc != host.libc);

        self.arch = self
            .arch
            .filter(|arch| !matches!(arch, ArchRequest::Explicit(explicit_arch) if *explicit_arch == host.arch));

        self
    }

    /// Drop patch and prerelease information so the request can be re-used for upgrades.
    #[must_use]
    pub(crate) fn without_patch(mut self) -> Self {
        self.version = self.version.take().map(VersionRequest::only_minor);
        self.prereleases = None;
        self.build = None;
        self
    }

    /// Return a compact string representation suitable for user-facing display.
    ///
    /// The resulting string only includes explicitly-set pieces of the request and returns
    /// [`None`] when no segments are explicitly set.
    pub(crate) fn simplified_display(self) -> Option<String> {
        let parts = [
            self.implementation
                .map(|implementation| implementation.to_string()),
            self.version.map(|version| version.to_string()),
            self.os.map(|os| os.to_string()),
            self.arch.map(|arch| arch.to_string()),
            self.libc.map(|libc| libc.to_string()),
        ];

        let joined = parts.into_iter().flatten().collect::<Vec<_>>().join("-");

        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    /// Whether this request is satisfied by an installation key.
    pub fn satisfied_by_key(&self, key: &PythonInstallationKey) -> bool {
        // Check platform requirements
        let request = PlatformRequest {
            os: self.os,
            arch: self.arch,
            libc: self.libc,
        };
        if !request.matches(key.platform()) {
            return false;
        }

        if let Some(implementation) = &self.implementation
            && key.implementation != LenientImplementationName::from(*implementation)
        {
            return false;
        }
        // If we don't allow pre-releases, don't match a key with a pre-release tag
        if !self.allows_prereleases() && key.prerelease.is_some() {
            return false;
        }
        if let Some(version) = &self.version {
            if !version.matches_major_minor_patch_prerelease(
                key.major,
                key.minor,
                key.patch,
                key.prerelease,
            ) {
                return false;
            }
            if let Some(variant) = version.variant()
                && variant != key.variant
            {
                return false;
            }
        }
        true
    }

    /// Whether this request is satisfied by a Python download.
    fn satisfied_by_download(&self, download: &ManagedPythonDownload) -> bool {
        // First check the key
        if !self.satisfied_by_key(download.key()) {
            return false;
        }

        // Then check the build if specified
        if let Some(ref requested_build) = self.build {
            let Some(download_build) = download.build() else {
                debug!(
                    "Skipping download `{}`: a build version was requested but is not available for this download",
                    download
                );
                return false;
            };

            if download_build != requested_build {
                debug!(
                    "Skipping download `{}`: requested build version `{}` does not match download build version `{}`",
                    download, requested_build, download_build
                );
                return false;
            }
        }

        true
    }

    /// Whether this download request opts-in to pre-release Python versions.
    pub(crate) fn allows_prereleases(&self) -> bool {
        self.prereleases.unwrap_or_else(|| {
            self.version
                .as_ref()
                .is_some_and(VersionRequest::allows_prereleases)
        })
    }

    /// Whether this download request opts-in to a debug Python version.
    pub(crate) fn allows_debug(&self) -> bool {
        self.version.as_ref().is_some_and(VersionRequest::is_debug)
    }

    /// Whether this download request opts-in to alternative Python implementations.
    pub(crate) fn allows_alternative_implementations(&self) -> bool {
        self.implementation
            .is_some_and(|implementation| !matches!(implementation, ImplementationName::CPython))
            || self.os.is_some_and(|os| os.is_emscripten())
    }

    pub(crate) fn satisfied_by_interpreter(&self, interpreter: &Interpreter) -> bool {
        let executable = interpreter.sys_executable().display();
        if let Some(version) = self.version()
            && !version.matches_interpreter(interpreter)
        {
            let interpreter_version = interpreter.python_version();
            debug!(
                "Skipping interpreter at `{executable}`: version `{interpreter_version}` does not match request `{version}`"
            );
            return false;
        }
        let platform = self.platform();
        let interpreter_platform = Platform::from(interpreter.platform());
        if !platform.matches(&interpreter_platform) {
            debug!(
                "Skipping interpreter at `{executable}`: platform `{interpreter_platform}` does not match request `{platform}`",
            );
            return false;
        }
        if let Some(implementation) = self.implementation()
            && !implementation.matches_interpreter(interpreter)
        {
            debug!(
                "Skipping interpreter at `{executable}`: implementation `{}` does not match request `{implementation}`",
                interpreter.implementation_name(),
            );
            return false;
        }
        true
    }

    /// Extract the platform components of this request.
    pub(crate) fn platform(&self) -> PlatformRequest {
        PlatformRequest {
            os: self.os,
            arch: self.arch,
            libc: self.libc,
        }
    }
}

impl TryFrom<&PythonInstallationKey> for PythonDownloadRequest {
    type Error = LenientImplementationName;

    fn try_from(key: &PythonInstallationKey) -> Result<Self, Self::Error> {
        let implementation = match key.implementation().into_owned() {
            LenientImplementationName::Known(name) => name,
            unknown @ LenientImplementationName::Unknown(_) => return Err(unknown),
        };

        Ok(Self::new(
            Some(VersionRequest::MajorMinor(
                key.major(),
                key.minor(),
                *key.variant(),
            )),
            Some(implementation),
            Some(ArchRequest::Explicit(*key.arch())),
            Some(*key.os()),
            Some(*key.libc()),
            Some(key.prerelease().is_some()),
        ))
    }
}

impl From<&ManagedPythonInstallation> for PythonDownloadRequest {
    fn from(installation: &ManagedPythonInstallation) -> Self {
        let key = installation.key();
        Self::new(
            Some(VersionRequest::from(&key.version())),
            match &key.implementation {
                LenientImplementationName::Known(implementation) => Some(*implementation),
                LenientImplementationName::Unknown(name) => unreachable!(
                    "Managed Python installations are expected to always have known implementation names, found {name}"
                ),
            },
            Some(ArchRequest::Explicit(*key.arch())),
            Some(*key.os()),
            Some(*key.libc()),
            Some(key.prerelease.is_some()),
        )
    }
}

impl Display for PythonDownloadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if let Some(implementation) = self.implementation {
            parts.push(implementation.to_string());
        } else {
            parts.push("any".to_string());
        }
        if let Some(version) = &self.version {
            parts.push(version.to_string());
        } else {
            parts.push("any".to_string());
        }
        if let Some(os) = &self.os {
            parts.push(os.to_string());
        } else {
            parts.push("any".to_string());
        }
        if let Some(arch) = self.arch {
            parts.push(arch.to_string());
        } else {
            parts.push("any".to_string());
        }
        if let Some(libc) = self.libc {
            parts.push(libc.to_string());
        } else {
            parts.push("any".to_string());
        }
        write!(f, "{}", parts.join("-"))
    }
}
impl FromStr for PythonDownloadRequest {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        #[derive(Debug, Clone)]
        enum Position {
            Start,
            Implementation,
            Version,
            Os,
            Arch,
            Libc,
            End,
        }

        impl Position {
            fn next(&self) -> Self {
                match self {
                    Self::Start => Self::Implementation,
                    Self::Implementation => Self::Version,
                    Self::Version => Self::Os,
                    Self::Os => Self::Arch,
                    Self::Arch => Self::Libc,
                    Self::Libc => Self::End,
                    Self::End => Self::End,
                }
            }
        }

        #[derive(Debug)]
        struct State<'a, P: Iterator<Item = &'a str>> {
            parts: P,
            part: Option<&'a str>,
            position: Position,
            error: Option<Error>,
            count: usize,
        }

        impl<'a, P: Iterator<Item = &'a str>> State<'a, P> {
            fn new(parts: P) -> Self {
                Self {
                    parts,
                    part: None,
                    position: Position::Start,
                    error: None,
                    count: 0,
                }
            }

            fn next_part(&mut self) {
                self.next_position();
                self.part = self.parts.next();
                self.count += 1;
                self.error.take();
            }

            fn next_position(&mut self) {
                self.position = self.position.next();
            }

            fn record_err(&mut self, err: Error) {
                // For now, we only record the first error encountered. We could record all of the
                // errors for a given part, then pick the most appropriate one later.
                self.error.get_or_insert(err);
            }
        }

        if s.is_empty() {
            return Err(Error::EmptyRequest);
        }

        let mut parts = s.split('-');

        let mut implementation = None;
        let mut version = None;
        let mut os = None;
        let mut arch = None;
        let mut libc = None;

        let mut state = State::new(parts.by_ref());
        state.next_part();

        while let Some(part) = state.part {
            match state.position {
                Position::Start => unreachable!("We start before the loop"),
                Position::Implementation => {
                    if part.eq_ignore_ascii_case("any") {
                        state.next_part();
                        continue;
                    }
                    match ImplementationName::from_str(part) {
                        Ok(val) => {
                            implementation = Some(val);
                            state.next_part();
                        }
                        Err(err) => {
                            state.next_position();
                            state.record_err(err.into());
                        }
                    }
                }
                Position::Version => {
                    if part.eq_ignore_ascii_case("any") {
                        state.next_part();
                        continue;
                    }
                    match VersionRequest::from_str(part)
                        .map_err(|_| Error::InvalidPythonVersion(part.to_string()))
                    {
                        // Err(err) if !first_part => return Err(err),
                        Ok(val) => {
                            version = Some(val);
                            state.next_part();
                        }
                        Err(err) => {
                            state.next_position();
                            state.record_err(err);
                        }
                    }
                }
                Position::Os => {
                    if part.eq_ignore_ascii_case("any") {
                        state.next_part();
                        continue;
                    }
                    match Os::from_str(part) {
                        Ok(val) => {
                            os = Some(val);
                            state.next_part();
                        }
                        Err(err) => {
                            state.next_position();
                            state.record_err(err.into());
                        }
                    }
                }
                Position::Arch => {
                    if part.eq_ignore_ascii_case("any") {
                        state.next_part();
                        continue;
                    }
                    match Arch::from_str(part) {
                        Ok(val) => {
                            arch = Some(ArchRequest::Explicit(val));
                            state.next_part();
                        }
                        Err(err) => {
                            state.next_position();
                            state.record_err(err.into());
                        }
                    }
                }
                Position::Libc => {
                    if part.eq_ignore_ascii_case("any") {
                        state.next_part();
                        continue;
                    }
                    match Libc::from_str(part) {
                        Ok(val) => {
                            libc = Some(val);
                            state.next_part();
                        }
                        Err(err) => {
                            state.next_position();
                            state.record_err(err.into());
                        }
                    }
                }
                Position::End => {
                    if state.count > 5 {
                        return Err(Error::TooManyParts(s.to_string()));
                    }

                    // Throw the first error for the current part
                    //
                    // TODO(zanieb): It's plausible another error variant is a better match but it
                    // sounds hard to explain how? We could peek at the next item in the parts, and
                    // see if that informs the type of this one, or we could use some sort of
                    // similarity or common error matching, but this sounds harder.
                    if let Some(err) = state.error {
                        return Err(err);
                    }
                    state.next_part();
                }
            }
        }

        Ok(Self::new(version, implementation, arch, os, libc, None))
    }
}

const BUILTIN_PYTHON_DOWNLOADS_JSON: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/download-metadata-minified.json"));

/// Default URL for the remote Python download metadata endpoint (NDJSON format).
const REMOTE_PYTHON_DOWNLOAD_METADATA_URL: &str = "https://raw.githubusercontent.com/astral-sh/versions/refs/heads/main/v1/python-build-standalone.ndjson";

const VERSIONS_CACHE_FILENAME: &str = "python-build-standalone.ndjson";
const VERSIONS_CACHE_META_FILENAME: &str = "python-build-standalone.meta.json";

#[derive(Debug, Deserialize, Serialize)]
struct VersionsCacheMeta {
    content_length: u64,
    etag: Option<String>,
}

fn versions_cache_entries(cache: &Cache, url: &DisplaySafeUrl) -> (CacheEntry, CacheEntry) {
    let shard = cache.shard(
        CacheBucket::Python,
        format!("versions/{}", cache_digest(&url.as_str())),
    );
    (
        shard.entry(VERSIONS_CACHE_FILENAME),
        shard.entry(VERSIONS_CACHE_META_FILENAME),
    )
}

async fn read_versions_cache(
    content_entry: &CacheEntry,
    meta_entry: &CacheEntry,
) -> Option<(Vec<u8>, VersionsCacheMeta)> {
    let metadata = fs_err::tokio::read(meta_entry.path()).await.ok()?;
    let metadata: VersionsCacheMeta = serde_json::from_slice(&metadata).ok()?;
    let content = fs_err::tokio::read(content_entry.path()).await.ok()?;

    if content.len() as u64 != metadata.content_length {
        debug!(
            "Cached Python downloads metadata length mismatch: expected {}, got {}",
            metadata.content_length,
            content.len()
        );
        return None;
    }

    Some((content, metadata))
}

async fn write_versions_cache(
    content_entry: &CacheEntry,
    meta_entry: &CacheEntry,
    source: &str,
    content: &[u8],
    etag: Option<String>,
) -> Result<(), Error> {
    // Avoid retaining truncated responses or otherwise invalid manifests.
    parse_ndjson_bytes(source, content)?;

    let metadata = VersionsCacheMeta {
        content_length: content.len() as u64,
        etag,
    };
    let metadata = serde_json::to_vec(&metadata)
        .map_err(|err| io::Error::other(format!("Failed to serialize cache metadata: {err}")))?;

    fs_err::tokio::create_dir_all(content_entry.dir()).await?;
    write_atomic(content_entry.path(), content).await?;
    write_atomic(meta_entry.path(), metadata).await?;
    Ok(())
}

async fn fetch_ndjson_full(
    client: &BaseClient,
    url: &DisplaySafeUrl,
) -> Result<(Vec<u8>, Option<String>), Error> {
    let start = Instant::now();
    let response = client
        .for_host(url)
        .get(Url::from(url.clone()))
        .send()
        .await
        .map_err(|err| Error::from_reqwest_middleware(url.clone(), err))?
        .error_for_status()
        .map_err(|err| Error::from_reqwest(url.clone(), err, None, start))?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let content = response
        .bytes()
        .await
        .map_err(|err| Error::from_reqwest(url.clone(), err, None, start))?;

    Ok((content.to_vec(), etag))
}

async fn fetch_ndjson_cached(
    client: &BaseClient,
    url: &DisplaySafeUrl,
    cache: &Cache,
) -> Result<Vec<u8>, Error> {
    let (content_entry, meta_entry) = versions_cache_entries(cache, url);
    let shard = content_entry.shard();
    let _lock = shard
        .lock()
        .await
        .map_err(|err| io::Error::other(format!("Failed to lock Python downloads cache: {err}")))?;
    let cached = read_versions_cache(&content_entry, &meta_entry).await;

    if client.connectivity().is_offline()
        && let Some((content, _)) = cached
    {
        debug!("Using cached Python downloads metadata in offline mode");
        return Ok(content);
    }

    if let Some((cached_content, cached_meta)) = &cached {
        let response = client
            .for_host(url)
            .head(Url::from(url.clone()))
            .send()
            .await;

        if let Ok(response) = response
            && let Ok(response) = response.error_for_status()
        {
            let current_etag = response
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned);
            if current_etag.is_some() && current_etag == cached_meta.etag {
                debug!("Using cached Python downloads metadata with matching ETag");
                return Ok(cached_content.clone());
            }
        }
    }

    match fetch_ndjson_full(client, url).await {
        Ok((content, etag)) => {
            if let Err(err) = write_versions_cache(
                &content_entry,
                &meta_entry,
                &url.to_string(),
                &content,
                etag,
            )
            .await
            {
                debug!("Failed to cache Python downloads metadata: {err}");
            }
            Ok(content)
        }
        Err(err) => {
            if let Some((content, _)) = cached {
                debug!("Using stale cached Python downloads metadata after refresh failed: {err}");
                Ok(content)
            } else {
                Err(err)
            }
        }
    }
}

pub struct ManagedPythonDownloadList {
    downloads: Vec<ManagedPythonDownload>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct JsonPythonDownload {
    name: String,
    arch: JsonArch,
    os: String,
    libc: String,
    major: u8,
    minor: u8,
    patch: u8,
    prerelease: Option<String>,
    url: String,
    sha256: Option<String>,
    variant: Option<String>,
    build: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct JsonArch {
    family: String,
    variant: Option<String>,
}

/// A Python version entry from the NDJSON format.
///
/// Each line represents one Python version with all its platform artifacts.
#[derive(Debug, Deserialize, Clone)]
struct NdjsonPythonVersionInfo {
    /// Version string in format "3.15.0a5+20260114" (version + build)
    version: String,
    /// All artifacts for this version across platforms
    artifacts: Vec<NdjsonPythonArtifact>,
}

/// A single artifact from the NDJSON format.
#[derive(Debug, Deserialize, Clone)]
struct NdjsonPythonArtifact {
    /// Platform string in Rust target triple format (e.g., "aarch64-apple-darwin")
    platform: String,
    /// Build variant (e.g., `install_only`, `freethreaded+pgo+lto+full`)
    variant: String,
    /// Download URL
    url: String,
    /// SHA256 hash of the artifact
    sha256: Option<String>,
}

/// Detected format for the download list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DownloadListFormat {
    Json,
    Ndjson,
}

/// Detect the format of a download list based on the URL or path extension.
fn detect_download_list_format(url_or_path: &str) -> DownloadListFormat {
    let is_ndjson = if let Ok(url) = Url::parse(url_or_path)
        && matches!(url.scheme(), "http" | "https" | "file")
    {
        url.path().ends_with(".ndjson")
    } else {
        url_or_path.ends_with(".ndjson")
    };

    if is_ndjson {
        DownloadListFormat::Ndjson
    } else {
        DownloadListFormat::Json
    }
}

/// Parse a version string with optional build suffix.
///
/// Format: "3.15.0a5+20260114" -> (PythonVersion("3.15.0a5"), Some("20260114"))
fn parse_version_with_build(value: &str) -> Result<(PythonVersion, Option<&str>), Error> {
    let (version, build) = value
        .split_once('+')
        .map_or((value, None), |(version, build)| (version, Some(build)));
    let version =
        PythonVersion::from_str(version).map_err(|_| Error::InvalidPythonVersion(value.into()))?;

    Ok((version, build))
}

/// Parse the NDJSON variant string to determine Python variant.
///
/// Variants can be: `install_only`, `install_only_stripped`, `freethreaded+pgo+lto+full`, etc.
fn parse_ndjson_variant(variant: &str) -> Option<PythonVariant> {
    let flavor = variant.rsplit('+').next()?;
    if !matches!(flavor, "full" | "install_only" | "install_only_stripped") {
        return None;
    }

    let mut debug = false;
    let mut freethreaded = false;
    for option in variant.split('+') {
        match option {
            "debug" => debug = true,
            "freethreaded" | "shared-freethreaded" => freethreaded = true,
            "static" | "static-noopt" => return None,
            _ => {}
        }
    }

    match (debug, freethreaded, flavor) {
        (true, true, _) => Some(PythonVariant::FreethreadedDebug),
        (true, false, _) => Some(PythonVariant::Debug),
        (false, true, _) => Some(PythonVariant::Freethreaded),
        (false, false, "install_only" | "install_only_stripped") => Some(PythonVariant::default()),
        _ => None,
    }
}

/// Prefer stripped install-only archives and optimized full archives for each Python variant.
fn ndjson_artifact_priority(variant: &str) -> (u8, u8) {
    let flavor = variant.rsplit('+').next().unwrap_or_default();
    let flavor_priority = match flavor {
        "install_only_stripped" => 0,
        "install_only" => 1,
        _ => 2,
    };
    let optimization_priority = 2
        - u8::from(variant.split('+').any(|option| option == "pgo"))
        - u8::from(variant.split('+').any(|option| option == "lto"));

    (flavor_priority, optimization_priority)
}

/// Parse an NDJSON version info into a list of [`ManagedPythonDownload`]s.
fn parse_ndjson_version_info(version_info: NdjsonPythonVersionInfo) -> Vec<ManagedPythonDownload> {
    let (version, build) = match parse_version_with_build(&version_info.version) {
        Ok((version, build)) => (version, build),
        Err(error) => {
            debug!(
                "Skipping NDJSON entry: Invalid version '{}' - {}",
                version_info.version, error
            );
            return Vec::new();
        }
    };

    let build = build.map(|build| Box::leak(build.to_owned().into_boxed_str()) as &'static str);
    let mut downloads = IndexMap::new();

    for artifact in version_info.artifacts {
        let priority = ndjson_artifact_priority(&artifact.variant);
        let Some(download) = parse_ndjson_artifact(&version, build, artifact) else {
            continue;
        };

        match downloads.entry(download.key().clone()) {
            Entry::Vacant(entry) => {
                entry.insert((priority, download));
            }
            Entry::Occupied(mut entry) if priority < entry.get().0 => {
                entry.insert((priority, download));
            }
            Entry::Occupied(_) => {}
        }
    }

    downloads
        .into_values()
        .map(|(_, download)| download)
        .collect()
}

/// Parse a single NDJSON artifact into a [`ManagedPythonDownload`].
fn parse_ndjson_artifact(
    version: &PythonVersion,
    build: Option<&'static str>,
    artifact: NdjsonPythonArtifact,
) -> Option<ManagedPythonDownload> {
    // Parse the variant to determine if this is a build we want
    let mut python_variant = parse_ndjson_variant(&artifact.variant)?;
    let mut platform = artifact.platform.as_str();
    for (suffix, suffix_variant) in [
        ("-debug", PythonVariant::Debug),
        ("-freethreaded", PythonVariant::Freethreaded),
    ] {
        if let Some(stripped) = platform.strip_suffix(suffix) {
            platform = stripped;
            python_variant = match (python_variant, suffix_variant) {
                (PythonVariant::Freethreaded, PythonVariant::Debug)
                | (PythonVariant::Debug, PythonVariant::Freethreaded) => {
                    PythonVariant::FreethreadedDebug
                }
                (PythonVariant::Default, variant) => variant,
                (variant, _) => variant,
            };
        }
    }

    // Parse the platform triple using the centralized Platform parser
    let platform = match Platform::from_cargo_dist_triple(platform) {
        Ok(platform) => platform,
        Err(error) => {
            debug!(
                "Skipping NDJSON artifact: Failed to parse platform '{}' - {}",
                artifact.platform, error
            );
            return None;
        }
    };

    // Implementation is always CPython for python-build-standalone
    let implementation = LenientImplementationName::Known(ImplementationName::CPython);

    Some(ManagedPythonDownload {
        key: PythonInstallationKey::new_from_version(
            implementation,
            version,
            platform,
            python_variant,
        ),
        url: Cow::Owned(artifact.url),
        sha256: artifact.sha256.map(Cow::Owned),
        build,
    })
}

#[derive(Debug, Clone)]
pub enum DownloadResult {
    AlreadyAvailable(PathBuf),
    Fetched(PathBuf),
}

impl ManagedPythonDownloadList {
    /// Iterate over all [`ManagedPythonDownload`]s.
    fn iter_all(&self) -> impl Iterator<Item = &ManagedPythonDownload> {
        self.downloads.iter()
    }

    /// Iterate over all [`ManagedPythonDownload`]s that match the request.
    pub fn iter_matching(
        &self,
        request: &PythonDownloadRequest,
    ) -> impl Iterator<Item = &ManagedPythonDownload> {
        self.iter_all()
            .filter(move |download| request.satisfied_by_download(download))
    }

    /// Return the first [`ManagedPythonDownload`] matching a request, if any.
    ///
    /// If there is no stable version matching the request, a compatible pre-release version will
    /// be searched for — even if a pre-release was not explicitly requested.
    pub fn find(&self, request: &PythonDownloadRequest) -> Result<&ManagedPythonDownload, Error> {
        if let Some(download) = self.iter_matching(request).next() {
            return Ok(download);
        }

        if !request.allows_prereleases()
            && let Some(download) = self
                .iter_matching(&request.clone().with_prereleases(true))
                .next()
        {
            return Ok(download);
        }

        Err(Error::NoDownloadFound(request.clone()))
    }

    /// Load available Python distributions from a provided source or the compiled-in list.
    ///
    /// `python_downloads_json_url` can be either `None`, to use the default list (taken from
    /// `crates/uv-python/download-metadata.json`), or `Some` local path
    /// or file://, http://, or https:// URL.
    ///
    /// When [`PreviewFeature::RemotePythonDownloadMetadata`] is enabled and no explicit URL is
    /// provided, the downloads are fetched from the default NDJSON endpoint.
    ///
    /// Returns an error if the provided list could not be opened, if the JSON is invalid, or if it
    /// does not parse into the expected data structure.
    pub async fn new(
        client_builder: &BaseClientBuilder<'_>,
        cache: &Cache,
        python_downloads_json_url: Option<&str>,
    ) -> Result<Self, Error> {
        // file:// URLs are converted to local file reads, and we also support parsing bare
        // filenames like "/tmp/py.json", not just "file:///tmp/py.json". Note that
        // "C:\Temp\py.json" should be considered a filename, even though Url::parse would
        // successfully misparse it as a URL with scheme "C".
        enum Source<'a> {
            BuiltIn,
            Path(Cow<'a, Path>),
            Http(DisplaySafeUrl),
            Ndjson(DisplaySafeUrl),
        }

        // Determine the source and format
        let source = if let Some(url_or_path) = python_downloads_json_url {
            // Explicit URL provided - detect format from extension
            let is_ndjson = detect_download_list_format(url_or_path) == DownloadListFormat::Ndjson;

            if let Ok(url) = DisplaySafeUrl::parse(url_or_path) {
                match url.scheme() {
                    "http" | "https" => {
                        if is_ndjson {
                            Source::Ndjson(url)
                        } else {
                            Source::Http(url)
                        }
                    }
                    "file" => Source::Path(Cow::Owned(
                        url.to_file_path().or(Err(Error::InvalidUrlFormat(url)))?,
                    )),
                    _ => Source::Path(Cow::Borrowed(Path::new(url_or_path))),
                }
            } else {
                Source::Path(Cow::Borrowed(Path::new(url_or_path)))
            }
        } else if uv_preview::is_enabled_explicitly(PreviewFeature::RemotePythonDownloadMetadata) {
            // Preview flag enabled - use default remote metadata endpoint
            let url = DisplaySafeUrl::parse(REMOTE_PYTHON_DOWNLOAD_METADATA_URL)?;
            Source::Ndjson(url)
        } else {
            Source::BuiltIn
        };

        let downloads = match source {
            Source::BuiltIn => parse_json_downloads(parse_downloads_json(
                BUILTIN_PYTHON_DOWNLOADS_JSON,
                "EMBEDDED IN THE BINARY".to_owned(),
            )?),
            Source::Path(ref path) => {
                let bytes = fs_err::read(path.as_ref())?;
                let source = path.to_string_lossy();
                if detect_download_list_format(&source) == DownloadListFormat::Ndjson {
                    parse_ndjson_bytes(&source, &bytes)?
                } else {
                    parse_json_downloads(parse_downloads_json(&bytes, source.into_owned())?)
                }
            }
            Source::Http(ref url) => {
                let client = CachedClient::new(
                    client_builder
                        .build()
                        .map_err(|err| Error::ClientBuild(Box::new(err)))?,
                );
                let downloads = fetch_downloads_from_url(&client, cache, url)
                    .await
                    .map_err(|err| match err {
                        err @ (Error::InvalidPythonDownloadsJSON(..)
                        | Error::UnsupportedPythonDownloadsJSON(..)) => err,
                        err => {
                            Error::FetchingPythonDownloadsJSONError(url.to_string(), Box::new(err))
                        }
                    })?;
                parse_json_downloads(downloads)
            }
            Source::Ndjson(ref url) => {
                let client = client_builder
                    .build()
                    .map_err(|err| Error::ClientBuild(Box::new(err)))?;
                let content = fetch_ndjson_cached(&client, url, cache)
                    .await
                    .map_err(|err| {
                        Error::FetchingPythonDownloadsNdjsonError(url.to_string(), Box::new(err))
                    })?;
                parse_ndjson_bytes(&url.to_string(), &content)?
            }
        };

        Ok(Self { downloads })
    }

    /// Load matching Python distributions, stopping after an optional result limit.
    pub async fn new_filtered(
        client_builder: &BaseClientBuilder<'_>,
        cache: &Cache,
        python_downloads_json_url: Option<&str>,
        filter: Option<&PythonDownloadRequest>,
        limit: Option<usize>,
    ) -> Result<Self, Error> {
        if let Some(url) = remote_ndjson_url(python_downloads_json_url)? {
            if limit == Some(1)
                && let Some(request) = filter
            {
                let download =
                    Self::find_streaming(client_builder, cache, python_downloads_json_url, request)
                        .await?;
                return Ok(Self {
                    downloads: download.into_iter().collect(),
                });
            }

            let client = client_builder
                .build()
                .map_err(|err| Error::ClientBuild(Box::new(err)))?;
            let content = fetch_ndjson_cached(&client, &url, cache)
                .await
                .map_err(|err| {
                    Error::FetchingPythonDownloadsNdjsonError(url.to_string(), Box::new(err))
                })?;
            let downloads = parse_ndjson_bytes_filtered(
                &url.to_string(),
                &content,
                |download| filter.is_none_or(|request| request.satisfied_by_download(download)),
                limit,
            )?;
            return Ok(Self { downloads });
        }

        let list = Self::new(client_builder, cache, python_downloads_json_url).await?;
        if limit == Some(1)
            && let Some(filter) = filter
        {
            return Ok(Self {
                downloads: list.find(filter).ok().cloned().into_iter().collect(),
            });
        }

        let mut downloads = list.downloads;
        if let Some(filter) = filter {
            downloads.retain(|download| filter.satisfied_by_download(download));
        }
        if let Some(limit) = limit {
            downloads.truncate(limit);
        }
        Ok(Self { downloads })
    }

    /// Find one matching Python distribution without consuming the whole remote manifest.
    async fn find_streaming(
        client_builder: &BaseClientBuilder<'_>,
        cache: &Cache,
        python_downloads_json_url: Option<&str>,
        request: &PythonDownloadRequest,
    ) -> Result<Option<ManagedPythonDownload>, Error> {
        let Some(url) = remote_ndjson_url(python_downloads_json_url)? else {
            return Ok(Self::new(client_builder, cache, python_downloads_json_url)
                .await?
                .find(request)
                .ok()
                .cloned());
        };

        let client = client_builder
            .build()
            .map_err(|err| Error::ClientBuild(Box::new(err)))?;
        let (content_entry, meta_entry) = versions_cache_entries(cache, &url);
        let prerelease_request =
            (!request.allows_prereleases()).then(|| request.clone().with_prereleases(true));

        if read_versions_cache(&content_entry, &meta_entry)
            .await
            .is_some()
        {
            let content = fetch_ndjson_cached(&client, &url, cache)
                .await
                .map_err(|err| {
                    Error::FetchingPythonDownloadsNdjsonError(url.to_string(), Box::new(err))
                })?;
            let mut prerelease = None;
            let found = parse_ndjson_bytes_with(&url.to_string(), &content, |download| {
                select_download(
                    request,
                    prerelease_request.as_ref(),
                    &mut prerelease,
                    download,
                )
            })?;
            return Ok(found.or(prerelease));
        }

        let mut prerelease = None;
        let found = fetch_ndjson_streaming(&client, &url, |download| {
            select_download(
                request,
                prerelease_request.as_ref(),
                &mut prerelease,
                download,
            )
        })
        .await
        .map_err(|err| Error::FetchingPythonDownloadsNdjsonError(url.to_string(), Box::new(err)))?;

        Ok(found.or(prerelease))
    }

    /// Load available Python distributions from the compiled-in list only.
    /// for testing purposes.
    pub fn new_only_embedded() -> Result<Self, Error> {
        let json_downloads: HashMap<String, JsonPythonDownload> =
            serde_json::from_slice(BUILTIN_PYTHON_DOWNLOADS_JSON).map_err(|e| {
                Error::InvalidPythonDownloadsJSON("EMBEDDED IN THE BINARY".to_owned(), e)
            })?;
        let result = parse_json_downloads(json_downloads);
        Ok(Self { downloads: result })
    }
}

fn remote_ndjson_url(
    python_downloads_json_url: Option<&str>,
) -> Result<Option<DisplaySafeUrl>, Error> {
    if let Some(source) = python_downloads_json_url {
        if detect_download_list_format(source) != DownloadListFormat::Ndjson {
            return Ok(None);
        }
        return match DisplaySafeUrl::parse(source) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => Ok(Some(url)),
            _ => Ok(None),
        };
    }

    if uv_preview::is_enabled_explicitly(PreviewFeature::RemotePythonDownloadMetadata) {
        return Ok(Some(DisplaySafeUrl::parse(
            REMOTE_PYTHON_DOWNLOAD_METADATA_URL,
        )?));
    }

    Ok(None)
}

/// Parse the downloads JSON.
///
/// `source` is where the JSON came from for error reporting.
fn parse_downloads_json(
    buf: &[u8],
    source: String,
) -> Result<HashMap<String, JsonPythonDownload>, Error> {
    match serde_json::from_slice(buf) {
        Ok(data) => Ok(data),
        Err(e) => {
            // As an explicit compatibility mechanism, if there's a top-level "version" key, it
            // means it's a newer format than we know how to deal with. Before reporting a
            // parse error about the format of JsonPythonDownload, check for that key. We can do
            // this by parsing into a Map<String, IgnoredAny> which allows any valid JSON on the
            // value side. (Because it's zero-sized, Clippy suggests Set<String>, but that won't
            // have the same parsing effect.)
            #[expect(clippy::zero_sized_map_values)]
            if let Ok(keys) = serde_json::from_slice::<HashMap<String, serde::de::IgnoredAny>>(buf)
                && keys.contains_key("version")
            {
                Err(Error::UnsupportedPythonDownloadsJSON(source))
            } else {
                Err(Error::InvalidPythonDownloadsJSON(source, e))
            }
        }
    }
}

async fn fetch_downloads_from_url(
    client: &CachedClient,
    cache: &Cache,
    url: &DisplaySafeUrl,
) -> Result<HashMap<String, JsonPythonDownload>, Error> {
    let cache_entry = cache.entry(
        CacheBucket::Python,
        "downloads-json",
        format!("{}.msgpack", cache_digest(&url.as_str())),
    );
    let cache_control = match client.uncached().connectivity() {
        Connectivity::Online => CacheControl::from(cache.freshness(&cache_entry, None, None)?),
        Connectivity::Offline => CacheControl::AllowStale,
    };

    let request = client
        .uncached()
        .for_host(url)
        .get(Url::from(url.clone()))
        .build()
        .map_err(|err| Error::NetworkError(url.clone(), WrappedReqwestError::from(err)))?;

    let response_callback = async |response: Response| {
        let bytes = response
            .bytes()
            .await
            .map_err(|err| Error::NetworkError(url.clone(), WrappedReqwestError::from(err)))?;
        parse_downloads_json(&bytes, url.to_string())
    };

    client
        .get_serde_with_retry(request, &cache_entry, cache_control, response_callback)
        .await
        .map_err(|err| match err {
            CachedClientError::Client(err) => Error::RemotePythonDownloadsJSONClient(Box::new(err)),
            CachedClientError::Callback {
                err,
                retries,
                duration,
            } => match err {
                // Avoid double-wrapping errors.
                err @ (Error::InvalidPythonDownloadsJSON(..)
                | Error::UnsupportedPythonDownloadsJSON(..)) => err,
                err if retries > 0 => err.into_retried(retries, duration),
                err => err,
            },
        })
}

/// Visit remote Python downloads in manifest order and stop as soon as a match is found.
async fn fetch_ndjson_streaming<T>(
    client: &BaseClient,
    url: &DisplaySafeUrl,
    mut visitor: impl FnMut(ManagedPythonDownload) -> ControlFlow<T>,
) -> Result<Option<T>, Error> {
    let start = Instant::now();
    let response = client
        .for_host(url)
        .get(Url::from(url.clone()))
        .send()
        .await
        .map_err(|err| Error::from_reqwest_middleware(url.clone(), err))?;

    let response = response
        .error_for_status()
        .map_err(|err| Error::from_reqwest(url.clone(), err, None, start))?;

    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| Error::from_reqwest(url.clone(), err, None, start))?;
        buffer.extend_from_slice(&chunk);

        while let Some(newline_position) = buffer.iter().position(|byte| *byte == b'\n') {
            if let Some(value) =
                visit_ndjson_line(&url.to_string(), &buffer[..newline_position], &mut visitor)?
            {
                return Ok(Some(value));
            }
            buffer.drain(..=newline_position);
        }
    }

    if let Some(value) = visit_ndjson_line(&url.to_string(), &buffer, &mut visitor)? {
        return Ok(Some(value));
    }

    Ok(None)
}

fn select_download(
    request: &PythonDownloadRequest,
    prerelease_request: Option<&PythonDownloadRequest>,
    prerelease: &mut Option<ManagedPythonDownload>,
    download: ManagedPythonDownload,
) -> ControlFlow<ManagedPythonDownload> {
    if request.satisfied_by_download(&download) {
        return ControlFlow::Break(download);
    }

    if prerelease.is_none()
        && prerelease_request.is_some_and(|request| request.satisfied_by_download(&download))
    {
        *prerelease = Some(download);
    }

    ControlFlow::Continue(())
}

fn visit_ndjson_line<T>(
    source: &str,
    line: &[u8],
    visitor: &mut impl FnMut(ManagedPythonDownload) -> ControlFlow<T>,
) -> Result<Option<T>, Error> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }

    let version_info: NdjsonPythonVersionInfo = serde_json::from_slice(line)
        .map_err(|err| Error::InvalidPythonDownloadsNdjsonLine(source.to_string(), err))?;
    for download in parse_ndjson_version_info(version_info) {
        if let ControlFlow::Break(value) = visitor(download) {
            return Ok(Some(value));
        }
    }

    Ok(None)
}

fn parse_ndjson_bytes_with<T>(
    source: &str,
    content: &[u8],
    mut visitor: impl FnMut(ManagedPythonDownload) -> ControlFlow<T>,
) -> Result<Option<T>, Error> {
    for line in content.split(|byte| *byte == b'\n') {
        if let Some(value) = visit_ndjson_line(source, line, &mut visitor)? {
            return Ok(Some(value));
        }
    }

    Ok(None)
}

/// Parse NDJSON content from bytes into a list of [`ManagedPythonDownload`]s.
fn parse_ndjson_bytes(source: &str, content: &[u8]) -> Result<Vec<ManagedPythonDownload>, Error> {
    let mut downloads = Vec::new();
    parse_ndjson_bytes_with(source, content, |download| {
        downloads.push(download);
        ControlFlow::<()>::Continue(())
    })?;
    downloads.sort_by(|a, b| Ord::cmp(&b.key, &a.key));
    Ok(downloads)
}

fn parse_ndjson_bytes_filtered(
    source: &str,
    content: &[u8],
    predicate: impl Fn(&ManagedPythonDownload) -> bool,
    limit: Option<usize>,
) -> Result<Vec<ManagedPythonDownload>, Error> {
    if limit == Some(0) {
        return Ok(Vec::new());
    }

    let mut downloads = Vec::new();
    parse_ndjson_bytes_with(source, content, |download| {
        if predicate(&download) {
            downloads.push(download);
            if limit.is_some_and(|limit| downloads.len() >= limit) {
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    })?;
    downloads.sort_by(|a, b| Ord::cmp(&b.key, &a.key));
    Ok(downloads)
}

impl ManagedPythonDownload {
    pub(crate) fn url(&self) -> &Cow<'static, str> {
        &self.url
    }

    pub fn key(&self) -> &PythonInstallationKey {
        &self.key
    }

    fn os(&self) -> &Os {
        self.key.os()
    }

    pub(crate) fn sha256(&self) -> Option<&Cow<'static, str>> {
        self.sha256.as_ref()
    }

    pub fn build(&self) -> Option<&'static str> {
        self.build
    }

    /// Download and extract a Python distribution, retrying on failure.
    ///
    /// For CPython without a user-configured mirror, the default Astral mirror is tried first.
    /// Each attempt tries all URLs in sequence without backoff between them; backoff is only
    /// applied after all URLs have been exhausted.
    #[instrument(skip_all, fields(download = % self.key()))]
    pub async fn fetch_with_retry(
        &self,
        client: &BaseClient,
        retry_policy: &ExponentialBackoff,
        installation_dir: &Path,
        scratch_dir: &Path,
        reinstall: bool,
        python_install_mirror: Option<&str>,
        pypy_install_mirror: Option<&str>,
        reporter: Option<&dyn Reporter>,
    ) -> Result<DownloadResult, Error> {
        let urls = self.download_urls(python_install_mirror, pypy_install_mirror)?;
        if urls.is_empty() {
            return Err(Error::NoPythonDownloadUrlFound);
        }
        fetch_with_url_fallback(&urls, *retry_policy, &format!("`{}`", self.key()), |url| {
            self.fetch_from_url(
                url,
                client,
                installation_dir,
                scratch_dir,
                reinstall,
                reporter,
            )
        })
        .await
    }

    /// Download and extract a Python distribution from the given URL.
    async fn fetch_from_url(
        &self,
        url: DisplaySafeUrl,
        client: &BaseClient,
        installation_dir: &Path,
        scratch_dir: &Path,
        reinstall: bool,
        reporter: Option<&dyn Reporter>,
    ) -> Result<DownloadResult, Error> {
        let path = installation_dir.join(self.key().to_string());

        // If it is not a reinstall and the dir already exists, return it.
        if !reinstall && path.is_dir() {
            return Ok(DownloadResult::AlreadyAvailable(path));
        }

        // We improve filesystem compatibility by using neither the URL-encoded `%2B` nor the `+` it
        // decodes to.
        let filename = url
            .path_segments()
            .ok_or_else(|| Error::InvalidUrlFormat(url.clone()))?
            .next_back()
            .ok_or_else(|| Error::InvalidUrlFormat(url.clone()))?
            .replace("%2B", "-");
        debug_assert!(
            filename
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'),
            "Unexpected char in filename: {filename}"
        );
        let ext = SourceDistExtension::from_path(&filename)
            .map_err(|err| Error::MissingExtension(url.to_string(), err))?;

        let temp_dir = tempfile::tempdir_in(scratch_dir).map_err(Error::DownloadDirError)?;

        if let Some(python_builds_dir) =
            env::var_os(EnvVars::UV_PYTHON_CACHE_DIR).filter(|s| !s.is_empty())
        {
            let python_builds_dir = PathBuf::from(python_builds_dir);
            fs_err::create_dir_all(&python_builds_dir)?;
            let hash_prefix = match self.sha256.as_deref() {
                Some(sha) => {
                    // Shorten the hash to avoid too-long-filename errors
                    &sha[..9]
                }
                None => "none",
            };
            let target_cache_file = python_builds_dir.join(format!("{hash_prefix}-{filename}"));

            // Download the archive to the cache, or return a reader if we have it in cache.
            // TODO(konsti): We should "tee" the write so we can do the download-to-cache and unpacking
            // in one step.
            let (reader, size): (Box<dyn AsyncRead + Unpin>, Option<u64>) =
                match fs_err::tokio::File::open(&target_cache_file).await {
                    Ok(file) => {
                        debug!(
                            "Extracting existing `{}`",
                            target_cache_file.simplified_display()
                        );
                        let size = file.metadata().await?.len();
                        let reader = Box::new(tokio::io::BufReader::new(file));
                        (reader, Some(size))
                    }
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {
                        // Point the user to which file is missing where and where to download it
                        if client.connectivity().is_offline() {
                            return Err(Error::OfflinePythonMissing {
                                file: Box::new(self.key().clone()),
                                url: Box::new(url.clone()),
                                python_builds_dir,
                            });
                        }

                        self.download_archive(
                            &url,
                            client,
                            reporter,
                            &python_builds_dir,
                            &target_cache_file,
                        )
                        .await?;

                        debug!("Extracting `{}`", target_cache_file.simplified_display());
                        let file = fs_err::tokio::File::open(&target_cache_file).await?;
                        let size = file.metadata().await?.len();
                        let reader = Box::new(tokio::io::BufReader::new(file));
                        (reader, Some(size))
                    }
                    Err(err) => return Err(err.into()),
                };

            // Extract the downloaded archive into a temporary directory.
            self.extract_reader(
                reader,
                temp_dir.path(),
                &filename,
                ext,
                size,
                reporter,
                Direction::Extract,
            )
            .await?;
        } else {
            // Avoid overlong log lines
            debug!("Downloading {url}");
            debug!(
                "Extracting {filename} to temporary location: {}",
                temp_dir.path().simplified_display()
            );

            let (reader, size) = read_url(&url, client).await?;
            self.extract_reader(
                reader,
                temp_dir.path(),
                &filename,
                ext,
                size,
                reporter,
                Direction::Download,
            )
            .await?;
        }

        // Extract the top-level directory.
        let mut extracted = match uv_extract::strip_component(temp_dir.path()) {
            Ok(top_level) => top_level,
            Err(uv_extract::Error::NonSingularArchive(_)) => temp_dir.path().to_path_buf(),
            Err(err) => return Err(Error::ExtractError(filename, err)),
        };

        // If the distribution is a `full` archive, the Python installation is in the `install` directory.
        if extracted.join("install").is_dir() {
            extracted = extracted.join("install");
        // If the distribution is a Pyodide archive, the Python installation is in the `pyodide-root/dist` directory.
        } else if self.os().is_emscripten() {
            extracted = extracted.join("pyodide-root").join("dist");
        }

        #[cfg(unix)]
        {
            // Pyodide distributions require all of the supporting files to be alongside the Python
            // executable, so they don't have a `bin` directory. We create it and link
            // `bin/pythonX.Y` to `dist/python`.
            if self.os().is_emscripten() {
                fs_err::create_dir_all(extracted.join("bin"))?;
                fs_err::os::unix::fs::symlink(
                    "../python",
                    extracted
                        .join("bin")
                        .join(format!("python{}.{}", self.key.major, self.key.minor)),
                )?;
            }

            // If the distribution is missing a `python` -> `pythonX.Y` symlink, add it.
            //
            // We skip for Windows distributions, allowing cross-installs from Unix.
            //
            // Pyodide releases never contain this link by default.
            //
            // PEP 394 permits it, and python-build-standalone releases after `20240726` include it,
            // but releases prior to that date do not.
            if !self.os().is_windows() {
                match fs_err::os::unix::fs::symlink(
                    format!("python{}.{}", self.key.major, self.key.minor),
                    extracted.join("bin").join("python"),
                ) {
                    Ok(()) => {}
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(err) => return Err(err.into()),
                }
            }
        }

        // Remove the target if it already exists.
        if path.is_dir() {
            debug!("Removing existing directory: {}", path.user_display());
            fs_err::tokio::remove_dir_all(&path).await?;
        }

        // Persist it to the target.
        debug!("Moving {} to {}", extracted.display(), path.user_display());
        rename_with_retry(extracted, &path)
            .await
            .map_err(|err| Error::CopyError {
                to: path.clone(),
                err,
            })?;

        Ok(DownloadResult::Fetched(path))
    }

    /// Download the managed Python archive into the cache directory.
    async fn download_archive(
        &self,
        url: &DisplaySafeUrl,
        client: &BaseClient,
        reporter: Option<&dyn Reporter>,
        python_builds_dir: &Path,
        target_cache_file: &Path,
    ) -> Result<(), Error> {
        debug!(
            "Downloading {} to `{}`",
            url,
            target_cache_file.simplified_display()
        );

        let (mut reader, size) = read_url(url, client).await?;
        let temp_dir = tempfile::tempdir_in(python_builds_dir)?;
        let temp_file = temp_dir.path().join("download");

        // Download to a temporary file. We verify the hash when unpacking the file.
        {
            let mut archive_writer = BufWriter::new(fs_err::tokio::File::create(&temp_file).await?);

            // Download with or without progress bar.
            if let Some(reporter) = reporter {
                let key = reporter.on_request_start(Direction::Download, &self.key, size);
                tokio::io::copy(
                    &mut ProgressReader::new(reader, key, reporter),
                    &mut archive_writer,
                )
                .await?;
                reporter.on_request_complete(Direction::Download, key);
            } else {
                tokio::io::copy(&mut reader, &mut archive_writer).await?;
            }

            archive_writer.flush().await?;
        }
        // Move the completed file into place, invalidating the `File` instance.
        match rename_with_retry(&temp_file, target_cache_file).await {
            Ok(()) => {}
            Err(_) if target_cache_file.is_file() => {}
            Err(err) => return Err(err.into()),
        }
        Ok(())
    }

    /// Extract a Python interpreter archive into a (temporary) directory, either from a file or
    /// from a download stream.
    async fn extract_reader(
        &self,
        reader: impl AsyncRead + Unpin,
        target: &Path,
        filename: &String,
        ext: SourceDistExtension,
        size: Option<u64>,
        reporter: Option<&dyn Reporter>,
        direction: Direction,
    ) -> Result<(), Error> {
        let mut hashers = if self.sha256.is_some() {
            vec![Hasher::from(HashAlgorithm::Sha256)]
        } else {
            vec![]
        };
        let mut hasher = uv_extract::hash::HashReader::new(reader, &mut hashers);

        if let Some(reporter) = reporter {
            let progress_key = reporter.on_request_start(direction, &self.key, size);
            let mut reader = ProgressReader::new(&mut hasher, progress_key, reporter);
            uv_extract::stream::archive(&mut reader, ext, target)
                .await
                .map_err(|err| Error::ExtractError(filename.to_owned(), err))?;
            reporter.on_request_complete(direction, progress_key);
        } else {
            uv_extract::stream::archive(&mut hasher, ext, target)
                .await
                .map_err(|err| Error::ExtractError(filename.to_owned(), err))?;
        }
        hasher.finish().await.map_err(Error::HashExhaustion)?;

        // Check the hash
        if let Some(expected) = self.sha256.as_deref() {
            let actual = HashDigest::from(hashers.pop().unwrap()).digest;
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(Error::HashMismatch {
                    installation: self.key.to_string(),
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                });
            }
        }

        Ok(())
    }

    #[cfg(test)]
    fn python_version(&self) -> PythonVersion {
        self.key.version()
    }

    /// Return the ordered list of [`Url`]s to try when downloading the distribution.
    ///
    /// For CPython without a user-configured mirror, the default Astral mirror is listed first,
    /// followed by the canonical GitHub URL as a fallback.
    ///
    /// For all other cases (user mirror explicitly set, PyPy, GraalPy, Pyodide), a single URL
    /// is returned with no fallback.
    pub fn download_urls(
        &self,
        python_install_mirror: Option<&str>,
        pypy_install_mirror: Option<&str>,
    ) -> Result<Vec<DisplaySafeUrl>, Error> {
        let custom_astral_mirror = astral_mirror_url_from_env();
        self.download_urls_with_astral_mirror(
            python_install_mirror,
            pypy_install_mirror,
            custom_astral_mirror.as_deref(),
        )
    }

    fn download_urls_with_astral_mirror(
        &self,
        python_install_mirror: Option<&str>,
        pypy_install_mirror: Option<&str>,
        astral_mirror_url: Option<&str>,
    ) -> Result<Vec<DisplaySafeUrl>, Error> {
        let astral_mirror_url = custom_astral_mirror_url(astral_mirror_url);
        match self.key.implementation {
            LenientImplementationName::Known(ImplementationName::CPython) => {
                if let Some(mirror) = python_install_mirror {
                    // User-configured mirror: use it exclusively, no automatic fallback.
                    let Some(suffix) = self.url.strip_prefix(CPYTHON_DOWNLOADS_URL_PREFIX) else {
                        return Err(Error::Mirror(
                            EnvVars::UV_PYTHON_INSTALL_MIRROR,
                            self.url.to_string(),
                        ));
                    };
                    return Ok(vec![DisplaySafeUrl::parse(
                        format!("{}/{}", mirror.trim_end_matches('/'), suffix).as_str(),
                    )?]);
                }
                // No user mirror: try the default/custom Astral mirror first.
                if let Some(suffix) = self.url.strip_prefix(CPYTHON_DOWNLOADS_URL_PREFIX) {
                    let effective_mirror = effective_cpython_mirror(astral_mirror_url);
                    let mirror_url = DisplaySafeUrl::parse(
                        format!("{}/{}", effective_mirror.trim_end_matches('/'), suffix).as_str(),
                    )?;
                    // When a custom Astral mirror is set, use it exclusively.
                    if astral_mirror_url.is_some() {
                        return Ok(vec![mirror_url]);
                    }
                    // Otherwise fall back to the canonical GitHub URL.
                    let canonical_url = DisplaySafeUrl::parse(&self.url)?;
                    return Ok(vec![mirror_url, canonical_url]);
                }
            }

            LenientImplementationName::Known(ImplementationName::PyPy) => {
                if let Some(mirror) = pypy_install_mirror {
                    let Some(suffix) = self.url.strip_prefix("https://downloads.python.org/pypy/")
                    else {
                        return Err(Error::Mirror(
                            EnvVars::UV_PYPY_INSTALL_MIRROR,
                            self.url.to_string(),
                        ));
                    };
                    return Ok(vec![DisplaySafeUrl::parse(
                        format!("{}/{}", mirror.trim_end_matches('/'), suffix).as_str(),
                    )?]);
                }
            }

            _ => {}
        }

        Ok(vec![DisplaySafeUrl::parse(&self.url)?])
    }
}

fn parse_json_downloads(
    json_downloads: HashMap<String, JsonPythonDownload>,
) -> Vec<ManagedPythonDownload> {
    json_downloads
        .into_iter()
        .filter_map(|(key, entry)| {
            let implementation = match entry.name.as_str() {
                "cpython" => LenientImplementationName::Known(ImplementationName::CPython),
                "pypy" => LenientImplementationName::Known(ImplementationName::PyPy),
                "graalpy" => LenientImplementationName::Known(ImplementationName::GraalPy),
                _ => LenientImplementationName::Unknown(entry.name.clone()),
            };

            let arch_str = match entry.arch.family.as_str() {
                "armv5tel" => Cow::Borrowed("armv5te"),
                // The `gc` variant of riscv64 is the common base instruction set and
                // is the target in `python-build-standalone`
                // See https://github.com/astral-sh/python-build-standalone/issues/504
                "riscv64" => Cow::Borrowed("riscv64gc"),
                value => Cow::Borrowed(value),
            };

            let arch_str = if let Some(variant) = entry.arch.variant {
                Cow::Owned(format!("{arch_str}_{variant}"))
            } else {
                arch_str
            };

            let arch = match Arch::from_str(&arch_str) {
                Ok(arch) => arch,
                Err(e) => {
                    debug!("Skipping entry {key}: Invalid arch '{arch_str}' - {e}");
                    return None;
                }
            };

            let os = match Os::from_str(&entry.os) {
                Ok(os) => os,
                Err(e) => {
                    debug!("Skipping entry {}: Invalid OS '{}' - {}", key, entry.os, e);
                    return None;
                }
            };

            let libc = match Libc::from_str(&entry.libc) {
                Ok(libc) => libc,
                Err(e) => {
                    debug!(
                        "Skipping entry {}: Invalid libc '{}' - {}",
                        key, entry.libc, e
                    );
                    return None;
                }
            };

            let variant = match entry
                .variant
                .as_deref()
                .map(PythonVariant::from_str)
                .transpose()
            {
                Ok(Some(variant)) => variant,
                Ok(None) => PythonVariant::default(),
                Err(()) => {
                    debug!(
                        "Skipping entry {key}: Unknown python variant - {}",
                        entry.variant.unwrap_or_default()
                    );
                    return None;
                }
            };

            let version_str = format!(
                "{}.{}.{}{}",
                entry.major,
                entry.minor,
                entry.patch,
                entry.prerelease.as_deref().unwrap_or_default()
            );

            let version = match PythonVersion::from_str(&version_str) {
                Ok(version) => version,
                Err(e) => {
                    debug!("Skipping entry {key}: Invalid version '{version_str}' - {e}");
                    return None;
                }
            };

            let url = Cow::Owned(entry.url);
            let sha256 = entry.sha256.map(Cow::Owned);
            let build = entry
                .build
                .map(|s| Box::leak(s.into_boxed_str()) as &'static str);

            Some(ManagedPythonDownload {
                key: PythonInstallationKey::new_from_version(
                    implementation,
                    &version,
                    Platform::new(os, arch, libc),
                    variant,
                ),
                url,
                sha256,
                build,
            })
        })
        .sorted_by(|a, b| Ord::cmp(&b.key, &a.key))
        .collect()
}

impl Error {
    fn from_reqwest(
        url: DisplaySafeUrl,
        err: reqwest::Error,
        retries: Option<u32>,
        start: Instant,
    ) -> Self {
        let err = Self::NetworkError(url, WrappedReqwestError::from(err));
        if let Some(retries) = retries {
            Self::NetworkErrorWithRetries {
                err: Box::new(err),
                retries,
                duration: start.elapsed(),
            }
        } else {
            err
        }
    }

    fn from_reqwest_middleware(url: DisplaySafeUrl, err: reqwest_middleware::Error) -> Self {
        match err {
            reqwest_middleware::Error::Middleware(error) => {
                Self::NetworkMiddlewareError(url, error)
            }
            reqwest_middleware::Error::Reqwest(error) => {
                Self::NetworkError(url, WrappedReqwestError::from(error))
            }
        }
    }
}

impl Display for ManagedPythonDownload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Download,
    Extract,
}

impl Direction {
    fn as_str(&self) -> &str {
        match self {
            Self::Download => "download",
            Self::Extract => "extract",
        }
    }
}

impl Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub trait Reporter: Send + Sync {
    fn on_request_start(
        &self,
        direction: Direction,
        name: &PythonInstallationKey,
        size: Option<u64>,
    ) -> usize;
    fn on_request_progress(&self, id: usize, inc: u64);
    fn on_request_complete(&self, direction: Direction, id: usize);
}

/// An asynchronous reader that reports progress as bytes are read.
struct ProgressReader<'a, R> {
    reader: R,
    index: usize,
    reporter: &'a dyn Reporter,
}

impl<'a, R> ProgressReader<'a, R> {
    /// Create a new [`ProgressReader`] that wraps another reader.
    fn new(reader: R, index: usize, reporter: &'a dyn Reporter) -> Self {
        Self {
            reader,
            index,
            reporter,
        }
    }
}

impl<R> AsyncRead for ProgressReader<'_, R>
where
    R: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.as_mut().reader)
            .poll_read(cx, buf)
            .map_ok(|()| {
                self.reporter
                    .on_request_progress(self.index, buf.filled().len() as u64);
            })
    }
}

/// Convert a [`Url`] into an [`AsyncRead`] stream.
async fn read_url(
    url: &DisplaySafeUrl,
    client: &BaseClient,
) -> Result<(impl AsyncRead + Unpin, Option<u64>), Error> {
    if url.scheme() == "file" {
        // Loads downloaded distribution from the given `file://` URL.
        let path = url
            .to_file_path()
            .map_err(|()| Error::InvalidFileUrl(url.to_string()))?;

        let size = fs_err::tokio::metadata(&path).await?.len();
        let reader = fs_err::tokio::File::open(&path).await?;

        Ok((Either::Left(reader), Some(size)))
    } else {
        let start = Instant::now();
        let response = client
            .for_host(url)
            .get(Url::from(url.clone()))
            .send()
            .await
            .map_err(|err| Error::from_reqwest_middleware(url.clone(), err))?;

        let retry_count = response
            .extensions()
            .get::<reqwest_retry::RetryCount>()
            .map(|retries| retries.value());

        // Check the status code.
        let response = response
            .error_for_status()
            .map_err(|err| Error::from_reqwest(url.clone(), err, retry_count, start))?;

        let size = response.content_length();
        let stream = response
            .bytes_stream()
            .map_err(io::Error::other)
            .into_async_read();

        Ok((Either::Right(stream.compat()), size))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use indoc::indoc;

    use crate::PythonVariant;
    use crate::implementation::LenientImplementationName;
    use crate::installation::PythonInstallationKey;
    use uv_platform::{Arch, Libc, Os, Platform};

    use super::*;

    /// Parse a request with all of its fields.
    #[test]
    fn test_python_download_request_from_str_complete() {
        let request = PythonDownloadRequest::from_str("cpython-3.12.0-linux-x86_64-gnu")
            .expect("Test request should be parsed");

        assert_eq!(request.implementation, Some(ImplementationName::CPython));
        assert_eq!(
            request.version,
            Some(VersionRequest::from_str("3.12.0").unwrap())
        );
        assert_eq!(
            request.os,
            Some(Os::new(target_lexicon::OperatingSystem::Linux))
        );
        assert_eq!(
            request.arch,
            Some(ArchRequest::Explicit(Arch::new(
                target_lexicon::Architecture::X86_64,
                None
            )))
        );
        assert_eq!(
            request.libc,
            Some(Libc::Some(target_lexicon::Environment::Gnu))
        );
    }

    /// Parse a request with `any` in various positions.
    #[test]
    fn test_python_download_request_from_str_with_any() {
        let request = PythonDownloadRequest::from_str("any-3.11-any-x86_64-any")
            .expect("Test request should be parsed");

        assert_eq!(request.implementation, None);
        assert_eq!(
            request.version,
            Some(VersionRequest::from_str("3.11").unwrap())
        );
        assert_eq!(request.os, None);
        assert_eq!(
            request.arch,
            Some(ArchRequest::Explicit(Arch::new(
                target_lexicon::Architecture::X86_64,
                None
            )))
        );
        assert_eq!(request.libc, None);
    }

    /// Parse a request with `any` implied by the omission of segments.
    #[test]
    fn test_python_download_request_from_str_missing_segment() {
        let request =
            PythonDownloadRequest::from_str("pypy-linux").expect("Test request should be parsed");

        assert_eq!(request.implementation, Some(ImplementationName::PyPy));
        assert_eq!(request.version, None);
        assert_eq!(
            request.os,
            Some(Os::new(target_lexicon::OperatingSystem::Linux))
        );
        assert_eq!(request.arch, None);
        assert_eq!(request.libc, None);
    }

    #[test]
    fn test_python_download_request_from_str_version_only() {
        let request =
            PythonDownloadRequest::from_str("3.10.5").expect("Test request should be parsed");

        assert_eq!(request.implementation, None);
        assert_eq!(
            request.version,
            Some(VersionRequest::from_str("3.10.5").unwrap())
        );
        assert_eq!(request.os, None);
        assert_eq!(request.arch, None);
        assert_eq!(request.libc, None);
    }

    #[test]
    fn test_python_download_request_from_str_implementation_only() {
        let request =
            PythonDownloadRequest::from_str("cpython").expect("Test request should be parsed");

        assert_eq!(request.implementation, Some(ImplementationName::CPython));
        assert_eq!(request.version, None);
        assert_eq!(request.os, None);
        assert_eq!(request.arch, None);
        assert_eq!(request.libc, None);
    }

    /// Parse a request with the OS and architecture specified.
    #[test]
    fn test_python_download_request_from_str_os_arch() {
        let request = PythonDownloadRequest::from_str("windows-x86_64")
            .expect("Test request should be parsed");

        assert_eq!(request.implementation, None);
        assert_eq!(request.version, None);
        assert_eq!(
            request.os,
            Some(Os::new(target_lexicon::OperatingSystem::Windows))
        );
        assert_eq!(
            request.arch,
            Some(ArchRequest::Explicit(Arch::new(
                target_lexicon::Architecture::X86_64,
                None
            )))
        );
        assert_eq!(request.libc, None);
    }

    /// Parse a request with a pre-release version.
    #[test]
    fn test_python_download_request_from_str_prerelease() {
        let request = PythonDownloadRequest::from_str("cpython-3.13.0rc1")
            .expect("Test request should be parsed");

        assert_eq!(request.implementation, Some(ImplementationName::CPython));
        assert_eq!(
            request.version,
            Some(VersionRequest::from_str("3.13.0rc1").unwrap())
        );
        assert_eq!(request.os, None);
        assert_eq!(request.arch, None);
        assert_eq!(request.libc, None);
    }

    /// We fail on extra parts in the request.
    #[test]
    fn test_python_download_request_from_str_too_many_parts() {
        let result = PythonDownloadRequest::from_str("cpython-3.12-linux-x86_64-gnu-extra");

        assert!(matches!(result, Err(Error::TooManyParts(_))));
    }

    /// We don't allow an empty request.
    #[test]
    fn test_python_download_request_from_str_empty() {
        let result = PythonDownloadRequest::from_str("");

        assert!(matches!(result, Err(Error::EmptyRequest)), "{result:?}");
    }

    /// Parse a request with all "any" segments.
    #[test]
    fn test_python_download_request_from_str_all_any() {
        let request = PythonDownloadRequest::from_str("any-any-any-any-any")
            .expect("Test request should be parsed");

        assert_eq!(request.implementation, None);
        assert_eq!(request.version, None);
        assert_eq!(request.os, None);
        assert_eq!(request.arch, None);
        assert_eq!(request.libc, None);
    }

    /// Test that "any" is case-insensitive in various positions.
    #[test]
    fn test_python_download_request_from_str_case_insensitive_any() {
        let request = PythonDownloadRequest::from_str("ANY-3.11-Any-x86_64-aNy")
            .expect("Test request should be parsed");

        assert_eq!(request.implementation, None);
        assert_eq!(
            request.version,
            Some(VersionRequest::from_str("3.11").unwrap())
        );
        assert_eq!(request.os, None);
        assert_eq!(
            request.arch,
            Some(ArchRequest::Explicit(Arch::new(
                target_lexicon::Architecture::X86_64,
                None
            )))
        );
        assert_eq!(request.libc, None);
    }

    /// Parse a request with an invalid leading segment.
    #[test]
    fn test_python_download_request_from_str_invalid_leading_segment() {
        let result = PythonDownloadRequest::from_str("foobar-3.14-windows");

        assert!(
            matches!(result, Err(Error::ImplementationError(_))),
            "{result:?}"
        );
    }

    /// Parse a request with segments in an invalid order.
    #[test]
    fn test_python_download_request_from_str_out_of_order() {
        let result = PythonDownloadRequest::from_str("3.12-cpython");

        assert!(
            matches!(result, Err(Error::InvalidRequestPlatform(_))),
            "{result:?}"
        );
    }

    /// Parse a request with too many "any" segments.
    #[test]
    fn test_python_download_request_from_str_too_many_any() {
        let result = PythonDownloadRequest::from_str("any-any-any-any-any-any");

        assert!(matches!(result, Err(Error::TooManyParts(_))));
    }

    /// Test that build filtering works correctly
    #[tokio::test]
    async fn test_python_download_request_build_filtering() {
        let _preview = uv_preview::test::with_features(&[]);
        let mut request = PythonDownloadRequest::default()
            .with_version(VersionRequest::from_str("3.12").unwrap())
            .with_implementation(ImplementationName::CPython);
        request.build = Some("20240814".to_string());

        let client_builder = uv_client::BaseClientBuilder::default();
        let cache = uv_cache::Cache::temp().expect("failed to create temp cache");
        let download_list = ManagedPythonDownloadList::new(&client_builder, &cache, None)
            .await
            .unwrap();

        let downloads: Vec<_> = download_list
            .iter_all()
            .filter(|d| request.satisfied_by_download(d))
            .collect();

        assert!(
            !downloads.is_empty(),
            "Should find at least one matching download"
        );
        for download in downloads {
            assert_eq!(download.build(), Some("20240814"));
        }
    }

    /// Test that an invalid build results in no matches
    #[tokio::test]
    async fn test_python_download_request_invalid_build() {
        let _preview = uv_preview::test::with_features(&[]);
        // Create a request with a non-existent build
        let mut request = PythonDownloadRequest::default()
            .with_version(VersionRequest::from_str("3.12").unwrap())
            .with_implementation(ImplementationName::CPython);
        request.build = Some("99999999".to_string());

        let client_builder = uv_client::BaseClientBuilder::default();
        let cache = uv_cache::Cache::temp().expect("failed to create temp cache");
        let download_list = ManagedPythonDownloadList::new(&client_builder, &cache, None)
            .await
            .unwrap();

        // Should find no matching downloads
        let downloads: Vec<_> = download_list
            .iter_all()
            .filter(|d| request.satisfied_by_download(d))
            .collect();

        assert_eq!(downloads.len(), 0);
    }

    #[test]
    fn upgrade_request_native_defaults() {
        let request = PythonDownloadRequest::default()
            .with_implementation(ImplementationName::CPython)
            .with_version(VersionRequest::MajorMinorPatch(
                3,
                13,
                1,
                PythonVariant::Default,
            ))
            .with_os(Os::from_str("linux").unwrap())
            .with_arch(Arch::from_str("x86_64").unwrap())
            .with_libc(Libc::from_str("gnu").unwrap())
            .with_prereleases(false);

        let host = Platform::new(
            Os::from_str("linux").unwrap(),
            Arch::from_str("x86_64").unwrap(),
            Libc::from_str("gnu").unwrap(),
        );

        assert_eq!(
            request
                .clone()
                .unset_defaults_for_host(&host)
                .without_patch()
                .simplified_display()
                .as_deref(),
            Some("3.13")
        );
    }

    #[test]
    fn upgrade_request_preserves_variant() {
        let request = PythonDownloadRequest::default()
            .with_implementation(ImplementationName::CPython)
            .with_version(VersionRequest::MajorMinorPatch(
                3,
                13,
                0,
                PythonVariant::Freethreaded,
            ))
            .with_os(Os::from_str("linux").unwrap())
            .with_arch(Arch::from_str("x86_64").unwrap())
            .with_libc(Libc::from_str("gnu").unwrap())
            .with_prereleases(false);

        let host = Platform::new(
            Os::from_str("linux").unwrap(),
            Arch::from_str("x86_64").unwrap(),
            Libc::from_str("gnu").unwrap(),
        );

        assert_eq!(
            request
                .clone()
                .unset_defaults_for_host(&host)
                .without_patch()
                .simplified_display()
                .as_deref(),
            Some("3.13+freethreaded")
        );
    }

    #[test]
    fn upgrade_request_preserves_non_default_platform() {
        let request = PythonDownloadRequest::default()
            .with_implementation(ImplementationName::CPython)
            .with_version(VersionRequest::MajorMinorPatch(
                3,
                12,
                4,
                PythonVariant::Default,
            ))
            .with_os(Os::from_str("linux").unwrap())
            .with_arch(Arch::from_str("aarch64").unwrap())
            .with_libc(Libc::from_str("gnu").unwrap())
            .with_prereleases(false);

        let host = Platform::new(
            Os::from_str("linux").unwrap(),
            Arch::from_str("x86_64").unwrap(),
            Libc::from_str("gnu").unwrap(),
        );

        assert_eq!(
            request
                .clone()
                .unset_defaults_for_host(&host)
                .without_patch()
                .simplified_display()
                .as_deref(),
            Some("3.12-aarch64")
        );
    }

    #[test]
    fn upgrade_request_preserves_custom_implementation() {
        let request = PythonDownloadRequest::default()
            .with_implementation(ImplementationName::PyPy)
            .with_version(VersionRequest::MajorMinorPatch(
                3,
                10,
                5,
                PythonVariant::Default,
            ))
            .with_os(Os::from_str("linux").unwrap())
            .with_arch(Arch::from_str("x86_64").unwrap())
            .with_libc(Libc::from_str("gnu").unwrap())
            .with_prereleases(false);

        let host = Platform::new(
            Os::from_str("linux").unwrap(),
            Arch::from_str("x86_64").unwrap(),
            Libc::from_str("gnu").unwrap(),
        );

        assert_eq!(
            request
                .clone()
                .unset_defaults_for_host(&host)
                .without_patch()
                .simplified_display()
                .as_deref(),
            Some("pypy-3.10")
        );
    }

    #[test]
    fn simplified_display_returns_none_when_empty() {
        let request = PythonDownloadRequest::default()
            .fill_platform()
            .expect("should populate defaults");

        let host = Platform::from_env().expect("host platform");

        assert_eq!(
            request.unset_defaults_for_host(&host).simplified_display(),
            None
        );
    }

    #[test]
    fn simplified_display_omits_environment_arch() {
        let mut request = PythonDownloadRequest::default()
            .with_version(VersionRequest::MajorMinor(3, 12, PythonVariant::Default))
            .with_os(Os::from_str("linux").unwrap())
            .with_libc(Libc::from_str("gnu").unwrap());

        request.arch = Some(ArchRequest::Environment(Arch::from_str("x86_64").unwrap()));

        let host = Platform::new(
            Os::from_str("linux").unwrap(),
            Arch::from_str("aarch64").unwrap(),
            Libc::from_str("gnu").unwrap(),
        );

        assert_eq!(
            request
                .unset_defaults_for_host(&host)
                .simplified_display()
                .as_deref(),
            Some("3.12")
        );
    }

    fn cpython_download_for_url(url: &'static str) -> ManagedPythonDownload {
        let key = PythonInstallationKey::new(
            LenientImplementationName::Known(crate::implementation::ImplementationName::CPython),
            3,
            12,
            4,
            None,
            Platform::new(
                Os::from_str("linux").unwrap(),
                Arch::from_str("x86_64").unwrap(),
                Libc::from_str("gnu").unwrap(),
            ),
            crate::PythonVariant::default(),
        );

        ManagedPythonDownload {
            key,
            url: Cow::Borrowed(url),
            sha256: Some(Cow::Borrowed("abc123")),
            build: Some("20240713"),
        }
    }

    #[test]
    fn test_cpython_download_urls_custom_astral_mirror() {
        let download = cpython_download_for_url(
            "https://github.com/astral-sh/python-build-standalone/releases/download/20240713/cpython-3.12.4%2B20240713-x86_64-unknown-linux-gnu-install_only.tar.gz",
        );

        let urls = download
            .download_urls_with_astral_mirror(
                None,
                None,
                Some("https://nexus.example.com/repository/releases.astral.sh/"),
            )
            .expect("download URLs should be valid");
        let urls = urls
            .into_iter()
            .map(|url| url.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec![
                "https://nexus.example.com/repository/releases.astral.sh/github/python-build-standalone/releases/download/20240713/cpython-3.12.4%2B20240713-x86_64-unknown-linux-gnu-install_only.tar.gz"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn test_cpython_specific_mirror_takes_precedence_over_astral_mirror() {
        let download = cpython_download_for_url(
            "https://github.com/astral-sh/python-build-standalone/releases/download/20240713/cpython-3.12.4%2B20240713-x86_64-unknown-linux-gnu-install_only.tar.gz",
        );

        let urls = download
            .download_urls_with_astral_mirror(
                Some("https://python-mirror.example.com/releases/"),
                None,
                Some("https://nexus.example.com/repository/releases.astral.sh/"),
            )
            .expect("download URLs should be valid");
        let urls = urls
            .into_iter()
            .map(|url| url.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec![
                "https://python-mirror.example.com/releases/20240713/cpython-3.12.4%2B20240713-x86_64-unknown-linux-gnu-install_only.tar.gz"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn test_cpython_download_urls_empty_astral_mirror_uses_default() {
        let download = cpython_download_for_url(
            "https://github.com/astral-sh/python-build-standalone/releases/download/20240713/cpython-3.12.4%2B20240713-x86_64-unknown-linux-gnu-install_only.tar.gz",
        );

        let default_urls = download
            .download_urls_with_astral_mirror(None, None, None)
            .expect("download URLs should be valid");
        let empty_urls = download
            .download_urls_with_astral_mirror(None, None, Some(""))
            .expect("download URLs should be valid");

        assert_eq!(default_urls, empty_urls);
    }

    /// A hash mismatch is a post-download integrity failure — retrying a different URL cannot fix
    /// it, so it should not trigger a fallback.
    #[test]
    fn test_should_try_next_url_hash_mismatch() {
        let err = Error::HashMismatch {
            installation: "cpython-3.12.0".to_string(),
            expected: "abc".to_string(),
            actual: "def".to_string(),
        };
        assert!(!err.should_try_next_url());
    }

    /// A local filesystem error during extraction (e.g. permission denied writing to disk) is not
    /// a network failure — a different URL would produce the same outcome.
    #[test]
    fn test_should_try_next_url_extract_error_filesystem() {
        let err = Error::ExtractError(
            "archive.tar.gz".to_string(),
            uv_extract::Error::Io(io::Error::new(io::ErrorKind::PermissionDenied, "")),
        );
        assert!(!err.should_try_next_url());
    }

    /// A generic IO error from a local filesystem operation (e.g. permission denied on cache
    /// directory) should not trigger a fallback to a different URL.
    #[test]
    fn test_should_try_next_url_io_error_filesystem() {
        let err = Error::Io(io::Error::new(io::ErrorKind::PermissionDenied, ""));
        assert!(!err.should_try_next_url());
    }

    /// A network IO error (e.g. connection reset mid-download) surfaces as `Error::Io` from
    /// `download_archive`. It should trigger a fallback because a different mirror may succeed.
    #[test]
    fn test_should_try_next_url_io_error_network() {
        let err = Error::Io(io::Error::new(io::ErrorKind::ConnectionReset, ""));
        assert!(err.should_try_next_url());
    }

    /// A 404 HTTP response from the mirror becomes `Error::NetworkError` — it should trigger a
    /// URL fallback, because a 404 on the mirror does not mean the file is absent from GitHub.
    #[test]
    fn test_should_try_next_url_network_error_404() {
        let url =
            DisplaySafeUrl::from_str("https://releases.astral.sh/python/cpython-3.12.0.tar.gz")
                .unwrap();
        // `NetworkError` wraps a `WrappedReqwestError`; we use a middleware error as a
        // stand-in because `should_try_next_url` only inspects the variant, not the contents.
        let wrapped = WrappedReqwestError::with_problem_details(
            reqwest_middleware::Error::Middleware(anyhow::anyhow!("404 Not Found")),
            None,
        );
        let err = Error::NetworkError(url, wrapped);
        assert!(err.should_try_next_url());
    }

    /// Every [`PythonVersion`] in the embedded download metadata must be convertible
    /// to a [`VersionRequest`] to avoid runtime panics.
    #[test]
    fn embedded_download_versions_convert_to_version_requests() {
        let downloads = ManagedPythonDownloadList::new_only_embedded()
            .expect("embedded download metadata should load");

        let unique_versions: HashSet<PythonVersion> = downloads
            .iter_all()
            .map(ManagedPythonDownload::python_version)
            .collect();

        for version in &unique_versions {
            let _ = VersionRequest::from(version);
        }
    }

    #[test]
    fn test_parse_version_with_build() -> Result<(), Error> {
        let (version, build) = parse_version_with_build("3.15.0a5+20260114")?;
        assert_eq!(version.to_string(), "3.15.0a5");
        assert_eq!(build, Some("20260114"));

        let (version, build) = parse_version_with_build("3.12.1")?;
        assert_eq!(version.to_string(), "3.12.1");
        assert_eq!(build, None);

        let (version, build) = parse_version_with_build("3.13.0rc1")?;
        assert_eq!(version.to_string(), "3.13.0rc1");
        assert_eq!(build, None);

        let (version, build) = parse_version_with_build("3.14.0b2+20251201")?;
        assert_eq!(version.to_string(), "3.14.0b2");
        assert_eq!(build, Some("20251201"));

        assert!(parse_version_with_build("invalid").is_err());

        Ok(())
    }

    #[test]
    fn test_detect_download_list_format() {
        // Test JSON format detection
        assert_eq!(
            detect_download_list_format("download-metadata.json"),
            DownloadListFormat::Json
        );
        assert_eq!(
            detect_download_list_format("https://example.com/downloads.json"),
            DownloadListFormat::Json
        );

        // Test NDJSON format detection
        assert_eq!(
            detect_download_list_format("versions.ndjson"),
            DownloadListFormat::Ndjson
        );
        assert_eq!(
            detect_download_list_format("https://example.com/python.ndjson"),
            DownloadListFormat::Ndjson
        );
        assert_eq!(
            detect_download_list_format("https://example.com/python.ndjson?token=value#metadata"),
            DownloadListFormat::Ndjson
        );
        assert_eq!(
            detect_download_list_format("https://example.com/python.json?name=python.ndjson"),
            DownloadListFormat::Json
        );

        // Test default (JSON) for unknown extensions
        assert_eq!(
            detect_download_list_format("downloads.txt"),
            DownloadListFormat::Json
        );
    }

    #[test]
    fn test_parse_ndjson_variant() {
        // install_only variants should be accepted
        assert_eq!(
            parse_ndjson_variant("install_only"),
            Some(PythonVariant::Default)
        );
        assert_eq!(
            parse_ndjson_variant("install_only_stripped"),
            Some(PythonVariant::Default)
        );

        // freethreaded variants should return Freethreaded
        assert_eq!(
            parse_ndjson_variant("freethreaded+pgo+lto+full"),
            Some(PythonVariant::Freethreaded)
        );
        assert_eq!(
            parse_ndjson_variant("freethreaded+install_only"),
            Some(PythonVariant::Freethreaded)
        );

        assert_eq!(
            parse_ndjson_variant("debug+full"),
            Some(PythonVariant::Debug)
        );
        assert_eq!(
            parse_ndjson_variant("freethreaded+debug+full"),
            Some(PythonVariant::FreethreadedDebug)
        );

        assert_eq!(parse_ndjson_variant("pgo+lto+full"), None);
        assert_eq!(parse_ndjson_variant("debug+static+full"), None);
        assert_eq!(parse_ndjson_variant("freethreaded+static+full"), None);
        assert_eq!(parse_ndjson_variant("install_only_unsupported"), None);
    }

    #[test]
    fn test_parse_ndjson_bytes() -> Result<(), Error> {
        let ndjson = indoc! {r#"
            {"version":"3.12.0+20240101","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/python.tar.gz","sha256":"abc123"}]}
            {"version":"3.11.5+20240101","artifacts":[{"platform":"aarch64-apple-darwin","variant":"install_only","url":"https://example.com/python2.tar.gz","sha256":"def456"}]}
        "#};

        let downloads = parse_ndjson_bytes("test", ndjson.as_bytes())?;

        assert_eq!(downloads.len(), 2);

        let download = &downloads[0];
        assert_eq!(download.key().version().to_string(), "3.12.0");
        assert_eq!(download.build(), Some("20240101"));

        let download = &downloads[1];
        assert_eq!(download.key().version().to_string(), "3.11.5");
        assert_eq!(download.build(), Some("20240101"));

        Ok(())
    }

    #[test]
    fn test_parse_ndjson_bytes_filtered_stops_at_limit() -> Result<(), Error> {
        let metadata = indoc! {r#"
            {"version":"3.12.0+20240101","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/python.tar.gz","sha256":null}]}
            not valid metadata
        "#};

        let downloads =
            parse_ndjson_bytes_filtered("test", metadata.as_bytes(), |_| true, Some(1))?;

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].key().version().to_string(), "3.12.0");

        assert!(
            parse_ndjson_bytes_filtered("test", metadata.as_bytes(), |_| true, Some(0))?.is_empty()
        );

        Ok(())
    }

    #[test]
    fn test_parse_ndjson_bytes_with_stops_at_match() -> Result<(), Error> {
        let metadata = indoc! {r#"
            {"version":"3.12.0+20240101","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/python.tar.gz","sha256":null}]}
            not valid metadata
        "#};

        let download = parse_ndjson_bytes_with("test", metadata.as_bytes(), ControlFlow::Break)?;

        assert_eq!(
            download
                .map(|download| download.key().version().to_string())
                .as_deref(),
            Some("3.12.0")
        );

        Ok(())
    }

    #[test]
    fn test_select_download_prefers_stable_over_prerelease() -> Result<(), Error> {
        let metadata = indoc! {r#"
            {"version":"3.13.0rc1+20240101","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/prerelease.tar.gz","sha256":null}]}
            {"version":"3.13.0+20240101","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/stable.tar.gz","sha256":null}]}
        "#};
        let request = PythonDownloadRequest::from_str("cpython-3.13-linux-x86_64-gnu")?;
        let prerelease_request = request.clone().with_prereleases(true);
        let mut prerelease = None;

        let found = parse_ndjson_bytes_with("test", metadata.as_bytes(), |download| {
            select_download(
                &request,
                Some(&prerelease_request),
                &mut prerelease,
                download,
            )
        })?;

        assert_eq!(
            found
                .map(|download| download.key().version().to_string())
                .as_deref(),
            Some("3.13.0")
        );
        assert_eq!(
            prerelease
                .map(|download| download.key().version().to_string())
                .as_deref(),
            Some("3.13.0rc1")
        );

        Ok(())
    }

    #[test]
    fn test_select_download_falls_back_to_prerelease() -> Result<(), Error> {
        let metadata = indoc! {r#"
            {"version":"3.13.0rc1+20240101","artifacts":[{"platform":"x86_64-unknown-linux-gnu","variant":"install_only","url":"https://example.com/prerelease.tar.gz","sha256":null}]}
        "#};
        let request = PythonDownloadRequest::from_str("cpython-3.13-linux-x86_64-gnu")?;
        let prerelease_request = request.clone().with_prereleases(true);
        let mut prerelease = None;

        let found = parse_ndjson_bytes_with("test", metadata.as_bytes(), |download| {
            select_download(
                &request,
                Some(&prerelease_request),
                &mut prerelease,
                download,
            )
        })?;

        assert!(found.is_none());
        assert_eq!(
            prerelease
                .map(|download| download.key().version().to_string())
                .as_deref(),
            Some("3.13.0rc1")
        );

        Ok(())
    }

    #[test]
    fn test_versions_cache_entries_are_scoped_to_url() {
        let cache = Cache::temp().expect("a temporary cache should be created");
        let first_url = DisplaySafeUrl::parse("https://one.example.com/python.ndjson")
            .expect("the first URL should be valid");
        let second_url = DisplaySafeUrl::parse("https://two.example.com/python.ndjson")
            .expect("the second URL should be valid");

        let (first_content, first_metadata) = versions_cache_entries(&cache, &first_url);
        let (second_content, second_metadata) = versions_cache_entries(&cache, &second_url);

        assert_ne!(first_content.path(), second_content.path());
        assert_ne!(first_metadata.path(), second_metadata.path());
    }

    #[test]
    fn test_parse_ndjson_version_info() {
        let version_info = NdjsonPythonVersionInfo {
            version: "3.12.1+20240815".to_string(),
            artifacts: vec![
                NdjsonPythonArtifact {
                    platform: "x86_64-unknown-linux-gnu".to_string(),
                    variant: "install_only".to_string(),
                    url: "https://example.com/python-linux.tar.gz".to_string(),
                    sha256: Some("abc123".to_string()),
                },
                NdjsonPythonArtifact {
                    platform: "aarch64-apple-darwin".to_string(),
                    variant: "install_only".to_string(),
                    url: "https://example.com/python-macos.tar.gz".to_string(),
                    sha256: Some("def456".to_string()),
                },
                NdjsonPythonArtifact {
                    platform: "x86_64-unknown-linux-gnu".to_string(),
                    variant: "debug+full".to_string(),
                    url: "https://example.com/python-debug.tar.gz".to_string(),
                    sha256: Some("ghi789".to_string()),
                },
            ],
        };

        let downloads = parse_ndjson_version_info(version_info);

        assert_eq!(downloads.len(), 3);
        assert!(
            downloads
                .iter()
                .any(|download| download.key().variant() == &PythonVariant::Debug)
        );

        // All downloads should have the same version and build
        for download in &downloads {
            assert_eq!(download.key().version().to_string(), "3.12.1");
            assert_eq!(download.build(), Some("20240815"));
        }
    }

    #[test]
    fn test_parse_ndjson_version_info_prefers_stripped_artifacts() {
        let version_info = NdjsonPythonVersionInfo {
            version: "3.12.1+20240815".to_string(),
            artifacts: vec![
                NdjsonPythonArtifact {
                    platform: "x86_64-unknown-linux-gnu".to_string(),
                    variant: "install_only".to_string(),
                    url: "https://example.com/install-only.tar.gz".to_string(),
                    sha256: None,
                },
                NdjsonPythonArtifact {
                    platform: "x86_64-unknown-linux-gnu".to_string(),
                    variant: "install_only_stripped".to_string(),
                    url: "https://example.com/install-only-stripped.tar.gz".to_string(),
                    sha256: None,
                },
                NdjsonPythonArtifact {
                    platform: "x86_64-unknown-linux-gnu".to_string(),
                    variant: "debug+static+full".to_string(),
                    url: "https://example.com/static-debug.tar.gz".to_string(),
                    sha256: None,
                },
            ],
        };

        let downloads = parse_ndjson_version_info(version_info);

        assert_eq!(downloads.len(), 1);
        assert_eq!(
            downloads[0].url().as_ref(),
            "https://example.com/install-only-stripped.tar.gz"
        );
    }

    #[test]
    fn test_parse_ndjson_artifact_freethreaded() {
        let version = PythonVersion::from_str("3.13.0").expect("version should be valid");
        let build = Some("20240901");

        let artifact = NdjsonPythonArtifact {
            platform: "x86_64-unknown-linux-gnu".to_string(),
            variant: "freethreaded+pgo+lto+full".to_string(),
            url: "https://example.com/python-ft.tar.gz".to_string(),
            sha256: Some("xyz789".to_string()),
        };

        let download = parse_ndjson_artifact(&version, build, artifact)
            .expect("freethreaded artifact should be retained");
        assert_eq!(download.key().variant(), &PythonVariant::Freethreaded);
    }

    #[test]
    fn test_parse_ndjson_artifact_platform_debug_suffix() {
        let version = PythonVersion::from_str("3.13.0").expect("version should be valid");
        let artifact = NdjsonPythonArtifact {
            platform: "aarch64-apple-darwin-debug".to_string(),
            variant: "install_only".to_string(),
            url: "https://example.com/python-debug.tar.gz".to_string(),
            sha256: None,
        };

        let download = parse_ndjson_artifact(&version, None, artifact)
            .expect("platform-specific debug artifact should be retained");

        assert_eq!(download.key().variant(), &PythonVariant::Debug);
    }
}
