use std::io;
use std::path::PathBuf;

use owo_colors::OwoColorize;
use uv_command_support::UvError;
use uv_distribution::dist_hints;
use uv_distribution_types::Name;
use uv_fs::Simplified;
use uv_fs::link::LinkError;
use uv_install_wheel::Error as InstallWheelError;
use uv_installer::InstallError;

use crate::Changelog;

/// An error while preparing or installing distributions.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to determine installation plan")]
    Plan(#[source] uv_installer::PlanError),
    #[error(transparent)]
    Prepare(#[from] uv_installer::PrepareError),
    #[error(transparent)]
    Install(#[from] uv_installer::InstallError),
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
    Anyhow(#[from] anyhow::Error),
    #[error(transparent)]
    SystemInstallPermissions(InstallError),
    #[error("The environment is outdated; run `{}` to update the environment", "uv sync".cyan())]
    OutdatedEnvironment(Box<Changelog>),
}

impl Error {
    /// Return the changes required by an environment that failed an up-to-date check.
    pub fn outdated_environment(&self) -> Option<&Changelog> {
        match self {
            Self::OutdatedEnvironment(changelog) => Some(changelog),
            Self::Plan(_)
            | Self::Prepare(_)
            | Self::Install(_)
            | Self::Uninstall(_)
            | Self::CompileTree { .. }
            | Self::CompileFiles(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Anyhow(_)
            | Self::SystemInstallPermissions(_) => None,
        }
    }

    /// Return whether this operation failure is an expected user-facing failure.
    fn is_user_failure(&self) -> bool {
        match self {
            Self::Prepare(error) => error.is_user_failure(),
            Self::Hash(_) | Self::OutdatedEnvironment(_) => true,
            Self::Plan(_)
            | Self::Install(_)
            | Self::Uninstall(_)
            | Self::CompileTree { .. }
            | Self::CompileFiles(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Anyhow(_)
            | Self::SystemInstallPermissions(_) => false,
        }
    }

    /// Attach a hint only when installation failed to create a destination directory. Other I/O
    /// errors can come from reading the cached wheel instead of writing to the environment.
    pub(crate) fn from_install(error: InstallError, is_system: bool) -> Self {
        let destination_permission_denied = match &error {
            InstallError::Wheel {
                source: InstallWheelError::Copy(LinkError::CreateDir { err: source, .. }),
                ..
            } => source.kind() == io::ErrorKind::PermissionDenied,
            InstallError::SymlinkWithoutCache
            | InstallError::WorkerPanicked
            | InstallError::Wheel { .. } => false,
        };
        if is_system && destination_permission_denied {
            Self::SystemInstallPermissions(error)
        } else {
            Self::Install(error)
        }
    }
}

impl From<Error> for UvError {
    fn from(error: Error) -> Self {
        if error.is_user_failure() {
            Self::User(error.into())
        } else {
            Self::Unexpected(error.into())
        }
    }
}

impl uv_errors::Hinted for Error {
    fn hints(&self) -> uv_errors::Hints<'_> {
        match self {
            Self::Prepare(uv_installer::PrepareError::Dist(_, dist, chain, error)) => {
                dist_hints(dist.name(), dist.version(), chain, error.hints())
            }
            Self::SystemInstallPermissions(_) => uv_errors::Hints::from(
                "It looks like you do not have permission to write to the system Python environment. Consider creating a virtual environment with `uv venv`, then retry the installation",
            ),
            Self::Plan(_)
            | Self::Prepare(_)
            | Self::Install(_)
            | Self::Uninstall(_)
            | Self::CompileTree { .. }
            | Self::CompileFiles(_)
            | Self::Hash(_)
            | Self::Io(_)
            | Self::Fmt(_)
            | Self::Anyhow(_)
            | Self::OutdatedEnvironment(_) => uv_errors::Hints::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error as StdError;
    use std::io;
    use std::path::PathBuf;

    use uv_distribution_types::{CachedDist, CachedRegistryDist};
    use uv_fs::link::LinkError;
    use uv_install_wheel::Error as WheelError;
    use uv_installer::InstallError;
    use uv_pypi_types::HashDigests;

    use super::Error;

    fn destination_error(kind: io::ErrorKind) -> WheelError {
        WheelError::Copy(LinkError::CreateDir {
            path: "site-packages".into(),
            err: io::Error::from(kind),
        })
    }

    fn default_value<T: Default>() -> T {
        T::default()
    }

    fn install_error(source: WheelError) -> anyhow::Result<InstallError> {
        Ok(InstallError::Wheel {
            wheel: Box::new(CachedDist::Registry(CachedRegistryDist {
                filename: "example-1.0.0-py3-none-any.whl".parse()?,
                path: PathBuf::from("example-1.0.0-py3-none-any.whl").into_boxed_path(),
                hashes: HashDigests::empty(),
                cache_info: default_value(),
                build_info: None,
            })),
            source,
        })
    }

    fn error_chain(error: &(dyn StdError + 'static)) -> Vec<String> {
        let mut chain = vec![error.to_string()];
        let mut source = error.source();
        while let Some(error) = source {
            chain.push(error.to_string());
            source = error.source();
        }
        chain
    }

    #[test]
    fn system_install_permissions_hints() -> anyhow::Result<()> {
        let cases = [
            (
                "system destination",
                true,
                install_error(destination_error(io::ErrorKind::PermissionDenied))?,
            ),
            (
                "virtual environment",
                false,
                install_error(destination_error(io::ErrorKind::PermissionDenied))?,
            ),
            (
                "target",
                false,
                install_error(destination_error(io::ErrorKind::PermissionDenied))?,
            ),
            (
                "prefix",
                false,
                install_error(destination_error(io::ErrorKind::PermissionDenied))?,
            ),
            (
                "cached wheel read",
                true,
                install_error(WheelError::Io(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "cached wheel read",
                )))?,
            ),
            (
                "ambiguous copy",
                true,
                install_error(WheelError::Copy(LinkError::Copy {
                    to: "site-packages/example.py".into(),
                    err: io::Error::from(io::ErrorKind::PermissionDenied),
                }))?,
            ),
            (
                "missing destination",
                true,
                install_error(destination_error(io::ErrorKind::NotFound))?,
            ),
            (
                "other destination error",
                true,
                install_error(destination_error(io::ErrorKind::Other))?,
            ),
        ];
        let mut outcomes = Vec::new();
        for (name, is_system, error) in cases {
            let original_chain = error_chain(&error);
            let error = Error::from_install(error, is_system);
            let hints = uv_errors::Hinted::hints(&error)
                .into_iter()
                .map(Cow::into_owned)
                .collect::<Vec<_>>();
            assert_eq!(error_chain(&error), original_chain, "{name}");
            outcomes.push(format!(
                "{name}: {}",
                if hints.is_empty() {
                    "none".to_string()
                } else {
                    hints.join("; ")
                }
            ));
        }
        insta::assert_snapshot!(outcomes.join("\n"), @"
        system destination: It looks like you do not have permission to write to the system Python environment. Consider creating a virtual environment with `uv venv`, then retry the installation
        virtual environment: none
        target: none
        prefix: none
        cached wheel read: none
        ambiguous copy: none
        missing destination: none
        other destination error: none
        ");
        Ok(())
    }
}
