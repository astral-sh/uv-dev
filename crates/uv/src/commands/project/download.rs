use std::fmt::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use futures::{StreamExt, TryStreamExt, stream};
use uv_cache::Cache;
use uv_client::{BaseClientBuilder, PackedArchive, RegistryClientBuilder};
use uv_configuration::Concurrency;
use uv_distribution_types::HashPolicy;
use uv_preview::{Preview, PreviewFeature};
use uv_warnings::warn_user;
use uv_workspace::{DiscoveryOptions, MemberDiscovery, VirtualProject, WorkspaceCache};

use crate::commands::ExitStatus;
use crate::commands::project::install_target::InstallTarget;
use crate::commands::project::lock_target::LockTarget;
use crate::commands::project::sync::store_credentials_from_target;
use crate::printer::Printer;
use crate::settings::ResolverSettings;

mod requirements;

/// Populate the packed cache from the existing universal lockfile.
pub(crate) async fn download(
    project_dir: &Path,
    requirements: &[PathBuf],
    settings: ResolverSettings,
    client_builder: BaseClientBuilder<'_>,
    concurrency: Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    if !preview.is_enabled(PreviewFeature::DownloadCommand) {
        warn_user!(
            "`uv download` is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::DownloadCommand
        );
    }

    if !requirements.is_empty() {
        return requirements::download(
            requirements,
            settings,
            client_builder,
            concurrency,
            cache,
            printer,
        )
        .await;
    }
    let project = VirtualProject::discover(
        project_dir,
        &DiscoveryOptions {
            members: MemberDiscovery::Existing,
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
    store_credentials_from_target(
        InstallTarget::Workspace {
            workspace: project.workspace(),
            lock: &lock,
        },
        &client_builder,
    )?;
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
                    artifact.hash.as_ref().map_or(HashPolicy::None, |hash| {
                        HashPolicy::All(std::slice::from_ref(hash))
                    }),
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
