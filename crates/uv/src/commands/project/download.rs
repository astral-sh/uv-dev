use std::fmt::Write;
use std::path::Path;

use anyhow::{Context, Result};
use futures::{StreamExt, TryStreamExt, stream};
use serde::Serialize;
use uv_cache::Cache;
use uv_cli::DownloadFormat;
use uv_client::{BaseClientBuilder, PackedArchive, RegistryClientBuilder};
use uv_configuration::Concurrency;
use uv_normalize::PackageName;
use uv_preview::{Preview, PreviewFeature};
use uv_resolver::LockedArtifact;
use uv_warnings::warn_user;
use uv_workspace::{DiscoveryOptions, MemberDiscovery, VirtualProject, WorkspaceCache};

use crate::commands::ExitStatus;
use crate::commands::project::lock_target::LockTarget;
use crate::printer::Printer;
use crate::settings::ResolverSettings;

/// Populate the packed cache from the existing universal lockfile.
pub(crate) async fn download(
    project_dir: &Path,
    dry_run: bool,
    output_format: DownloadFormat,
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
    if dry_run {
        let distributions = stream::iter(artifacts)
            .map(|(name, artifact)| async move {
                let cached = PackedArchive::is_cached(
                    cache,
                    &name,
                    &artifact.url,
                    artifact.hash.as_ref(),
                    artifact.size,
                )
                .await
                .with_context(|| format!("Failed to inspect `{name}` at {}", artifact.url))?;
                Ok::<_, anyhow::Error>(DistributionReport::new(
                    &name,
                    artifact,
                    if cached {
                        DistributionStatus::Cached
                    } else {
                        DistributionStatus::WouldDownload
                    },
                ))
            })
            .buffered(concurrency.downloads)
            .try_collect()
            .await?;
        DownloadReport::new(distributions, true).write(output_format, printer)?;
        return Ok(ExitStatus::Success);
    }
    let client = RegistryClientBuilder::new(client_builder, cache.clone())
        .index_locations(settings.index_locations)
        .index_strategy(settings.index_strategy)
        .keyring(settings.keyring_provider)
        .build()?;
    let distributions = stream::iter(artifacts)
        .map(|(name, artifact)| {
            let client = &client;
            async move {
                let downloaded = PackedArchive::download(
                    cache,
                    client,
                    &name,
                    &artifact.url,
                    artifact.hash.as_ref(),
                    artifact.size,
                )
                .await
                .with_context(|| format!("Failed to download `{name}` from {}", artifact.url))?;
                Ok::<_, anyhow::Error>(DistributionReport::new(
                    &name,
                    artifact,
                    if downloaded {
                        DistributionStatus::Downloaded
                    } else {
                        DistributionStatus::Cached
                    },
                ))
            }
        })
        .buffered(concurrency.downloads)
        .try_collect()
        .await?;
    DownloadReport::new(distributions, false).write(output_format, printer)?;
    Ok(ExitStatus::Success)
}

#[derive(Debug, Serialize)]
struct DownloadReport {
    schema: SchemaReport,
    dry_run: bool,
    distributions: Vec<DistributionReport>,
    summary: SummaryReport,
}

impl DownloadReport {
    fn new(distributions: Vec<DistributionReport>, dry_run: bool) -> Self {
        let summary = SummaryReport::from_distributions(&distributions);
        Self {
            schema: SchemaReport::default(),
            dry_run,
            distributions,
            summary,
        }
    }

    fn write(&self, output_format: DownloadFormat, printer: Printer) -> Result<()> {
        match output_format {
            DownloadFormat::Text if self.dry_run => writeln!(
                printer.stderr(),
                "Would download {} distributions ({} total)",
                self.summary.would_download,
                self.summary.total
            )?,
            DownloadFormat::Text => writeln!(
                printer.stderr(),
                "Downloaded {} distributions ({} total)",
                self.summary.downloaded,
                self.summary.total
            )?,
            DownloadFormat::Json => writeln!(
                printer.stdout_important(),
                "{}",
                serde_json::to_string_pretty(self)?
            )?,
        }
        Ok(())
    }
}

#[derive(Debug, Default, Serialize)]
struct SchemaReport {
    version: SchemaVersion,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
enum SchemaVersion {
    #[default]
    Preview,
}

#[derive(Debug, Serialize)]
struct DistributionReport {
    name: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    status: DistributionStatus,
}

impl DistributionReport {
    fn new(name: &PackageName, artifact: LockedArtifact, status: DistributionStatus) -> Self {
        Self {
            name: name.to_string(),
            url: artifact.url.to_string(),
            hash: artifact.hash.map(|hash| hash.to_string()),
            size: artifact.size,
            status,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DistributionStatus {
    Cached,
    Downloaded,
    WouldDownload,
}

#[derive(Debug, Serialize)]
struct SummaryReport {
    downloaded: usize,
    cached: usize,
    would_download: usize,
    total: usize,
}

impl SummaryReport {
    fn from_distributions(distributions: &[DistributionReport]) -> Self {
        Self {
            downloaded: distributions
                .iter()
                .filter(|distribution| distribution.status == DistributionStatus::Downloaded)
                .count(),
            cached: distributions
                .iter()
                .filter(|distribution| distribution.status == DistributionStatus::Cached)
                .count(),
            would_download: distributions
                .iter()
                .filter(|distribution| distribution.status == DistributionStatus::WouldDownload)
                .count(),
            total: distributions.len(),
        }
    }
}
