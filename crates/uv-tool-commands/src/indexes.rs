use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_dispatch::PlatformState;
use uv_distribution::{GitWorkspaceMember, StaticMetadataDatabase};
use uv_distribution_types::{GitDirectorySourceUrl, Index, IndexLocations, IndexUrl};
use uv_pep508::VerbatimUrl;
use uv_settings::{IndexOptions, ResolverInstallerOptions};
use uv_tool::{Tool, ToolIndexSource};

/// Preserve Git-relative sources for indexes belonging to the imported repository.
pub(super) fn capture_index_sources(
    options: &ResolverInstallerOptions,
    member: &GitWorkspaceMember,
) -> Result<Vec<ToolIndexSource>> {
    let locations = IndexLocations::from(options.indexes.clone());
    let mut sources = BTreeMap::new();
    for index in locations.simple_indexes().chain(locations.flat_indexes()) {
        let IndexUrl::Path(url) = index.url() else {
            continue;
        };
        let path = url
            .to_file_path()
            .map_err(|()| anyhow!("Invalid local index URL: {}", index.url()))?;
        if let Some(source) = member.directory_source(&path)? {
            sources.insert(index.url().clone(), source);
        }
    }
    Ok(sources
        .into_iter()
        .map(|(index, source)| ToolIndexSource::new(index, source))
        .collect::<Result<_, _>>()?)
}

/// Restore checkout-local indexes before resolution reads their directories.
pub(super) async fn restore_index_sources(
    options: &mut ResolverInstallerOptions,
    receipt: &mut Tool,
    state: &PlatformState,
    client_builder: &BaseClientBuilder<'_>,
    cache: &Cache,
) -> Result<Vec<ToolIndexSource>> {
    let sources = receipt.index_sources();
    let database = StaticMetadataDatabase::new(client_builder, state.git(), cache);
    let mut replacements = BTreeMap::new();
    let mut restored = Vec::with_capacity(sources.len());
    for source in sources {
        // A stored requirement can retain an explicit index even when CLI settings replace the
        // corresponding option. Restore every retained source before resolving those requirements.
        let GitDirectorySourceUrl {
            git,
            subdirectory,
            url,
        } = source.source();
        // An unused explicit index may not exist. Materialize its repository; consumers validate
        // the index directory only if it participates in resolution.
        let fetch = database
            .fetch_git_source(GitDirectorySourceUrl {
                git,
                subdirectory: None,
                url,
            })
            .await?;
        let path = subdirectory.map_or_else(
            || fetch.path().to_path_buf(),
            |path| fetch.path().join(path),
        );
        let index = IndexUrl::from(VerbatimUrl::from_absolute_path(&path)?);
        replacements.insert(source.index().clone(), index.clone());
        restored.push(source.clone().with_index(index));
    }
    replace_index_urls(&mut options.indexes, &replacements);
    receipt.replace_requirement_indexes(&replacements);
    Ok(restored)
}

fn replace_index_urls(options: &mut IndexOptions, replacements: &BTreeMap<IndexUrl, IndexUrl>) {
    let replace = |mut index: Index| {
        if let Some(url) = replacements.get(index.url()) {
            index.url = url.clone();
        }
        index
    };
    options.index = options
        .index
        .take()
        .map(|indexes| indexes.into_iter().map(replace).collect());
    options.index_url = options
        .index_url
        .take()
        .map(|index| replace(Index::from(index)).into());
    options.extra_index_url = options.extra_index_url.take().map(|indexes| {
        indexes
            .into_iter()
            .map(|index| replace(Index::from(index)).into())
            .collect()
    });
    options.find_links = options.find_links.take().map(|indexes| {
        indexes
            .into_iter()
            .map(|index| replace(Index::from(index)).into())
            .collect()
    });
}
