use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use uv_fs::Simplified;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
pub use uv_python_managed::UpgradePolicy;

pub use virtualenv::{ClearNonVirtualenv, OnExisting, RemovalReason, Seed};

mod virtualenv;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(
        "Could not find a suitable Python executable for the virtual environment based on the interpreter: {0}"
    )]
    NotFound(String),
    #[error(transparent)]
    Python(#[from] uv_python_managed::Error),
    #[error("A {name} already exists at: {}", path.user_display())]
    Exists {
        /// The type of environment (e.g., "virtual environment" or "directory").
        name: &'static str,
        /// The path to the existing environment.
        path: PathBuf,
    },
    #[error("uv will not clear a directory that is not a virtual environment")]
    ClearNonVirtualenv {
        /// The non-virtual environment directory that would have been cleared.
        path: PathBuf,
    },
    #[error("Virtual environment path is not valid UTF-8: {}", path.user_display())]
    NonUtf8Path {
        /// The non-UTF-8 virtual environment path.
        path: PathBuf,
    },
}

impl uv_errors::Hinted for Error {
    fn hints(&self) -> uv_errors::Hints<'_> {
        match self {
            Self::Exists { name, .. } => uv_errors::Hints::from(format!(
                "Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing {name}",
            )),
            Self::ClearNonVirtualenv { .. } => uv_errors::Hints::from(
                "Use the `--force` flag to remove the existing directory anyway",
            ),
            _ => uv_errors::Hints::none(),
        }
    }
}

/// The value to use for the shell prompt when inside a virtual environment.
#[derive(Debug)]
pub enum Prompt {
    /// Use the current directory name as the prompt.
    CurrentDirectoryName,
    /// Use the fixed string as the prompt.
    Static(String),
    /// Default to no prompt. The prompt is then set by the activator script
    /// to the virtual environment's directory name.
    None,
}

impl Prompt {
    /// Determine the prompt value to be used from the command line arguments.
    pub fn from_args(prompt: Option<String>) -> Self {
        match prompt {
            Some(prompt) if prompt == "." => Self::CurrentDirectoryName,
            Some(prompt) => Self::Static(prompt),
            None => Self::None,
        }
    }
}

/// Create a virtualenv.
pub fn create_venv(
    location: &Path,
    interpreter: Interpreter,
    prompt: Prompt,
    system_site_packages: bool,
    on_existing: OnExisting,
    relocatable: bool,
    seed: Seed,
    upgrade_policy: UpgradePolicy,
) -> Result<PythonEnvironment, Error> {
    PreparedEnvironment::new(
        interpreter,
        prompt,
        system_site_packages,
        relocatable,
        seed,
        upgrade_policy,
    )?
    .create(location, on_existing)
}

/// A virtual environment whose complete configuration has been validated before destination cleanup.
pub struct PreparedEnvironment {
    interpreter: Interpreter,
    configuration: virtualenv::Configuration,
}

impl PreparedEnvironment {
    /// Resolve and validate configuration without modifying the destination.
    pub fn new(
        interpreter: Interpreter,
        prompt: Prompt,
        system_site_packages: bool,
        relocatable: bool,
        seed: Seed,
        upgrade_policy: UpgradePolicy,
    ) -> Result<Self, Error> {
        let configuration = virtualenv::Configuration::new(
            &interpreter,
            prompt,
            system_site_packages,
            relocatable,
            seed,
            upgrade_policy,
        )?;
        Ok(Self {
            interpreter,
            configuration,
        })
    }

    /// Create the environment using its checked configuration.
    pub fn create(
        self,
        location: &Path,
        on_existing: OnExisting,
    ) -> Result<PythonEnvironment, Error> {
        let virtualenv =
            virtualenv::create(location, &self.interpreter, self.configuration, on_existing)?;
        let interpreter = self.interpreter.with_virtualenv(virtualenv);
        Ok(PythonEnvironment::from_interpreter(interpreter))
    }
}
