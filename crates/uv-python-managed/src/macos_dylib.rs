use std::{io::ErrorKind, path::PathBuf};

use uv_errors::{Hinted, Hints};
use uv_fs::Simplified as _;
use uv_warnings::warn_user_with_chain;

use crate::managed::ManagedPythonInstallation;

pub(crate) fn patch_dylib_install_name(dylib: PathBuf) -> Result<(), Error> {
    let output = match std::process::Command::new("install_name_tool")
        .arg("-id")
        .arg(&dylib)
        .arg(&dylib)
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            let e = if e.kind() == ErrorKind::NotFound {
                Error::MissingInstallNameTool
            } else {
                e.into()
            };
            return Err(e);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(Error::RenameError { dylib, stderr });
    }

    Ok(())
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("`install_name_tool` is not available on this system")]
    MissingInstallNameTool,
    #[error("Failed to update the install name of the Python dynamic library located at `{}`{}", dylib.user_display(), format_stderr(stderr))]
    RenameError { dylib: PathBuf, stderr: String },
}

impl Hinted for Error {
    fn hints(&self) -> Hints<'_> {
        match self {
            Self::MissingInstallNameTool => {
                Hints::from("Install the Xcode Command Line Tools with `xcode-select --install`.")
            }
            Self::Io(_) | Self::RenameError { .. } => Hints::none(),
        }
    }
}

fn format_stderr(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        return String::new();
    }
    if let Some((end, _)) = stderr.char_indices().nth(4096) {
        format!("\n\n[stderr]\n{}\n[output truncated]", &stderr[..end])
    } else {
        format!("\n\n[stderr]\n{stderr}")
    }
}

impl Error {
    /// Emit a user-friendly warning about the patching failure.
    pub fn warn_user(self, installation: &ManagedPythonInstallation) {
        let hints = self.hints().into_owned();
        warn_user_with_chain!(
            anyhow::Error::new(self).context(format!(
                "Failed to patch the install name of the dynamic library for `{}`. This may cause issues when building Python native extensions.",
                installation.executable(false).simplified_display(),
            )).as_ref(),
            hints
        );
    }
}
