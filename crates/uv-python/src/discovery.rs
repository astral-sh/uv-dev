use itertools::{Either, Itertools};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use regex::Regex;
use rustc_hash::{FxBuildHasher, FxHashSet};
use same_file::is_same_file;
use std::borrow::Cow;
use std::cmp::Reverse;
use std::env::consts::EXE_SUFFIX;
use std::fmt::{self, Debug, Formatter};
use std::{env, io, iter};
use std::{path::Path, path::PathBuf, str::FromStr};
use thiserror::Error;
use tracing::{debug, instrument, trace};
use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_distribution_types::RequiresPython;
use uv_errors::Hints;
use uv_fs::Simplified;
use uv_fs::which::is_executable;
use uv_pep440::{
    LowerBound, Prerelease, UpperBound, Version, VersionSpecifier, VersionSpecifiers,
    release_specifiers_to_ranges,
};
use uv_static::EnvVars;
use uv_warnings::{warn_user_once, write_warning_chain};
use which::{which, which_all};

use crate::downloads::{ManagedPythonDownloadList, PlatformRequest, PythonDownloadRequest};
use crate::implementation::ImplementationName;
use crate::installation::{PythonInstallation, PythonInstallationKey};
use crate::interpreter::Error as InterpreterError;
use crate::interpreter::{StatusCodeError, UnexpectedResponseError};
use crate::managed::{ManagedPythonInstallations, PythonMinorVersionLink};
#[cfg(windows)]
use crate::microsoft_store::find_microsoft_store_pythons;
use crate::python_version::python_build_versions_from_env;
use crate::virtualenv::Error as VirtualEnvError;
use crate::virtualenv::{
    CondaEnvironmentKind, conda_environment_from_env, virtualenv_from_env,
    virtualenv_from_working_dir, virtualenv_python_executable,
};
#[cfg(windows)]
use crate::windows_registry::{WindowsPython, registry_pythons};
use crate::{BrokenLink, Interpreter, PythonVersion};

/// A request to find a Python installation.
///
/// See [`PythonRequest::from_str`].
#[derive(Debug, Clone, Eq, Default)]
pub enum PythonRequest {
    /// A suitable default Python installation.
    ///
    /// This can exclude pre-release versions and alternative implementations.
    #[default]
    Default,
    /// Any Python installation.
    Any,
    /// A Python version without an implementation name, such as `3.10` or `>=3.12,<3.13`.
    Version(VersionRequest),
    /// A directory that contains a Python installation, such as `.venv`.
    Directory(PathBuf),
    /// A Python executable path, such as `~/bin/python`.
    File(PathBuf),
    /// A Python executable name to find in `PATH`, such as `foopython3`.
    ExecutableName(String),
    /// A Python implementation without a version, such as `pypy` or `pp`.
    Implementation(ImplementationName),
    /// A Python implementation and version, such as `pypy3.8`, `pypy@3.8`, or `pp38`.
    ImplementationVersion(ImplementationName, VersionRequest),
    /// A Python installation key, such as `cpython-3.12-x86_64-linux-gnu`.
    ///
    /// These keys usually identify managed Python downloads.
    Key(PythonDownloadRequest),
}

impl PartialEq for PythonRequest {
    fn eq(&self, other: &Self) -> bool {
        self.to_canonical_string() == other.to_canonical_string()
    }
}

impl std::hash::Hash for PythonRequest {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.to_canonical_string().hash(state);
    }
}

impl<'a> serde::Deserialize<'a> for PythonRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let s = <Cow<'_, str>>::deserialize(deserializer)?;
        Ok(Self::parse(&s))
    }
}

impl serde::Serialize for PythonRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = self.to_canonical_string();
        serializer.serialize_str(&s)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum PythonPreference {
    /// Use only managed Python installations. Do not use system Python installations.
    OnlyManaged,
    #[default]
    /// Prefer managed Python installations over system Python installations.
    ///
    /// Use an existing system Python installation before downloading a managed version.
    /// Use `only-managed` to always download a managed Python version.
    Managed,
    /// Prefer system Python installations over managed Python installations.
    ///
    /// Use a managed installation if no system installation is available.
    System,
    /// Use only system Python installations. Do not use managed Python installations.
    OnlySystem,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum PythonDownloads {
    /// Download managed Python installations automatically when needed.
    #[default]
    #[serde(alias = "auto")]
    Automatic,
    /// Require explicit installation. Do not download managed Python installations automatically.
    Manual,
    /// Do not allow Python downloads.
    Never,
}

impl FromStr for PythonDownloads {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" | "automatic" | "true" | "1" => Ok(Self::Automatic),
            "manual" => Ok(Self::Manual),
            "never" | "false" | "0" => Ok(Self::Never),
            _ => Err(format!("Invalid value for `python-download`: '{s}'")),
        }
    }
}

impl From<bool> for PythonDownloads {
    fn from(value: bool) -> Self {
        if value { Self::Automatic } else { Self::Never }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvironmentPreference {
    /// Use only virtual environments. Do not allow a system environment.
    #[default]
    OnlyVirtual,
    /// Prefer virtual environments. Allow a system environment only when explicitly requested.
    ExplicitSystem,
    /// Use only a system environment. Ignore virtual environments.
    OnlySystem,
    /// Allow any environment.
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DiscoveryPreferences {
    python_preference: PythonPreference,
    environment_preference: EnvironmentPreference,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PythonVariant {
    #[default]
    Default,
    Debug,
    Freethreaded,
    FreethreadedDebug,
    Gil,
    GilDebug,
}

/// A Python discovery version request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum VersionRequest {
    /// Allow a suitable default Python version.
    #[default]
    Default,
    /// Allow any Python version.
    Any,
    Major(u8, PythonVariant),
    MajorMinor(u8, u8, PythonVariant),
    MajorMinorPatch(u8, u8, u8, PythonVariant),
    MajorMinorPrerelease(u8, u8, Prerelease, PythonVariant),
    MajorMinorPatchPrerelease(u8, u8, u8, Prerelease, PythonVariant),
    Range(VersionSpecifiers, PythonVariant),
}

/// The result of a Python installation search.
///
/// Returned by [`find_python_installation`].
type FindPythonResult = Result<PythonInstallation, PythonNotFound>;

/// The result of a failed Python installation search.
///
/// See [`FindPythonResult`].
#[derive(Clone, Debug, Error)]
pub struct PythonNotFound {
    pub(super) request: PythonRequest,
    pub(super) python_preference: PythonPreference,
    pub(super) environment_preference: EnvironmentPreference,
}

/// A location where uv can find a Python installation or interpreter.
#[derive(Debug, Clone, PartialEq, Eq, Copy, Hash, PartialOrd, Ord)]
pub enum PythonSource {
    /// A path provided directly.
    ProvidedPath,
    /// An active virtual environment, such as one set by `VIRTUAL_ENV`.
    ActiveEnvironment,
    /// An active conda environment, such as one set by `CONDA_PREFIX`.
    CondaPrefix,
    /// An active base conda environment, such as one set by `CONDA_PREFIX`.
    BaseCondaPrefix,
    /// A discovered virtual environment, such as `.venv`.
    DiscoveredEnvironment,
    /// An executable found in `PATH`.
    SearchPath,
    /// The first executable found in `PATH`.
    SearchPathFirst,
    /// An executable found in the Windows registry with PEP 514.
    Registry,
    /// An executable found in a known Microsoft Store location.
    MicrosoftStore,
    /// A Python installation found in the uv-managed Python directory.
    Managed,
    /// The Python interpreter that ran uv, such as with `python -m uv ...`.
    ParentInterpreter,
}

/// A non-empty group of equally preferred Python executables.
///
/// Minor-version fallback candidates from one `PATH` directory share a group. Preferred executable
/// names and interpreters from other sources form singleton groups.
struct PythonExecutableGroup(Vec<(PythonSource, PathBuf)>);

impl PythonExecutableGroup {
    fn new(executables: Vec<(PythonSource, PathBuf)>) -> Option<Self> {
        (!executables.is_empty()).then_some(Self(executables))
    }

    fn filter(mut self, mut predicate: impl FnMut(PythonSource, &Path) -> bool) -> Option<Self> {
        self.0.retain(|(source, path)| predicate(*source, path));
        (!self.0.is_empty()).then_some(self)
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),

    /// Could not read interpreter information.
    #[error("Failed to inspect Python interpreter from {} at `{}` ", _2, _1.user_display())]
    Query(
        #[source] Box<crate::interpreter::Error>,
        PathBuf,
        PythonSource,
    ),

    /// Could not find a managed Python installation for the current platform.
    #[error("Failed to discover managed Python installations")]
    ManagedPython(#[from] crate::managed::Error),

    /// Could not inspect a virtual environment.
    #[error(transparent)]
    VirtualEnv(#[from] crate::virtualenv::Error),

    #[cfg(windows)]
    #[error("Failed to query installed Python versions from the Windows registry")]
    RegistryError(#[from] windows::core::Error),

    #[error(transparent)]
    InvalidEnvironmentVariable(#[from] uv_static::InvalidEnvironmentVariable),

    /// The version request is invalid.
    #[error("Invalid version request: {0}")]
    InvalidVersionRequest(String),

    /// The version request uses `@latest`.
    #[error("Requesting the 'latest' Python version is not yet supported")]
    LatestVersionRequest,

    // TODO(zanieb): Is this error case necessary still? We should probably drop it.
    #[error("Interpreter discovery for `{0}` requires `{1}` but only `{2}` is allowed")]
    SourceNotAllowed(PythonRequest, PythonSource, PythonPreference),

    #[error(transparent)]
    BuildVersion(#[from] crate::python_version::BuildVersionError),
}

impl uv_errors::Hint for Error {
    fn hints(&self) -> uv_errors::Hints<'_> {
        match self {
            Self::Query(err, _, _) => err.hints(),
            _ => uv_errors::Hints::none(),
        }
    }
}

/// Iterate lazily over Python executables in writable virtual environments.
///
/// Supported sources include:
///
/// - An active virtual environment set by `VIRTUAL_ENV`.
/// - A discovered virtual environment, such as `.venv` in a parent directory.
///
/// System environments are excluded. See [`python_executables_from_installed`].
fn python_executables_from_virtual_environments<'a>()
-> impl Iterator<Item = Result<(PythonSource, PathBuf), Error>> + 'a {
    let from_active_environment = iter::once_with(|| {
        virtualenv_from_env()
            .into_iter()
            .map(virtualenv_python_executable)
            .map(|path| Ok((PythonSource::ActiveEnvironment, path)))
    })
    .flatten();

    // Prefer the conda environment over discovered virtual environments.
    let from_conda_environment = iter::once_with(move || {
        conda_environment_from_env(CondaEnvironmentKind::Child)
            .into_iter()
            .map(virtualenv_python_executable)
            .map(|path| Ok((PythonSource::CondaPrefix, path)))
    })
    .flatten();

    let from_discovered_environment = iter::once_with(|| {
        virtualenv_from_working_dir()
            .map(|path| {
                path.map(virtualenv_python_executable)
                    .map(|path| (PythonSource::DiscoveredEnvironment, path))
                    .into_iter()
            })
            .map_err(Error::from)
    })
    .flatten_ok();

    from_active_environment
        .chain(from_conda_environment)
        .chain(from_discovered_environment)
}

/// Iterate lazily over Python executables installed on the system.
///
/// Supported sources include:
///
/// - Managed Python installations, such as those from `uv python install`.
/// - The `PATH` search path.
/// - The Windows registry.
///
/// [`PythonPreference`] controls which sources are used and their order.
///
/// When a [`VersionRequest`] is present, skip executables that cannot satisfy it. The search can
/// include extra version-specific executables. See [`python_executables_from_search_path`].
///
/// The caller MUST query each returned executable to verify its version. This function does not
/// guarantee that an executable provides a specific version. See [`find_python_installation`].
///
/// This function does not guarantee that the executables are valid Python interpreters.
/// See [`python_interpreters_from_executables`].
fn python_executables_from_installed<'a>(
    version: &'a VersionRequest,
    implementation: Option<&'a ImplementationName>,
    platform: PlatformRequest,
    preference: PythonPreference,
) -> Box<dyn Iterator<Item = Result<PythonExecutableGroup, Error>> + 'a> {
    let from_managed_installations = iter::once_with(move || {
        ManagedPythonInstallations::from_settings(None)
            .map_err(Error::from)
            .and_then(|installed_installations| {
                debug!(
                    "Searching for managed installations at `{}`",
                    installed_installations.root().user_display()
                );
                let installations = ManagedPythonInstallations::find_matching_current_platform()?;

                let build_versions = python_build_versions_from_env()?;

                // Check the Python version and platform before querying the interpreter.
                Ok(installations
                    .into_iter()
                    .filter(move |installation| {
                        if !version.matches_version(&installation.version()) {
                            debug!("Skipping managed installation `{installation}`: does not satisfy `{version}`");
                            return false;
                        }
                        if !platform.matches(installation.platform()) {
                            debug!("Skipping managed installation `{installation}`: does not satisfy requested platform `{platform}`");
                            return false;
                        }

                        if let Some(requested_build) = build_versions.get(&installation.implementation()) {
                            let Some(installation_build) = installation.build() else {
                                debug!(
                                    "Skipping managed installation `{installation}`: a build version was requested but is not recorded for this installation"
                                );
                                return false;
                            };
                            if installation_build != requested_build {
                                debug!(
                                    "Skipping managed installation `{installation}`: requested build version `{requested_build}` does not match installation build version `{installation_build}`"
                                );
                                return false;
                            }
                        }

                        true
                    })
                    .inspect(|installation| debug!("Found managed installation `{installation}`"))
                    .map(move |installation| {
                        // Read the stable minor-version link unless the request specifies a patch.
                        let executable = version
                                .patch()
                                .is_none()
                                .then(|| {
                                    PythonMinorVersionLink::from_installation(
                                        &installation,
                                    )
                                    .filter(PythonMinorVersionLink::exists)
                                    .map(
                                        |minor_version_link| {
                                            minor_version_link.symlink_executable.clone()
                                        },
                                    )
                                })
                                .flatten()
                                .unwrap_or_else(|| installation.executable(false));
                        (PythonSource::Managed, executable)
                    })
                )
            })
    })
    .flatten_ok()
    .map_ok(|executable| PythonExecutableGroup(vec![executable]));

    let from_search_path = iter::once_with(move || {
        let mut first = true;
        python_executables_from_search_path(version, implementation).filter_map(move |paths| {
            let executables = paths
                .into_iter()
                .map(|path| {
                    let source = if first {
                        first = false;
                        PythonSource::SearchPathFirst
                    } else {
                        PythonSource::SearchPath
                    };
                    (source, path)
                })
                .collect();
            PythonExecutableGroup::new(executables).map(Ok)
        })
    })
    .flatten();

    #[cfg(windows)]
    let from_windows_registry: Box<
        dyn Iterator<Item = Result<PythonExecutableGroup, Error>> + 'a,
    > = match uv_static::parse_boolish_environment_variable(EnvVars::UV_PYTHON_NO_REGISTRY) {
        Ok(Some(true)) => Box::new(iter::empty()),
        Ok(Some(false) | None) => Box::new(
            iter::once_with(move || {
                // Skip interpreter queries when the version does not match.
                let version_filter = move |entry: &WindowsPython| {
                    if let Some(found) = &entry.version {
                        // Some distributions emit the patch version (example: `SysVersion: 3.9`)
                        if found.string.chars().filter(|c| *c == '.').count() == 1 {
                            version.matches_major_minor(found.major(), found.minor())
                        } else {
                            version.matches_version(found)
                        }
                    } else {
                        true
                    }
                };

                registry_pythons()
                    .map(|entries| {
                        entries
                            .into_iter()
                            .filter(version_filter)
                            .map(|entry| (PythonSource::Registry, entry.path))
                            .chain(
                                find_microsoft_store_pythons()
                                    .filter(version_filter)
                                    .map(|entry| (PythonSource::MicrosoftStore, entry.path)),
                            )
                    })
                    .map_err(Error::from)
            })
            .flatten_ok()
            .map_ok(|executable| PythonExecutableGroup(vec![executable])),
        ),
        Err(err) => Box::new(iter::once(Err(Error::from(err)))),
    };

    #[cfg(not(windows))]
    let from_windows_registry: Box<
        dyn Iterator<Item = Result<PythonExecutableGroup, Error>> + 'a,
    > = Box::new(iter::empty());

    match preference {
        PythonPreference::OnlyManaged => {
            // TODO(zanieb): Ideally, we'd create "fake" managed installation directories for tests,
            // but for now... we'll just include the test interpreters which are always on the
            // search path.
            if std::env::var(uv_static::EnvVars::UV_INTERNAL__TEST_PYTHON_MANAGED).is_ok() {
                Box::new(from_managed_installations.chain(from_search_path))
            } else {
                Box::new(from_managed_installations)
            }
        }
        PythonPreference::Managed => Box::new(
            from_managed_installations
                .chain(from_search_path)
                .chain(from_windows_registry),
        ),
        PythonPreference::System => Box::new(
            from_search_path
                .chain(from_windows_registry)
                .chain(from_managed_installations),
        ),
        PythonPreference::OnlySystem => Box::new(from_search_path.chain(from_windows_registry)),
    }
}

/// Iterate lazily over available Python executables.
///
/// [`EnvironmentPreference`], [`PythonPreference`], and [`PlatformRequest`] can exclude some
/// executables. These filters only improve performance. Query the interpreter to confirm that it
/// satisfies all requests and preferences.
///
/// See [`python_executables_from_installed`] and [`python_executables_from_virtual_environments`]
/// for details about discovery.
fn python_executables<'a>(
    version: &'a VersionRequest,
    implementation: Option<&'a ImplementationName>,
    platform: PlatformRequest,
    environments: EnvironmentPreference,
    preference: PythonPreference,
) -> Box<dyn Iterator<Item = Result<PythonExecutableGroup, Error>> + 'a> {
    // Always read `UV_INTERNAL__PARENT_INTERPRETER`. It can refer to a system interpreter.
    let from_parent_interpreter = iter::once_with(|| {
        env::var_os(EnvVars::UV_INTERNAL__PARENT_INTERPRETER)
            .into_iter()
            .map(|path| {
                Ok(PythonExecutableGroup(vec![(
                    PythonSource::ParentInterpreter,
                    PathBuf::from(path),
                )]))
            })
    })
    .flatten();

    // Check whether the base conda environment is active.
    let from_base_conda_environment = iter::once_with(move || {
        conda_environment_from_env(CondaEnvironmentKind::Base)
            .into_iter()
            .map(virtualenv_python_executable)
            .map(|path| {
                Ok(PythonExecutableGroup(vec![(
                    PythonSource::BaseCondaPrefix,
                    path,
                )]))
            })
    })
    .flatten();

    let from_virtual_environments = python_executables_from_virtual_environments()
        .map_ok(|executable| PythonExecutableGroup(vec![executable]));
    let from_installed =
        python_executables_from_installed(version, implementation, platform, preference);

    // Limit the search to the selected environment preference to avoid extra file system access.
    // The caller must also filter with `source_satisfies_environment_preference` and
    // `EnvironmentPreference::allows_installation`.
    match environments {
        EnvironmentPreference::OnlyVirtual => {
            Box::new(from_parent_interpreter.chain(from_virtual_environments))
        }
        EnvironmentPreference::ExplicitSystem | EnvironmentPreference::Any => Box::new(
            from_parent_interpreter
                .chain(from_virtual_environments)
                .chain(from_base_conda_environment)
                .chain(from_installed),
        ),
        EnvironmentPreference::OnlySystem => Box::new(
            from_parent_interpreter
                .chain(from_base_conda_environment)
                .chain(from_installed),
        ),
    }
}

/// Iterate lazily over Python executables in `PATH`.
///
/// [`VersionRequest`] and [`ImplementationName`] select possible executable names. For example,
/// Python 3.9 adds `python3.9`. `PyPy` adds `pypy`. Both searches include the default names.
///
/// Return executables in search-path order. Within each directory, prefer more specific names.
/// For example, prefer `python3.9` over `python3` and `pypy3.9` over `python3.9`.
///
/// For a `PATH` directory containing `python`, `python3`, `python3.14`, `python3.15`, and
/// `python3.15t`, an exact `3.15` request returns these groups:
///
/// ```text
/// [python3.15], [python3], [python]
/// ```
///
/// A `>=3.14,<3.16` request returns these groups:
///
/// ```text
/// [python3], [python], [python3.14, python3.15, python3.15t]
/// ```
///
/// Group minor-version fallback candidates from the same directory. This grouping lets their
/// queried installation keys determine their relative order. It does not override search-path
/// precedence.
///
/// Without a `version`, search only for default names such as `python3` and `python`. Exclude
/// version-specific names such as `python3.9`.
fn python_executables_from_search_path<'a>(
    version: &'a VersionRequest,
    implementation: Option<&'a ImplementationName>,
) -> impl Iterator<Item = Vec<PathBuf>> + 'a {
    // `UV_PYTHON_SEARCH_PATH` overrides `PATH` for Python executable discovery.
    let search_path = env::var_os(EnvVars::UV_PYTHON_SEARCH_PATH)
        .unwrap_or(env::var_os(EnvVars::PATH).unwrap_or_default());

    let possible_names: Vec<_> = version
        .executable_names(implementation)
        .into_iter()
        .map(|name| name.to_string())
        .collect();

    trace!(
        "Searching PATH for executables: {}",
        possible_names.join(", ")
    );

    // Search each directory separately instead of using `which_all`. This preserves search-path
    // order and executable-name priority while checking multiple names per directory.
    let search_dirs: Vec<_> = env::split_paths(&search_path).collect();
    let mut seen_dirs = FxHashSet::with_capacity_and_hasher(search_dirs.len(), FxBuildHasher);
    search_dirs
        .into_iter()
        .filter(|dir| dir.is_dir())
        .flat_map(move |dir| {
            // Clone the directory for the second closure.
            let dir_clone = dir.clone();
            trace!(
                "Checking `PATH` directory for interpreters: {}",
                dir.display()
            );
            same_file::Handle::from_path(&dir)
                // Skip repeated or linked directories to avoid querying the same interpreter twice.
                .map(|handle| seen_dirs.insert(handle))
                .inspect(|fresh_dir| {
                    if !fresh_dir {
                        trace!("Skipping already seen directory: {}", dir.display());
                    }
                })
                // Treat the directory as unique if its identity cannot be determined.
                .unwrap_or(true)
                .then(|| {
                    let minor_version_directory = dir_clone.clone();

                    possible_names
                        .clone()
                        .into_iter()
                        .flat_map(move |name| {
                            // Collect results from one directory to simplify ownership.
                            which::which_in_global(&*name, Some(&dir))
                                .into_iter()
                                .flatten()
                                .filter(|path| !is_windows_store_shim(path))
                                .map(|path| vec![path])
                                // Collect because the returned iterator must outlive the local directory.
                                .collect::<Vec<_>>()
                        })
                        .chain(
                            iter::once_with(move || {
                                find_all_minor(implementation, version, &minor_version_directory)
                                    .filter(|path| !is_windows_store_shim(path))
                                    .collect::<Vec<_>>()
                            })
                            .filter(|paths| !paths.is_empty()),
                        )
                        .inspect(|paths| {
                            for path in paths {
                                trace!("Found possible Python executable: {}", path.display());
                            }
                        })
                        .chain(
                            // TODO(zanieb): Consider moving `python.bat` into `possible_names` to avoid a chain
                            cfg!(windows)
                                .then(move || {
                                    which::which_in_global("python.bat", Some(&dir_clone))
                                        .into_iter()
                                        .flatten()
                                        .map(|path| vec![path])
                                        .collect::<Vec<_>>()
                                })
                                .into_iter()
                                .flatten(),
                        )
                })
                .into_iter()
                .flatten()
        })
}

/// Find all acceptable `python3.x` minor versions.
///
/// For example, `python` and `python3` can both refer to Python 3.10. A request for `>=3.11` must
/// still find `python3.12` in `PATH`.
fn find_all_minor(
    implementation: Option<&ImplementationName>,
    version_request: &VersionRequest,
    dir: &Path,
) -> impl Iterator<Item = PathBuf> + use<> {
    match version_request {
        &VersionRequest::Any
        | VersionRequest::Default
        | VersionRequest::Major(_, _)
        | VersionRequest::Range(_, _) => {
            let regex = if let Some(implementation) = implementation {
                Regex::new(&format!(
                    r"^({}|python3)\.(?<minor>\d\d?)t?{}$",
                    regex::escape(&implementation.to_string()),
                    regex::escape(EXE_SUFFIX)
                ))
                .unwrap()
            } else {
                Regex::new(&format!(
                    r"^python3\.(?<minor>\d\d?)t?{}$",
                    regex::escape(EXE_SUFFIX)
                ))
                .unwrap()
            };
            let all_minors = fs_err::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(move |path| {
                    let Some(filename) = path.file_name() else {
                        return false;
                    };
                    let Some(filename) = filename.to_str() else {
                        return false;
                    };
                    let Some(captures) = regex.captures(filename) else {
                        return false;
                    };

                    // Skip interpreters with a minor version that is too low.
                    let minor = captures["minor"].parse().ok();
                    if let Some(minor) = minor {
                        // Skip unsupported Python versions without querying them.
                        if minor < 6 {
                            return false;
                        }
                        // Skip excluded Python minor versions without querying them.
                        if !version_request.matches_major_minor(3, minor) {
                            return false;
                        }
                    }
                    true
                })
                .filter(|path| is_executable(path))
                .collect::<Vec<_>>();
            Either::Left(all_minors.into_iter())
        }
        VersionRequest::MajorMinor(_, _, _)
        | VersionRequest::MajorMinorPatch(_, _, _, _)
        | VersionRequest::MajorMinorPrerelease(_, _, _, _)
        | VersionRequest::MajorMinorPatchPrerelease(_, _, _, _, _) => Either::Right(iter::empty()),
    }
}

/// How to query discovered Python executables.
#[derive(Debug, Clone, Copy)]
enum QueryStrategy {
    /// Lazily query one executable group at a time.
    Sequential,
    /// Query groups and their executables concurrently before yielding results.
    Parallel,
}

/// Iterate over all discoverable Python interpreters.
///
/// [`EnvironmentPreference`], [`PythonPreference`], [`VersionRequest`], and [`PlatformRequest`]
/// can exclude interpreters.
///
/// Before querying an interpreter, [`PlatformRequest`] applies only to managed installations. The
/// caller must check the platform for other installations.
///
/// See [`python_executables`] for details about discovery.
fn python_installations<'a>(
    version: &'a VersionRequest,
    implementation: Option<&'a ImplementationName>,
    platform: PlatformRequest,
    environments: EnvironmentPreference,
    preference: PythonPreference,
    cache: &'a Cache,
    strategy: QueryStrategy,
) -> Box<dyn Iterator<Item = Result<PythonInstallation, Error>> + 'a> {
    Box::new(
        python_installations_from_executables(
            // Filter executable sources before running expensive interpreter queries. After each
            // query, filter again with `PythonInstallation::satisfies_preferences`.
            python_executables(version, implementation, platform, environments, preference)
                .filter_map(move |result| match result {
                    Ok(group) => group
                        .filter(|source, path| {
                            source_satisfies_environment_preference(source, path, environments)
                        })
                        .map(Ok),
                    Err(error) => Some(Err(error)),
                }),
            cache,
            strategy,
        )
        .filter_ok(move |installation| {
            installation.satisfies_preferences(version, environments, preference)
        })
        .map_ok(PythonInstallation::maybe_with_test_source),
    )
}

/// Query one Python executable and return a [`PythonInstallation`] on success.
fn python_installation_from_executable(
    source: PythonSource,
    path: PathBuf,
    cache: &Cache,
) -> Result<PythonInstallation, Error> {
    Interpreter::query(&path, cache)
        .map(|interpreter| PythonInstallation {
            source,
            interpreter,
        })
        .inspect(|installation| {
            debug!(
                "Found `{}` at `{}` ({source})",
                installation.key(),
                path.display()
            );
        })
        .map_err(|err| Error::Query(Box::new(err), path, source))
        .inspect_err(|err| debug!("{err}"))
}

/// Convert Python executables into installations with the specified query strategy.
fn python_installations_from_executables<'a>(
    executables: impl Iterator<Item = Result<PythonExecutableGroup, Error>> + 'a,
    cache: &'a Cache,
    strategy: QueryStrategy,
) -> Box<dyn Iterator<Item = Result<PythonInstallation, Error>> + 'a> {
    match strategy {
        QueryStrategy::Sequential => Box::new(executables.flat_map(move |group| {
            python_installations_from_executable_group(group, cache, strategy)
        })),
        QueryStrategy::Parallel => {
            let items: Vec<Result<PythonExecutableGroup, Error>> = executables.collect();
            let results: Vec<Vec<Result<PythonInstallation, Error>>> = items
                .into_par_iter()
                .map(|group| {
                    python_installations_from_executable_group(group, cache, strategy)
                        .collect::<Vec<_>>()
                })
                .collect();
            Box::new(results.into_iter().flatten())
        }
    }
}

/// Query an executable group, ordering equally preferred installations by their installation keys.
fn python_installations_from_executable_group(
    group: Result<PythonExecutableGroup, Error>,
    cache: &Cache,
    strategy: QueryStrategy,
) -> impl Iterator<Item = Result<PythonInstallation, Error>> + use<> {
    match group {
        Err(error) => Either::Left(iter::once(Err(error))),
        Ok(PythonExecutableGroup(executables)) => {
            let mut installations = match strategy {
                QueryStrategy::Sequential => executables
                    .into_iter()
                    .map(|(source, path)| python_installation_from_executable(source, path, cache))
                    .collect::<Vec<_>>(),
                QueryStrategy::Parallel => executables
                    .into_par_iter()
                    .map(|(source, path)| python_installation_from_executable(source, path, cache))
                    .collect::<Vec<_>>(),
            };

            sort_installations_by_key(&mut installations, PythonInstallation::key);

            Either::Right(installations.into_iter())
        }
    }
}

/// Sort successful installations without moving them across critical query errors.
fn sort_installations_by_key<T, K: Ord>(
    installations: &mut [Result<T, Error>],
    key: impl Fn(&T) -> K,
) {
    // Critical errors preserve discovery order; non-critical errors must not interrupt
    // installation-key ordering and can follow successful queries.
    for candidates in
        installations.split_mut(|result| result.as_ref().is_err_and(Error::is_critical))
    {
        candidates.sort_by_key(|result| Reverse(result.as_ref().ok().map(&key)));
    }
}

/// Return `true` if an [`Interpreter`] matches the [`EnvironmentPreference`].
///
/// Query the interpreter to check the preference. The
/// [`source_satisfies_environment_preference`] filter only checks whether a [`PythonSource`]
/// could match. It cannot confirm whether an interpreter belongs to a virtual environment.
fn interpreter_satisfies_environment_preference(
    source: PythonSource,
    interpreter: &Interpreter,
    preference: EnvironmentPreference,
) -> bool {
    match (
        preference,
        // Treat conda environments as virtual environments even though they do not follow PEP 405.
        interpreter.is_virtualenv() || (matches!(source, PythonSource::CondaPrefix)),
    ) {
        (EnvironmentPreference::Any, _) => true,
        (EnvironmentPreference::OnlyVirtual, true) => true,
        (EnvironmentPreference::OnlyVirtual, false) => {
            debug!(
                "Ignoring Python interpreter at `{}`: only virtual environments allowed",
                interpreter.sys_executable().display()
            );
            false
        }
        (EnvironmentPreference::ExplicitSystem, true) => true,
        (EnvironmentPreference::ExplicitSystem, false) => {
            if matches!(
                source,
                PythonSource::ProvidedPath | PythonSource::ParentInterpreter
            ) {
                debug!(
                    "Allowing explicitly requested system Python interpreter at `{}`",
                    interpreter.sys_executable().display()
                );
                true
            } else {
                debug!(
                    "Ignoring Python interpreter at `{}`: system interpreter not explicitly requested",
                    interpreter.sys_executable().display()
                );
                false
            }
        }
        (EnvironmentPreference::OnlySystem, true) => {
            debug!(
                "Ignoring Python interpreter at `{}`: system interpreter required",
                interpreter.sys_executable().display()
            );
            false
        }
        (EnvironmentPreference::OnlySystem, false) => true,
    }
}

/// Return `true` if a [`PythonSource`] could satisfy the [`EnvironmentPreference`].
///
/// Use this as an initial filter. Call [`EnvironmentPreference::allows_installation`] to confirm
/// that an [`Interpreter`] satisfies the preference.
///
/// The interpreter path is only used for debug messages.
fn source_satisfies_environment_preference(
    source: PythonSource,
    interpreter_path: &Path,
    preference: EnvironmentPreference,
) -> bool {
    match preference {
        EnvironmentPreference::Any => true,
        EnvironmentPreference::OnlyVirtual => {
            if source.is_maybe_virtualenv() {
                true
            } else {
                debug!(
                    "Ignoring Python interpreter at `{}`: only virtual environments allowed",
                    interpreter_path.display()
                );
                false
            }
        }
        EnvironmentPreference::ExplicitSystem => {
            if source.is_maybe_virtualenv() {
                true
            } else {
                debug!(
                    "Ignoring Python interpreter at `{}`: system interpreter not explicitly requested",
                    interpreter_path.display()
                );
                false
            }
        }
        EnvironmentPreference::OnlySystem => {
            if source.is_maybe_system() {
                true
            } else {
                debug!(
                    "Ignoring Python interpreter at `{}`: system interpreter required",
                    interpreter_path.display()
                );
                false
            }
        }
    }
}

/// Check whether an error is critical and must stop discovery.
///
/// Return `false` if the error can come from a broken Python installation. Continue searching for
/// a working installation in that case.
impl Error {
    pub(crate) fn is_critical(&self) -> bool {
        match self {
            // Stop only for errors that indicate a critical failure. If an interpreter returns an
            // invalid response, continue searching for a working interpreter.
            Self::Query(err, _, source) => match &**err {
                InterpreterError::Encode(_)
                | InterpreterError::Io(_)
                | InterpreterError::SpawnFailed { .. } => true,
                InterpreterError::UnexpectedResponse(UnexpectedResponseError { path, .. })
                | InterpreterError::StatusCode(StatusCodeError { path, .. }) => {
                    debug!(
                        "Skipping bad interpreter at {} from {source}: {err}",
                        path.display()
                    );
                    false
                }
                InterpreterError::QueryScript { path, err } => {
                    debug!(
                        "Skipping bad interpreter at {} from {source}: {err}",
                        path.display()
                    );
                    false
                }
                #[cfg(windows)]
                InterpreterError::CorruptWindowsPackage { path, err } => {
                    debug!(
                        "Skipping bad interpreter at {} from {source}: {err}",
                        path.display()
                    );
                    false
                }
                InterpreterError::PermissionDenied { path, err } => {
                    debug!(
                        "Skipping unexecutable interpreter at {} from {source}: {err}",
                        path.display()
                    );
                    false
                }
                InterpreterError::NotFound(path)
                | InterpreterError::BrokenLink(BrokenLink { path, .. }) => {
                    // Fail if the missing interpreter belongs to an active virtual environment.
                    if matches!(source, PythonSource::ActiveEnvironment)
                        && uv_fs::is_virtualenv_executable(path)
                    {
                        true
                    } else {
                        trace!("Skipping missing interpreter at {}", path.display());
                        false
                    }
                }
            },
            Self::VirtualEnv(VirtualEnvError::MissingPyVenvCfg(path)) => {
                trace!("Skipping broken virtualenv at {}", path.display());
                false
            }
            _ => true,
        }
    }
}

/// Create a [`PythonInstallation`] from a Python installation root directory.
fn python_installation_from_directory(
    path: &PathBuf,
    cache: &Cache,
) -> Result<PythonInstallation, crate::interpreter::Error> {
    let executable = virtualenv_python_executable(path);
    Ok(PythonInstallation {
        source: PythonSource::ProvidedPath,
        interpreter: Interpreter::query(&executable, cache)?,
    })
}

/// Iterate lazily over Python executables in `PATH` with the specified name.
fn python_executables_with_name(
    name: &str,
) -> impl Iterator<Item = Result<(PythonSource, PathBuf), Error>> + '_ {
    which_all(name)
        .into_iter()
        .flat_map(|inner| inner.map(|path| Ok((PythonSource::SearchPath, path))))
}

/// Iterate lazily over Python installations in `PATH` with the specified executable name.
fn python_installations_with_name<'a>(
    name: &'a str,
    cache: &'a Cache,
    strategy: QueryStrategy,
) -> Box<dyn Iterator<Item = Result<PythonInstallation, Error>> + 'a> {
    python_installations_from_executables(
        python_executables_with_name(name)
            .map_ok(|executable| PythonExecutableGroup(vec![executable])),
        cache,
        strategy,
    )
}

/// Iterate over Python installations that satisfy the request.
pub(crate) fn find_python_installations<'a>(
    request: &'a PythonRequest,
    environments: EnvironmentPreference,
    preference: PythonPreference,
    cache: &'a Cache,
) -> Box<dyn Iterator<Item = Result<FindPythonResult, Error>> + 'a> {
    find_python_installations_with_strategy(
        request,
        environments,
        preference,
        cache,
        QueryStrategy::Sequential,
    )
}

/// Iterate over matching Python installations with the specified query strategy.
fn find_python_installations_with_strategy<'a>(
    request: &'a PythonRequest,
    environments: EnvironmentPreference,
    preference: PythonPreference,
    cache: &'a Cache,
    strategy: QueryStrategy,
) -> Box<dyn Iterator<Item = Result<FindPythonResult, Error>> + 'a> {
    let sources = DiscoveryPreferences {
        python_preference: preference,
        environment_preference: environments,
    }
    .sources(request);

    match request {
        PythonRequest::File(path) => Box::new(iter::once({
            if preference.allows_source(PythonSource::ProvidedPath) {
                debug!("Checking for Python interpreter at {request}");
                match Interpreter::query(path, cache) {
                    Ok(interpreter) => Ok(Ok(PythonInstallation {
                        source: PythonSource::ProvidedPath,
                        interpreter,
                    })),
                    Err(InterpreterError::NotFound(_) | InterpreterError::BrokenLink(_)) => {
                        Ok(Err(PythonNotFound {
                            request: request.clone(),
                            python_preference: preference,
                            environment_preference: environments,
                        }))
                    }
                    Err(err) => Err(Error::Query(
                        Box::new(err),
                        path.clone(),
                        PythonSource::ProvidedPath,
                    )),
                }
            } else {
                Err(Error::SourceNotAllowed(
                    request.clone(),
                    PythonSource::ProvidedPath,
                    preference,
                ))
            }
        })),
        PythonRequest::Directory(path) => Box::new(iter::once({
            if preference.allows_source(PythonSource::ProvidedPath) {
                debug!("Checking for Python interpreter in {request}");
                match python_installation_from_directory(path, cache) {
                    Ok(installation) => Ok(Ok(installation)),
                    Err(InterpreterError::NotFound(_) | InterpreterError::BrokenLink(_)) => {
                        Ok(Err(PythonNotFound {
                            request: request.clone(),
                            python_preference: preference,
                            environment_preference: environments,
                        }))
                    }
                    Err(err) => Err(Error::Query(
                        Box::new(err),
                        path.clone(),
                        PythonSource::ProvidedPath,
                    )),
                }
            } else {
                Err(Error::SourceNotAllowed(
                    request.clone(),
                    PythonSource::ProvidedPath,
                    preference,
                ))
            }
        })),
        PythonRequest::ExecutableName(name) => {
            if preference.allows_source(PythonSource::SearchPath) {
                debug!("Searching for Python interpreter with {request}");
                Box::new(
                    python_installations_with_name(name, cache, strategy)
                        .filter_ok(move |installation| {
                            environments.allows_installation(installation)
                        })
                        .map_ok(Ok),
                )
            } else {
                Box::new(iter::once(Err(Error::SourceNotAllowed(
                    request.clone(),
                    PythonSource::SearchPath,
                    preference,
                ))))
            }
        }
        PythonRequest::Any => Box::new({
            debug!("Searching for any Python interpreter in {sources}");
            python_installations(
                &VersionRequest::Any,
                None,
                PlatformRequest::default(),
                environments,
                preference,
                cache,
                strategy,
            )
            .map_ok(Ok)
        }),
        PythonRequest::Default => Box::new({
            debug!("Searching for default Python interpreter in {sources}");
            python_installations(
                &VersionRequest::Default,
                None,
                PlatformRequest::default(),
                environments,
                preference,
                cache,
                strategy,
            )
            .map_ok(Ok)
        }),
        PythonRequest::Version(version) => {
            if let Err(err) = version.check_supported() {
                return Box::new(iter::once(Err(Error::InvalidVersionRequest(err))));
            }
            Box::new({
                debug!("Searching for {request} in {sources}");
                python_installations(
                    version,
                    None,
                    PlatformRequest::default(),
                    environments,
                    preference,
                    cache,
                    strategy,
                )
                .map_ok(Ok)
            })
        }
        PythonRequest::Implementation(implementation) => Box::new({
            debug!("Searching for a {request} interpreter in {sources}");
            python_installations(
                &VersionRequest::Default,
                Some(implementation),
                PlatformRequest::default(),
                environments,
                preference,
                cache,
                strategy,
            )
            .filter_ok(|installation| implementation.matches_interpreter(&installation.interpreter))
            .map_ok(Ok)
        }),
        PythonRequest::ImplementationVersion(implementation, version) => {
            if let Err(err) = version.check_supported() {
                return Box::new(iter::once(Err(Error::InvalidVersionRequest(err))));
            }
            Box::new({
                debug!("Searching for {request} in {sources}");
                python_installations(
                    version,
                    Some(implementation),
                    PlatformRequest::default(),
                    environments,
                    preference,
                    cache,
                    strategy,
                )
                .filter_ok(|installation| {
                    implementation.matches_interpreter(&installation.interpreter)
                })
                .map_ok(Ok)
            })
        }
        PythonRequest::Key(request) => {
            if let Some(version) = request.version()
                && let Err(err) = version.check_supported()
            {
                return Box::new(iter::once(Err(Error::InvalidVersionRequest(err))));
            }

            Box::new({
                debug!("Searching for {request} in {sources}");
                python_installations(
                    request.version().unwrap_or(&VersionRequest::Default),
                    request.implementation(),
                    request.platform(),
                    environments,
                    preference,
                    cache,
                    strategy,
                )
                .filter_ok(move |installation| {
                    request.satisfied_by_interpreter(&installation.interpreter)
                })
                .map_ok(Ok)
            })
        }
    }
}

/// Find all matching Python installations and query their interpreters concurrently.
///
/// Unlike [`find_python_installations`], collect all matching installations immediately. Ignore
/// non-critical discovery errors. Return critical errors in discovery order.
pub fn find_all_python_installations(
    request: &PythonRequest,
    environments: EnvironmentPreference,
    preference: PythonPreference,
    cache: &Cache,
) -> Result<Vec<PythonInstallation>, Error> {
    let results = find_python_installations_with_strategy(
        request,
        environments,
        preference,
        cache,
        QueryStrategy::Parallel,
    );
    let mut installations = Vec::new();
    for result in results {
        match result {
            Ok(Ok(installation)) => installations.push(installation),
            Ok(Err(_)) => {}
            Err(err) if err.is_critical() => return Err(err),
            Err(_) => {}
        }
    }
    Ok(installations)
}

/// Find a Python installation that satisfies the request.
///
/// If a critical error occurs while locating or inspecting an installation, return that error.
pub(crate) fn find_python_installation(
    request: &PythonRequest,
    environments: EnvironmentPreference,
    preference: PythonPreference,
    cache: &Cache,
) -> Result<FindPythonResult, Error> {
    let installations = find_python_installations(request, environments, preference, cache);
    let mut first_prerelease = None;
    let mut first_debug = None;
    let mut first_managed = None;
    let mut first_error = None;
    for result in installations {
        // Stop at the first critical error or accepted installation.
        if !result.as_ref().err().is_none_or(Error::is_critical) {
            // Save the first non-critical error.
            if first_error.is_none()
                && let Err(err) = result
            {
                first_error = Some(err);
            }
            continue;
        }

        // Return immediately for a critical error.
        let Ok(Ok(ref installation)) = result else {
            return result;
        };

        // Skip interpreters that require explicit selection, such as pre-releases or alternative
        // implementations.

        // A default executable name in the search path, such as `python`, counts as an explicit
        // selection.
        let has_default_executable_name = installation.interpreter.has_default_executable_name()
            && matches!(
                installation.source,
                PythonSource::SearchPath | PythonSource::SearchPathFirst
            );

        // Save a disallowed pre-release as a fallback when no other version is available.
        if installation.python_version().pre().is_some()
            && !request.allows_prereleases()
            && !installation.source.allows_prereleases()
            && !has_default_executable_name
        {
            debug!("Skipping pre-release installation {}", installation.key());
            if first_prerelease.is_none() {
                first_prerelease = Some(installation.clone());
            }
            continue;
        }

        // Save a disallowed debug build as a fallback when no other version is available.
        if installation.key().variant().is_debug()
            && !request.allows_debug()
            && !installation.source.allows_debug()
            && !has_default_executable_name
        {
            debug!("Skipping debug installation {}", installation.key());
            if first_debug.is_none() {
                first_debug = Some(installation.clone());
            }
            continue;
        }

        // Skip alternative implementations unless explicitly allowed. Unrequested alternatives in
        // the search path are not queried, but managed installations can still contain them.
        if installation.is_alternative_implementation()
            && !request.allows_alternative_implementations()
            && !installation.source.allows_alternative_implementations()
            && !has_default_executable_name
        {
            debug!("Skipping alternative implementation {}", installation.key());
            continue;
        }

        // Save a managed installation as a fallback when system interpreters are preferred.
        if matches!(preference, PythonPreference::System) && installation.is_managed() {
            debug!(
                "Skipping managed installation {}: system installation preferred",
                installation.key()
            );
            if first_managed.is_none() {
                first_managed = Some(installation.clone());
            }
            continue;
        }

        // Use the first installation that was not skipped.
        return result;
    }

    // Return the first managed installation if no system installation was found.
    if let Some(installation) = first_managed {
        debug!(
            "Allowing managed installation {}: no system installations",
            installation.key()
        );
        return Ok(Ok(installation));
    }

    // Return the first debug installation if no non-debug installation was found.
    if let Some(installation) = first_debug {
        debug!(
            "Allowing debug installation {}: no non-debug installations",
            installation.key()
        );
        return Ok(Ok(installation));
    }

    // Return the first pre-release if no stable installation was found.
    if let Some(installation) = first_prerelease {
        debug!(
            "Allowing pre-release installation {}: no stable installations",
            installation.key()
        );
        return Ok(Ok(installation));
    }

    // Report an unusable Python installation instead of claiming that none was found.
    if let Some(err) = first_error {
        return Err(err);
    }

    Ok(Err(PythonNotFound {
        request: request.clone(),
        environment_preference: environments,
        python_preference: preference,
    }))
}

/// Find the Python installation that best matches the request.
///
/// If no Python version is specified, use the first available installation.
///
/// If a Python version is specified, first look for an exact match. If a requested patch version
/// is unavailable, match the major and minor version instead. If that also fails, use the first
/// available version.
///
/// At each step, download the requested version if it is unavailable and downloads are enabled.
///
/// See [`find_python_installation`] for details about installation discovery.
#[instrument(skip_all, fields(request))]
pub(crate) async fn find_best_python_installation(
    request: &PythonRequest,
    environments: EnvironmentPreference,
    preference: PythonPreference,
    downloads_enabled: bool,
    client_builder: &BaseClientBuilder<'_>,
    cache: &Cache,
    reporter: Option<&dyn crate::downloads::Reporter>,
    python_install_mirror: Option<&str>,
    pypy_install_mirror: Option<&str>,
    python_downloads_json_url: Option<&str>,
) -> Result<PythonInstallation, crate::Error> {
    debug!("Starting Python discovery for {request}");
    let original_request = request;

    let mut previous_fetch_failed = false;
    let mut download_state = None;

    let request_without_patch = match request {
        PythonRequest::Version(version) => {
            if version.has_patch() {
                Some(PythonRequest::Version(version.clone().without_patch()))
            } else {
                None
            }
        }
        PythonRequest::ImplementationVersion(implementation, version) => Some(
            PythonRequest::ImplementationVersion(*implementation, version.clone().without_patch()),
        ),
        _ => None,
    };

    for (attempt, request) in iter::once(original_request)
        .chain(request_without_patch.iter())
        .chain(iter::once(&PythonRequest::Default))
        .enumerate()
    {
        debug!(
            "Looking for {request}{}",
            if request != original_request {
                format!(" attempt {attempt} (fallback after failing to find: {original_request})")
            } else {
                String::new()
            }
        );
        let result = find_python_installation(request, environments, preference, cache);
        let error = match result {
            Ok(Ok(installation)) => {
                warn_on_unsupported_python(installation.interpreter());
                return Ok(installation);
            }
            // Continue when no Python matches or when discovery returns a non-critical error.
            Ok(Err(error)) => error.into(),
            Err(error) if !error.is_critical() => error.into(),
            Err(error) => return Err(error.into()),
        };

        // Download the version when downloads are enabled.
        if downloads_enabled
            && !previous_fetch_failed
            && let Some(download_request) = PythonDownloadRequest::from_request(request)
        {
            let (client, retry_policy, download_list) =
                if let Some(download_state) = &mut download_state {
                    download_state
                } else {
                    let download_list = ManagedPythonDownloadList::new(
                        client_builder,
                        cache,
                        python_downloads_json_url,
                    )
                    .await?;
                    let retry_policy = client_builder.retry_policy();

                    // Python downloads retry stream errors. Disable middleware retries to avoid
                    // extra, uncontrolled attempts.
                    let client = client_builder.clone().retries(0).build()?;
                    download_state.insert((client, retry_policy, download_list))
                };

            let download = download_request
                .clone()
                .fill()
                .map(|request| download_list.find(&request));

            let result = match download {
                Ok(Ok(download)) => PythonInstallation::fetch(
                    download,
                    client,
                    retry_policy,
                    cache,
                    reporter,
                    python_install_mirror,
                    pypy_install_mirror,
                )
                .await
                .map(Some),
                Ok(Err(crate::downloads::Error::NoDownloadFound(_))) => Ok(None),
                Ok(Err(error)) => Err(error.into()),
                Err(error) => Err(error.into()),
            };
            if let Ok(Some(installation)) = result {
                return Ok(installation);
            }
            // Warn instead of failing because a later, less specific request can find a system
            // interpreter. Older versions did not download in this path, so avoid new fatal errors.
            // These failures usually come from the network or configuration.
            if let Err(error) = result {
                // Return the error for a default or unrestricted request. No later fallback can
                // recover from it.
                if matches!(request, PythonRequest::Default | PythonRequest::Any) {
                    return Err(error);
                }

                let error = anyhow::Error::from(error).context(format!(
                    "A managed Python download is available for {request}, but an error occurred when attempting to download it."
                ));
                write_warning_chain(error.as_ref(), Hints::none())
                    .expect("writing to stderr should not fail");
                previous_fetch_failed = true;
            }
        }

        // A default or unrestricted request is either the original request or the final fallback.
        // Return its discovery error.
        if matches!(request, PythonRequest::Default | PythonRequest::Any) {
            return Err(match error {
                crate::Error::MissingPython(err, _) => PythonNotFound {
                    // Use the original request because the search covered multiple versions.
                    request: original_request.clone(),
                    python_preference: err.python_preference,
                    environment_preference: err.environment_preference,
                }
                .into(),
                other => other,
            });
        }
    }

    unreachable!("The loop should have terminated when it reached PythonRequest::Default");
}

/// Warn if uv does not support the Python version of the [`Interpreter`].
fn warn_on_unsupported_python(interpreter: &Interpreter) {
    // Warn when the Python version is unsupported.
    if interpreter.python_tuple() < (3, 8) {
        warn_user_once!(
            "uv is only compatible with Python >=3.8, found Python {}",
            interpreter.python_version()
        );
    }
}

/// Detect the Windows Store proxy shim.
///
/// Windows can enable this shim in Settings > Apps > Advanced app settings > App execution aliases.
/// If Python is not installed from the Windows Store, `python.exe` and `python3.exe` can open the
/// Windows Store installer. Do not treat those files as Python executables.
///
/// This method comes from Rye:
///
/// > This is a pretty dumb way.  We know how to parse this reparse point, but Microsoft
/// > does not want us to do this as the format is unstable.  So this is a best effort way.
/// > we just hope that the reparse point has the python redirector in it, when it's not
/// > pointing to a valid Python.
///
/// See: <https://github.com/astral-sh/rye/blob/b0e9eccf05fe4ff0ae7b0250a248c54f2d780b4d/rye/src/cli/shim.rs#L108>
#[cfg(windows)]
fn is_windows_store_shim(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::prelude::OsStrExt;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_MODE, MAXIMUM_REPARSE_DATA_BUFFER_SIZE,
        OPEN_EXISTING,
    };
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
    use windows::core::PCWSTR;

    // The path must be absolute.
    if !path.is_absolute() {
        return false;
    }

    // The path must have this form:
    //   `C:\Users\crmar\AppData\Local\Microsoft\WindowsApps\python3.exe`
    let mut components = path.components().rev();

    // Match `python.exe`, `python3.exe`, or a version-specific name such as `python3.12.exe`.
    if !components
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .is_some_and(|component| {
            component.starts_with("python")
                && std::path::Path::new(component)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        })
    {
        return false;
    }

    // Match the `WindowsApps` directory.
    if components
        .next()
        .is_none_or(|component| component.as_os_str() != "WindowsApps")
    {
        return false;
    }

    // Match the `Microsoft` directory.
    if components
        .next()
        .is_none_or(|component| component.as_os_str() != "Microsoft")
    {
        return false;
    }

    // Only inspect files that are reparse points.
    let Ok(md) = fs_err::symlink_metadata(path) else {
        return false;
    };
    if md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0 {
        return false;
    }

    let mut path_encoded = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    // SAFETY: The path is null-terminated.
    #[allow(unsafe_code)]
    let reparse_handle = unsafe {
        CreateFileW(
            PCWSTR(path_encoded.as_mut_ptr()),
            0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    };

    let Ok(reparse_handle) = reparse_handle else {
        return false;
    };

    let mut buf = [0u16; MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize];
    let mut bytes_returned = 0;

    // SAFETY: The buffer is large enough to hold the reparse point.
    #[allow(unsafe_code, clippy::cast_possible_truncation)]
    let success = unsafe {
        DeviceIoControl(
            reparse_handle,
            FSCTL_GET_REPARSE_POINT,
            None,
            0,
            Some(buf.as_mut_ptr().cast()),
            buf.len() as u32 * 2,
            Some(&raw mut bytes_returned),
            None,
        )
        .is_ok()
    };

    // SAFETY: The handle is valid.
    #[allow(unsafe_code)]
    unsafe {
        let _ = CloseHandle(reparse_handle);
    }

    // Treat a failed operation as a file that is not a reparse point.
    if !success {
        return false;
    }

    let reparse_point = String::from_utf16_lossy(&buf[..bytes_returned as usize]);
    reparse_point.contains("\\AppInstallerPythonRedirector.exe")
}

/// Return `false` on Unix because Windows Store shims are not relevant.
///
/// See the Windows implementation for details.
#[cfg(not(windows))]
fn is_windows_store_shim(_path: &Path) -> bool {
    false
}

impl PythonVariant {
    fn matches_interpreter(self, interpreter: &Interpreter) -> bool {
        match self {
            Self::Default => {
                // TODO(zanieb): Consider removing the default debug-interpreter selection after
                // checking backward compatibility.
                if (interpreter.python_major(), interpreter.python_minor()) >= (3, 14) {
                    // Python 3.14 and later allow free-threaded builds by default.
                    true
                } else {
                    // Python 3.13 and earlier require explicit selection for free-threaded builds.
                    !interpreter.gil_disabled()
                }
            }
            Self::Debug => interpreter.debug_enabled(),
            Self::Freethreaded => interpreter.gil_disabled(),
            Self::FreethreadedDebug => interpreter.gil_disabled() && interpreter.debug_enabled(),
            Self::Gil => !interpreter.gil_disabled(),
            Self::GilDebug => !interpreter.gil_disabled() && interpreter.debug_enabled(),
        }
    }

    /// Return the executable suffix for the variant, such as `t` for `python3.13t`.
    ///
    /// Return an empty string for the default Python variant.
    pub fn executable_suffix(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Debug => "d",
            Self::Freethreaded => "t",
            Self::FreethreadedDebug => "td",
            Self::Gil => "",
            Self::GilDebug => "d",
        }
    }

    /// Return the display suffix, such as `+gil`.
    pub fn display_suffix(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Debug => "+debug",
            Self::Freethreaded => "+freethreaded",
            Self::FreethreadedDebug => "+freethreaded+debug",
            Self::Gil => "+gil",
            Self::GilDebug => "+gil+debug",
        }
    }

    /// Return the library suffix for the variant.
    ///
    /// Return `t` for `python3.13t`. Return an empty string for `python3.13d` or `python3.13`.
    pub(crate) fn lib_suffix(self) -> &'static str {
        match self {
            Self::Default | Self::Debug | Self::Gil | Self::GilDebug => "",
            Self::Freethreaded | Self::FreethreadedDebug => "t",
        }
    }

    fn is_freethreaded(self) -> bool {
        match self {
            Self::Default | Self::Debug | Self::Gil | Self::GilDebug => false,
            Self::Freethreaded | Self::FreethreadedDebug => true,
        }
    }

    pub fn is_debug(self) -> bool {
        match self {
            Self::Default | Self::Freethreaded | Self::Gil => false,
            Self::Debug | Self::FreethreadedDebug | Self::GilDebug => true,
        }
    }
}
impl PythonRequest {
    /// Create a request from a `Requires-Python` constraint.
    pub fn from_requires_python(requires_python: &RequiresPython) -> Option<Self> {
        let specifiers = requires_python.specifiers().clone();
        if specifiers.is_empty() {
            return None;
        }

        Some(Self::Version(VersionRequest::from_specifiers(
            specifiers,
            PythonVariant::Default,
        )))
    }

    /// Create a request from a string.
    ///
    /// This method cannot fail. Parse unrecognized inputs as [`PythonRequest::File`] or
    /// [`PythonRequest::ExecutableName`].
    ///
    /// Use this method to parse the `--python` argument. See also
    /// [`try_from_tool_name`][Self::try_from_tool_name].
    pub fn parse(value: &str) -> Self {
        let lowercase_value = &value.to_ascii_lowercase();

        // Match literal values such as `any` and `default`.
        if lowercase_value == "any" {
            return Self::Any;
        }
        if lowercase_value == "default" {
            return Self::Default;
        }

        // Match the `python` prefix in `python312` and the empty prefix in `312`.
        let abstract_version_prefixes = ["python", ""];
        let all_implementation_names = ImplementationName::iter_all().flat_map(|implementation| {
            std::iter::once(implementation.long_name()).chain(implementation.short_name())
        });
        // Match version requests such as `python@312`, `python312`, and `312`. Also match
        // implementation requests such as `pypy`, `pypy@312`, and `pypy312`.
        if let Ok(Some(request)) = Self::parse_versions_and_implementations(
            abstract_version_prefixes,
            all_implementation_names,
            lowercase_value,
        ) {
            return request;
        }

        let value_as_path = PathBuf::from(value);
        // Match an environment directory such as `/path/to/.venv`.
        if value_as_path.is_dir() {
            return Self::Directory(value_as_path);
        }
        // Match an executable such as `/path/to/python`.
        if value_as_path.is_file() {
            return Self::File(value_as_path);
        }

        // On Windows, `path/to/python` can refer to `path/to/python.exe`.
        #[cfg(windows)]
        if value_as_path.extension().is_none() {
            let value_as_path = value_as_path.with_extension(EXE_SUFFIX);
            if value_as_path.is_file() {
                return Self::File(value_as_path);
            }
        }

        // Unit tests cannot change the process working directory. Check paths relative to the mock
        // working directory instead. CLI-level tests could avoid this special case.
        #[cfg(test)]
        if value_as_path.is_relative() {
            if let Ok(current_dir) = crate::current_dir() {
                let relative = current_dir.join(&value_as_path);
                if relative.is_dir() {
                    return Self::Directory(relative);
                }
                if relative.is_file() {
                    return Self::File(relative);
                }
            }
        }
        // Treat a value with a path separator as a path even if it does not exist.
        if value.contains(std::path::MAIN_SEPARATOR) {
            return Self::File(value_as_path);
        }
        // Also accept Unix path separators on Windows.
        if cfg!(windows) && value.contains('/') {
            return Self::File(value_as_path);
        }
        if let Ok(request) = PythonDownloadRequest::from_str(value) {
            return Self::Key(request);
        }
        // Otherwise, treat the value as an executable name to find in `PATH`.
        Self::ExecutableName(value.to_string())
    }

    /// Parse a tool name as a Python version, such as `uvx python311`.
    ///
    /// [`PythonRequest::parse`] handles `--python`, where the value identifies a Python request.
    /// This method handles `uvx` and `uvx --from`, where a value can identify either a Python
    /// version or a package.
    ///
    /// - Accept long names such as `pypy39`. Do not accept `pp39` or `39`.
    /// - On Windows, accept `pythonw` as an alias for `python`.
    /// - Accept `python` as an alias for `default`. On Windows, also accept `pythonw`.
    ///
    /// Return `Err` only when the value uses `@`. Return `Ok(None)` when no value matches.
    pub fn try_from_tool_name(value: &str) -> Result<Option<Self>, Error> {
        let lowercase_value = &value.to_ascii_lowercase();
        // Omitting the empty string from these lists excludes bare versions like "39".
        let abstract_version_prefixes = if cfg!(windows) {
            &["python", "pythonw"][..]
        } else {
            &["python"][..]
        };
        // Match an executable name without a version, such as `python`.
        if abstract_version_prefixes.contains(&lowercase_value.as_str()) {
            return Ok(Some(Self::Default));
        }
        Self::parse_versions_and_implementations(
            abstract_version_prefixes.iter().copied(),
            ImplementationName::iter_all().map(ImplementationName::long_name),
            lowercase_value,
        )
    }

    /// Parse a Python version from a value such as `"python3.11"`.
    ///
    /// Match a generic prefix such as `"python"`, `"pythonw"`, or `""`. Also match specific
    /// implementations such as `"cpython"`, `"pypy"`, and their supported abbreviations.
    ///
    /// Return `Err` only when the value uses `@`. Return `Ok(None)` when no value matches. See
    /// [`try_split_prefix_and_version`][Self::try_split_prefix_and_version].
    fn parse_versions_and_implementations<'a>(
        // Generic prefixes include "python", "pythonw", and "" for bare versions.
        abstract_version_prefixes: impl IntoIterator<Item = &'a str>,
        // Include either long implementation names or all implementation names.
        implementation_names: impl IntoIterator<Item = &'a str>,
        // The string to parse.
        lowercase_value: &str,
    ) -> Result<Option<Self>, Error> {
        for prefix in abstract_version_prefixes {
            if let Some(version_request) =
                Self::try_split_prefix_and_version(prefix, lowercase_value)?
            {
                // Match requests such as `python39` and `python@39`. Handle `python` without a
                // version separately. It is valid for tool executables, but not for `--python`.
                return Ok(Some(Self::Version(version_request)));
            }
        }
        for implementation in implementation_names {
            if lowercase_value == implementation {
                return Ok(Some(Self::Implementation(
                    // For example, `pypy`.
                    // Safety: The name matched the possible names above
                    ImplementationName::from_str(implementation).unwrap(),
                )));
            }
            if let Some(version_request) =
                Self::try_split_prefix_and_version(implementation, lowercase_value)?
            {
                // For example, `pypy39`.
                return Ok(Some(Self::ImplementationVersion(
                    // Safety: The name matched the possible names above
                    ImplementationName::from_str(implementation).unwrap(),
                    version_request,
                )));
            }
        }
        Ok(None)
    }

    /// Parse a version from a value that matches a target prefix.
    ///
    /// For example, `"python3.11"` matches the `"python"` prefix. Other prefixes include
    /// `"pypy"` and `""`.
    ///
    /// Return `Ok(None)` if the prefix does not match or the version cannot be parsed. The `@`
    /// separator is optional. Return `Err` only in these cases:
    ///
    /// - The value starts with `@`, such as `@3.11`.
    /// - The prefix matches, but the version after `@` is invalid, such as `python@3.not.a.version`.
    fn try_split_prefix_and_version(
        prefix: &str,
        lowercase_value: &str,
    ) -> Result<Option<VersionRequest>, Error> {
        if lowercase_value.starts_with('@') {
            return Err(Error::InvalidVersionRequest(lowercase_value.to_string()));
        }
        let Some(rest) = lowercase_value.strip_prefix(prefix) else {
            return Ok(None);
        };
        // Handle a prefix without a version, such as "python", elsewhere.
        if rest.is_empty() {
            return Ok(None);
        }
        // The `@` separator is optional. When present, return errors for an invalid version.
        if let Some(after_at) = rest.strip_prefix('@') {
            if after_at == "latest" {
                // Return a special error for `@latest` until it is supported.
                // TODO(zanieb): Add `PythonRequest::Latest`.
                return Err(Error::LatestVersionRequest);
            }
            return after_at.parse().map(Some);
        }
        // Without `@`, return `Ok(None)` for an invalid version such as `python3stuff`.
        Ok(rest.parse().ok())
    }

    /// Return `true` if this request includes a specific patch version.
    pub fn includes_patch(&self) -> bool {
        match self {
            Self::Default => false,
            Self::Any => false,
            Self::Version(version_request) => version_request.patch().is_some(),
            Self::Directory(..) => false,
            Self::File(..) => false,
            Self::ExecutableName(..) => false,
            Self::Implementation(..) => false,
            Self::ImplementationVersion(_, version) => version.patch().is_some(),
            Self::Key(request) => request
                .version
                .as_ref()
                .is_some_and(|request| request.patch().is_some()),
        }
    }

    /// Return `true` if this request includes a specific pre-release version.
    pub fn includes_prerelease(&self) -> bool {
        match self {
            Self::Default => false,
            Self::Any => false,
            Self::Version(version_request) => version_request.prerelease().is_some(),
            Self::Directory(..) => false,
            Self::File(..) => false,
            Self::ExecutableName(..) => false,
            Self::Implementation(..) => false,
            Self::ImplementationVersion(_, version) => version.prerelease().is_some(),
            Self::Key(request) => request
                .version
                .as_ref()
                .is_some_and(|request| request.prerelease().is_some()),
        }
    }

    /// Return `true` if an interpreter satisfies this request.
    pub fn satisfied(&self, interpreter: &Interpreter, cache: &Cache) -> bool {
        /// Return `true` if both paths refer to the same interpreter executable.
        fn is_same_executable(path1: &Path, path2: &Path) -> bool {
            path1 == path2 || is_same_file(path1, path2).unwrap_or(false)
        }

        match self {
            Self::Default | Self::Any => true,
            Self::Version(version_request) => version_request.matches_interpreter(interpreter),
            Self::Directory(directory) => {
                // Match the environment root or its Python executable.
                is_same_executable(directory, interpreter.sys_prefix())
                    || is_same_executable(
                        virtualenv_python_executable(directory).as_path(),
                        interpreter.sys_executable(),
                    )
            }
            Self::File(file) => {
                // The interpreter satisfies the request both if it is the venv...
                if is_same_executable(interpreter.sys_executable(), file) {
                    return true;
                }
                // ...or if it is the base interpreter the venv was created from.
                if interpreter
                    .sys_base_executable()
                    .is_some_and(|sys_base_executable| {
                        is_same_executable(sys_base_executable, file)
                    })
                {
                    return true;
                }
                // ...or, on Windows, if both interpreters have the same base executable. On
                // Windows, interpreters are copied rather than symlinked, so a virtual environment
                // created from within a virtual environment will _not_ evaluate to the same
                // `sys.executable`, but will have the same `sys._base_executable`.
                if cfg!(windows) {
                    if let Ok(file_interpreter) = Interpreter::query(file, cache) {
                        if let (Some(file_base), Some(interpreter_base)) = (
                            file_interpreter.sys_base_executable(),
                            interpreter.sys_base_executable(),
                        ) {
                            if is_same_executable(file_base, interpreter_base) {
                                return true;
                            }
                        }
                    }
                }
                false
            }
            Self::ExecutableName(name) => {
                // First, check the virtual environment executable.
                if interpreter
                    .sys_executable()
                    .file_name()
                    .is_some_and(|filename| filename == name.as_str())
                {
                    return true;
                }
                // Next, check the base interpreter without performing IO.
                if interpreter
                    .sys_base_executable()
                    .and_then(|executable| executable.file_name())
                    .is_some_and(|file_name| file_name == name.as_str())
                {
                    return true;
                }
                // Finally, check `PATH`. The discovered name can differ from the installed name.
                // For example, `foopython` can be installed as `python`.
                if which(name)
                    .ok()
                    .as_ref()
                    .and_then(|executable| executable.file_name())
                    .is_some_and(|file_name| file_name == name.as_str())
                {
                    return true;
                }
                false
            }
            Self::Implementation(implementation) => interpreter
                .implementation_name()
                .eq_ignore_ascii_case(implementation.long_name()),
            Self::ImplementationVersion(implementation, version) => {
                version.matches_interpreter(interpreter)
                    && interpreter
                        .implementation_name()
                        .eq_ignore_ascii_case(implementation.long_name())
            }
            Self::Key(request) => request.satisfied_by_interpreter(interpreter),
        }
    }

    /// Return `true` if this request allows a pre-release Python version.
    pub(crate) fn allows_prereleases(&self) -> bool {
        match self {
            Self::Default => false,
            Self::Any => true,
            Self::Version(version) => version.allows_prereleases(),
            Self::Directory(_) | Self::File(_) | Self::ExecutableName(_) => true,
            Self::Implementation(_) => false,
            Self::ImplementationVersion(_, _) => true,
            Self::Key(request) => request.allows_prereleases(),
        }
    }

    /// Return `true` if this request allows a debug Python version.
    fn allows_debug(&self) -> bool {
        match self {
            Self::Default => false,
            Self::Any => true,
            Self::Version(version) => version.is_debug(),
            Self::Directory(_) | Self::File(_) | Self::ExecutableName(_) => true,
            Self::Implementation(_) => false,
            Self::ImplementationVersion(_, _) => true,
            Self::Key(request) => request.allows_debug(),
        }
    }

    /// Return `true` if this request allows an alternative Python implementation, such as PyPy.
    fn allows_alternative_implementations(&self) -> bool {
        match self {
            Self::Default => false,
            Self::Any => true,
            Self::Version(_) => false,
            Self::Directory(_) | Self::File(_) | Self::ExecutableName(_) => true,
            Self::Implementation(implementation)
            | Self::ImplementationVersion(implementation, _) => {
                !matches!(implementation, ImplementationName::CPython)
            }
            Self::Key(request) => request.allows_alternative_implementations(),
        }
    }

    pub(crate) fn is_explicit_system(&self) -> bool {
        matches!(self, Self::File(_) | Self::Directory(_))
    }

    /// Convert the request to its canonical string.
    ///
    /// [`Self::parse`] must return the same request when it receives this string.
    pub fn to_canonical_string(&self) -> Cow<'_, str> {
        match self {
            Self::Any => Cow::Borrowed("any"),
            Self::Default => Cow::Borrowed("default"),
            Self::Version(version) => Cow::Owned(version.to_string()),
            Self::Directory(path) | Self::File(path) => path.to_string_lossy(),
            Self::ExecutableName(name) => Cow::Borrowed(name),
            Self::Implementation(implementation) => Cow::Borrowed(implementation.long_name()),
            Self::ImplementationVersion(implementation, version) => {
                Cow::Owned(format!("{implementation}@{version}"))
            }
            Self::Key(request) => Cow::Owned(request.to_string()),
        }
    }

    /// Convert this interpreter request into a concrete PEP 440 `Version`, when possible.
    ///
    /// Return `None` if the request does not specify an exact version.
    pub fn as_pep440_version(&self) -> Option<Version> {
        match self {
            Self::Version(v) | Self::ImplementationVersion(_, v) => v.as_pep440_version(),
            Self::Key(download_request) => download_request
                .version()
                .and_then(VersionRequest::as_pep440_version),
            Self::Default
            | Self::Any
            | Self::Directory(_)
            | Self::File(_)
            | Self::ExecutableName(_)
            | Self::Implementation(_) => None,
        }
    }

    /// Convert this interpreter request into [`VersionSpecifiers`] for compatible versions.
    ///
    /// Return `None` if the request has no version constraints, such as a path or executable name.
    fn as_version_specifiers(&self) -> Option<VersionSpecifiers> {
        match self {
            Self::Version(version) | Self::ImplementationVersion(_, version) => {
                version.as_version_specifiers()
            }
            Self::Key(download_request) => download_request
                .version()
                .and_then(VersionRequest::as_version_specifiers),
            Self::Default
            | Self::Any
            | Self::Directory(_)
            | Self::File(_)
            | Self::ExecutableName(_)
            | Self::Implementation(_) => None,
        }
    }

    /// Return `true` if this request is compatible with the `requires-python` specifier.
    ///
    /// Paths and executable names have no version constraints, so they are always compatible. A
    /// versioned request is compatible when its range overlaps the `requires-python` range.
    pub fn intersects_requires_python(&self, requires_python: &RequiresPython) -> bool {
        let Some(specifiers) = self.as_version_specifiers() else {
            return true;
        };

        let request_range = release_specifiers_to_ranges(specifiers);
        let requires_python_range =
            release_specifiers_to_ranges(requires_python.specifiers().clone());
        !request_range
            .intersection(&requires_python_range)
            .is_empty()
    }
}

impl PythonSource {
    pub fn is_managed(self) -> bool {
        matches!(self, Self::Managed)
    }

    /// Return `true` if this source allows pre-release Python without explicit selection.
    fn allows_prereleases(self) -> bool {
        match self {
            Self::Managed | Self::Registry | Self::MicrosoftStore => false,
            Self::SearchPath
            | Self::SearchPathFirst
            | Self::CondaPrefix
            | Self::BaseCondaPrefix
            | Self::ProvidedPath
            | Self::ParentInterpreter
            | Self::ActiveEnvironment
            | Self::DiscoveredEnvironment => true,
        }
    }

    /// Return `true` if this source allows debug Python without explicit selection.
    fn allows_debug(self) -> bool {
        match self {
            Self::Managed | Self::Registry | Self::MicrosoftStore => false,
            Self::SearchPath
            | Self::SearchPathFirst
            | Self::CondaPrefix
            | Self::BaseCondaPrefix
            | Self::ProvidedPath
            | Self::ParentInterpreter
            | Self::ActiveEnvironment
            | Self::DiscoveredEnvironment => true,
        }
    }

    /// Return `true` if this source allows alternative implementations without explicit selection.
    fn allows_alternative_implementations(self) -> bool {
        match self {
            Self::Managed
            | Self::Registry
            | Self::SearchPath
            // TODO(zanieb): Consider allowing this while preserving existing behavior.
            | Self::SearchPathFirst
            | Self::MicrosoftStore => false,
            Self::CondaPrefix
            | Self::BaseCondaPrefix
            | Self::ProvidedPath
            | Self::ParentInterpreter
            | Self::ActiveEnvironment
            | Self::DiscoveredEnvironment => true,
        }
    }

    /// Return `true` if this source could be a virtual environment.
    ///
    /// Exclude [`PythonSource::SearchPath`] to avoid querying every system interpreter. A later
    /// `PATH` entry can belong to a virtual environment, but uv does not select it automatically.
    ///
    /// Check the first `PATH` executable through [`PythonSource::SearchPathFirst`]. This lets a
    /// virtual environment work when its `bin/` directory is first in `PATH`, even without
    /// `VIRTUAL_ENV`. If another interpreter appears first, ignore the environment.
    fn is_maybe_virtualenv(self) -> bool {
        match self {
            Self::ProvidedPath
            | Self::ActiveEnvironment
            | Self::DiscoveredEnvironment
            | Self::CondaPrefix
            | Self::BaseCondaPrefix
            | Self::ParentInterpreter
            | Self::SearchPathFirst => true,
            Self::Managed | Self::SearchPath | Self::Registry | Self::MicrosoftStore => false,
        }
    }

    /// Return `true` if the user explicitly selected this source.
    ///
    /// Explicit sources include provided paths and active virtual environments.
    fn is_explicit(self) -> bool {
        match self {
            Self::ProvidedPath
            | Self::ParentInterpreter
            | Self::ActiveEnvironment
            | Self::CondaPrefix => true,
            Self::Managed
            | Self::DiscoveredEnvironment
            | Self::SearchPath
            | Self::SearchPathFirst
            | Self::Registry
            | Self::MicrosoftStore
            | Self::BaseCondaPrefix => false,
        }
    }

    /// Return `true` if this source could be a system interpreter.
    fn is_maybe_system(self) -> bool {
        match self {
            Self::CondaPrefix
            | Self::BaseCondaPrefix
            | Self::ParentInterpreter
            | Self::ProvidedPath
            | Self::Managed
            | Self::SearchPath
            | Self::SearchPathFirst
            | Self::Registry
            | Self::MicrosoftStore => true,
            Self::ActiveEnvironment | Self::DiscoveredEnvironment => false,
        }
    }
}

impl PythonPreference {
    fn allows_source(self, source: PythonSource) -> bool {
        // Ignore the preference for sources that are not system interpreter sources.
        if !matches!(
            source,
            PythonSource::Managed | PythonSource::SearchPath | PythonSource::Registry
        ) {
            return true;
        }

        match self {
            Self::OnlyManaged => matches!(source, PythonSource::Managed),
            Self::Managed | Self::System => matches!(
                source,
                PythonSource::Managed | PythonSource::SearchPath | PythonSource::Registry
            ),
            Self::OnlySystem => {
                matches!(source, PythonSource::SearchPath | PythonSource::Registry)
            }
        }
    }

    pub(crate) fn allows_managed(self) -> bool {
        match self {
            Self::OnlySystem => false,
            Self::Managed | Self::System | Self::OnlyManaged => true,
        }
    }

    /// Return `true` if this preference allows the interpreter.
    ///
    /// [`PythonPreference::allows_source`] checks the [`PythonSource`]. This method checks whether
    /// the base prefix is in a managed directory.
    fn allows_interpreter(self, interpreter: &Interpreter) -> bool {
        match self {
            Self::OnlyManaged => interpreter.is_managed(),
            Self::OnlySystem => !interpreter.is_managed(),
            Self::Managed | Self::System => true,
        }
    }

    /// Return `true` if this preference allows the installation.
    ///
    /// Always allow explicit sources, such as provided paths and active environments. They can
    /// conflict with the preference. Do not invalidate an environment because the preference might
    /// come from a persistent configuration file instead of an explicit request.
    pub fn allows_installation(self, installation: &PythonInstallation) -> bool {
        let source = installation.source;
        let interpreter = &installation.interpreter;

        match self {
            Self::OnlyManaged => {
                if self.allows_interpreter(interpreter) {
                    true
                } else if source.is_explicit() {
                    debug!(
                        "Allowing unmanaged Python interpreter at `{}` (in conflict with the `python-preference`) since it is from source: {source}",
                        interpreter.sys_executable().display()
                    );
                    true
                } else {
                    debug!(
                        "Ignoring Python interpreter at `{}`: only managed interpreters allowed",
                        interpreter.sys_executable().display()
                    );
                    false
                }
            }
            // A non-exclusive preference allows any interpreter.
            Self::Managed | Self::System => true,
            Self::OnlySystem => {
                if self.allows_interpreter(interpreter) {
                    true
                } else if source.is_explicit() {
                    debug!(
                        "Allowing managed Python interpreter at `{}` (in conflict with the `python-preference`) since it is from source: {source}",
                        interpreter.sys_executable().display()
                    );
                    true
                } else {
                    debug!(
                        "Ignoring Python interpreter at `{}`: only system interpreters allowed",
                        interpreter.sys_executable().display()
                    );
                    false
                }
            }
        }
    }

    /// Return the preference selected by the `--system` flag.
    ///
    /// Convert [`PythonPreference::Managed`] to [`PythonPreference::System`] when `system` is set.
    #[must_use]
    pub fn with_system_flag(self, system: bool) -> Self {
        match self {
            // TODO(zanieb): Decide whether `--system` can override `--managed-python`. An
            // `Option<PythonPreference>` could distinguish explicit values from defaults.
            Self::OnlyManaged => self,
            Self::Managed => {
                if system {
                    Self::System
                } else {
                    self
                }
            }
            Self::System => self,
            Self::OnlySystem => self,
        }
    }
}

impl PythonDownloads {
    pub fn is_automatic(self) -> bool {
        matches!(self, Self::Automatic)
    }
}

impl EnvironmentPreference {
    pub fn from_system_flag(system: bool, mutable: bool) -> Self {
        match (system, mutable) {
            // Ignore virtual environments when `--system` is set.
            (true, _) => Self::OnlySystem,
            // Allow system environments for mutable operations only when explicitly selected.
            (false, true) => Self::ExplicitSystem,
            // Allow system environments for immutable operations.
            (false, false) => Self::Any,
        }
    }

    /// Return `true` if this preference allows the installation.
    ///
    /// [`source_satisfies_environment_preference`] only checks whether a [`PythonSource`] could
    /// match. Query the interpreter to confirm whether it belongs to a virtual environment.
    pub(crate) fn allows_installation(self, installation: &PythonInstallation) -> bool {
        interpreter_satisfies_environment_preference(
            installation.source,
            &installation.interpreter,
            self,
        )
    }
}

#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub(crate) struct ExecutableName {
    implementation: Option<ImplementationName>,
    major: Option<u8>,
    minor: Option<u8>,
    patch: Option<u8>,
    prerelease: Option<Prerelease>,
    variant: PythonVariant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutableNameComparator<'a> {
    name: ExecutableName,
    request: &'a VersionRequest,
    implementation: Option<&'a ImplementationName>,
}

impl Ord for ExecutableNameComparator<'_> {
    /// Compare executable names in reverse priority order.
    ///
    /// Higher-priority names compare as `Greater`.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Prefer the default name unless the request specifies an implementation.
        let name_ordering = if self.implementation.is_some() {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Less
        };
        if self.name.implementation.is_none() && other.name.implementation.is_some() {
            return name_ordering.reverse();
        }
        if self.name.implementation.is_some() && other.name.implementation.is_none() {
            return name_ordering;
        }
        // Otherwise, use the supported implementation order.
        let ordering = self.name.implementation.cmp(&other.name.implementation);
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
        let ordering = self.name.major.cmp(&other.name.major);
        let is_default_request =
            matches!(self.request, VersionRequest::Any | VersionRequest::Default);
        if ordering != std::cmp::Ordering::Equal {
            return if is_default_request {
                ordering.reverse()
            } else {
                ordering
            };
        }
        let ordering = self.name.minor.cmp(&other.name.minor);
        if ordering != std::cmp::Ordering::Equal {
            return if is_default_request {
                ordering.reverse()
            } else {
                ordering
            };
        }
        let ordering = self.name.patch.cmp(&other.name.patch);
        if ordering != std::cmp::Ordering::Equal {
            return if is_default_request {
                ordering.reverse()
            } else {
                ordering
            };
        }
        let ordering = self.name.prerelease.cmp(&other.name.prerelease);
        if ordering != std::cmp::Ordering::Equal {
            return if is_default_request {
                ordering.reverse()
            } else {
                ordering
            };
        }
        let ordering = self.name.variant.cmp(&other.name.variant);
        if ordering != std::cmp::Ordering::Equal {
            return if is_default_request {
                ordering.reverse()
            } else {
                ordering
            };
        }
        ordering
    }
}

impl PartialOrd for ExecutableNameComparator<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl ExecutableName {
    #[must_use]
    fn with_implementation(mut self, implementation: ImplementationName) -> Self {
        self.implementation = Some(implementation);
        self
    }

    #[must_use]
    fn with_major(mut self, major: u8) -> Self {
        self.major = Some(major);
        self
    }

    #[must_use]
    fn with_minor(mut self, minor: u8) -> Self {
        self.minor = Some(minor);
        self
    }

    #[must_use]
    fn with_patch(mut self, patch: u8) -> Self {
        self.patch = Some(patch);
        self
    }

    #[must_use]
    fn with_prerelease(mut self, prerelease: Prerelease) -> Self {
        self.prerelease = Some(prerelease);
        self
    }

    #[must_use]
    fn with_variant(mut self, variant: PythonVariant) -> Self {
        self.variant = variant;
        self
    }

    fn into_comparator<'a>(
        self,
        request: &'a VersionRequest,
        implementation: Option<&'a ImplementationName>,
    ) -> ExecutableNameComparator<'a> {
        ExecutableNameComparator {
            name: self,
            request,
            implementation,
        }
    }
}

impl fmt::Display for ExecutableName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some(implementation) = self.implementation {
            write!(f, "{implementation}")?;
        } else {
            f.write_str("python")?;
        }
        if let Some(major) = self.major {
            write!(f, "{major}")?;
            if let Some(minor) = self.minor {
                write!(f, ".{minor}")?;
                if let Some(patch) = self.patch {
                    write!(f, ".{patch}")?;
                }
            }
        }
        if let Some(prerelease) = &self.prerelease {
            write!(f, "{prerelease}")?;
        }
        f.write_str(self.variant.executable_suffix())?;
        f.write_str(EXE_SUFFIX)?;
        Ok(())
    }
}

impl VersionRequest {
    /// Create a [`VersionRequest`] from [`VersionSpecifiers`].
    ///
    /// Parse one `==` constraint as a concrete version request, such as `MajorMinorPatch`. Parse
    /// other constraints as a range.
    pub fn from_specifiers(specifiers: VersionSpecifiers, variant: PythonVariant) -> Self {
        if let [specifier] = specifiers.iter().as_slice()
            && specifier.operator() == &uv_pep440::Operator::Equal
            && let Ok(request) = Self::from_str(&specifier.version().to_string())
        {
            return request;
        }
        Self::Range(specifiers, variant)
    }

    /// Remove patch and pre-release information from the version request.
    #[must_use]
    pub fn only_minor(self) -> Self {
        match self {
            Self::Any => self,
            Self::Default => self,
            Self::Range(specifiers, variant) => Self::Range(
                specifiers
                    .into_iter()
                    .map(|s| s.only_minor_release())
                    .collect(),
                variant,
            ),
            Self::Major(..) => self,
            Self::MajorMinor(..) => self,
            Self::MajorMinorPatch(major, minor, _, variant)
            | Self::MajorMinorPrerelease(major, minor, _, variant)
            | Self::MajorMinorPatchPrerelease(major, minor, _, _, variant) => {
                Self::MajorMinor(major, minor, variant)
            }
        }
    }

    /// Return possible executable names for this version request.
    pub(crate) fn executable_names(
        &self,
        implementation: Option<&ImplementationName>,
    ) -> Vec<ExecutableName> {
        let prerelease = match self {
            Self::MajorMinorPrerelease(_, _, prerelease, _)
            | Self::MajorMinorPatchPrerelease(_, _, _, prerelease, _) => {
                // Include the pre-release version, such as `python3.8a`.
                Some(prerelease)
            }
            _ => None,
        };

        // Add the default executable name.
        let mut names = Vec::new();
        names.push(ExecutableName::default());

        // Add names for each available version component.
        if let Some(major) = self.major() {
            // For example, `python3`.
            names.push(ExecutableName::default().with_major(major));
            if let Some(minor) = self.minor() {
                // For example, `python3.12`.
                names.push(
                    ExecutableName::default()
                        .with_major(major)
                        .with_minor(minor),
                );
                if let Some(patch) = self.patch() {
                    // For example, `python3.12.1`.
                    names.push(
                        ExecutableName::default()
                            .with_major(major)
                            .with_minor(minor)
                            .with_patch(patch),
                    );
                }
            }
        } else {
            // Include Python 3 by default, such as `python3`.
            names.push(ExecutableName::default().with_major(3));
        }

        if let Some(prerelease) = prerelease {
            // Include the pre-release version, such as `python3.8a`.
            for i in 0..names.len() {
                let name = names[i];
                if name.minor.is_none() {
                    // Do not add a pre-release marker without a minor version.
                    // Names such as `pythonrc1` and `python3rc1` are invalid.
                    continue;
                }
                names.push(name.with_prerelease(*prerelease));
            }
        }

        // Add all implementation-specific names.
        if let Some(implementation) = implementation {
            for i in 0..names.len() {
                let name = names[i].with_implementation(*implementation);
                names.push(name);
            }
        } else {
            // Include every name when the request allows all implementations.
            if matches!(self, Self::Any) {
                for i in 0..names.len() {
                    for implementation in ImplementationName::iter_all() {
                        let name = names[i].with_implementation(implementation);
                        names.push(name);
                    }
                }
            }
        }

        // Include free-threaded variants.
        if let Some(variant) = self.variant()
            && variant != PythonVariant::Default
        {
            for i in 0..names.len() {
                let name = names[i].with_variant(variant);
                names.push(name);
            }
        }

        names.sort_unstable_by_key(|name| name.into_comparator(self, implementation));
        names.reverse();

        names
    }

    /// Return the major version segment of the request, if any.
    fn major(&self) -> Option<u8> {
        match self {
            Self::Any | Self::Default | Self::Range(_, _) => None,
            Self::Major(major, _) => Some(*major),
            Self::MajorMinor(major, _, _) => Some(*major),
            Self::MajorMinorPatch(major, _, _, _) => Some(*major),
            Self::MajorMinorPrerelease(major, _, _, _) => Some(*major),
            Self::MajorMinorPatchPrerelease(major, _, _, _, _) => Some(*major),
        }
    }

    /// Return the minor version segment of the request, if any.
    fn minor(&self) -> Option<u8> {
        match self {
            Self::Any | Self::Default | Self::Range(_, _) => None,
            Self::Major(_, _) => None,
            Self::MajorMinor(_, minor, _) => Some(*minor),
            Self::MajorMinorPatch(_, minor, _, _) => Some(*minor),
            Self::MajorMinorPrerelease(_, minor, _, _) => Some(*minor),
            Self::MajorMinorPatchPrerelease(_, minor, _, _, _) => Some(*minor),
        }
    }

    /// Return the patch version segment of the request, if any.
    fn patch(&self) -> Option<u8> {
        match self {
            Self::Any | Self::Default | Self::Range(_, _) => None,
            Self::Major(_, _) => None,
            Self::MajorMinor(_, _, _) => None,
            Self::MajorMinorPatch(_, _, patch, _) => Some(*patch),
            Self::MajorMinorPrerelease(_, _, _, _) => None,
            Self::MajorMinorPatchPrerelease(_, _, patch, _, _) => Some(*patch),
        }
    }

    /// Return the pre-release segment of the request, if any.
    fn prerelease(&self) -> Option<&Prerelease> {
        match self {
            Self::Any | Self::Default | Self::Range(_, _) => None,
            Self::Major(_, _) => None,
            Self::MajorMinor(_, _, _) => None,
            Self::MajorMinorPatch(_, _, _, _) => None,
            Self::MajorMinorPrerelease(_, _, prerelease, _) => Some(prerelease),
            Self::MajorMinorPatchPrerelease(_, _, _, prerelease, _) => Some(prerelease),
        }
    }

    /// Check whether uv supports the requested version.
    ///
    /// Return `Err` with an explanation if the version is unsupported.
    fn check_supported(&self) -> Result<(), String> {
        match self {
            Self::Any | Self::Default => (),
            Self::Major(major, _) => {
                if *major < 3 {
                    return Err(format!(
                        "Python <3 is not supported but {major} was requested."
                    ));
                }
            }
            Self::MajorMinor(major, minor, _) => {
                if (*major, *minor) < (3, 6) {
                    return Err(format!(
                        "Python <3.6 is not supported but {major}.{minor} was requested."
                    ));
                }
            }
            Self::MajorMinorPatch(major, minor, patch, _) => {
                if (*major, *minor) < (3, 6) {
                    return Err(format!(
                        "Python <3.6 is not supported but {major}.{minor}.{patch} was requested."
                    ));
                }
            }
            Self::MajorMinorPrerelease(major, minor, prerelease, _) => {
                if (*major, *minor) < (3, 6) {
                    return Err(format!(
                        "Python <3.6 is not supported but {major}.{minor}{prerelease} was requested."
                    ));
                }
            }
            Self::MajorMinorPatchPrerelease(major, minor, patch, prerelease, _) => {
                if (*major, *minor) < (3, 6) {
                    return Err(format!(
                        "Python <3.6 is not supported but {major}.{minor}.{patch}{prerelease} was requested."
                    ));
                }
            }
            // TODO(zanieb): Check whether this version range can be satisfied.
            Self::Range(_, _) => (),
        }

        if self.is_freethreaded()
            && let Self::MajorMinor(major, minor, _) = self.clone().without_patch()
            && (major, minor) < (3, 13)
        {
            return Err(format!(
                "Python <3.13 does not support free-threading but {self} was requested."
            ));
        }

        Ok(())
    }

    /// Adjust this request for the specified [`PythonSource`].
    ///
    /// Convert [`VersionRequest::Default`] to [`VersionRequest::Any`] for sources that allow
    /// non-default interpreters, such as free-threaded variants.
    #[must_use]
    fn into_request_for_source(self, source: PythonSource) -> Self {
        match self {
            Self::Default => match source {
                PythonSource::ParentInterpreter
                | PythonSource::CondaPrefix
                | PythonSource::BaseCondaPrefix
                | PythonSource::ProvidedPath
                | PythonSource::DiscoveredEnvironment
                | PythonSource::ActiveEnvironment => Self::Any,
                PythonSource::SearchPath
                | PythonSource::SearchPathFirst
                | PythonSource::Registry
                | PythonSource::MicrosoftStore
                | PythonSource::Managed => Self::Default,
            },
            _ => self,
        }
    }

    /// Check whether an installation matches this request after adjusting it for the source.
    pub(crate) fn matches_installation(&self, installation: &PythonInstallation) -> bool {
        let request = self.clone().into_request_for_source(installation.source);
        request.matches_interpreter(&installation.interpreter)
    }

    /// Check whether an interpreter matches this request.
    pub(crate) fn matches_interpreter(&self, interpreter: &Interpreter) -> bool {
        match self {
            Self::Any => true,
            // Do not use free-threaded interpreters by default.
            Self::Default => PythonVariant::Default.matches_interpreter(interpreter),
            Self::Major(major, variant) => {
                interpreter.python_major() == *major && variant.matches_interpreter(interpreter)
            }
            Self::MajorMinor(major, minor, variant) => {
                (interpreter.python_major(), interpreter.python_minor()) == (*major, *minor)
                    && variant.matches_interpreter(interpreter)
            }
            Self::MajorMinorPatch(major, minor, patch, variant) => {
                (
                    interpreter.python_major(),
                    interpreter.python_minor(),
                    interpreter.python_patch(),
                ) == (*major, *minor, *patch)
                    // A patch version requests a stable release.
                    && interpreter.python_version().pre().is_none()
                    && variant.matches_interpreter(interpreter)
            }
            Self::Range(specifiers, variant) => {
                // If the specifier contains pre-releases, use the full version for comparison.
                // Otherwise, strip pre-release so that, e.g., `>=3.14` matches `3.14.0rc3`.
                let version = if specifiers
                    .iter()
                    .any(uv_pep440::VersionSpecifier::any_prerelease)
                {
                    Cow::Borrowed(interpreter.python_version())
                } else {
                    Cow::Owned(interpreter.python_version().only_release())
                };
                specifiers.contains(&version) && variant.matches_interpreter(interpreter)
            }
            Self::MajorMinorPrerelease(major, minor, prerelease, variant) => {
                let version = interpreter.python_version();
                let Some(interpreter_prerelease) = version.pre() else {
                    return false;
                };
                (
                    interpreter.python_major(),
                    interpreter.python_minor(),
                    interpreter_prerelease,
                ) == (*major, *minor, *prerelease)
                    && variant.matches_interpreter(interpreter)
            }
            Self::MajorMinorPatchPrerelease(major, minor, patch, prerelease, variant) => {
                let version = interpreter.python_version();
                let Some(interpreter_prerelease) = version.pre() else {
                    return false;
                };
                (
                    interpreter.python_major(),
                    interpreter.python_minor(),
                    interpreter.python_patch(),
                    interpreter_prerelease,
                ) == (*major, *minor, *patch, *prerelease)
                    && variant.matches_interpreter(interpreter)
            }
        }
    }

    /// Check whether a version is compatible with this request.
    ///
    /// WARNING: Also use [`VersionRequest::matches_interpreter`]. Use this method only to skip
    /// interpreters that cannot satisfy the request.
    fn matches_version(&self, version: &PythonVersion) -> bool {
        match self {
            Self::Any | Self::Default => true,
            Self::Major(major, _) => version.major() == *major,
            Self::MajorMinor(major, minor, _) => {
                (version.major(), version.minor()) == (*major, *minor)
            }
            Self::MajorMinorPatch(major, minor, patch, _) => {
                (version.major(), version.minor(), version.patch())
                    == (*major, *minor, Some(*patch))
            }
            Self::Range(specifiers, _) => {
                // If the specifier contains pre-releases, use the full version for comparison.
                // Otherwise, strip pre-release so that, e.g., `>=3.14` matches `3.14.0rc3`.
                let version = if specifiers
                    .iter()
                    .any(uv_pep440::VersionSpecifier::any_prerelease)
                {
                    Cow::Borrowed(&version.version)
                } else {
                    Cow::Owned(version.version.only_release())
                };
                specifiers.contains(&version)
            }
            Self::MajorMinorPrerelease(major, minor, prerelease, _) => {
                (version.major(), version.minor(), version.pre())
                    == (*major, *minor, Some(*prerelease))
            }
            Self::MajorMinorPatchPrerelease(major, minor, patch, prerelease, _) => {
                (
                    version.major(),
                    version.minor(),
                    version.patch(),
                    version.pre(),
                ) == (*major, *minor, Some(*patch), Some(*prerelease))
            }
        }
    }

    /// Check whether major and minor version components match this request.
    ///
    /// WARNING: Also use [`VersionRequest::matches_interpreter`]. Use this method only to skip
    /// interpreters that cannot satisfy the request.
    fn matches_major_minor(&self, major: u8, minor: u8) -> bool {
        match self {
            Self::Any | Self::Default => true,
            Self::Major(self_major, _) => *self_major == major,
            Self::MajorMinor(self_major, self_minor, _) => {
                (*self_major, *self_minor) == (major, minor)
            }
            Self::MajorMinorPatch(self_major, self_minor, _, _) => {
                (*self_major, *self_minor) == (major, minor)
            }
            Self::Range(specifiers, _) => {
                let range = release_specifiers_to_ranges(specifiers.clone());
                let Some((lower, upper)) = range.bounding_range() else {
                    return true;
                };
                let version = Version::new([u64::from(major), u64::from(minor)]);

                let lower = LowerBound::new(lower.cloned());
                if !lower.major_minor().contains(&version) {
                    return false;
                }

                let upper = UpperBound::new(upper.cloned());
                if !upper.major_minor().contains(&version) {
                    return false;
                }

                true
            }
            Self::MajorMinorPrerelease(self_major, self_minor, _, _) => {
                (*self_major, *self_minor) == (major, minor)
            }
            Self::MajorMinorPatchPrerelease(self_major, self_minor, _, _, _) => {
                (*self_major, *self_minor) == (major, minor)
            }
        }
    }

    /// Check whether major, minor, patch, and pre-release components match this request.
    ///
    /// WARNING: Also use [`VersionRequest::matches_interpreter`]. Use this method only to skip
    /// interpreters that cannot satisfy the request.
    pub(crate) fn matches_major_minor_patch_prerelease(
        &self,
        major: u8,
        minor: u8,
        patch: u8,
        prerelease: Option<Prerelease>,
    ) -> bool {
        match self {
            Self::Any | Self::Default => true,
            Self::Major(self_major, _) => *self_major == major,
            Self::MajorMinor(self_major, self_minor, _) => {
                (*self_major, *self_minor) == (major, minor)
            }
            Self::MajorMinorPatch(self_major, self_minor, self_patch, _) => {
                (*self_major, *self_minor, *self_patch) == (major, minor, patch)
                    // A patch version requests a stable release.
                    && prerelease.is_none()
            }
            Self::Range(specifiers, _) => specifiers.contains(
                &Version::new([u64::from(major), u64::from(minor), u64::from(patch)])
                    .with_pre(prerelease),
            ),
            Self::MajorMinorPrerelease(self_major, self_minor, self_prerelease, _) => {
                // A pre-release without a patch matches patch version zero.
                (*self_major, *self_minor, 0, Some(*self_prerelease))
                    == (major, minor, patch, prerelease)
            }
            Self::MajorMinorPatchPrerelease(
                self_major,
                self_minor,
                self_patch,
                self_prerelease,
                _,
            ) => {
                (
                    *self_major,
                    *self_minor,
                    *self_patch,
                    Some(*self_prerelease),
                ) == (major, minor, patch, prerelease)
            }
        }
    }

    /// Check whether a [`PythonInstallationKey`] matches this request.
    ///
    /// WARNING: Also use [`VersionRequest::matches_interpreter`]. Use this method only to skip
    /// interpreters that cannot satisfy the request.
    pub(crate) fn matches_installation_key(&self, key: &PythonInstallationKey) -> bool {
        self.matches_major_minor_patch_prerelease(key.major, key.minor, key.patch, key.prerelease())
    }

    /// Return `true` if the request includes a patch version.
    fn has_patch(&self) -> bool {
        match self {
            Self::Any | Self::Default => false,
            Self::Major(..) => false,
            Self::MajorMinor(..) => false,
            Self::MajorMinorPatch(..) => true,
            Self::MajorMinorPrerelease(..) => false,
            Self::MajorMinorPatchPrerelease(..) => true,
            Self::Range(_, _) => false,
        }
    }

    /// Return a [`VersionRequest`] without its patch version, when possible.
    ///
    /// Return the original request if it has no patch version.
    #[must_use]
    fn without_patch(self) -> Self {
        match self {
            Self::Default => Self::Default,
            Self::Any => Self::Any,
            Self::Major(major, variant) => Self::Major(major, variant),
            Self::MajorMinor(major, minor, variant) => Self::MajorMinor(major, minor, variant),
            Self::MajorMinorPatch(major, minor, _, variant) => {
                Self::MajorMinor(major, minor, variant)
            }
            Self::MajorMinorPrerelease(major, minor, prerelease, variant) => {
                Self::MajorMinorPrerelease(major, minor, prerelease, variant)
            }
            Self::MajorMinorPatchPrerelease(major, minor, _, prerelease, variant) => {
                Self::MajorMinorPrerelease(major, minor, prerelease, variant)
            }
            Self::Range(_, _) => self,
        }
    }

    /// Return `true` if this request allows pre-release versions.
    pub(crate) fn allows_prereleases(&self) -> bool {
        match self {
            Self::Default => false,
            Self::Any => true,
            Self::Major(..) => false,
            Self::MajorMinor(..) => false,
            Self::MajorMinorPatch(..) => false,
            Self::MajorMinorPrerelease(..) => true,
            Self::MajorMinorPatchPrerelease(..) => true,
            Self::Range(specifiers, _) => specifiers.iter().any(VersionSpecifier::any_prerelease),
        }
    }

    /// Return `true` if this request is for a debug Python variant.
    pub(crate) fn is_debug(&self) -> bool {
        match self {
            Self::Any | Self::Default => false,
            Self::Major(_, variant)
            | Self::MajorMinor(_, _, variant)
            | Self::MajorMinorPatch(_, _, _, variant)
            | Self::MajorMinorPrerelease(_, _, _, variant)
            | Self::MajorMinorPatchPrerelease(_, _, _, _, variant)
            | Self::Range(_, variant) => variant.is_debug(),
        }
    }

    /// Return `true` if this request is for a free-threaded Python variant.
    fn is_freethreaded(&self) -> bool {
        match self {
            Self::Any | Self::Default => false,
            Self::Major(_, variant)
            | Self::MajorMinor(_, _, variant)
            | Self::MajorMinorPatch(_, _, _, variant)
            | Self::MajorMinorPrerelease(_, _, _, variant)
            | Self::MajorMinorPatchPrerelease(_, _, _, _, variant)
            | Self::Range(_, variant) => variant.is_freethreaded(),
        }
    }

    /// Return the [`PythonVariant`] of the request, if any.
    pub(crate) fn variant(&self) -> Option<PythonVariant> {
        match self {
            Self::Any => None,
            Self::Default => Some(PythonVariant::Default),
            Self::Major(_, variant)
            | Self::MajorMinor(_, _, variant)
            | Self::MajorMinorPatch(_, _, _, variant)
            | Self::MajorMinorPrerelease(_, _, _, variant)
            | Self::MajorMinorPatchPrerelease(_, _, _, _, variant)
            | Self::Range(_, variant) => Some(*variant),
        }
    }

    /// Convert this request into a concrete PEP 440 `Version`, when possible.
    ///
    /// Return `None` for requests without a concrete version.
    fn as_pep440_version(&self) -> Option<Version> {
        match self {
            Self::Default | Self::Any | Self::Range(_, _) => None,
            Self::Major(major, _) => Some(Version::new([u64::from(*major)])),
            Self::MajorMinor(major, minor, _) => {
                Some(Version::new([u64::from(*major), u64::from(*minor)]))
            }
            Self::MajorMinorPatch(major, minor, patch, _) => Some(Version::new([
                u64::from(*major),
                u64::from(*minor),
                u64::from(*patch),
            ])),
            // A pre-release without a patch uses patch version zero.
            Self::MajorMinorPrerelease(major, minor, prerelease, _) => Some(
                Version::new([u64::from(*major), u64::from(*minor), 0]).with_pre(Some(*prerelease)),
            ),
            Self::MajorMinorPatchPrerelease(major, minor, patch, prerelease, _) => Some(
                Version::new([u64::from(*major), u64::from(*minor), u64::from(*patch)])
                    .with_pre(Some(*prerelease)),
            ),
        }
    }

    /// Convert this request into [`VersionSpecifiers`] for compatible versions.
    ///
    /// Return `None` for requests without version constraints, such as [`VersionRequest::Default`]
    /// and [`VersionRequest::Any`].
    fn as_version_specifiers(&self) -> Option<VersionSpecifiers> {
        match self {
            Self::Default | Self::Any => None,
            Self::Major(major, _) => Some(VersionSpecifiers::from(
                VersionSpecifier::equals_star_version(Version::new([u64::from(*major)])),
            )),
            Self::MajorMinor(major, minor, _) => Some(VersionSpecifiers::from(
                VersionSpecifier::equals_star_version(Version::new([
                    u64::from(*major),
                    u64::from(*minor),
                ])),
            )),
            Self::MajorMinorPatch(major, minor, patch, _) => {
                Some(VersionSpecifiers::from(VersionSpecifier::equals_version(
                    Version::new([u64::from(*major), u64::from(*minor), u64::from(*patch)]),
                )))
            }
            Self::MajorMinorPrerelease(major, minor, prerelease, _) => {
                Some(VersionSpecifiers::from(VersionSpecifier::equals_version(
                    Version::new([u64::from(*major), u64::from(*minor), 0])
                        .with_pre(Some(*prerelease)),
                )))
            }
            Self::MajorMinorPatchPrerelease(major, minor, patch, prerelease, _) => {
                Some(VersionSpecifiers::from(VersionSpecifier::equals_version(
                    Version::new([u64::from(*major), u64::from(*minor), u64::from(*patch)])
                        .with_pre(Some(*prerelease)),
                )))
            }
            Self::Range(specifiers, _) => Some(specifiers.clone()),
        }
    }
}

impl FromStr for VersionRequest {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        /// Extract the variant from a version request. Return the prefix and variant type.
        fn parse_variant(s: &str) -> Result<(&str, PythonVariant), Error> {
            // Return an error because letters alone are not a valid version.
            if s.chars().all(char::is_alphabetic) {
                return Err(Error::InvalidVersionRequest(s.to_string()));
            }

            let Some(mut start) = s.rfind(|c: char| c.is_ascii_digit()) else {
                return Ok((s, PythonVariant::Default));
            };

            // Advance past the first digit.
            start += 1;

            // Check that the index is within bounds.
            if start + 1 > s.len() {
                return Ok((s, PythonVariant::Default));
            }

            let variant = &s[start..];
            let prefix = &s[..start];

            // Remove a leading `+`, if present.
            let variant = variant.strip_prefix('+').unwrap_or(variant);

            // TODO(zanieb): Return a specific error when `dt` is used instead of `td`.

            // Let [`Version::from_str`] reject an invalid variant.
            let Ok(variant) = PythonVariant::from_str(variant) else {
                return Ok((s, PythonVariant::Default));
            };

            Ok((prefix, variant))
        }

        let (s, variant) = parse_variant(s)?;
        let Ok(version) = Version::from_str(s) else {
            return parse_version_specifiers_request(s, variant);
        };

        // Split a wheel-tag release such as `38` into separate version components.
        let version = split_wheel_tag_release_version(version);

        // Reject post-release and development versions.
        if version.post().is_some() || version.dev().is_some() {
            return Err(Error::InvalidVersionRequest(s.to_string()));
        }

        // Reject local version suffixes. Supported variant suffixes were already removed.
        if !version.local().is_empty() {
            return Err(Error::InvalidVersionRequest(s.to_string()));
        }

        // Convert release components to the `u8` values used by `VersionRequest`.
        let Ok(release) = try_into_u8_slice(&version.release()) else {
            return Err(Error::InvalidVersionRequest(s.to_string()));
        };

        let prerelease = version.pre();

        match release.as_slice() {
            // For example, `3`.
            [major] => {
                // Reject pre-releases without a minor version, such as `3rc1`.
                if prerelease.is_some() {
                    return Err(Error::InvalidVersionRequest(s.to_string()));
                }
                Ok(Self::Major(*major, variant))
            }
            // For example, `3.12`, `312`, or `3.13rc1`.
            [major, minor] => {
                if let Some(prerelease) = prerelease {
                    return Ok(Self::MajorMinorPrerelease(
                        *major, *minor, prerelease, variant,
                    ));
                }
                Ok(Self::MajorMinor(*major, *minor, variant))
            }
            // For example, `3.12.1`, `3.13.0rc1`, or `3.14.5rc1`.
            [major, minor, patch] => {
                if let Some(prerelease) = prerelease {
                    if *patch == 0 {
                        return Ok(Self::MajorMinorPrerelease(
                            *major, *minor, prerelease, variant,
                        ));
                    }
                    return Ok(Self::MajorMinorPatchPrerelease(
                        *major, *minor, *patch, prerelease, variant,
                    ));
                }
                Ok(Self::MajorMinorPatch(*major, *minor, *patch, variant))
            }
            _ => Err(Error::InvalidVersionRequest(s.to_string())),
        }
    }
}

impl FromStr for PythonVariant {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "t" | "freethreaded" => Ok(Self::Freethreaded),
            "d" | "debug" => Ok(Self::Debug),
            "td" | "freethreaded+debug" => Ok(Self::FreethreadedDebug),
            "gil" => Ok(Self::Gil),
            "gil+debug" => Ok(Self::GilDebug),
            "" => Ok(Self::Default),
            _ => Err(()),
        }
    }
}

impl fmt::Display for PythonVariant {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::Debug => f.write_str("debug"),
            Self::Freethreaded => f.write_str("freethreaded"),
            Self::FreethreadedDebug => f.write_str("freethreaded+debug"),
            Self::Gil => f.write_str("gil"),
            Self::GilDebug => f.write_str("gil+debug"),
        }
    }
}

fn parse_version_specifiers_request(
    s: &str,
    variant: PythonVariant,
) -> Result<VersionRequest, Error> {
    let Ok(specifiers) = VersionSpecifiers::from_str(s) else {
        return Err(Error::InvalidVersionRequest(s.to_string()));
    };
    if specifiers.is_empty() {
        return Err(Error::InvalidVersionRequest(s.to_string()));
    }
    Ok(VersionRequest::from_specifiers(specifiers, variant))
}

impl From<&PythonVersion> for VersionRequest {
    fn from(version: &PythonVersion) -> Self {
        Self::from_str(&version.string)
            .expect("Valid `PythonVersion`s should be valid `VersionRequest`s")
    }
}

impl fmt::Display for VersionRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => f.write_str("any"),
            Self::Default => f.write_str("default"),
            Self::Major(major, variant) => write!(f, "{major}{}", variant.display_suffix()),
            Self::MajorMinor(major, minor, variant) => {
                write!(f, "{major}.{minor}{}", variant.display_suffix())
            }
            Self::MajorMinorPatch(major, minor, patch, variant) => {
                write!(f, "{major}.{minor}.{patch}{}", variant.display_suffix())
            }
            Self::MajorMinorPrerelease(major, minor, prerelease, variant) => {
                write!(f, "{major}.{minor}{prerelease}{}", variant.display_suffix())
            }
            Self::MajorMinorPatchPrerelease(major, minor, patch, prerelease, variant) => {
                write!(
                    f,
                    "{major}.{minor}.{patch}{prerelease}{}",
                    variant.display_suffix()
                )
            }
            Self::Range(specifiers, _) => write!(f, "{specifiers}"),
        }
    }
}

impl fmt::Display for PythonRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => write!(f, "a default Python"),
            Self::Any => write!(f, "any Python"),
            Self::Version(version) => write!(f, "Python {version}"),
            Self::Directory(path) => write!(f, "directory `{}`", path.user_display()),
            Self::File(path) => write!(f, "path `{}`", path.user_display()),
            Self::ExecutableName(name) => write!(f, "executable name `{name}`"),
            Self::Implementation(implementation) => {
                write!(f, "{}", implementation.pretty())
            }
            Self::ImplementationVersion(implementation, version) => {
                write!(f, "{} {version}", implementation.pretty())
            }
            Self::Key(request) => write!(f, "{request}"),
        }
    }
}

impl fmt::Display for PythonSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProvidedPath => f.write_str("provided path"),
            Self::ActiveEnvironment => f.write_str("active virtual environment"),
            Self::CondaPrefix | Self::BaseCondaPrefix => f.write_str("conda prefix"),
            Self::DiscoveredEnvironment => f.write_str("virtual environment"),
            Self::SearchPath => f.write_str("search path"),
            Self::SearchPathFirst => f.write_str("first executable in the search path"),
            Self::Registry => f.write_str("registry"),
            Self::MicrosoftStore => f.write_str("Microsoft Store"),
            Self::Managed => f.write_str("managed installations"),
            Self::ParentInterpreter => f.write_str("parent interpreter"),
        }
    }
}

impl PythonPreference {
    /// Return the interpreter sources allowed by this preference.
    fn sources(self) -> &'static [PythonSource] {
        match self {
            Self::OnlyManaged => &[PythonSource::Managed],
            Self::Managed => {
                if cfg!(windows) {
                    &[
                        PythonSource::Managed,
                        PythonSource::SearchPath,
                        PythonSource::Registry,
                    ]
                } else {
                    &[PythonSource::Managed, PythonSource::SearchPath]
                }
            }
            Self::System => {
                if cfg!(windows) {
                    &[
                        PythonSource::SearchPath,
                        PythonSource::Registry,
                        PythonSource::Managed,
                    ]
                } else {
                    &[PythonSource::SearchPath, PythonSource::Managed]
                }
            }
            Self::OnlySystem => {
                if cfg!(windows) {
                    &[PythonSource::SearchPath, PythonSource::Registry]
                } else {
                    &[PythonSource::SearchPath]
                }
            }
        }
    }
}

impl fmt::Display for PythonPreference {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::OnlyManaged => "only managed",
            Self::Managed => "prefer managed",
            Self::System => "prefer system",
            Self::OnlySystem => "only system",
        })
    }
}

impl DiscoveryPreferences {
    /// Describe the Python sources allowed by these preferences.
    fn sources(&self, request: &PythonRequest) -> String {
        let python_sources = self
            .python_preference
            .sources()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        match self.environment_preference {
            EnvironmentPreference::Any => disjunction(
                &["virtual environments"]
                    .into_iter()
                    .chain(python_sources.iter().map(String::as_str))
                    .collect::<Vec<_>>(),
            ),
            EnvironmentPreference::ExplicitSystem => {
                if request.is_explicit_system() {
                    disjunction(
                        &["virtual environments"]
                            .into_iter()
                            .chain(python_sources.iter().map(String::as_str))
                            .collect::<Vec<_>>(),
                    )
                } else {
                    disjunction(&["virtual environments"])
                }
            }
            EnvironmentPreference::OnlySystem => disjunction(
                &python_sources
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            ),
            EnvironmentPreference::OnlyVirtual => disjunction(&["virtual environments"]),
        }
    }
}

impl fmt::Display for PythonNotFound {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let sources = DiscoveryPreferences {
            python_preference: self.python_preference,
            environment_preference: self.environment_preference,
        }
        .sources(&self.request);

        match self.request {
            PythonRequest::Default | PythonRequest::Any => {
                write!(f, "No interpreter found in {sources}")
            }
            PythonRequest::File(_) => {
                write!(f, "No interpreter found at {}", self.request)
            }
            PythonRequest::Directory(_) => {
                write!(f, "No interpreter found in {}", self.request)
            }
            _ => {
                write!(f, "No interpreter found for {} in {sources}", self.request)
            }
        }
    }
}

/// Join items with `or`. Add commas when needed.
fn disjunction(items: &[&str]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].to_string(),
        2 => format!("{} or {}", items[0], items[1]),
        _ => {
            let last = items.last().unwrap();
            format!(
                "{}, or {}",
                items.iter().take(items.len() - 1).join(", "),
                last
            )
        }
    }
}

fn try_into_u8_slice(release: &[u64]) -> Result<Vec<u8>, std::num::TryFromIntError> {
    release
        .iter()
        .map(|x| match u8::try_from(*x) {
            Ok(x) => Ok(x),
            Err(e) => Err(e),
        })
        .collect()
}

/// Convert a wheel-tag version such as `38` into separate components such as `3.8`.
///
/// The first digit is the major version. The remaining digits are the minor version.
///
/// Return the original input if it is not a wheel-tag version.
fn split_wheel_tag_release_version(version: Version) -> Version {
    let release = version.release();
    if release.len() != 1 {
        return version;
    }

    let release = release[0].to_string();
    let mut chars = release.chars();
    let Some(major) = chars.next().and_then(|c| c.to_digit(10)) else {
        return version;
    };

    let Ok(minor) = chars.as_str().parse::<u32>() else {
        return version;
    };

    version.with_release([u64::from(major), u64::from(minor)])
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::{cell::Cell, io, path::PathBuf, str::FromStr};

    use assert_fs::{TempDir, prelude::*};
    use target_lexicon::{Aarch64Architecture, Architecture};
    use test_log::test;
    use uv_cache::Cache;
    use uv_distribution_types::RequiresPython;
    use uv_pep440::{Prerelease, PrereleaseKind, Version, VersionSpecifiers};

    use crate::{
        discovery::{PythonRequest, VersionRequest},
        downloads::{ArchRequest, PythonDownloadRequest},
        implementation::ImplementationName,
    };
    use uv_platform::{Arch, Libc, Os};

    use super::{
        DiscoveryPreferences, EnvironmentPreference, Error, InterpreterError,
        PythonExecutableGroup, PythonPreference, PythonSource, PythonVariant, QueryStrategy,
        python_installations_from_executables, sort_installations_by_key,
    };

    // Testing this at a higher level would necessitate relying on filesystem ordering.
    #[test]
    fn installation_key_order_only_partitions_critical_errors() {
        let query_error = |error| {
            Error::Query(
                Box::new(error),
                PathBuf::from("python"),
                PythonSource::SearchPath,
            )
        };

        let mut installations = [
            Ok(1_u8),
            Err(query_error(InterpreterError::NotFound(PathBuf::from(
                "missing",
            )))),
            Ok(2),
            Err(query_error(InterpreterError::Io(io::Error::other(
                "critical",
            )))),
            Ok(3),
        ];

        sort_installations_by_key(&mut installations, |key| *key);

        assert_matches!(
            &installations[..],
            [Ok(2), Ok(1), Err(noncritical), Err(critical), Ok(3)]
                if !noncritical.is_critical() && critical.is_critical()
        );
    }

    #[test]
    fn sequential_query_strategy_does_not_prefetch_executable_groups() -> anyhow::Result<()> {
        let cache = Cache::temp()?;
        let pulls = Cell::new(0);
        let executables = (0..2).map(|_| {
            pulls.set(pulls.get() + 1);
            Err::<PythonExecutableGroup, _>(Error::SourceNotAllowed(
                PythonRequest::Default,
                PythonSource::SearchPath,
                PythonPreference::OnlyManaged,
            ))
        });

        let mut installations =
            python_installations_from_executables(executables, &cache, QueryStrategy::Sequential);

        assert_eq!(pulls.get(), 0);
        assert!(installations.next().is_some_and(|result| result.is_err()));
        assert_eq!(pulls.get(), 1);

        Ok(())
    }

    #[test]
    fn interpreter_request_from_str() {
        assert_eq!(PythonRequest::parse("any"), PythonRequest::Any);
        assert_eq!(PythonRequest::parse("default"), PythonRequest::Default);
        assert_eq!(
            PythonRequest::parse("3.12"),
            PythonRequest::Version(VersionRequest::from_str("3.12").unwrap())
        );
        assert_eq!(
            PythonRequest::parse(">=3.12"),
            PythonRequest::Version(VersionRequest::from_str(">=3.12").unwrap())
        );
        assert_eq!(
            PythonRequest::parse(">=3.12,<3.13"),
            PythonRequest::Version(VersionRequest::from_str(">=3.12,<3.13").unwrap())
        );
        assert_eq!(
            PythonRequest::parse(">=3.12,<3.13"),
            PythonRequest::Version(VersionRequest::from_str(">=3.12,<3.13").unwrap())
        );

        assert_eq!(
            PythonRequest::parse("3.13.0a1"),
            PythonRequest::Version(VersionRequest::from_str("3.13.0a1").unwrap())
        );
        assert_eq!(
            PythonRequest::parse("3.13.0b5"),
            PythonRequest::Version(VersionRequest::from_str("3.13.0b5").unwrap())
        );
        assert_eq!(
            PythonRequest::parse("3.13.0rc1"),
            PythonRequest::Version(VersionRequest::from_str("3.13.0rc1").unwrap())
        );
        assert_eq!(
            PythonRequest::parse("3.13.1rc1"),
            PythonRequest::ExecutableName("3.13.1rc1".to_string()),
            "Pre-release version requests require a patch version of zero"
        );
        assert_eq!(
            PythonRequest::parse("3rc1"),
            PythonRequest::ExecutableName("3rc1".to_string()),
            "Pre-release version requests require a minor version"
        );

        assert_eq!(
            PythonRequest::parse("cpython"),
            PythonRequest::Implementation(ImplementationName::CPython)
        );

        assert_eq!(
            PythonRequest::parse("cpython3.12.2"),
            PythonRequest::ImplementationVersion(
                ImplementationName::CPython,
                VersionRequest::from_str("3.12.2").unwrap(),
            )
        );

        assert_eq!(
            PythonRequest::parse("cpython-3.13.2"),
            PythonRequest::Key(PythonDownloadRequest {
                version: Some(VersionRequest::MajorMinorPatch(
                    3,
                    13,
                    2,
                    PythonVariant::Default
                )),
                implementation: Some(ImplementationName::CPython),
                arch: None,
                os: None,
                libc: None,
                build: None,
                prereleases: None
            })
        );
        assert_eq!(
            PythonRequest::parse("cpython-3.13.2-macos-aarch64-none"),
            PythonRequest::Key(PythonDownloadRequest {
                version: Some(VersionRequest::MajorMinorPatch(
                    3,
                    13,
                    2,
                    PythonVariant::Default
                )),
                implementation: Some(ImplementationName::CPython),
                arch: Some(ArchRequest::Explicit(Arch::new(
                    Architecture::Aarch64(Aarch64Architecture::Aarch64),
                    None
                ))),
                os: Some(Os::new(target_lexicon::OperatingSystem::Darwin(None))),
                libc: Some(Libc::None),
                build: None,
                prereleases: None
            })
        );
        assert_eq!(
            PythonRequest::parse("any-3.13.2"),
            PythonRequest::Key(PythonDownloadRequest {
                version: Some(VersionRequest::MajorMinorPatch(
                    3,
                    13,
                    2,
                    PythonVariant::Default
                )),
                implementation: None,
                arch: None,
                os: None,
                libc: None,
                build: None,
                prereleases: None
            })
        );
        assert_eq!(
            PythonRequest::parse("any-3.13.2-any-aarch64"),
            PythonRequest::Key(PythonDownloadRequest {
                version: Some(VersionRequest::MajorMinorPatch(
                    3,
                    13,
                    2,
                    PythonVariant::Default
                )),
                implementation: None,
                arch: Some(ArchRequest::Explicit(Arch::new(
                    Architecture::Aarch64(Aarch64Architecture::Aarch64),
                    None
                ))),
                os: None,
                libc: None,
                build: None,
                prereleases: None
            })
        );

        assert_eq!(
            PythonRequest::parse("pypy"),
            PythonRequest::Implementation(ImplementationName::PyPy)
        );
        assert_eq!(
            PythonRequest::parse("pp"),
            PythonRequest::Implementation(ImplementationName::PyPy)
        );
        assert_eq!(
            PythonRequest::parse("graalpy"),
            PythonRequest::Implementation(ImplementationName::GraalPy)
        );
        assert_eq!(
            PythonRequest::parse("gp"),
            PythonRequest::Implementation(ImplementationName::GraalPy)
        );
        assert_eq!(
            PythonRequest::parse("cp"),
            PythonRequest::Implementation(ImplementationName::CPython)
        );
        assert_eq!(
            PythonRequest::parse("pypy3.10"),
            PythonRequest::ImplementationVersion(
                ImplementationName::PyPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
        );
        assert_eq!(
            PythonRequest::parse("pp310"),
            PythonRequest::ImplementationVersion(
                ImplementationName::PyPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
        );
        assert_eq!(
            PythonRequest::parse("graalpy3.10"),
            PythonRequest::ImplementationVersion(
                ImplementationName::GraalPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
        );
        assert_eq!(
            PythonRequest::parse("gp310"),
            PythonRequest::ImplementationVersion(
                ImplementationName::GraalPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
        );
        assert_eq!(
            PythonRequest::parse("cp38"),
            PythonRequest::ImplementationVersion(
                ImplementationName::CPython,
                VersionRequest::from_str("3.8").unwrap(),
            )
        );
        assert_eq!(
            PythonRequest::parse("pypy@3.10"),
            PythonRequest::ImplementationVersion(
                ImplementationName::PyPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
        );
        assert_eq!(
            PythonRequest::parse("pypy310"),
            PythonRequest::ImplementationVersion(
                ImplementationName::PyPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
        );
        assert_eq!(
            PythonRequest::parse("graalpy@3.10"),
            PythonRequest::ImplementationVersion(
                ImplementationName::GraalPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
        );
        assert_eq!(
            PythonRequest::parse("graalpy310"),
            PythonRequest::ImplementationVersion(
                ImplementationName::GraalPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
        );

        let tempdir = TempDir::new().unwrap();
        assert_eq!(
            PythonRequest::parse(tempdir.path().to_str().unwrap()),
            PythonRequest::Directory(tempdir.path().to_path_buf()),
            "An existing directory is treated as a directory"
        );
        assert_eq!(
            PythonRequest::parse(tempdir.child("foo").path().to_str().unwrap()),
            PythonRequest::File(tempdir.child("foo").path().to_path_buf()),
            "A path that does not exist is treated as a file"
        );
        tempdir.child("bar").touch().unwrap();
        assert_eq!(
            PythonRequest::parse(tempdir.child("bar").path().to_str().unwrap()),
            PythonRequest::File(tempdir.child("bar").path().to_path_buf()),
            "An existing file is treated as a file"
        );
        assert_eq!(
            PythonRequest::parse("./foo"),
            PythonRequest::File(PathBuf::from_str("./foo").unwrap()),
            "A string with a file system separator is treated as a file"
        );
        assert_eq!(
            PythonRequest::parse("3.13t"),
            PythonRequest::Version(VersionRequest::from_str("3.13t").unwrap())
        );
    }

    #[test]
    fn discovery_sources_prefer_system_orders_search_path_first() {
        let preferences = DiscoveryPreferences {
            python_preference: PythonPreference::System,
            environment_preference: EnvironmentPreference::OnlySystem,
        };
        let sources = preferences.sources(&PythonRequest::Default);

        if cfg!(windows) {
            assert_eq!(sources, "search path, registry, or managed installations");
        } else {
            assert_eq!(sources, "search path or managed installations");
        }
    }

    #[test]
    fn discovery_sources_only_system_matches_platform_order() {
        let preferences = DiscoveryPreferences {
            python_preference: PythonPreference::OnlySystem,
            environment_preference: EnvironmentPreference::OnlySystem,
        };
        let sources = preferences.sources(&PythonRequest::Default);

        if cfg!(windows) {
            assert_eq!(sources, "search path or registry");
        } else {
            assert_eq!(sources, "search path");
        }
    }

    #[test]
    fn interpreter_request_to_canonical_string() {
        assert_eq!(PythonRequest::Default.to_canonical_string(), "default");
        assert_eq!(PythonRequest::Any.to_canonical_string(), "any");
        assert_eq!(
            PythonRequest::Version(VersionRequest::from_str("3.12").unwrap()).to_canonical_string(),
            "3.12"
        );
        assert_eq!(
            PythonRequest::Version(VersionRequest::from_str(">=3.12").unwrap())
                .to_canonical_string(),
            ">=3.12"
        );
        assert_eq!(
            PythonRequest::Version(VersionRequest::from_str(">=3.12,<3.13").unwrap())
                .to_canonical_string(),
            ">=3.12, <3.13"
        );

        assert_eq!(
            PythonRequest::Version(VersionRequest::from_str("3.13.0a1").unwrap())
                .to_canonical_string(),
            "3.13a1"
        );

        assert_eq!(
            PythonRequest::Version(VersionRequest::from_str("3.13.0b5").unwrap())
                .to_canonical_string(),
            "3.13b5"
        );

        assert_eq!(
            PythonRequest::Version(VersionRequest::from_str("3.13.0rc1").unwrap())
                .to_canonical_string(),
            "3.13rc1"
        );

        assert_eq!(
            PythonRequest::Version(VersionRequest::from_str("313rc4").unwrap())
                .to_canonical_string(),
            "3.13rc4"
        );

        assert_eq!(
            PythonRequest::Version(VersionRequest::from_str("3.14.5rc1").unwrap())
                .to_canonical_string(),
            "3.14.5rc1"
        );

        assert_eq!(
            PythonRequest::ExecutableName("foo".to_string()).to_canonical_string(),
            "foo"
        );
        assert_eq!(
            PythonRequest::Implementation(ImplementationName::CPython).to_canonical_string(),
            "cpython"
        );
        assert_eq!(
            PythonRequest::ImplementationVersion(
                ImplementationName::CPython,
                VersionRequest::from_str("3.12.2").unwrap(),
            )
            .to_canonical_string(),
            "cpython@3.12.2"
        );
        assert_eq!(
            PythonRequest::Implementation(ImplementationName::PyPy).to_canonical_string(),
            "pypy"
        );
        assert_eq!(
            PythonRequest::ImplementationVersion(
                ImplementationName::PyPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
            .to_canonical_string(),
            "pypy@3.10"
        );
        assert_eq!(
            PythonRequest::Implementation(ImplementationName::GraalPy).to_canonical_string(),
            "graalpy"
        );
        assert_eq!(
            PythonRequest::ImplementationVersion(
                ImplementationName::GraalPy,
                VersionRequest::from_str("3.10").unwrap(),
            )
            .to_canonical_string(),
            "graalpy@3.10"
        );

        let tempdir = TempDir::new().unwrap();
        assert_eq!(
            PythonRequest::Directory(tempdir.path().to_path_buf()).to_canonical_string(),
            tempdir.path().to_str().unwrap(),
            "An existing directory is treated as a directory"
        );
        assert_eq!(
            PythonRequest::File(tempdir.child("foo").path().to_path_buf()).to_canonical_string(),
            tempdir.child("foo").path().to_str().unwrap(),
            "A path that does not exist is treated as a file"
        );
        tempdir.child("bar").touch().unwrap();
        assert_eq!(
            PythonRequest::File(tempdir.child("bar").path().to_path_buf()).to_canonical_string(),
            tempdir.child("bar").path().to_str().unwrap(),
            "An existing file is treated as a file"
        );
        assert_eq!(
            PythonRequest::File(PathBuf::from_str("./foo").unwrap()).to_canonical_string(),
            "./foo",
            "A string with a file system separator is treated as a file"
        );
    }

    #[test]
    fn version_request_from_str() {
        assert_eq!(
            VersionRequest::from_str("3").unwrap(),
            VersionRequest::Major(3, PythonVariant::Default)
        );
        assert_eq!(
            VersionRequest::from_str("3.12").unwrap(),
            VersionRequest::MajorMinor(3, 12, PythonVariant::Default)
        );
        assert_eq!(
            VersionRequest::from_str("3.12.1").unwrap(),
            VersionRequest::MajorMinorPatch(3, 12, 1, PythonVariant::Default)
        );
        assert!(VersionRequest::from_str("1.foo.1").is_err());
        assert_eq!(
            VersionRequest::from_str("3").unwrap(),
            VersionRequest::Major(3, PythonVariant::Default)
        );
        assert_eq!(
            VersionRequest::from_str("38").unwrap(),
            VersionRequest::MajorMinor(3, 8, PythonVariant::Default)
        );
        assert_eq!(
            VersionRequest::from_str("312").unwrap(),
            VersionRequest::MajorMinor(3, 12, PythonVariant::Default)
        );
        assert_eq!(
            VersionRequest::from_str("3100").unwrap(),
            VersionRequest::MajorMinor(3, 100, PythonVariant::Default)
        );
        assert_eq!(
            VersionRequest::from_str("3.13a1").unwrap(),
            VersionRequest::MajorMinorPrerelease(
                3,
                13,
                Prerelease {
                    kind: PrereleaseKind::Alpha,
                    number: 1
                },
                PythonVariant::Default
            )
        );
        assert_eq!(
            VersionRequest::from_str("313b1").unwrap(),
            VersionRequest::MajorMinorPrerelease(
                3,
                13,
                Prerelease {
                    kind: PrereleaseKind::Beta,
                    number: 1
                },
                PythonVariant::Default
            )
        );
        assert_eq!(
            VersionRequest::from_str("3.13.0b2").unwrap(),
            VersionRequest::MajorMinorPrerelease(
                3,
                13,
                Prerelease {
                    kind: PrereleaseKind::Beta,
                    number: 2
                },
                PythonVariant::Default
            )
        );
        assert_eq!(
            VersionRequest::from_str("3.13.0rc3").unwrap(),
            VersionRequest::MajorMinorPrerelease(
                3,
                13,
                Prerelease {
                    kind: PrereleaseKind::Rc,
                    number: 3
                },
                PythonVariant::Default
            )
        );
        assert_matches!(
            VersionRequest::from_str("3rc1"),
            Err(Error::InvalidVersionRequest(_)),
            "Pre-release version requests require a minor version"
        );
        assert_eq!(
            VersionRequest::from_str("3.14.5rc1").unwrap(),
            VersionRequest::MajorMinorPatchPrerelease(
                3,
                14,
                5,
                Prerelease {
                    kind: PrereleaseKind::Rc,
                    number: 1
                },
                PythonVariant::Default
            ),
            "Pre-release version requests with a non-zero patch are allowed (e.g., `3.14.5rc1`)"
        );
        assert_eq!(
            VersionRequest::from_str("3.13.2rc1").unwrap(),
            VersionRequest::MajorMinorPatchPrerelease(
                3,
                13,
                2,
                Prerelease {
                    kind: PrereleaseKind::Rc,
                    number: 1
                },
                PythonVariant::Default
            )
        );
        assert_matches!(
            VersionRequest::from_str("3.12-dev"),
            Err(Error::InvalidVersionRequest(_)),
            "Development version segments are not allowed"
        );
        assert_matches!(
            VersionRequest::from_str("3.12+local"),
            Err(Error::InvalidVersionRequest(_)),
            "Local version segments are not allowed"
        );
        assert_matches!(
            VersionRequest::from_str("3.12.post0"),
            Err(Error::InvalidVersionRequest(_)),
            "Post version segments are not allowed"
        );
        assert!(
            // Test for overflow
            matches!(
                VersionRequest::from_str("31000"),
                Err(Error::InvalidVersionRequest(_))
            )
        );
        assert_eq!(
            VersionRequest::from_str("3t").unwrap(),
            VersionRequest::Major(3, PythonVariant::Freethreaded)
        );
        assert_eq!(
            VersionRequest::from_str("313t").unwrap(),
            VersionRequest::MajorMinor(3, 13, PythonVariant::Freethreaded)
        );
        assert_eq!(
            VersionRequest::from_str("3.13t").unwrap(),
            VersionRequest::MajorMinor(3, 13, PythonVariant::Freethreaded)
        );
        assert_eq!(
            VersionRequest::from_str(">=3.13t").unwrap(),
            VersionRequest::Range(
                VersionSpecifiers::from_str(">=3.13").unwrap(),
                PythonVariant::Freethreaded
            )
        );
        assert_eq!(
            VersionRequest::from_str(">=3.13").unwrap(),
            VersionRequest::Range(
                VersionSpecifiers::from_str(">=3.13").unwrap(),
                PythonVariant::Default
            )
        );
        assert_eq!(
            VersionRequest::from_str(">=3.12,<3.14t").unwrap(),
            VersionRequest::Range(
                VersionSpecifiers::from_str(">=3.12,<3.14").unwrap(),
                PythonVariant::Freethreaded
            )
        );
        assert_matches!(
            VersionRequest::from_str("3.13tt"),
            Err(Error::InvalidVersionRequest(_))
        );
        assert_matches!(
            VersionRequest::from_str("3.12²t"),
            Err(Error::InvalidVersionRequest(_))
        );

        // `==` specifiers are parsed as concrete version requests via `from_specifiers`
        assert_eq!(
            VersionRequest::from_str("==3.12").unwrap(),
            VersionRequest::MajorMinor(3, 12, PythonVariant::Default)
        );
        assert_eq!(
            VersionRequest::from_str("==3.12.1").unwrap(),
            VersionRequest::MajorMinorPatch(3, 12, 1, PythonVariant::Default)
        );
    }

    #[test]
    fn version_request_from_specifiers() {
        // A single `==` specifier is parsed as a concrete version request
        assert_eq!(
            VersionRequest::from_specifiers(
                VersionSpecifiers::from_str("==3.12").unwrap(),
                PythonVariant::Default
            ),
            VersionRequest::MajorMinor(3, 12, PythonVariant::Default)
        );
        assert_eq!(
            VersionRequest::from_specifiers(
                VersionSpecifiers::from_str("==3.12.1").unwrap(),
                PythonVariant::Default
            ),
            VersionRequest::MajorMinorPatch(3, 12, 1, PythonVariant::Default)
        );

        // Wildcard `==` specifiers remain as ranges
        assert_eq!(
            VersionRequest::from_specifiers(
                VersionSpecifiers::from_str("==3.12.*").unwrap(),
                PythonVariant::Default
            ),
            VersionRequest::Range(
                VersionSpecifiers::from_str("==3.12.*").unwrap(),
                PythonVariant::Default
            )
        );

        // Range specifiers remain as ranges
        assert_eq!(
            VersionRequest::from_specifiers(
                VersionSpecifiers::from_str(">=3.12").unwrap(),
                PythonVariant::Default
            ),
            VersionRequest::Range(
                VersionSpecifiers::from_str(">=3.12").unwrap(),
                PythonVariant::Default
            )
        );

        // Multi-specifier constraints remain as ranges
        assert_eq!(
            VersionRequest::from_specifiers(
                VersionSpecifiers::from_str(">=3.12,<3.14").unwrap(),
                PythonVariant::Default
            ),
            VersionRequest::Range(
                VersionSpecifiers::from_str(">=3.12,<3.14").unwrap(),
                PythonVariant::Default
            )
        );
    }

    #[test]
    fn executable_names_from_request() {
        fn case(request: &str, expected: &[&str]) {
            let (implementation, version) = match PythonRequest::parse(request) {
                PythonRequest::Any => (None, VersionRequest::Any),
                PythonRequest::Default => (None, VersionRequest::Default),
                PythonRequest::Version(version) => (None, version),
                PythonRequest::ImplementationVersion(implementation, version) => {
                    (Some(implementation), version)
                }
                PythonRequest::Implementation(implementation) => {
                    (Some(implementation), VersionRequest::Default)
                }
                result => {
                    panic!("Test cases should request versions or implementations; got {result:?}")
                }
            };

            let result: Vec<_> = version
                .executable_names(implementation.as_ref())
                .into_iter()
                .map(|name| name.to_string())
                .collect();

            let expected: Vec<_> = expected
                .iter()
                .map(|name| format!("{name}{exe}", exe = std::env::consts::EXE_SUFFIX))
                .collect();

            assert_eq!(result, expected, "mismatch for case \"{request}\"");
        }

        case(
            "any",
            &[
                "python", "python3", "cpython", "cpython3", "pypy", "pypy3", "graalpy", "graalpy3",
                "pyodide", "pyodide3",
            ],
        );

        case("default", &["python", "python3"]);

        case("3", &["python3", "python"]);

        case("4", &["python4", "python"]);

        case("3.13", &["python3.13", "python3", "python"]);

        case("pypy", &["pypy", "pypy3", "python", "python3"]);

        case(
            "pypy@3.10",
            &[
                "pypy3.10",
                "pypy3",
                "pypy",
                "python3.10",
                "python3",
                "python",
            ],
        );

        case(
            "3.13t",
            &[
                "python3.13t",
                "python3.13",
                "python3t",
                "python3",
                "pythont",
                "python",
            ],
        );
        case("3t", &["python3t", "python3", "pythont", "python"]);

        case(
            "3.13.2",
            &["python3.13.2", "python3.13", "python3", "python"],
        );

        case(
            "3.13rc2",
            &["python3.13rc2", "python3.13", "python3", "python"],
        );
    }

    #[test]
    fn test_try_split_prefix_and_version() {
        assert_matches!(
            PythonRequest::try_split_prefix_and_version("prefix", "prefix"),
            Ok(None),
        );
        assert_matches!(
            PythonRequest::try_split_prefix_and_version("prefix", "prefix3"),
            Ok(Some(_)),
        );
        assert_matches!(
            PythonRequest::try_split_prefix_and_version("prefix", "prefix@3"),
            Ok(Some(_)),
        );
        assert_matches!(
            PythonRequest::try_split_prefix_and_version("prefix", "prefix3notaversion"),
            Ok(None),
        );
        // Version parsing errors are only raised if @ is present.
        assert!(
            PythonRequest::try_split_prefix_and_version("prefix", "prefix@3notaversion").is_err()
        );
        // @ is not allowed if the prefix is empty.
        assert!(PythonRequest::try_split_prefix_and_version("", "@3").is_err());
    }

    #[test]
    fn version_request_as_pep440_version() {
        // Non-concrete requests return `None`
        assert_eq!(VersionRequest::Default.as_pep440_version(), None);
        assert_eq!(VersionRequest::Any.as_pep440_version(), None);
        assert_eq!(
            VersionRequest::from_str(">=3.10")
                .unwrap()
                .as_pep440_version(),
            None
        );

        // `VersionRequest::Major`
        assert_eq!(
            VersionRequest::Major(3, PythonVariant::Default).as_pep440_version(),
            Some(Version::from_str("3").unwrap())
        );

        // `VersionRequest::MajorMinor`
        assert_eq!(
            VersionRequest::MajorMinor(3, 12, PythonVariant::Default).as_pep440_version(),
            Some(Version::from_str("3.12").unwrap())
        );

        // `VersionRequest::MajorMinorPatch`
        assert_eq!(
            VersionRequest::MajorMinorPatch(3, 12, 5, PythonVariant::Default).as_pep440_version(),
            Some(Version::from_str("3.12.5").unwrap())
        );

        // `VersionRequest::MajorMinorPrerelease`
        assert_eq!(
            VersionRequest::MajorMinorPrerelease(
                3,
                14,
                Prerelease {
                    kind: PrereleaseKind::Alpha,
                    number: 1
                },
                PythonVariant::Default
            )
            .as_pep440_version(),
            Some(Version::from_str("3.14.0a1").unwrap())
        );
        assert_eq!(
            VersionRequest::MajorMinorPrerelease(
                3,
                14,
                Prerelease {
                    kind: PrereleaseKind::Beta,
                    number: 2
                },
                PythonVariant::Default
            )
            .as_pep440_version(),
            Some(Version::from_str("3.14.0b2").unwrap())
        );
        assert_eq!(
            VersionRequest::MajorMinorPrerelease(
                3,
                13,
                Prerelease {
                    kind: PrereleaseKind::Rc,
                    number: 3
                },
                PythonVariant::Default
            )
            .as_pep440_version(),
            Some(Version::from_str("3.13.0rc3").unwrap())
        );

        // Variant is ignored
        assert_eq!(
            VersionRequest::Major(3, PythonVariant::Freethreaded).as_pep440_version(),
            Some(Version::from_str("3").unwrap())
        );
        assert_eq!(
            VersionRequest::MajorMinor(3, 13, PythonVariant::Freethreaded).as_pep440_version(),
            Some(Version::from_str("3.13").unwrap())
        );
    }

    #[test]
    fn python_request_as_pep440_version() {
        // `PythonRequest::Any` and `PythonRequest::Default` return `None`
        assert_eq!(PythonRequest::Any.as_pep440_version(), None);
        assert_eq!(PythonRequest::Default.as_pep440_version(), None);

        // `PythonRequest::Version` delegates to `VersionRequest`
        assert_eq!(
            PythonRequest::Version(VersionRequest::MajorMinor(3, 11, PythonVariant::Default))
                .as_pep440_version(),
            Some(Version::from_str("3.11").unwrap())
        );

        // `PythonRequest::ImplementationVersion` extracts version
        assert_eq!(
            PythonRequest::ImplementationVersion(
                ImplementationName::CPython,
                VersionRequest::MajorMinorPatch(3, 12, 1, PythonVariant::Default),
            )
            .as_pep440_version(),
            Some(Version::from_str("3.12.1").unwrap())
        );

        // `PythonRequest::Implementation` returns `None` (no version)
        assert_eq!(
            PythonRequest::Implementation(ImplementationName::CPython).as_pep440_version(),
            None
        );

        // `PythonRequest::Key` with version
        assert_eq!(
            PythonRequest::parse("cpython-3.13.2").as_pep440_version(),
            Some(Version::from_str("3.13.2").unwrap())
        );

        // `PythonRequest::Key` without version returns `None`
        assert_eq!(
            PythonRequest::parse("cpython-macos-aarch64-none").as_pep440_version(),
            None
        );

        // Range versions return `None`
        assert_eq!(
            PythonRequest::Version(VersionRequest::from_str(">=3.10").unwrap()).as_pep440_version(),
            None
        );
    }

    #[test]
    fn intersects_requires_python_exact() {
        let requires_python =
            RequiresPython::from_specifiers(VersionSpecifiers::from_str(">=3.12").unwrap());

        assert!(PythonRequest::parse("3.12").intersects_requires_python(&requires_python));
        assert!(!PythonRequest::parse("3.11").intersects_requires_python(&requires_python));
    }

    #[test]
    fn intersects_requires_python_major() {
        let requires_python =
            RequiresPython::from_specifiers(VersionSpecifiers::from_str(">=3.12").unwrap());

        // `3` overlaps with `>=3.12` (e.g., 3.12, 3.13, ... are all Python 3)
        assert!(PythonRequest::parse("3").intersects_requires_python(&requires_python));
        // `2` does not overlap with `>=3.12`
        assert!(!PythonRequest::parse("2").intersects_requires_python(&requires_python));
    }

    #[test]
    fn intersects_requires_python_range() {
        let requires_python =
            RequiresPython::from_specifiers(VersionSpecifiers::from_str(">=3.12").unwrap());

        assert!(PythonRequest::parse(">=3.12,<3.13").intersects_requires_python(&requires_python));
        assert!(!PythonRequest::parse(">=3.10,<3.12").intersects_requires_python(&requires_python));
    }

    #[test]
    fn intersects_requires_python_implementation_range() {
        let requires_python =
            RequiresPython::from_specifiers(VersionSpecifiers::from_str(">=3.12").unwrap());

        assert!(
            PythonRequest::parse("cpython@>=3.12,<3.13")
                .intersects_requires_python(&requires_python)
        );
        assert!(
            !PythonRequest::parse("cpython@>=3.10,<3.12")
                .intersects_requires_python(&requires_python)
        );
    }

    #[test]
    fn intersects_requires_python_no_version() {
        let requires_python =
            RequiresPython::from_specifiers(VersionSpecifiers::from_str(">=3.12").unwrap());

        // Requests without version constraints are always compatible
        assert!(PythonRequest::Any.intersects_requires_python(&requires_python));
        assert!(PythonRequest::Default.intersects_requires_python(&requires_python));
        assert!(
            PythonRequest::Implementation(ImplementationName::CPython)
                .intersects_requires_python(&requires_python)
        );
    }
}
