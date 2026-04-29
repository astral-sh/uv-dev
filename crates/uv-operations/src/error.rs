use owo_colors::OwoColorize;

use uv_distribution_types::DerivationChain;
use uv_distribution_types::Name;
use uv_resolver::NoSolutionError;
use uv_resolver::NoSolutionHeader;
use uv_resolver::ResolveError;

use crate::installation::Changelog;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Prepare(#[from] uv_installer::PrepareError),

    #[error("{header}")]
    NoSolution {
        header: NoSolutionHeader,
        #[source]
        source: Box<NoSolutionError>,
    },

    #[error(transparent)]
    Resolve(#[from] ResolveError),

    #[error(transparent)]
    Uninstall(#[from] uv_installer::UninstallError),

    #[error(transparent)]
    Hash(#[from] uv_types::HashStrategyError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Fmt(#[from] std::fmt::Error),

    #[error(transparent)]
    Requirements(#[from] uv_requirements::Error),

    #[error("Failed to resolve {context} requirement")]
    RequirementsWithContext {
        context: &'static str,
        #[source]
        source: uv_requirements::Error,
    },

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error("The environment is outdated; run `{}` to update the environment", "uv sync".cyan())]
    OutdatedEnvironment(Box<Changelog>),
}

impl Error {
    /// Return the solver failure for an unsatisfiable resolution.
    pub fn as_no_solution(&self) -> Option<&NoSolutionError> {
        match self {
            Self::NoSolution { source, .. } | Self::Resolve(ResolveError::NoSolution(source)) => {
                Some(source)
            }
            Self::Resolve(_)
            | Self::Prepare(_)
            | Self::Uninstall(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Requirements(_)
            | Self::RequirementsWithContext { .. }
            | Self::Anyhow(_)
            | Self::OutdatedEnvironment(_) => None,
        }
    }

    /// Return the changes required by an environment that failed an up-to-date check.
    pub fn outdated_environment(&self) -> Option<&Changelog> {
        match self {
            Self::OutdatedEnvironment(changelog) => Some(changelog),
            Self::NoSolution { .. }
            | Self::Resolve(_)
            | Self::Prepare(_)
            | Self::Uninstall(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Requirements(_)
            | Self::RequirementsWithContext { .. }
            | Self::Anyhow(_) => None,
        }
    }

    /// Add the default heading when this operation is the final command error.
    ///
    /// Nested operation errors may already have a more specific heading from their caller.
    #[must_use]
    pub fn with_default_resolution_context(self) -> Self {
        match self {
            Self::Resolve(ResolveError::NoSolution(source)) => Self::NoSolution {
                header: NoSolutionHeader::new(source.environment().clone()),
                source,
            },
            error @ (Self::Prepare(_)
            | Self::NoSolution { .. }
            | Self::Resolve(_)
            | Self::Uninstall(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Requirements(_)
            | Self::RequirementsWithContext { .. }
            | Self::Anyhow(_)
            | Self::OutdatedEnvironment(_)) => error,
        }
    }

    /// Set the command-specific context for a resolution failure.
    #[must_use]
    pub fn with_resolution_context(self, context: &'static str) -> Self {
        match self.with_default_resolution_context() {
            Self::NoSolution { header, source } => Self::NoSolution {
                header: header.with_context(context),
                source,
            },
            Self::Requirements(source) | Self::RequirementsWithContext { source, .. } => {
                Self::RequirementsWithContext { context, source }
            }
            error @ (Self::Prepare(_)
            | Self::Resolve(_)
            | Self::Uninstall(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Anyhow(_)
            | Self::OutdatedEnvironment(_)) => error,
        }
    }

    /// Return whether this operation failure is an expected user-facing failure.
    pub fn is_user_failure(&self) -> bool {
        match self {
            Self::Prepare(error) => error.is_user_failure(),
            Self::NoSolution { .. } => true,
            Self::Resolve(error) => error.is_user_failure(),
            Self::Hash(_) | Self::OutdatedEnvironment(_) => true,
            Self::Requirements(error) | Self::RequirementsWithContext { source: error, .. } => {
                error.is_user_failure()
            }
            Self::Uninstall(_) | Self::Io(_) | Self::Fmt(_) | Self::Anyhow(_) => false,
        }
    }
}

impl uv_errors::Hinted for Error {
    fn hints(&self) -> uv_errors::Hints<'_> {
        match self {
            Self::NoSolution { source, .. } => source.hints(),
            Self::Resolve(uv_resolver::ResolveError::Dist(_, dist, chain, error)) => {
                crate::diagnostics::dist_hints(dist.name(), dist.version(), chain, error.hints())
            }
            Self::Resolve(uv_resolver::ResolveError::Dependencies(error, name, version, chain)) => {
                crate::diagnostics::dist_hints(name, Some(version), chain, error.hints())
            }
            Self::Resolve(error) => error.hints(),
            Self::Requirements(uv_requirements::Error::Dist(_, dist, error))
            | Self::RequirementsWithContext {
                source: uv_requirements::Error::Dist(_, dist, error),
                ..
            } => crate::diagnostics::dist_hints(
                dist.name(),
                dist.version(),
                &DerivationChain::default(),
                error.hints(),
            ),
            Self::Prepare(uv_installer::PrepareError::Dist(_, dist, chain, error)) => {
                crate::diagnostics::dist_hints(dist.name(), dist.version(), chain, error.hints())
            }
            Self::Anyhow(err) => {
                for cause in err.chain() {
                    if let Some(extra_err) = cause.downcast_ref::<ExtrasWithoutSourceError>() {
                        return uv_errors::Hinted::hints(extra_err);
                    }
                }
                uv_errors::Hints::none()
            }
            Self::Prepare(_)
            | Self::Uninstall(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Requirements(_)
            | Self::RequirementsWithContext { .. }
            | Self::OutdatedEnvironment(_) => uv_errors::Hints::none(),
        }
    }
}

/// Extras were requested but no valid source was provided.
#[derive(Debug, thiserror::Error)]
#[error(
    "Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file"
)]
pub struct ExtrasWithoutSourceError {
    pub has_editable: bool,
}

impl uv_errors::Hinted for ExtrasWithoutSourceError {
    fn hints(&self) -> uv_errors::Hints<'_> {
        uv_errors::Hints::from(if self.has_editable {
            "Use `<dir>[extra]` syntax or `-r <file>` instead"
        } else {
            "Use `package[extra]` syntax instead"
        })
    }
}
