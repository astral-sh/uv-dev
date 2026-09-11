use std::ops::{Add, Range};
use std::path::{Path, PathBuf};

use fs_err as fs;
use itertools::Itertools;
use tracing::debug;
use uv_dirs::user_uv_config_dir;
use uv_errors::{SourceAnnotation, SourceFile, SourceSnippet};
use uv_fs::Simplified;
use uv_warnings::warn_user_once;

use crate::PythonRequest;

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
    versions: Vec<PythonVersionEntry>,
    /// The original decoded contents, when this file was read from disk.
    source: Option<SourceFile>,
}

#[derive(Debug, Clone)]
struct PythonVersionEntry {
    request: PythonRequest,
    /// The exact accepted occurrence in the original decoded file.
    span: Option<Range<usize>>,
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
        let allow_local = !options.no_local;
        let Some(path) = allow_local.then(|| {
            // First, try to find a local version file.
            let local = Self::find_nearest(&working_directory, options);
            if local.is_none() {
                // Log where we searched for the file, if not found
                if let Some(stop_discovery_at) = options.stop_discovery_at {
                    if stop_discovery_at == working_directory.as_ref() {
                        debug!(
                            "No Python version file found in workspace: {}",
                            working_directory.as_ref().display()
                        );
                    } else {
                        debug!(
                            "No Python version file found between working directory `{}` and workspace root `{}`",
                            working_directory.as_ref().display(),
                            stop_discovery_at.display()
                        );
                    }
                } else {
                    debug!(
                        "No Python version file found in ancestors of working directory: {}",
                        working_directory.as_ref().display()
                    );
                }
            }
            local
        }).flatten().or_else(|| {
            // Search for a global config
            Self::find_global(options)
        }) else {
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

    fn find_nearest(path: impl AsRef<Path>, options: &DiscoveryOptions<'_>) -> Option<PathBuf> {
        path.as_ref()
            .ancestors()
            .take_while(|path| {
                // Only walk up the given directory, if any.
                options
                    .stop_discovery_at
                    .and_then(Path::parent)
                    .is_none_or(|stop_discovery_at| stop_discovery_at != *path)
            })
            .find_map(|path| Self::find_in_directory(path, options))
    }

    fn find_in_directory(path: &Path, options: &DiscoveryOptions<'_>) -> Option<PathBuf> {
        let version_path = path.join(PYTHON_VERSION_FILENAME);
        let versions_path = path.join(PYTHON_VERSIONS_FILENAME);

        let paths = match options.preference {
            FilePreference::Versions => [versions_path, version_path],
            FilePreference::Version => [version_path, versions_path],
        };

        paths.into_iter().find(|path| path.is_file())
    }

    /// Try to read a Python version file at the given path.
    ///
    /// If the file does not exist, `Ok(None)` is returned.
    async fn try_from_path(path: PathBuf) -> Result<Option<Self>, std::io::Error> {
        match fs::tokio::read_to_string(&path).await {
            Ok(content) => {
                debug!(
                    "Reading Python requests from version file at `{}`",
                    path.display()
                );
                Ok(Some(Self::from_string(path, content)))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn from_string(path: PathBuf, content: String) -> Self {
        let source = SourceFile::new(path.portable_display().to_string(), content);
        let mut offset = 0;
        let versions = source
            .text()
            .split_inclusive('\n')
            .filter_map(|line| {
                let line_start = offset;
                offset += line.len();
                let trimmed = line.trim();

                // Skip comments and empty lines.
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    return None;
                }

                let request = PythonRequest::parse(trimmed);
                if let PythonRequest::ExecutableName(name) = &request {
                    warn_user_once!(
                        "Ignoring unsupported Python request `{name}` in version file: {}",
                        path.display()
                    );
                    return None;
                }

                let start = line_start + line.len() - line.trim_start().len();
                Some(PythonVersionEntry {
                    request,
                    span: Some(start..start + trimmed.len()),
                })
            })
            .collect();
        Self {
            path,
            versions,
            source: Some(source),
        }
    }

    /// Create a new representation of a version file at the given path.
    ///
    /// The file will not any include versions; see [`PythonVersionFile::with_versions`].
    /// The file will not be created; see [`PythonVersionFile::write`].
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            versions: vec![],
            source: None,
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
        Self::global().is_some_and(|global| self.path() == global.path())
    }

    /// Return the first request declared in the file, if any.
    pub fn version(&self) -> Option<&PythonRequest> {
        self.versions.first().map(|entry| &entry.request)
    }

    /// Return the source location of the first accepted Python request, if it was read from disk.
    ///
    /// Paths can contain arbitrary user text. Retain their location without exposing their source
    /// line; validated version and implementation requests can be shown directly.
    pub fn version_source(&self) -> Option<SourceSnippet<'static>> {
        let source = self.source.as_ref()?;
        let entry = self.versions.first()?;
        let snippet = SourceSnippet::new(source.clone()).with_annotation(
            SourceAnnotation::primary(entry.span.clone()?).with_label("Python request"),
        );
        match &entry.request {
            PythonRequest::Default
            | PythonRequest::Any
            | PythonRequest::Version(_)
            | PythonRequest::Implementation(_)
            | PythonRequest::ImplementationVersion(..)
            | PythonRequest::Key(_) => Some(snippet),
            PythonRequest::Directory(_)
            | PythonRequest::File(_)
            | PythonRequest::ExecutableName(_) => Some(snippet.without_source_text()),
        }
    }

    /// Iterate of all versions declared in the file.
    pub fn versions(&self) -> impl Iterator<Item = &PythonRequest> {
        self.versions.iter().map(|entry| &entry.request)
    }

    /// Cast to a list of all versions declared in the file.
    pub fn into_versions(self) -> Vec<PythonRequest> {
        self.versions
            .into_iter()
            .map(|entry| entry.request)
            .collect()
    }

    /// Cast to the first version declared in the file, if any.
    pub fn into_version(self) -> Option<PythonRequest> {
        self.versions.into_iter().next().map(|entry| entry.request)
    }

    /// Return the path to the version file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the file name of the version file (guaranteed to be one of `.python-version` or
    /// `.python-versions`).
    pub fn file_name(&self) -> &str {
        self.path.file_name().unwrap().to_str().unwrap()
    }

    /// Set the versions for the file.
    #[must_use]
    pub fn with_versions(self, versions: Vec<PythonRequest>) -> Self {
        Self {
            path: self.path,
            versions: versions
                .into_iter()
                .map(|request| PythonVersionEntry {
                    request,
                    span: None,
                })
                .collect(),
            source: None,
        }
    }

    /// Update the version file on the file system.
    pub async fn write(&self) -> Result<(), std::io::Error> {
        debug!("Writing Python versions to `{}`", self.path.display());
        if let Some(parent) = self.path.parent() {
            fs_err::tokio::create_dir_all(parent).await?;
        }
        fs::tokio::write(
            &self.path,
            self.versions()
                .map(PythonRequest::to_canonical_string)
                .join("\n")
                .add("\n")
                .as_bytes(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    use insta::{assert_debug_snapshot, assert_snapshot};
    use uv_errors::{Diagnostic, ErrorOptions, Hints, write_error_chain_with_options};

    use super::PythonVersionFile;
    use crate::PythonRequest;

    #[derive(Debug, thiserror::Error)]
    #[error("The pinned Python request is incompatible")]
    struct VersionFileError(PythonVersionFile);

    fn diagnostic<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        Some(
            Diagnostic::default().with_snippet(
                error
                    .downcast_ref::<VersionFileError>()?
                    .0
                    .version_source()?,
            ),
        )
    }

    fn format_source(file: PythonVersionFile) -> anyhow::Result<String> {
        let mut output = String::new();
        write_error_chain_with_options(
            &VersionFileError(file),
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(80)
                .with_diagnostic(diagnostic)
                .with_stream(&mut output),
        )?;
        Ok(anstream::adapter::strip_str(&output).to_string())
    }

    fn version_file(contents: &str) -> PythonVersionFile {
        PythonVersionFile::from_string(PathBuf::from(".python-version"), contents.to_owned())
    }

    #[test]
    fn version_file_retains_selected_occurrence() -> anyhow::Result<()> {
        let file = version_file(
            "# café\r\nnot-a-supported-python-request\r\n\u{2003}3.11\r\n3.11\t\r\n3.12",
        );
        let entries = file
            .versions
            .iter()
            .map(|entry| {
                (
                    entry.request.to_canonical_string(),
                    entry.span.clone(),
                    entry
                        .span
                        .clone()
                        .and_then(|span| file.source.as_ref()?.text().get(span)),
                )
            })
            .collect::<Vec<_>>();
        assert_debug_snapshot!(entries, @r#"
        [
            (
                "3.11",
                Some(
                    44..48,
                ),
                Some(
                    "3.11",
                ),
            ),
            (
                "3.11",
                Some(
                    50..54,
                ),
                Some(
                    "3.11",
                ),
            ),
            (
                "3.12",
                Some(
                    57..61,
                ),
                Some(
                    "3.12",
                ),
            ),
        ]
        "#);
        assert_snapshot!(format_source(file)?, @"
        error: The pinned Python request is incompatible
           --> .python-version:3:2
            |
          3 |  3.11
            |  ^^^^ Python request
        ");
        Ok(())
    }

    #[test]
    fn version_file_hides_path_requests() -> anyhow::Result<()> {
        assert_snapshot!(
            format_source(version_file("# private\n./credentials-containing-path/python\n"))?,
            @"
        error: The pinned Python request is incompatible
           --> .python-version:2:1
        "
        );
        Ok(())
    }

    #[test]
    fn version_file_forgets_source_after_mutation() {
        let original = version_file("# pinned\n3.11\n");
        let changed = original
            .clone()
            .with_versions(vec![PythonRequest::parse("3.12")]);
        let new = PythonVersionFile::new(PathBuf::from(".python-version"))
            .with_versions(vec![PythonRequest::parse("3.13")]);

        assert!(original.version_source().is_some());
        assert!(changed.version_source().is_none());
        assert!(new.version_source().is_none());
        assert_debug_snapshot!(
            (
                original.version().map(PythonRequest::to_canonical_string),
                changed
                    .versions()
                    .map(PythonRequest::to_canonical_string)
                    .collect::<Vec<_>>(),
                changed
                    .clone()
                    .into_versions()
                    .into_iter()
                    .map(|request| request.to_canonical_string().into_owned())
                    .collect::<Vec<_>>(),
                new.into_version()
                    .map(|request| request.to_canonical_string().into_owned()),
            ),
            @r#"
        (
            Some(
                "3.11",
            ),
            [
                "3.12",
            ],
            [
                "3.12",
            ],
            Some(
                "3.13",
            ),
        )
        "#
        );
    }
}
