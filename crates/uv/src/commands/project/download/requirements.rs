use std::fmt::Write;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use futures::{StreamExt, TryStreamExt, stream};
use uv_cache::Cache;
use uv_client::{
    BaseClientBuilder, MetadataFormat, PackedArchive, PackedArchiveHashMismatch, RegistryClient,
    RegistryClientBuilder, VersionFiles,
};
use uv_configuration::{BuildOptions, Concurrency, HashCheckingMode};
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{
    File, HashPolicy, Index, IndexCapabilities, Origin, RemoteSource, RequirementSource,
    UnresolvedRequirement,
};
use uv_normalize::PackageName;
use uv_pep440::{Operator, Version};
use uv_pep508::MarkerTree;
use uv_pypi_types::HashDigests;
use uv_redacted::DisplaySafeUrl;
use uv_requirements::{RequirementsSource, RequirementsSpecification};
use uv_types::HashStrategy;

use crate::commands::ExitStatus;
use crate::printer::Printer;
use crate::settings::ResolverSettings;

struct Artifact {
    url: DisplaySafeUrl,
    hashes: HashDigests,
    size: Option<u64>,
}

/// Download a pre-resolved requirements manifest without reading distribution metadata.
pub(super) async fn download(
    requirements: &[PathBuf],
    settings: ResolverSettings,
    client_builder: BaseClientBuilder<'_>,
    concurrency: Concurrency,
    cache: &Cache,
    printer: Printer,
) -> Result<ExitStatus> {
    let client_builder = client_builder.keyring(settings.keyring_provider);
    let sources = requirements
        .iter()
        .cloned()
        .map(RequirementsSource::from_requirements_txt)
        .collect::<Result<Vec<_>>>()?;
    let mut specification =
        RequirementsSpecification::from_simple_sources(&sources, &client_builder).await?;
    if !specification.constraints.is_empty() {
        bail!("Constraints are not supported by `uv download -r`; compile the requirements first");
    }
    for entry in &mut specification.requirements {
        match entry.requirement.source().as_ref() {
            RequirementSource::Registry { specifier, .. } => {
                if !matches!(specifier.as_ref(), [specifier] if *specifier.operator() == Operator::Equal)
                {
                    bail!(
                        "`uv download -r` requires exact `==` pins or archive URLs; found `{}`",
                        entry.requirement
                    );
                }
            }
            RequirementSource::Url { .. } | RequirementSource::Path { .. } => {}
            _ => bail!(
                "`uv download -r` does not support source trees or Git requirements: `{}`",
                entry.requirement
            ),
        }
        // This is a universal artifact manifest, not an installation for the current environment.
        // Keep the hash policy of each marker alternative separate.
        match &mut entry.requirement {
            UnresolvedRequirement::Named(requirement) => requirement.marker = MarkerTree::TRUE,
            UnresolvedRequirement::Unnamed(requirement) => requirement.marker = MarkerTree::TRUE,
        }
    }
    let index_locations = settings.index_locations.combine(
        specification
            .extra_index_urls
            .into_iter()
            .map(Index::from_extra_index_url)
            .chain(specification.index_url.map(Index::from_index_url))
            .map(|index| index.with_origin(Origin::RequirementsTxt))
            .collect(),
        specification
            .find_links
            .into_iter()
            .map(Index::from_find_links)
            .map(|index| index.with_origin(Origin::RequirementsTxt))
            .collect(),
        specification.no_index,
    );
    let client = RegistryClientBuilder::new(client_builder, cache.clone())
        .index_locations(index_locations)
        .index_strategy(settings.index_strategy)
        .build()?;
    let build_options = BuildOptions::new(specification.no_binary, specification.no_build);
    let capabilities = IndexCapabilities::default();
    let mut total = 0;
    let mut downloaded = 0;
    for entry in specification.requirements {
        // Reuse the installer's hash rules: alternatives for registry pins, all digests for
        // concrete URLs, and explicit errors for conflicting URL hashes or missing required hashes.
        let hasher = HashStrategy::from_requirements(
            std::iter::once((&entry.requirement, entry.hashes.as_slice())),
            std::iter::empty(),
            None,
            if specification.require_hashes {
                HashCheckingMode::Require
            } else {
                HashCheckingMode::Verify
            },
        )?;
        match entry.requirement.source().as_ref() {
            RequirementSource::Registry { specifier, .. } => {
                let UnresolvedRequirement::Named(requirement) = &entry.requirement else {
                    bail!("Registry requirement has no package name");
                };
                let [specifier] = specifier.as_ref() else {
                    bail!("Registry requirement is not pinned");
                };
                let version = specifier.version();
                let files = registry_files(
                    &client,
                    &requirement.name,
                    version,
                    &capabilities,
                    &concurrency,
                )
                .await?;
                let mut artifacts = Vec::new();
                for (filename, file) in files {
                    match filename {
                        DistFilename::WheelFilename(_)
                            if build_options.no_binary_package(&requirement.name) =>
                        {
                            continue;
                        }
                        DistFilename::SourceDistFilename(_)
                            if build_options.no_build_package(&requirement.name) =>
                        {
                            continue;
                        }
                        _ => {}
                    }
                    let url = file.url.to_url()?;
                    if let Some(zstd) = file.zstd {
                        let mut url = url.clone();
                        let path = format!("{}.tar.zst", url.path());
                        url.set_path(&path);
                        artifacts.push(Artifact {
                            url,
                            hashes: zstd.hashes,
                            size: zstd.size,
                        });
                    }
                    artifacts.push(Artifact {
                        url,
                        hashes: file.hashes,
                        size: file.size,
                    });
                }
                let hashes = hasher.get_package(&requirement.name, version);
                let (new_downloads, matches) = stream::iter(artifacts)
                    .map(|artifact| {
                        let name = &requirement.name;
                        let client = &client;
                        async move {
                            // Skip files the index identifies as outside the allowed hash set.
                            // Unknown algorithms still require downloading and hashing the bytes.
                            if hashes.requires_validation()
                                && !hashes.digests().iter().any(|expected| {
                                    artifact.hashes.iter().any(|actual| actual == expected)
                                        || !artifact
                                            .hashes
                                            .iter()
                                            .any(|actual| actual.algorithm == expected.algorithm)
                                })
                            {
                                return Ok(None);
                            }
                            let policy = if hashes.is_none() && !artifact.hashes.is_empty() {
                                HashPolicy::All(artifact.hashes.as_slice())
                            } else {
                                hashes
                            };
                            match PackedArchive::download(
                                cache,
                                client,
                                name,
                                &artifact.url,
                                policy,
                                artifact.size,
                            )
                            .await
                            {
                                Ok(downloaded) => Ok(Some(downloaded)),
                                Err(err)
                                    if hashes.requires_validation()
                                        && err.is::<PackedArchiveHashMismatch>() =>
                                {
                                    Ok(None)
                                }
                                Err(err) => Err(err).with_context(|| {
                                    format!("Failed to download `{name}` from {}", artifact.url)
                                }),
                            }
                        }
                    })
                    .buffer_unordered(concurrency.downloads)
                    .try_fold((0usize, 0usize), async |(downloaded, total), result| {
                        Ok::<_, anyhow::Error>(match result {
                            Some(new) => (downloaded + usize::from(new), total + 1),
                            None => (downloaded, total),
                        })
                    })
                    .await?;
                if matches == 0 {
                    bail!(
                        "No distributions found for `{}=={version}` matching the requested hashes and archive types",
                        requirement.name
                    );
                }
                downloaded += new_downloads;
                total += matches;
            }
            RequirementSource::Url { url, .. } | RequirementSource::Path { url, .. } => {
                let name = match &entry.requirement {
                    UnresolvedRequirement::Named(requirement) => requirement.name.clone(),
                    UnresolvedRequirement::Unnamed(_) => {
                        DistFilename::try_from_normalized_filename(&url.filename()?)
                            .with_context(|| {
                                format!(
                                    "Cannot infer a package name from `{url}`; use `name @ URL`"
                                )
                            })?
                            .name()
                            .clone()
                    }
                };
                let url = url.to_url();
                downloaded += usize::from(
                    PackedArchive::download(
                        cache,
                        &client,
                        &name,
                        &url,
                        hasher.get_url(&url),
                        None,
                    )
                    .await
                    .with_context(|| format!("Failed to download `{name}` from {url}"))?,
                );
                total += 1;
            }
            _ => bail!("Unsupported requirement: `{}`", entry.requirement),
        }
    }
    writeln!(
        printer.stderr(),
        "Downloaded {downloaded} distributions ({total} total)"
    )?;
    Ok(ExitStatus::Success)
}

async fn registry_files(
    client: &RegistryClient,
    name: &PackageName,
    version: &Version,
    capabilities: &IndexCapabilities,
    concurrency: &Concurrency,
) -> Result<Vec<(DistFilename, File)>> {
    let metadata = match client
        .simple_detail(name, None, capabilities, &concurrency.downloads_semaphore)
        .await
    {
        Ok(metadata) => metadata,
        Err(err)
            if matches!(
                err.kind(),
                uv_client::ErrorKind::NoIndex(_)
                    | uv_client::ErrorKind::RemotePackageNotFound(_)
                    | uv_client::ErrorKind::Offline(_)
            ) =>
        {
            Vec::new()
        }
        Err(err) => return Err(err.into()),
    };
    let mut files = Vec::new();
    for (_, metadata) in metadata {
        match metadata {
            MetadataFormat::Simple(metadata) => {
                for datum in metadata.iter() {
                    if rkyv::deserialize::<Version, rkyv::rancor::Error>(&datum.version)?
                        == *version
                    {
                        let version_files =
                            rkyv::deserialize::<VersionFiles, rkyv::rancor::Error>(&datum.files)?;
                        files.extend(version_files.all(name));
                    }
                }
            }
            MetadataFormat::Flat(entries) => {
                files.extend(
                    entries
                        .into_iter()
                        .map(|entry| {
                            let (filename, file, _) = entry.into_parts();
                            (filename, file)
                        })
                        .filter(|(filename, _)| filename.version() == version),
                );
            }
        }
        // Every index strategy takes a given version from only its first matching index.
        if !files.is_empty() {
            break;
        }
    }
    files.extend(
        client
            .find_links_entries(name, &concurrency.downloads_semaphore)
            .await?
            .into_iter()
            .map(|entry| {
                let (filename, file, _) = entry.into_parts();
                (filename, file)
            })
            .filter(|(filename, _)| filename.version() == version),
    );
    Ok(files)
}
