//! Git operations for initializing a uv project.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use uv_preview::PreviewFeature;
use uv_static::EnvVars;

use crate::{GIT, gitoxide};

#[derive(Debug, thiserror::Error)]
pub enum VersionControlError {
    #[error("Attempted to initialize a Git repository, but `git` was not found in PATH")]
    GitNotInstalled,
    #[error("Failed to initialize Git repository at `{0}`\nstdout: {1}\nstderr: {2}")]
    GitInit(PathBuf, String, String),
    #[error("`git` command failed")]
    GitCommand(#[source] std::io::Error),
    #[error("Failed to initialize Git repository at `{0}`")]
    NativeInit(PathBuf, #[source] gix::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn discover(path: &Path) -> Result<Option<gix::Repository>> {
    match gix::discover_opts(
        path,
        Default::default(),
        gitoxide::options().bail_if_untrusted(true),
    ) {
        Ok(repo) => Ok(Some(repo)),
        Err(err)
            if err.is_not_found()
                && err
                    .downcast_any_ref::<gix::discover::upwards::Error>()
                    .is_some() =>
        {
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

/// Return whether the project directory belongs to a Git worktree.
pub fn inside_worktree(path: &Path) -> Result<bool> {
    if uv_preview::is_enabled(PreviewFeature::Gitoxide) {
        let Some(repo) = discover(path)? else {
            return Ok(false);
        };
        ensure!(
            repo.workdir().is_some()
                && !path
                    .canonicalize()?
                    .starts_with(repo.git_dir().canonicalize()?),
            "inside Git metadata"
        );
        return Ok(true);
    }
    let output = GIT
        .as_ref()
        .cloned()?
        .build_command()
        .args(["rev-parse", "--is-inside-work-tree"])
        .env(EnvVars::LC_ALL, "C")
        .current_dir(path)
        .output()?;
    if output.status.success() && output.stdout.trim_ascii() == b"true" {
        return Ok(true);
    }
    if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("not a git repository")
    {
        return Ok(false);
    }
    anyhow::bail!("Git worktree discovery failed for {}", path.display())
}

/// Create a Git repository in the project directory.
pub fn init(path: &Path) -> Result<(), VersionControlError> {
    if uv_preview::is_enabled(PreviewFeature::Gitoxide) {
        gitoxide::init(path)
            .map_err(|err| VersionControlError::NativeInit(path.to_owned(), err))?;
        return Ok(());
    }
    let git = GIT
        .as_ref()
        .map_err(|_| VersionControlError::GitNotInstalled)?;
    let output = git
        .build_command()
        .arg("init")
        .current_dir(path)
        .output()
        .map_err(VersionControlError::GitCommand)?;
    if !output.status.success() {
        return Err(VersionControlError::GitInit(
            path.to_owned(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}

/// Read the configured author name and email, including project-local overrides.
pub fn author(path: &Path) -> Result<(Option<String>, Option<String>)> {
    if uv_preview::is_enabled(PreviewFeature::Gitoxide) {
        let repo = discover(path)?;
        let global;
        let snapshot;
        let config = if let Some(repo) = &repo {
            snapshot = repo.config_snapshot();
            snapshot.plumbing()
        } else {
            global = gix::config(None, &gitoxide::options())?;
            &global
        };
        let value = |key: &str| {
            config
                .string(key)
                .map(|s| String::from_utf8_lossy(&s).trim().to_owned())
        };
        return Ok((value("user.name"), value("user.email")));
    }
    let git = GIT.as_ref().cloned().context("`git` not found in PATH")?;
    let value = |key| -> Result<_> {
        let output = git
            .build_command()
            .args(["config", "--get", key])
            .current_dir(path)
            .output()?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned()))
    };
    Ok((value("user.name")?, value("user.email")?))
}
