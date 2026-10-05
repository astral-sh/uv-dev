use std::sync::Arc;

use thiserror::Error;

use uv_lock::Lock;
use uv_resolver::ResolveError as ResolverError;

/// An error that occurs when operating on a project
#[derive(Error, Debug)]
pub enum ProjectError {
    #[error(
        "The lockfile at `uv.lock` needs to be updated, but `--locked` was provided. To update the lockfile, run `uv lock`."
    )]
    LockMismatch(Option<Box<Lock>>, Box<Lock>),

    #[error(
        "The lockfile at `uv.lock` needs to be updated, but `--locked` was provided. To update the lockfile, run `uv lock`."
    )]
    LockOutOfDate,

    #[error("Failed to download `{0}`")]
    Download(Box<dyn std::error::Error + Send + Sync>),

    #[error(transparent)]
    Resolve(Arc<ResolverError>),

    #[error("Failed to build `{0}`")]
    Build(
        uv_normalize::PackageName,
        Box<dyn std::error::Error + Send + Sync>,
    ),

    #[error("Failed to wait for file lock")]
    FileLock(Arc<std::io::Error>),

    #[error("Failed to open file `{0}`")]
    FileOpen(std::path::PathBuf, std::io::Error),

    #[error("Failed to open file `{0}`")]
    ReadWrite(std::path::PathBuf, std::io::Error),

    #[error("Failed to write file `{0}`")]
    Write(std::path::PathBuf, std::io::Error),

    #[error(transparent)]
    Requirements(#[from] uv_requirements::Error),

    #[error(transparent)]
    Tool(#[from] uv_tool::Error),

    // TODO: These error types are not present in the current codebase
    // They need to be defined or imported from appropriate locations
    #[error("No site packages found in environment")]
    NoSitePackages,

    #[error(transparent)]
    PythonError(#[from] uv_python::Error),

    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error(transparent)]
    VirtualEnvError(#[from] uv_virtualenv::Error),

    #[error(transparent)]
    Operation(#[from] uv_cli_pip::operations::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}
