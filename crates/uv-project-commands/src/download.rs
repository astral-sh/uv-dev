use std::fmt::Write;
use std::path::Path;

use anyhow::{Context, Result};
use futures::{StreamExt, TryStreamExt, stream};
use uv_cache::Cache;
use uv_client::{BaseClientBuilder, PackedArchiveEntry, RegistryClientBuilder};
use uv_command_support::{ExitStatus, Printer};
use uv_configuration::Concurrency;
use uv_environment_operations::install_target::InstallTarget;
use uv_environment_operations::store_credentials_from_target;
use uv_lock::LockedArtifactKind;
use uv_lock_operations::LockTarget;
use uv_preview::{Preview, PreviewFeature};
use uv_settings::ResolverSettings;
use uv_warnings::warn_user;
use uv_workspace::{DiscoveryOptions, MemberDiscovery, VirtualProject, WorkspaceCache};

/// Populate the packed cache from the existing universal lockfile.
pub async fn download(
    project_dir: &Path,
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
            project_name: project.project_name(),
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
                let (entry, expected_size) = match artifact.kind {
                    LockedArtifactKind::Wheel { filename, index } => (
                        PackedArchiveEntry::wheel(cache, index.as_ref(), &artifact.url, &filename),
                        artifact.size.filter(|_| index.is_none()),
                    ),
                    LockedArtifactKind::Source {
                        extension,
                        registry,
                    } => (
                        PackedArchiveEntry::source(
                            cache,
                            registry.as_ref().map(|(index, version)| (index, version)),
                            &name,
                            &artifact.url,
                            extension,
                        ),
                        artifact.size.filter(|_| registry.is_none()),
                    ),
                };
                entry
                    .download(client, artifact.hash.as_ref(), expected_size)
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
