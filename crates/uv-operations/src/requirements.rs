use uv_client::BaseClientBuilder;
use uv_configuration::ExtrasSpecification;
use uv_distribution_types::NameRequirementSpecification;
use uv_requirements::GroupsSpecification;
use uv_requirements::RequirementsSource;
use uv_requirements::RequirementsSpecification;

use crate::Error;
use crate::error::ExtrasWithoutSourceError;

/// Consolidate the requirements for an installation.
pub async fn read_requirements(
    requirements: &[RequirementsSource],
    constraints: &[RequirementsSource],
    overrides: &[RequirementsSource],
    excludes: &[RequirementsSource],
    extras: &ExtrasSpecification,
    groups: Option<&GroupsSpecification>,
    client_builder: &BaseClientBuilder<'_>,
) -> Result<RequirementsSpecification, Error> {
    // If the user requests `extras` but does not provide a valid source (e.g., a `pyproject.toml`),
    // return an error.
    if !extras.is_empty() && !requirements.iter().any(RequirementsSource::allows_extras) {
        let has_editable = requirements
            .iter()
            .any(|source| matches!(source, RequirementsSource::Editable(_)));
        return Err(anyhow::Error::new(ExtrasWithoutSourceError { has_editable }).into());
    }

    // Read all requirements from the provided sources.
    Ok(RequirementsSpecification::from_sources(
        requirements,
        constraints,
        overrides,
        excludes,
        groups,
        client_builder,
    )
    .await?)
}

/// Resolve a set of constraints.
pub async fn read_constraints(
    constraints: &[RequirementsSource],
    client_builder: &BaseClientBuilder<'_>,
) -> Result<Vec<NameRequirementSpecification>, Error> {
    Ok(
        RequirementsSpecification::from_sources(&[], constraints, &[], &[], None, client_builder)
            .await?
            .constraints,
    )
}
