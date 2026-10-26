use std::io::Write;
use std::path::Path;

use serde::Deserialize;

pub use uv_git::project::VersionControlError;

/// The version control system to use.
#[derive(Clone, Copy, Debug, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum VersionControlSystem {
    /// Use Git for version control.
    #[default]
    Git,
    /// Do not use any version control system.
    None,
}

impl VersionControlSystem {
    /// Initializes the VCS system based on the provided path.
    pub fn init(&self, path: &Path) -> Result<(), VersionControlError> {
        match self {
            Self::Git => {
                uv_git::project::init(path)?;

                // Create the `.gitignore`, if it doesn't exist.
                match fs_err::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path.join(".gitignore"))
                {
                    Ok(mut file) => file.write_all(GITIGNORE.as_bytes())?,
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => (),
                    Err(err) => return Err(err.into()),
                }

                Ok(())
            }
            Self::None => Ok(()),
        }
    }
}

impl std::fmt::Display for VersionControlSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git => write!(f, "git"),
            Self::None => write!(f, "none"),
        }
    }
}

const GITIGNORE: &str = "# Python-generated files
__pycache__/
*.py[oc]
build/
dist/
wheels/
*.egg-info

# Virtual environments
.venv
";

/// Setting for Git LFS (Large File Storage) support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GitLfsSetting {
    /// Git LFS is disabled (default).
    #[default]
    Disabled,
    /// Git LFS is enabled. Tracks whether it came from an environment variable.
    Enabled { from_env: bool },
}

impl GitLfsSetting {
    pub fn new(from_arg: Option<bool>, from_env: Option<bool>) -> Self {
        match (from_arg, from_env) {
            (Some(true), _) => Self::Enabled { from_env: false },
            (_, Some(true)) => Self::Enabled { from_env: true },
            _ => Self::Disabled,
        }
    }
}

impl From<GitLfsSetting> for Option<bool> {
    fn from(setting: GitLfsSetting) -> Self {
        match setting {
            GitLfsSetting::Enabled { .. } => Some(true),
            GitLfsSetting::Disabled => None,
        }
    }
}
