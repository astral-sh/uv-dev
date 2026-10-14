use std::ops::Add;
use std::path::{Path, PathBuf};

use fs_err as fs;
use itertools::Itertools;
use tracing::debug;
use uv_dirs::user_uv_config_dir;
use uv_fs::Simplified;
use uv_warnings::warn_user_once;

use uv_python_types::PythonRequest;

use crate::PythonRequestSource;

/// The file name for Python version pins.
pub static PYTHON_VERSION_FILENAME: &str = ".python-version";

/// The file name for multiple Python version declarations.
pub static PYTHON_VERSIONS_FILENAME: &str = ".python-versions";

/// A `.python-version` or `.python-versions` file.
#[derive(Debug, Clone)]
pub struct PythonVersionFile {
    /// The path to the version file.
    path: PathBuf,
    /// The Python version requests declared in the file.
    versions: Vec<PythonRequest>,
}

/// Whether to prefer the `.python-version` or `.python-versions` file.
#[derive(Debug, Clone, Copy, Default)]
pub enum FilePreference {
    #[default]
    Version,
    Versions,
}

/// Whether configuration files should be discovered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConfigDiscovery {
    #[default]
    Enabled,
    Disabled,
}

impl ConfigDiscovery {
    /// Determine the [`ConfigDiscovery`] setting based on the command-line arguments.
    pub fn from_args(no_config: bool) -> Self {
        if no_config {
            Self::Disabled
        } else {
            Self::Enabled
        }
    }

    /// Returns `true` if configuration discovery is enabled.
    pub const fn enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

#[derive(Debug, Default, Clone)]
pub struct DiscoveryOptions<'a> {
    /// The path to stop discovery at.
    stop_discovery_at: Option<&'a Path>,
    /// Ignore Python version files.
    ///
    /// Discovery will still run in order to display a log about the ignored file.
    config_discovery: ConfigDiscovery,
    /// Whether `.python-version` or `.python-versions` should be preferred.
    preference: FilePreference,
    /// Whether to ignore local version files, and only search for a global one.
    no_local: bool,
}

impl<'a> DiscoveryOptions<'a> {
    #[must_use]
    pub fn with_config_discovery(self, config_discovery: ConfigDiscovery) -> Self {
        Self {
            config_discovery,
            ..self
        }
    }

    #[must_use]
    pub fn with_preference(self, preference: FilePreference) -> Self {
        Self { preference, ..self }
    }

    #[must_use]
    pub fn with_stop_discovery_at(self, stop_discovery_at: Option<&'a Path>) -> Self {
        Self {
            stop_discovery_at,
            ..self
        }
    }

    #[must_use]
    pub fn with_no_local(self, no_local: bool) -> Self {
        Self { no_local, ..self }
    }
}

impl PythonVersionFile {
    /// Find a Python version file in the given directory or any of its parents.
    pub async fn discover(
        working_directory: impl AsRef<Path>,
        options: &DiscoveryOptions<'_>,
    ) -> Result<Option<Self>, std::io::Error> {
        let local = if options.no_local {
            None
        } else {
            let working_directory = working_directory.as_ref();
            let local = Self::find_nearest(working_directory, options);
            if local.is_none() {
                match options.stop_discovery_at {
                    Some(stop_discovery_at) if stop_discovery_at == working_directory => {
                        debug!(
                            "No Python version file found in workspace: {}",
                            working_directory.display()
                        );
                    }
                    Some(stop_discovery_at) => {
                        debug!(
                            "No Python version file found between working directory `{}` and workspace root `{}`",
                            working_directory.display(),
                            stop_discovery_at.display()
                        );
                    }
                    None => {
                        debug!(
                            "No Python version file found in ancestors of working directory: {}",
                            working_directory.display()
                        );
                    }
                }
            }
            local
        };
        let Some(path) = local.or_else(|| Self::find_global(options)) else {
            return Ok(None);
        };

        if !options.config_discovery.enabled() {
            debug!(
                "Ignoring Python version file at `{}` due to `--no-config`",
                path.user_display()
            );
            return Ok(None);
        }

        // Uses `try_from_path` instead of `from_path` to avoid TOCTOU failures.
        Self::try_from_path(path).await
    }

    fn find_global(options: &DiscoveryOptions<'_>) -> Option<PathBuf> {
        let user_config_dir = user_uv_config_dir()?;
        Self::find_in_directory(&user_config_dir, options)
    }

    fn find_nearest(path: &Path, options: &DiscoveryOptions<'_>) -> Option<PathBuf> {
        let stop_before = options.stop_discovery_at.and_then(Path::parent);
        path.ancestors()
            .take_while(|ancestor| Some(*ancestor) != stop_before)
            .find_map(|path| Self::find_in_directory(path, options))
    }

    fn find_in_directory(path: &Path, options: &DiscoveryOptions<'_>) -> Option<PathBuf> {
        let filenames = match options.preference {
            FilePreference::Versions => [PYTHON_VERSIONS_FILENAME, PYTHON_VERSION_FILENAME],
            FilePreference::Version => [PYTHON_VERSION_FILENAME, PYTHON_VERSIONS_FILENAME],
        };
        filenames
            .into_iter()
            .map(|filename| path.join(filename))
            .find(|path| path.is_file())
    }

    /// Try to read a Python version file at the given path.
    ///
    /// If the file does not exist, `Ok(None)` is returned.
    async fn try_from_path(path: PathBuf) -> Result<Option<Self>, std::io::Error> {
        let content = match fs::tokio::read_to_string(&path).await {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        debug!(
            "Reading Python requests from version file at `{}`",
            path.display()
        );
        let versions = content
            .lines()
            .map(str::trim)
            .filter(|line| {
                // Skip comments and empty lines.
                !(line.is_empty() || line.starts_with('#'))
            })
            .map(PythonRequest::parse)
            .filter(|request| {
                if let PythonRequest::ExecutableName(name) = request {
                    warn_user_once!(
                        "Ignoring unsupported Python request `{name}` in version file: {}",
                        path.display()
                    );
                    false
                } else {
                    true
                }
            })
            .collect();
        Ok(Some(Self { path, versions }))
    }

    /// Create a new representation of a version file at the given path.
    ///
    /// The file will not any include versions; see [`PythonVersionFile::with_versions`].
    /// The file will not be created; see [`PythonVersionFile::write`].
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            versions: vec![],
        }
    }

    /// Create a new representation of a global Python version file.
    ///
    /// Returns [`None`] if the user configuration directory cannot be determined.
    pub fn global() -> Option<Self> {
        let path = user_uv_config_dir()?.join(PYTHON_VERSION_FILENAME);
        Some(Self::new(path))
    }

    /// Returns `true` if the version file is a global version file.
    pub fn is_global(&self) -> bool {
        user_uv_config_dir().is_some_and(|directory| {
            self.path() == directory.join(PYTHON_VERSION_FILENAME)
                || self.path() == directory.join(PYTHON_VERSIONS_FILENAME)
        })
    }

    /// Return the first request declared in the file, if any.
    pub fn version(&self) -> Option<&PythonRequest> {
        self.versions.first()
    }

    /// Iterate of all versions declared in the file.
    pub fn versions(&self) -> impl Iterator<Item = &PythonRequest> {
        self.versions.iter()
    }

    /// Cast to a list of all versions declared in the file.
    pub fn into_versions(self) -> Vec<PythonRequest> {
        self.versions
    }

    /// Cast to the first version declared in the file, if any.
    pub fn into_version(self) -> Option<PythonRequest> {
        self.versions.into_iter().next()
    }

    /// Consume the version file into its source and first request for interpreter selection.
    pub fn into_sourced_request(self) -> (PythonRequestSource, Option<PythonRequest>) {
        (
            PythonRequestSource::DotPythonVersion(self.path),
            self.versions.into_iter().next(),
        )
    }

    /// Return the path to the version file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Set the versions for the file.
    #[must_use]
    pub fn with_versions(self, versions: Vec<PythonRequest>) -> Self {
        Self {
            path: self.path,
            versions,
        }
    }

    /// Update the version file on the file system.
    pub async fn write(&self) -> Result<(), std::io::Error> {
        debug!("Writing Python versions to `{}`", self.path.display());
        if let Some(parent) = self.path.parent() {
            fs_err::tokio::create_dir_all(parent).await?;
        }
        uv_fs::write_atomic_preserve(
            &self.path,
            self.versions
                .iter()
                .map(PythonRequest::to_canonical_string)
                .join("\n")
                .add("\n")
                .as_bytes(),
        )
        .await
    }
}
