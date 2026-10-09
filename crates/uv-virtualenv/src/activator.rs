use std::borrow::Cow;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use itertools::Itertools;

use uv_fs::{PythonExt, Simplified, persist_with_retry_sync};
use uv_pypi_types::Scheme;
use uv_python::PythonEnvironment;
use uv_shell::escape_posix_for_single_quotes;

use crate::Error;

#[cfg(test)]
mod tests;

/// An unchanged generated activator whose contents now refer to the final environment.
#[derive(Debug)]
pub struct ActivatorUpdate {
    path: PathBuf,
    before: String,
    after: String,
}

impl ActivatorUpdate {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn before(&self) -> &[u8] {
        self.before.as_bytes()
    }

    pub fn after(&self) -> &[u8] {
        self.after.as_bytes()
    }
}

/// Finalize unchanged generated activators in a private tool environment before publication.
///
/// Tool environments use [`crate::Prompt::None`]. Builds and bytecode compilation must finish
/// before this operation. Modified or unrecognized activation files are left unchanged.
/// Returned updates can reconcile package RECORD entries that include a generated activator.
pub fn finalize_activators(
    environment: &PythonEnvironment,
    final_root: &Path,
) -> Result<Vec<ActivatorUpdate>, Error> {
    finalize(
        environment.root(),
        environment.scripts(),
        environment.interpreter().virtualenv(),
        &std::path::absolute(final_root)?,
        environment.relocatable(),
    )
}

fn finalize(
    root: &Path,
    scripts: &Path,
    scheme: &Scheme,
    final_root: &Path,
    relocatable: bool,
) -> Result<Vec<ActivatorUpdate>, Error> {
    if !fs_err::symlink_metadata(scripts)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Activation script directory is not a directory: {}",
                scripts.display()
            ),
        )
        .into());
    }
    let original = render(root, scheme, None, relocatable)?;
    let finalized = render(final_root, scheme, None, relocatable)?;
    let mut updates = Vec::new();
    for ((name, original), (_, finalized)) in original.into_iter().zip(finalized) {
        if original == finalized {
            continue;
        }
        let path = scripts.join(name);
        let metadata = match fs_err::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        if !metadata.is_file()
            || (metadata.len() != original.len() as u64 && metadata.len() != finalized.len() as u64)
        {
            continue;
        }
        let contents = fs_err::read(&path)?;
        if contents != original.as_bytes() && contents != finalized.as_bytes() {
            continue;
        }
        if contents != finalized.as_bytes() {
            let mut temporary = uv_fs::tempfile_in(scripts)?;
            temporary
                .as_file()
                .set_permissions(metadata.permissions())?;
            temporary.write_all(finalized.as_bytes())?;
            persist_with_retry_sync(temporary, &path)?;
        }
        updates.push(ActivatorUpdate {
            path,
            before: original,
            after: finalized,
        });
    }
    Ok(updates)
}

/// Activation scripts for the environment, with dependent paths templated out.
const ACTIVATE_TEMPLATES: &[(&str, &str)] = &[
    ("activate", include_str!("activator/activate")),
    ("activate.csh", include_str!("activator/activate.csh")),
    ("activate.fish", include_str!("activator/activate.fish")),
    ("activate.nu", include_str!("activator/activate.nu")),
    ("activate.xsh", include_str!("activator/activate.xsh")),
    ("activate.ps1", include_str!("activator/activate.ps1")),
    ("activate.bat", include_str!("activator/activate.bat")),
    ("deactivate.bat", include_str!("activator/deactivate.bat")),
    ("pydoc.bat", include_str!("activator/pydoc.bat")),
    (
        "activate_this.py",
        include_str!("activator/activate_this.py"),
    ),
];

pub(super) fn render(
    location: &Path,
    scheme: &Scheme,
    prompt: Option<&str>,
    relocatable: bool,
) -> Result<Vec<(&'static str, String)>, Error> {
    let bin_name = if cfg!(windows) { "Scripts" } else { "bin" };
    let mut scripts = Vec::new();
    for (name, template) in ACTIVATE_TEMPLATES {
        // csh has no way to determine its own script location, so a relocatable
        // activate.csh is not possible. Skip it entirely instead of generating a
        // non-functional script.
        if relocatable && *name == "activate.csh" {
            continue;
        }

        let path_sep = if cfg!(windows) { ";" } else { ":" };

        let relative_site_packages = [scheme.purelib.as_path(), scheme.platlib.as_path()]
            .iter()
            .dedup()
            .map(|path| {
                pathdiff::diff_paths(path, &scheme.scripts)
                    .expect("Failed to calculate relative path to site-packages")
            })
            .map(|path| path.simplified().to_str().unwrap().replace('\\', "\\\\"))
            .join(path_sep);

        let location_string = location
            .simplified()
            .to_str()
            .ok_or_else(|| Error::NonUtf8Path {
                path: location.to_path_buf(),
            })?;
        let virtual_env_dir = match (relocatable, name.to_owned()) {
            (true, "activate") => Cow::Borrowed(
                r#"'"$(dirname -- "$(dirname -- "$(realpath -- "$SCRIPT_PATH")")")"'"#,
            ),
            (true, "activate.bat") => Cow::Borrowed(r"%~dp0.."),
            (true, "activate.fish") => {
                Cow::Borrowed(r"'(dirname -- (dirname -- (realpath -- (status -f))))'")
            }
            (true, "activate.nu") => Cow::Borrowed(r"(path self | path dirname | path dirname)"),
            (false, "activate.nu") => Cow::Owned(format!(
                "'{}'",
                escape_posix_for_single_quotes(location_string)
            )),
            // Note: `activate.ps1` is already relocatable by default.
            _ => escape_posix_for_single_quotes(location_string),
        };

        let virtual_prompt = prompt.unwrap_or_default();
        let virtual_prompt = match *name {
            "activate.xsh" => Cow::Owned(format!(
                r#"b"{}".decode("utf-8")"#,
                virtual_prompt.as_bytes().escape_ascii(),
            )),
            _ => Cow::Borrowed(virtual_prompt),
        };

        let bin_name = match *name {
            "activate.xsh" => Cow::Owned(bin_name.escape_for_python()),
            _ => Cow::Borrowed(bin_name),
        };

        let activator = template
            .replace("{{ VIRTUAL_ENV_DIR }}", &virtual_env_dir)
            .replace("{{ BIN_NAME }}", &bin_name)
            .replace("{{ VIRTUAL_PROMPT }}", &virtual_prompt)
            .replace("{{ PATH_SEP }}", path_sep)
            .replace("{{ RELATIVE_SITE_PACKAGES }}", &relative_site_packages);
        scripts.push((*name, activator));
    }

    Ok(scripts)
}
