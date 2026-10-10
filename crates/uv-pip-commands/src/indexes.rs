use uv_distribution_types::{Index, IndexLocations, IndexUrl, Origin};

/// Append requirements-file indexes with their origin and declaration priority intact.
pub(crate) fn combine_requirements_indexes(
    configured: IndexLocations,
    index_url: Option<IndexUrl>,
    extra_index_urls: Vec<IndexUrl>,
    find_links: Vec<IndexUrl>,
    no_index: bool,
) -> IndexLocations {
    configured.combine(
        extra_index_urls
            .into_iter()
            .map(Index::from_extra_index_url)
            .chain(index_url.map(Index::from_index_url))
            .map(|index| index.with_origin(Origin::RequirementsTxt))
            .collect(),
        find_links
            .into_iter()
            .map(Index::from_find_links)
            .map(|index| index.with_origin(Origin::RequirementsTxt))
            .collect(),
        no_index,
    )
}
