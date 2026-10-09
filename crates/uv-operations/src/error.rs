use std::path::PathBuf;

use itertools::Itertools;
use owo_colors::OwoColorize;

use uv_distribution_types::{DerivationChain, Name};
use uv_fs::Simplified;
use uv_normalize::{ExtraName, GroupName};
use uv_resolver::{NoSolutionError, NoSolutionHeader, ResolveError};

use crate::installation::Changelog;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to determine installation plan")]
    Plan(#[source] uv_installer::PlanError),

    #[error(transparent)]
    Prepare(#[from] uv_installer::PrepareError),

    #[error(transparent)]
    Install(#[from] uv_installer::InstallError),

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

    #[error("Failed to bytecode-compile Python file in: {}", path.user_display())]
    CompileTree {
        path: PathBuf,
        #[source]
        source: uv_installer::CompileError,
    },

    #[error("Failed to bytecode-compile installed packages")]
    CompileFiles(#[source] uv_installer::CompileError),

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

    #[error(
        "Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file"
    )]
    ExtrasWithoutSource { has_editable: bool },

    #[error(
        "Requested extra{} not found: {}",
        if .0.len() == 1 { "" } else { "s" },
        .0.iter().join(", ")
    )]
    MissingExtras(Vec<ExtraName>),

    #[error("The dependency group '{name}' was not found in the project: {}", path.user_display())]
    MissingGroup { name: GroupName, path: PathBuf },

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
            Self::Plan(_)
            | Self::Resolve(_)
            | Self::Prepare(_)
            | Self::Install(_)
            | Self::Uninstall(_)
            | Self::CompileTree { .. }
            | Self::CompileFiles(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Requirements(_)
            | Self::RequirementsWithContext { .. }
            | Self::ExtrasWithoutSource { .. }
            | Self::MissingExtras(_)
            | Self::MissingGroup { .. }
            | Self::Anyhow(_)
            | Self::OutdatedEnvironment(_) => None,
        }
    }

    /// Return the changes required by an environment that failed an up-to-date check.
    pub fn outdated_environment(&self) -> Option<&Changelog> {
        match self {
            Self::OutdatedEnvironment(changelog) => Some(changelog),
            Self::Plan(_)
            | Self::NoSolution { .. }
            | Self::Resolve(_)
            | Self::Prepare(_)
            | Self::Install(_)
            | Self::Uninstall(_)
            | Self::CompileTree { .. }
            | Self::CompileFiles(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Requirements(_)
            | Self::RequirementsWithContext { .. }
            | Self::ExtrasWithoutSource { .. }
            | Self::MissingExtras(_)
            | Self::MissingGroup { .. }
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
            error @ (Self::Plan(_)
            | Self::Prepare(_)
            | Self::Install(_)
            | Self::NoSolution { .. }
            | Self::Resolve(_)
            | Self::Uninstall(_)
            | Self::CompileTree { .. }
            | Self::CompileFiles(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Requirements(_)
            | Self::RequirementsWithContext { .. }
            | Self::ExtrasWithoutSource { .. }
            | Self::MissingExtras(_)
            | Self::MissingGroup { .. }
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
            error @ (Self::Plan(_)
            | Self::Prepare(_)
            | Self::Install(_)
            | Self::Resolve(_)
            | Self::Uninstall(_)
            | Self::CompileTree { .. }
            | Self::CompileFiles(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::ExtrasWithoutSource { .. }
            | Self::MissingExtras(_)
            | Self::MissingGroup { .. }
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
            Self::Plan(_)
            | Self::Install(_)
            | Self::Uninstall(_)
            | Self::CompileTree { .. }
            | Self::CompileFiles(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::ExtrasWithoutSource { .. }
            | Self::MissingExtras(_)
            | Self::MissingGroup { .. }
            | Self::Anyhow(_) => false,
        }
    }
}

impl uv_errors::Hinted for Error {
    fn hints(&self) -> uv_errors::Hints<'_> {
        match self {
            Self::NoSolution { source, .. } => source.hints(),
            Self::Resolve(uv_resolver::ResolveError::Dist(_, dist, chain, error)) => {
                uv_distribution::dist_hints(dist.name(), dist.version(), chain, error.hints())
            }
            Self::Resolve(uv_resolver::ResolveError::Dependencies(error, name, version, chain)) => {
                uv_distribution::dist_hints(name, Some(version), chain, error.hints())
            }
            Self::Resolve(error) => error.hints(),
            Self::Requirements(uv_requirements::Error::Dist(_, dist, error))
            | Self::RequirementsWithContext {
                source: uv_requirements::Error::Dist(_, dist, error),
                ..
            } => uv_distribution::dist_hints(
                dist.name(),
                dist.version(),
                &DerivationChain::default(),
                error.hints(),
            ),
            Self::Prepare(uv_installer::PrepareError::Dist(_, dist, chain, error)) => {
                uv_distribution::dist_hints(dist.name(), dist.version(), chain, error.hints())
            }
            Self::ExtrasWithoutSource { has_editable } => {
                uv_errors::Hints::from(if *has_editable {
                    "Use `<dir>[extra]` syntax or `-r <file>` instead"
                } else {
                    "Use `package[extra]` syntax instead"
                })
            }
            Self::Anyhow(_) => uv_errors::Hints::none(),
            Self::Plan(_)
            | Self::Prepare(_)
            | Self::Install(_)
            | Self::Uninstall(_)
            | Self::CompileTree { .. }
            | Self::CompileFiles(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Requirements(_)
            | Self::RequirementsWithContext { .. }
            | Self::MissingExtras(_)
            | Self::MissingGroup { .. }
            | Self::OutdatedEnvironment(_) => uv_errors::Hints::none(),
        }
    }
}
