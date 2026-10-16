//! Shared helpers for deriving a [`Resolution`] and [`HashStrategy`] from `pylock.toml` (PEP 751)
//! files, used by `uv pip install` and `uv pip sync`.

use std::path::Path;

use uv_configuration::{BuildOptions, HashCheckingMode, TargetTriple};
use uv_distribution_types::{RequiresPython, Resolution};
use uv_lock::{PylockToml, PylockTomlError};
use uv_normalize::{ExtraName, GroupName};
use uv_pep440::Version;
use uv_platform_tags::TagsError;
use uv_python_interpreter::Interpreter;
use uv_python_types::PythonVersion;
use uv_types::{HashStrategy, HashStrategyError};

use uv_resolve_operations::{resolution_markers, resolution_tags};

/// A failure while resolving the packages recorded in a `pylock.toml`.
#[derive(Debug, thiserror::Error)]
pub enum PylockResolutionError {
    #[error(
        "The requested interpreter resolved to Python {python_version}, which is incompatible with the `pylock.toml`'s Python requirement: `{requires_python}`"
    )]
    IncompatiblePython {
        python_version: Version,
        requires_python: RequiresPython,
    },
    #[error(transparent)]
    Tags(#[from] TagsError),
    #[error(transparent)]
    Pylock(#[from] PylockTomlError),
    #[error(transparent)]
    Hash(#[from] HashStrategyError),
}

impl uv_errors::Hinted for PylockResolutionError {
    fn hints(&self) -> uv_errors::Hints<'_> {
        match self {
            Self::Pylock(error) => error.hints(),
            Self::IncompatiblePython { .. } | Self::Tags(_) | Self::Hash(_) => {
                uv_errors::Hints::none()
            }
        }
    }
}

/// Verify Python compatibility and convert a parsed [`PylockToml`] into a [`Resolution`] with its
/// [`HashStrategy`].
pub(crate) fn resolve_pylock_toml(
    lock: PylockToml,
    install_path: &Path,
    interpreter: &Interpreter,
    python_version: Option<&PythonVersion>,
    python_platform: Option<&TargetTriple>,
    extras: &[ExtraName],
    groups: &[GroupName],
    build_options: &BuildOptions,
    hash_checking: Option<HashCheckingMode>,
) -> Result<(Resolution, HashStrategy), PylockResolutionError> {
    if let Some(requires_python) = lock.requires_python.as_ref() {
        if !requires_python.contains(interpreter.python_version()) {
            return Err(PylockResolutionError::IncompatiblePython {
                python_version: interpreter.python_version().clone(),
                requires_python: requires_python.clone(),
            });
        }
    }

    let tags = resolution_tags(python_version, python_platform, interpreter)?;
    let marker_env = resolution_markers(python_version, python_platform, interpreter);

    let resolution = lock.to_resolution(
        install_path,
        marker_env.markers(),
        extras,
        groups,
        &tags,
        build_options,
    )?;
    let hasher = if let Some(hash_checking) = hash_checking {
        HashStrategy::from_resolution(&resolution, hash_checking)?
    } else {
        HashStrategy::default()
    };

    Ok((resolution, hasher))
}
