//! Dependency resolution workflows used by uv commands.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;

use itertools::Itertools;
use owo_colors::OwoColorize;
use uv_client::{BaseClientBuilder, RegistryClient};
use uv_command_support::Printer;
use uv_configuration::{
    Concurrency, Constraints, DependencyGroups, DependencyModifiers, ExcludeDependency, Excludes,
    ExtrasSpecification, Override, Overrides, Reinstall, Upgrade,
};
use uv_dispatch::BuildDispatch;
use uv_distribution::{DistributionDatabase, SourcedDependencyGroups};
use uv_distribution_types::{
    Diagnostic, NameRequirementSpecification, Requirement, RequirementScope, RequirementSource,
    ResolutionDiagnostic, ResolutionRecorder, UnresolvedRequirement,
    UnresolvedRequirementSpecification,
};
use uv_installer::SitePackages;
use uv_normalize::PackageName;
use uv_pep508::{MarkerEnvironment, RequirementOrigin};
use uv_platform_tags::Tags;
use uv_pypi_types::Conflicts;
use uv_requirements::{
    GroupsSpecification, LookaheadResolver, NamedRequirementsResolver, RequirementsSource,
    RequirementsSpecification, SourceTree, SourceTreeResolution, SourceTreeResolver,
};
use uv_resolver::{
    DependencyMode, Exclusions, FlatIndex, InMemoryIndex, Manifest, Options, Preference,
    Preferences, PythonRequirement, Resolver, ResolverEnvironment, ResolverOutput, UpgradePackages,
};
use uv_types::{BuildContext, HashStrategy};

use crate::loggers::ResolveLogger;
use crate::reporters::ResolverReporter;

mod error;
pub mod latest;
pub mod locked_requirements;
pub mod loggers;
mod markers;
pub mod reporters;

pub use error::Error;
pub use markers::{resolution_markers, resolution_tags};

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
        return Err(Error::ExtrasWithoutSource { has_editable });
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

/// Resolve a set of requirements, similar to running `pip compile`.
pub async fn resolve(
    requirements: Vec<UnresolvedRequirementSpecification>,
    constraints: Vec<NameRequirementSpecification>,
    overrides: Vec<UnresolvedRequirementSpecification>,
    lowered_overrides: Vec<Override<Requirement>>,
    excludes: Vec<ExcludeDependency>,
    source_trees: Vec<SourceTree>,
    mut project: Option<PackageName>,
    workspace_members: BTreeMap<PackageName, RequirementSource>,
    extras: &ExtrasSpecification,
    groups: &BTreeMap<PathBuf, DependencyGroups>,
    preferences: Vec<Preference>,
    installed_packages: Option<SitePackages>,
    hasher: &HashStrategy,
    reinstall: &Reinstall,
    upgrade: &Upgrade,
    tags: Option<&Tags>,
    resolver_env: ResolverEnvironment,
    python_requirement: PythonRequirement,
    current_environment: &MarkerEnvironment,
    conflicts: Conflicts,
    client: &RegistryClient,
    flat_index: &FlatIndex,
    index: &InMemoryIndex,
    build_dispatch: &BuildDispatch<'_>,
    concurrency: &Concurrency,
    options: Options,
    recorder: Option<ResolutionRecorder>,
    logger: Box<dyn ResolveLogger>,
    printer: Printer,
) -> Result<(ResolverOutput, HashStrategy), Error> {
    let start = std::time::Instant::now();

    let new_database = || {
        DistributionDatabase::new(
            client,
            build_dispatch,
            concurrency.downloads_semaphore.clone(),
        )
        .with_recorder(recorder.clone())
    };

    let resolve_names = async |specifications: Vec<UnresolvedRequirementSpecification>,
                               hasher: &HashStrategy|
           -> Result<Vec<Requirement>, Error> {
        let (mut named, unnamed): (Vec<_>, Vec<_>) =
            specifications
                .into_iter()
                .partition_map(|spec| match spec.requirement {
                    UnresolvedRequirement::Named(requirement) => {
                        itertools::Either::Left(requirement)
                    }
                    UnresolvedRequirement::Unnamed(requirement) => {
                        itertools::Either::Right(requirement)
                    }
                });

        if !unnamed.is_empty() {
            named.extend(
                NamedRequirementsResolver::new(hasher, index, new_database())
                    .with_reporter(Arc::new(ResolverReporter::from(printer)))
                    .resolve(unnamed.into_iter())
                    .await?,
            );
        }

        Ok(named)
    };

    // Resolve the requirements from the provided sources.
    let requirements = {
        let mut requirements = resolve_names(requirements, hasher).await?;

        // Resolve any source trees into requirements.
        if !source_trees.is_empty() {
            let resolutions = SourceTreeResolver::new(extras, hasher, index, new_database())
                .with_reporter(Arc::new(ResolverReporter::from(printer)))
                .resolve(source_trees.iter())
                .await?;

            // If we resolved a single project, use it for the project name.
            project = project.or_else(|| {
                if let [resolution] = &resolutions[..] {
                    Some(resolution.project().clone())
                } else {
                    None
                }
            });

            // If any of the extras were unused, surface a warning.
            let mut unused_extras = extras
                .explicit_names()
                .filter(|extra| {
                    !resolutions
                        .iter()
                        .any(|resolution| resolution.extras().contains(extra))
                })
                .collect::<Vec<_>>();
            if !unused_extras.is_empty() {
                unused_extras.sort_unstable();
                unused_extras.dedup();
                return Err(Error::MissingExtras(
                    unused_extras.into_iter().cloned().collect(),
                ));
            }

            // Extend the requirements with the resolved source trees.
            requirements.extend(
                resolutions
                    .into_iter()
                    .flat_map(SourceTreeResolution::into_requirements),
            );
        }

        for (pyproject_path, groups) in groups {
            let metadata = SourcedDependencyGroups::from_virtual_project(
                pyproject_path,
                None,
                build_dispatch.locations(),
                build_dispatch.sources().clone(),
                build_dispatch.cache(),
                build_dispatch.workspace_cache(),
                client.credentials_cache(),
            )
            .await
            .map_err(|source| Error::DependencyGroups {
                path: pyproject_path.clone(),
                source: Box::new(source),
            })?;

            // Complain if dependency groups are named that don't appear.
            for name in groups.explicit_names() {
                if !metadata.dependency_groups.contains_key(name) {
                    return Err(Error::MissingGroup {
                        name: name.clone(),
                        path: pyproject_path.clone(),
                    });
                }
            }
            // Apply dependency-groups
            for (group_name, group) in &metadata.dependency_groups {
                if groups.contains(group_name) {
                    let scope =
                        metadata
                            .name
                            .as_ref()
                            .map_or(RequirementScope::Global, |package| {
                                RequirementScope::Group {
                                    package: package.clone(),
                                    group: group_name.clone(),
                                }
                            });
                    requirements.extend(group.iter().cloned().map(|group| Requirement {
                        scope: scope.clone(),
                        origin: Some(RequirementOrigin::Group(
                            pyproject_path.clone(),
                            metadata.name.clone(),
                            group_name.clone(),
                        )),
                        ..group
                    }));
                }
            }
        }

        requirements
    };

    // Incorporate hashes from requirements discovered while resolving source trees and groups.
    let hasher = hasher
        .clone()
        .augment_with_requirements(requirements.iter())?;

    // Resolve the overrides from the provided sources.
    let overrides = resolve_names(overrides, &hasher).await?;

    // Collect constraints, overrides, and excludes.
    let constraints = Constraints::from_requirements(
        constraints
            .into_iter()
            .map(|constraint| constraint.requirement)
            .chain(upgrade.constraints().cloned()),
    );
    let overrides = Overrides::from_entries(
        lowered_overrides
            .into_iter()
            .chain(overrides.into_iter().map(Override::Requirement))
            .collect(),
    )?;
    let excludes = Excludes::from_entries(excludes);
    let modifiers = DependencyModifiers::new(overrides, excludes);
    let preferences = Preferences::from_iter(preferences, &resolver_env);

    // Determine any lookahead requirements.
    let (lookaheads, hasher) = match options.dependency_mode {
        DependencyMode::Transitive => {
            let constraints = constraints.clone().with_recorder(recorder.clone());
            let modifiers = modifiers.clone().with_recorder(recorder.clone());
            LookaheadResolver::new(
                &requirements,
                &constraints,
                &modifiers,
                &hasher,
                index,
                new_database(),
            )
            .with_reporter(Arc::new(ResolverReporter::from(printer)))
            .resolve(&resolver_env)
            .await?
        }
        DependencyMode::Direct => (Vec::new(), hasher),
    };

    // TODO(zanieb): Consider consuming these instead of cloning
    let exclusions = Exclusions::new(reinstall.clone(), UpgradePackages::for_non_project(upgrade));

    // Create a manifest of the requirements.
    let manifest = Manifest::new(
        requirements,
        constraints,
        modifiers,
        preferences,
        project,
        workspace_members,
        exclusions,
        lookaheads,
    )
    .with_recorder(recorder.clone());

    // Resolve the dependencies.
    let resolution = {
        // If possible, create a bound on the progress bar.
        let reporter = match options.dependency_mode {
            DependencyMode::Transitive => ResolverReporter::from(printer),
            DependencyMode::Direct => {
                ResolverReporter::from(printer).with_length(manifest.num_requirements() as u64)
            }
        };

        let resolver = Resolver::new(
            manifest,
            options,
            &python_requirement,
            resolver_env,
            current_environment,
            conflicts,
            tags,
            flat_index,
            index,
            &hasher,
            build_dispatch,
            installed_packages,
            new_database(),
        )?
        .with_reporter(Arc::new(reporter));

        resolver.resolve().await?
    };

    logger.on_complete(resolution.len(), start, printer)?;

    Ok((resolution, hasher))
}

/// Report any diagnostics on resolved distributions.
pub fn diagnose_resolution(
    diagnostics: &[ResolutionDiagnostic],
    printer: Printer,
) -> Result<(), Error> {
    for diagnostic in diagnostics {
        writeln!(
            printer.stderr(),
            "{}{} {}",
            "warning".yellow().bold(),
            ":".bold(),
            diagnostic.message().bold()
        )?;
    }
    Ok(())
}
