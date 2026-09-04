use std::io;
use std::path::Path;

use console::Term;
use owo_colors::OwoColorize;
use tracing::debug;
use uv_fs::{ClearNonVirtualenv, Simplified};

use crate::Error;

/// Why an existing environment is being removed.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RemovalReason {
    /// The removal was explicitly requested with `--clear`.
    UserRequest,
    /// The user confirmed an interactive replacement prompt.
    UserConfirmation,
    /// The removal is for a temporary environment, e.g., a build environment.
    TemporaryEnvironment,
    /// The removal is part of managing a project, script, or tool environment.
    ManagedEnvironment,
}

impl std::fmt::Display for RemovalReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserRequest => f.write_str("requested with `--clear`"),
            Self::UserConfirmation => f.write_str("confirmed by the user"),
            Self::TemporaryEnvironment => f.write_str("environment is temporary"),
            Self::ManagedEnvironment => f.write_str("environment is managed by uv"),
        }
    }
}

/// The intent and permissions for removing an existing environment.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Removal {
    /// The diagnostic context, independent of permission to remove user data.
    pub reason: RemovalReason,
    /// Whether an entry without a virtual environment marker may be removed.
    pub clear_non_virtualenv: ClearNonVirtualenv,
}

impl Removal {
    /// Remove the environment entry without following a link at `location`.
    pub fn remove(self, location: &Path) -> io::Result<()> {
        debug!(
            "Removing existing environment at {} ({})",
            location.user_display(),
            self.reason
        );
        uv_fs::remove_virtualenv(location, self.clear_non_virtualenv)
    }

    /// Clear an environment, preserving links to its directory.
    fn clear(self, location: &Path) -> io::Result<bool> {
        debug!(
            "Clearing existing environment at {} ({})",
            location.user_display(),
            self.reason
        );
        uv_fs::clear_virtualenv(location, self.clear_non_virtualenv)
    }

    fn check(self, location: &Path, is_virtualenv: bool) -> Result<(), Error> {
        match self.clear_non_virtualenv {
            ClearNonVirtualenv::Allow => Ok(()),
            ClearNonVirtualenv::Error if is_virtualenv => Ok(()),
            ClearNonVirtualenv::Error => Err(Error::ClearNonVirtualenv {
                path: location.to_path_buf(),
            }),
        }
    }
}

/// Whether creation initializes a destination or replaces an existing entry.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum CreationAction {
    /// Initialize the destination without removing an existing entry.
    Create,
    /// Remove an existing entry before initializing the destination.
    Replace,
}

/// An event emitted while creating a virtual environment.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum CreationEvent {
    /// An existing entry was successfully removed.
    Removed,
    /// Existing-entry handling has finished and creation is about to begin.
    Creating,
}

/// How to handle an existing virtual environment destination.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum OnExisting {
    /// Prompt before clearing a virtual environment. Fail without a TTY.
    #[default]
    Prompt,
    /// Fail if the destination already exists and is non-empty.
    Fail,
    /// Overwrite virtual environment files while retaining other existing files.
    Allow,
    /// Clear the environment directory, preserving links to it.
    Clear(Removal),
    /// Replace the destination entry without following a link to another directory.
    Replace(Removal),
}

/// A read-only decision. It is never retained as authorization for a later removal.
enum ExistingAction {
    Create,
    Allow,
    Clear(Removal),
    Replace(Removal),
    Prompt,
}

impl OnExisting {
    pub fn from_args(
        allow_existing: bool,
        clear: bool,
        no_clear: bool,
        clear_non_virtualenv: ClearNonVirtualenv,
    ) -> Self {
        if allow_existing {
            Self::Allow
        } else if clear {
            Self::Clear(Removal {
                reason: RemovalReason::UserRequest,
                clear_non_virtualenv,
            })
        } else if no_clear {
            Self::Fail
        } else {
            Self::Prompt
        }
    }

    /// Check the destination without modifying it or prompting.
    ///
    /// Creation repeats this check and enforces the removal policy at the filesystem operation.
    pub fn check(self, location: &Path) -> Result<CreationAction, Error> {
        Ok(match self.action(location)? {
            ExistingAction::Create | ExistingAction::Allow => CreationAction::Create,
            ExistingAction::Clear(_) | ExistingAction::Replace(_) | ExistingAction::Prompt => {
                CreationAction::Replace
            }
        })
    }

    fn action(self, location: &Path) -> Result<ExistingAction, Error> {
        let inspect_error = |source| Error::InspectExisting {
            path: location.to_path_buf(),
            source,
        };
        let entry = match fs_err::symlink_metadata(location) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(ExistingAction::Create);
            }
            Err(err) => return Err(inspect_error(err)),
        };

        // An explicitly owned entry can be replaced without inspecting its target. This also
        // handles dangling links and centralized-environment path files.
        if let Self::Replace(removal) = self
            && removal.clear_non_virtualenv == ClearNonVirtualenv::Allow
            && !entry.is_dir()
        {
            return Ok(ExistingAction::Replace(removal));
        }

        let metadata = if entry.file_type().is_symlink() {
            match fs_err::metadata(location) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    if let Self::Replace(removal) = self {
                        removal.check(location, false)?;
                    }
                    return Ok(ExistingAction::Create);
                }
                Err(err) => return Err(inspect_error(err)),
            }
        } else {
            entry
        };
        if !metadata.is_dir() {
            if let Self::Replace(removal) = self {
                removal.check(location, false)?;
            }
            let message = if metadata.is_file() {
                "File exists"
            } else {
                "Object already exists"
            };
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{message} at `{}`", location.user_display()),
            )));
        }
        if fs_err::read_dir(location)
            .map_err(&inspect_error)?
            .next()
            .transpose()
            .map_err(&inspect_error)?
            .is_none()
        {
            return Ok(ExistingAction::Create);
        }

        // A uv-owned destination can be repaired even if its marker cannot be inspected.
        let is_virtualenv = match self {
            Self::Clear(removal) | Self::Replace(removal)
                if removal.clear_non_virtualenv == ClearNonVirtualenv::Allow =>
            {
                false
            }
            Self::Allow => false,
            Self::Prompt | Self::Fail | Self::Clear(_) | Self::Replace(_) => {
                uv_fs::try_is_virtualenv_base(location).map_err(inspect_error)?
            }
        };
        let exists = || Error::Exists {
            name: if is_virtualenv {
                "virtual environment"
            } else {
                "directory"
            },
            path: location.to_path_buf(),
        };
        match self {
            Self::Allow => Ok(ExistingAction::Allow),
            Self::Fail => Err(exists()),
            Self::Prompt if is_virtualenv => Ok(ExistingAction::Prompt),
            Self::Prompt => Err(exists()),
            Self::Clear(removal) => {
                removal.check(location, is_virtualenv)?;
                Ok(ExistingAction::Clear(removal))
            }
            Self::Replace(removal) => {
                removal.check(location, is_virtualenv)?;
                Ok(ExistingAction::Replace(removal))
            }
        }
    }

    pub(crate) fn prepare(
        self,
        location: &Path,
        reporter: &mut impl FnMut(CreationEvent) -> io::Result<()>,
    ) -> Result<CreationAction, Error> {
        let replaced = match self.action(location)? {
            ExistingAction::Create => false,
            ExistingAction::Allow => {
                debug!("Allowing existing directory due to `--allow-existing`");
                false
            }
            ExistingAction::Clear(removal) => removal.clear(location)?,
            ExistingAction::Replace(removal) => match removal.remove(location) {
                Ok(()) => true,
                Err(err) if err.kind() == io::ErrorKind::NotFound => false,
                Err(err) => return Err(err.into()),
            },
            ExistingAction::Prompt => match confirm_clear(location)? {
                Some(true) => Removal {
                    reason: RemovalReason::UserConfirmation,
                    clear_non_virtualenv: ClearNonVirtualenv::Error,
                }
                .clear(location)?,
                Some(false) | None => {
                    return Err(Error::Exists {
                        name: "virtual environment",
                        path: location.to_path_buf(),
                    });
                }
            },
        };
        if replaced {
            reporter(CreationEvent::Removed)?;
        }
        reporter(CreationEvent::Creating)?;
        fs_err::create_dir_all(location)?;
        Ok(if replaced {
            CreationAction::Replace
        } else {
            CreationAction::Create
        })
    }
}

/// If not a TTY, returns `None`.
fn confirm_clear(location: &Path) -> io::Result<Option<bool>> {
    let term = Term::stderr();
    if term.is_term() {
        let prompt = format!(
            "A virtual environment already exists at `{}`. Do you want to replace it?",
            location.user_display(),
        );
        let hint = format!(
            "Use the `{}` flag or set `{}` to skip this prompt",
            "--clear".green(),
            "UV_VENV_CLEAR=1".green()
        );
        Ok(Some(uv_console::confirm_with_hint(
            &prompt, &hint, &term, true,
        )?))
    } else {
        Ok(None)
    }
}
