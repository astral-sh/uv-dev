use std::fmt::Write;
use std::path::Path;

use anyhow::{Context, Result};
use futures::{StreamExt, TryStreamExt, stream};
use uv_cache::Cache;
use uv_client::{BaseClientBuilder, PackedArchive, RegistryClientBuilder};
use uv_configuration::Concurrency;
use uv_warnings::warn_user;
use uv_workspace::{DiscoveryOptions, MemberDiscovery, VirtualProject, WorkspaceCache};

use crate::commands::ExitStatus;
use crate::commands::project::lock_target::LockTarget;
use crate::printer::Printer;
use crate::settings::ResolverSettings;

/// Populate the packed cache from the existing universal lockfile.
pub(crate) async fn download(
    project_dir: &Path,
    settings: ResolverSettings,
    client_builder: BaseClientBuilder<'_>,
    concurrency: Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
) -> Result<ExitStatus> {
    let project = VirtualProject::discover(
        project_dir,
        &DiscoveryOptions {
            members: MemberDiscovery::None,
            ..DiscoveryOptions::default()
        },
        cache,
        workspace_cache,
    )
    .await?;
    let target = LockTarget::Workspace(project.workspace());
    let lock = target
        .read()
        .await?
        .context("No uv.lock found; run `uv lock` first")?;
    let client = RegistryClientBuilder::new(client_builder, cache.clone())
        .index_locations(settings.index_locations)
        .index_strategy(settings.index_strategy)
        .keyring(settings.keyring_provider)
        .build()?;
    let mut artifacts = Vec::new();
    for package in lock.packages() {
        if package.git_sha().is_some() {
            warn_user!(
                "Git source `{}` is not included in the packed archive cache",
                package.name()
            );
        }
        for artifact in package.artifacts(target.install_path())? {
            artifacts.push((package.name().clone(), artifact));
        }
    }
    let count = artifacts.len();
    let downloaded = stream::iter(artifacts)
        .map(|(name, artifact)| {
            let client = &client;
            async move {
                PackedArchive::download(
                    cache,
                    client,
                    &name,
                    &artifact.url,
                    artifact.hash.as_ref(),
                    artifact.size,
                )
                .await
                .with_context(|| format!("Failed to download `{name}` from {}", artifact.url))
            }
        })
        .buffer_unordered(concurrency.downloads)
        .try_fold(0usize, async |count, downloaded| {
            Ok(count + usize::from(downloaded))
        })
        .await?;
    writeln!(
        printer.stderr(),
        "Downloaded {downloaded} distributions ({count} total)"
    )?;
    Ok(ExitStatus::Success)
}
