use std::path::PathBuf;

use owo_colors::OwoColorize;
use thiserror::Error;
use tracing::debug;

use uv_errors::{Hinted, Hints};
use uv_fs::Simplified;
use uv_python_interpreter::PythonEnvironment;

/// The mutation determines which command-specific advice accompanies a refusal.
#[derive(Clone, Copy)]
pub(crate) enum PipMutation {
    Install { system: bool },
    Sync,
    Uninstall,
}

/// The interpreter is externally managed and cannot be modified.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct ExternallyManagedError {
    message: String,
    root: PathBuf,
    system: bool,
}

impl Hinted for ExternallyManagedError {
    fn hints(&self) -> Hints<'_> {
        if self.system {
            Hints::from("Virtual environments were not considered due to the `--system` flag")
        } else {
            Hints::from("Consider creating a virtual environment, e.g., with `uv venv`")
        }
    }
}

/// Check whether the selected environment allows package changes under PEP 668.
///
/// Call after applying a target or prefix, before acquiring the environment lock.
pub(crate) fn check_externally_managed(
    environment: &PythonEnvironment,
    break_system_packages: bool,
    mutation: PipMutation,
) -> anyhow::Result<()> {
    let Some(externally_managed) = environment.interpreter().is_externally_managed() else {
        return Ok(());
    };
    if break_system_packages {
        debug!("Ignoring externally managed environment due to `--break-system-packages`");
        return Ok(());
    }

    let root = environment.root();
    let message = match externally_managed.into_error() {
        Some(error) => format!(
            "The interpreter at `{}` is externally managed, and indicates the following:\n\n{}\n",
            root.user_display().cyan(),
            textwrap::indent(&error, "  ").green(),
        ),
        None => match mutation {
            PipMutation::Install { .. } => format!(
                "The interpreter at `{}` is externally managed and cannot be modified.",
                root.user_display().cyan()
            ),
            PipMutation::Sync | PipMutation::Uninstall => {
                return Err(anyhow::anyhow!(
                    "The interpreter at `{}` is externally managed. Instead, create a virtual environment with `uv venv`.",
                    root.user_display().cyan()
                ));
            }
        },
    };

    match mutation {
        PipMutation::Install { system } => Err(ExternallyManagedError {
            message,
            root: root.to_path_buf(),
            system,
        }
        .into()),
        PipMutation::Sync | PipMutation::Uninstall => Err(anyhow::anyhow!(
            "{message}\nConsider creating a virtual environment with `uv venv`."
        )),
    }
}
