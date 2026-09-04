use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use uv_fs::Simplified;
use uv_python::{Interpreter, PythonEnvironment};

pub use existing::{CreationAction, CreationEvent, OnExisting, Removal, RemovalReason};
pub use uv_fs::ClearNonVirtualenv;
pub use virtualenv::Seed;

mod existing;
mod virtualenv;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Failed to inspect virtual environment directory at {}", path.user_display())]
    InspectExisting {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "Could not find a suitable Python executable for the virtual environment based on the interpreter: {0}"
    )]
    NotFound(String),
    #[error(transparent)]
    Python(#[from] uv_python::managed::Error),
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

impl uv_errors::Hint for Error {
    fn hints(&self) -> uv_errors::Hints<'_> {
        match self {
            Self::Exists { name, .. } => uv_errors::Hints::from(format!(
                "Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing {name}",
            )),
            Self::ClearNonVirtualenv { .. } => uv_errors::Hints::from(
                "Use the `--force` flag to remove the existing directory anyway",
            ),
            Self::Io(_)
            | Self::InspectExisting { .. }
            | Self::NotFound(_)
            | Self::Python(_)
            | Self::NonUtf8Path { .. } => uv_errors::Hints::none(),
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
    upgradeable: bool,
) -> Result<PythonEnvironment, Error> {
    create_venv_with_reporter(
        location,
        interpreter,
        prompt,
        system_site_packages,
        on_existing,
        relocatable,
        seed,
        upgradeable,
        |_| Ok(()),
    )
    .map(CreatedVenv::into_environment)
}

/// The result of creating a virtual environment.
#[derive(Debug)]
pub enum CreatedVenv {
    Created(PythonEnvironment),
    Replaced(PythonEnvironment),
}

impl CreatedVenv {
    pub fn environment(&self) -> &PythonEnvironment {
        match self {
            Self::Created(environment) | Self::Replaced(environment) => environment,
        }
    }

    pub fn into_environment(self) -> PythonEnvironment {
        match self {
            Self::Created(environment) | Self::Replaced(environment) => environment,
        }
    }
}

/// Create a virtualenv and report the actual removal and creation events.
pub fn create_venv_with_reporter(
    location: &Path,
    interpreter: Interpreter,
    prompt: Prompt,
    system_site_packages: bool,
    on_existing: OnExisting,
    relocatable: bool,
    seed: Seed,
    upgradeable: bool,
    mut reporter: impl FnMut(CreationEvent) -> io::Result<()>,
) -> Result<CreatedVenv, Error> {
    // Create the virtualenv at the given location.
    let (virtualenv, action) = virtualenv::create(
        location,
        &interpreter,
        prompt,
        system_site_packages,
        on_existing,
        relocatable,
        seed,
        upgradeable,
        &mut reporter,
    )?;

    // Create the corresponding `PythonEnvironment`.
    let interpreter = interpreter.with_virtualenv(virtualenv);
    let environment = PythonEnvironment::from_interpreter(interpreter);
    Ok(match action {
        CreationAction::Create => CreatedVenv::Created(environment),
        CreationAction::Replace => CreatedVenv::Replaced(environment),
    })
}
