#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use uv_fs::{Simplified, relative_to};
use uv_python_interpreter::{PyVenvConfiguration, virtualenv_python_executable};
use walkdir::WalkDir;

/// Clone the files of a quiescent, relocatable uv environment into a new directory.
pub fn clone_venv(source: &Path, destination: &Path) -> Result<()> {
    let source = fs_err::canonicalize(source)?;
    let cfg = PyVenvConfiguration::parse(source.join("pyvenv.cfg"))?;
    ensure!(
        cfg.is_uv() && cfg.is_relocatable(),
        "The source must be a virtual environment created with `uv venv --relocatable`"
    );
    let python = virtualenv_python_executable(&source);
    let scripts = python.parent().context("Missing interpreter directory")?;
    let base_python = fs_err::canonicalize(&python)?;
    let destination = std::path::absolute(destination)?;
    let parent = destination.parent().context("Missing destination parent")?;
    let destination = fs_err::canonicalize(parent)?.join(
        destination
            .file_name()
            .context("Missing destination name")?,
    );
    ensure!(
        !destination.starts_with(&source),
        "The destination must be outside the source environment"
    );

    // Reserve the new path before copying so simultaneous clones cannot overwrite one another.
    #[cfg(unix)]
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&destination)
        .with_context(|| format!("Failed to create `{}`", destination.user_display()))?;
    #[cfg(not(unix))]
    fs_err::create_dir(&destination)?;
    let result = copy_tree(&source, &destination, scripts, &base_python);
    if result.is_err() {
        fs_err::remove_dir_all(&destination)?;
    }
    result
}

fn copy_tree(source: &Path, destination: &Path, scripts: &Path, base_python: &Path) -> Result<()> {
    #[cfg(unix)]
    let mut directory_permissions = Vec::new();
    for entry in WalkDir::new(source).min_depth(1) {
        let entry = entry?;
        let path = entry.path();
        let target = destination.join(path.strip_prefix(source)?);
        if entry.file_type().is_dir() {
            fs_err::create_dir(&target)?;
            #[cfg(unix)]
            directory_permissions.push((target, entry.metadata()?.permissions()));
        } else if entry.file_type().is_file() {
            reflink_copy::reflink_or_copy(path, &target)
                .with_context(|| format!("Failed to clone `{}`", path.user_display()))?;
            // CPython validates timestamp-based bytecode against the source file's mtime and size.
            let modified = filetime::FileTime::from_last_modification_time(&entry.metadata()?);
            filetime::set_file_mtime(target, modified)?;
        } else if entry.file_type().is_symlink() {
            let resolved = fs_err::canonicalize(path)?;
            let referent = if resolved.starts_with(source) {
                relative_to(&resolved, path.parent().context("Missing symlink parent")?)?
            } else if path.parent() == Some(scripts) && resolved == base_python {
                resolved.clone()
            } else {
                bail!(
                    "Symlink `{}` points outside the source environment",
                    path.user_display()
                );
            };
            #[cfg(unix)]
            fs_err::os::unix::fs::symlink(referent, target)?;
            #[cfg(windows)]
            if resolved.is_dir() {
                fs_err::os::windows::fs::symlink_dir(referent, target)?;
            } else {
                fs_err::os::windows::fs::symlink_file(referent, target)?;
            }
        } else {
            bail!("Cannot clone special file `{}`", path.user_display());
        }
    }
    // Apply directory modes after populating their children, including read-only directories.
    #[cfg(unix)]
    for (path, permissions) in directory_permissions.into_iter().rev() {
        fs_err::set_permissions(path, permissions)?;
    }
    Ok(())
}
