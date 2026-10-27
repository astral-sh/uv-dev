//! Shared helpers for reading `pylock.toml` (PEP 751) files and deriving a [`Resolution`] and
//! [`HashStrategy`] from them, used by `uv pip install` and `uv pip sync`.

use anyhow::Context;
use tracing::info_span;

use uv_client::BaseClientBuilder;
use uv_configuration::{BuildOptions, HashCheckingMode, RequirementsInput, TargetTriple};
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

/// Read a `pylock.toml` from a local path or remote URL and parse it.
///
/// Return the location used to resolve package paths. For a remote lock, use the URL after redirects.
pub(crate) async fn read_pylock_toml(
    pylock: &RequirementsInput,
    client_builder: &BaseClientBuilder<'_>,
) -> anyhow::Result<(RequirementsInput, PylockToml)> {
    let (location, content) = match pylock {
        RequirementsInput::Stdin => (
            RequirementsInput::Stdin,
            uv_fs::read_stdin_to_string_transcode()?,
        ),
        RequirementsInput::Remote(url) => {
            let client = client_builder.build()?;
            let response = client.for_host(url).get(url.clone()).send().await?;
            response.error_for_status_ref()?;
            let location = RequirementsInput::Remote(response.url().clone().into());
            (location, response.text().await?)
        }
        RequirementsInput::Local(path) => {
            let location = RequirementsInput::Local(std::path::absolute(path)?);
            let content = fs_err::tokio::read_to_string(path).await?;
            (location, content)
        }
    };

    let pylock = pylock.user_display();
    let lock = info_span!("toml::from_str pylock.toml", path = %pylock)
        .in_scope(|| toml::from_str::<PylockToml>(&content))
        .with_context(|| format!("Not a valid `pylock.toml` file: {pylock}"))?;

    Ok((location, lock))
}

/// Verify Python compatibility and convert a parsed [`PylockToml`] into a [`Resolution`] with its
/// [`HashStrategy`].
pub(crate) fn resolve_pylock_toml(
    lock: PylockToml,
    location: &RequirementsInput,
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
        location,
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
