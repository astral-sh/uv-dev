use std::borrow::Cow;
use std::collections::BTreeMap;

use anyhow::Context;

use uv_configuration::{
    DependencyModifierScope, DependencyModifiers, ExcludeDependency, Excludes, Override, Overrides,
};
use uv_distribution_types::{InstalledDist, Name, Requirement, UnresolvedRequirementSpecification};
use uv_fs::Simplified;
use uv_install_wheel::RequestedExtras;
use uv_installer::SitePackages;
use uv_normalize::PackageName;
use uv_pypi_types::ResolverMarkerEnvironment;

pub(crate) type RequestedExtrasMap = BTreeMap<PackageName, RequestedExtras>;

pub(crate) fn read(dist: &InstalledDist) -> anyhow::Result<Option<RequestedExtras>> {
    let Some(path) = dist.dist_info_path() else {
        return Ok(None);
    };
    RequestedExtras::read(path).with_context(|| {
        format!(
            "Failed to read requested extras for `{}` at `{}`",
            dist.name(),
            path.user_display()
        )
    })
}

/// Record effective direct selections after overrides and exclusions.
/// Extras activated only by dependencies are resolved afresh.
pub(crate) fn from_requirements<'a>(
    requirements: impl IntoIterator<Item = &'a Requirement>,
    markers: &ResolverMarkerEnvironment,
    modifiers: &'a DependencyModifiers,
) -> RequestedExtrasMap {
    let mut requested = RequestedExtrasMap::new();
    for requirement in modifiers.apply(DependencyModifierScope::Global, requirements) {
        if !requirement.extras.is_empty() && requirement.evaluate_markers(Some(markers), &[]) {
            requested
                .entry(requirement.name.clone())
                .or_default()
                .extras
                .extend(requirement.extras.iter().cloned());
        }
    }
    requested
}

/// Recover direct selections on the fast path, where every requirement is already satisfied.
pub(crate) fn from_satisfied_requirements(
    requirements: &[UnresolvedRequirementSpecification],
    site_packages: &SitePackages,
    markers: &ResolverMarkerEnvironment,
    overrides: &[UnresolvedRequirementSpecification],
    override_dependencies: &[Override<Requirement>],
    exclude_dependencies: &[ExcludeDependency],
) -> anyhow::Result<RequestedExtrasMap> {
    let overrides = Overrides::from_entries(
        override_dependencies
            .iter()
            .cloned()
            .chain(
                named_requirements(overrides, site_packages)?
                    .into_iter()
                    .map(Cow::into_owned)
                    .map(Override::Requirement),
            )
            .collect(),
    )?;
    let modifiers = DependencyModifiers::new(
        overrides,
        Excludes::from_entries(exclude_dependencies.iter().cloned()),
    );
    let requirements = named_requirements(requirements, site_packages)?;
    Ok(from_requirements(
        requirements.iter().map(Cow::as_ref),
        markers,
        &modifiers,
    ))
}

/// Recover the names that [`SitePackages::satisfies_spec`] verified for unnamed requirements.
fn named_requirements<'a>(
    requirements: &'a [UnresolvedRequirementSpecification],
    site_packages: &SitePackages,
) -> anyhow::Result<Vec<Cow<'a, Requirement>>> {
    requirements
        .iter()
        .map(|specification| {
            site_packages
                .get_named_requirement(&specification.requirement)
                .context("Cannot recover the installed name of an already-satisfied requirement")
        })
        .collect()
}

/// Add earlier explicit selections before any of their distributions are replaced.
pub(crate) fn extend_with_installed<'a>(
    requested: &mut RequestedExtrasMap,
    site_packages: &SitePackages,
    packages: impl IntoIterator<Item = &'a PackageName>,
) -> anyhow::Result<()> {
    for name in packages {
        for dist in site_packages.get_packages(name) {
            if let Some(previous) = read(dist)? {
                requested
                    .entry(name.clone())
                    .or_default()
                    .extras
                    .extend(previous.extras);
            }
        }
    }
    Ok(())
}
