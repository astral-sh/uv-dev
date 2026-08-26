use std::fmt::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
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

fn output_path(directory: &Path, filename: &str) -> Result<PathBuf> {
    if !matches!(
        Path::new(filename)
            .components()
            .collect::<Vec<_>>()
            .as_slice(),
        [std::path::Component::Normal(_)]
    ) {
        bail!("Distribution has an invalid filename: `{filename}`");
    }
    Ok(directory.join(filename))
}

/// Populate the packed cache from the existing universal lockfile.
pub(crate) async fn download(
    project_dir: &Path,
    requirements: &[PathBuf],
    output_dir: Option<&Path>,
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

    if let Some(output_dir) = output_dir {
        fs_err::tokio::create_dir_all(output_dir)
            .await
            .with_context(|| format!("Failed to create `{}`", output_dir.display()))?;
    }

    if !requirements.is_empty() {
        return requirements::download(
            requirements,
            output_dir,
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
            let filename = output_dir
                .map(|output_dir| output_path(output_dir, &artifact.filename))
                .transpose()?;
            artifacts.push((package.name().clone(), artifact, filename));
        }
    }
    if output_dir.is_some() {
        artifacts.sort_unstable_by(|(_, _, left), (_, _, right)| left.cmp(right));
        for pair in artifacts.windows(2) {
            let [
                (_, previous, Some(previous_filename)),
                (_, current, Some(current_filename)),
            ] = pair
            else {
                continue;
            };
            if previous_filename == current_filename {
                let same_artifact = previous.url == current.url
                    && previous.hash == current.hash
                    && previous.size == current.size;
                let same_content = previous.hash.is_some()
                    && previous.hash == current.hash
                    && (previous.size.is_none()
                        || current.size.is_none()
                        || previous.size == current.size);
                if same_artifact || same_content {
                    continue;
                }
                bail!(
                    "Multiple distributions would be written to `{}`: {} and {}",
                    current_filename.display(),
                    previous.url,
                    current.url
                );
            }
        }
        artifacts.dedup_by(|left, right| left.2 == right.2);
    }
    let count = artifacts.len();
    let downloaded = stream::iter(artifacts)
        .map(|(name, artifact, filename)| {
            let client = &client;
            async move {
                let hashes = artifact.hash.as_ref().map_or(HashPolicy::None, |hash| {
                    HashPolicy::All(std::slice::from_ref(hash))
                });
                let downloaded = if let Some(destination) = filename {
                    PackedArchive::download_to(
                        client,
                        &artifact.url,
                        hashes,
                        artifact.size,
                        &destination,
                        cache.must_revalidate_package(&name),
                    )
                    .await
                } else {
                    PackedArchive::download(
                        cache,
                        client,
                        &name,
                        &artifact.url,
                        hashes,
                        artifact.size,
                    )
                    .await
                };
                downloaded
                    .with_context(|| format!("Failed to download `{name}` from {}", artifact.url))
            }
        })
        .buffer_unordered(concurrency.downloads)
        .try_fold(0usize, async |count, downloaded| {
            Ok(count + usize::from(downloaded))
        })
        .await?;
    if let Some(output_dir) = output_dir {
        writeln!(
            printer.stderr(),
            "Downloaded {downloaded} distributions to {} ({count} total)",
            output_dir.display()
        )?;
    } else {
        writeln!(
            printer.stderr(),
            "Downloaded {downloaded} distributions ({count} total)"
        )?;
    }
    Ok(ExitStatus::Success)
}
