use std::fmt::Write;
use std::path::Path;

use anyhow::{Result, bail};

use owo_colors::OwoColorize;
use uv_cache::Cache;
use uv_fs::Simplified;
use uv_normalize::PackageName;
use uv_workspace::{DiscoveryOptions, Workspace, WorkspaceCache};

use uv_cli_output::printer::Printer;
use uv_cli_types::exit::ExitStatus;

/// Print the path to the workspace dir
pub async fn dir(
    package_name: Option<PackageName>,
    project_dir: &Path,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
) -> Result<ExitStatus> {
    let workspace = Workspace::discover(
        project_dir,
        &DiscoveryOptions::default(),
        cache,
        workspace_cache,
    )
    .await?;

    let dir = match package_name {
        None => workspace.install_path(),
        Some(package) if let Some(project) = workspace.packages().get(&package) => project.root(),
        Some(package) => {
            bail!("Package `{package}` not found in workspace.")
        }
    };

    writeln!(printer.stdout(), "{}", dir.simplified_display().cyan())?;

    Ok(ExitStatus::Success)
}
