use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use anyhow::anyhow;
use itertools::Itertools;

use uv_cli_output::printer::Printer;
use uv_cli_output::reporters::ResolverReporter;
use uv_client::RegistryClient;
use uv_configuration::Concurrency;
use uv_configuration::Constraints;
use uv_configuration::DependencyGroups;
use uv_configuration::DependencyModifiers;
use uv_configuration::ExcludeDependency;
use uv_configuration::Excludes;
use uv_configuration::ExtrasSpecification;
use uv_configuration::Override;
use uv_configuration::Overrides;
use uv_configuration::Reinstall;
use uv_configuration::TargetTriple;
use uv_configuration::Upgrade;
use uv_dispatch::BuildDispatch;
use uv_distribution::DistributionDatabase;
use uv_distribution::SourcedDependencyGroups;
use uv_distribution_types::NameRequirementSpecification;
use uv_distribution_types::Requirement;
use uv_distribution_types::RequirementScope;
use uv_distribution_types::RequirementSource;
use uv_distribution_types::ResolutionRecorder;
use uv_distribution_types::UnresolvedRequirement;
use uv_distribution_types::UnresolvedRequirementSpecification;
use uv_fs::Simplified;
use uv_installer::SitePackages;
use uv_normalize::PackageName;
use uv_pep508::MarkerEnvironment;
use uv_pep508::RequirementOrigin;
use uv_platform_tags::Tags;
use uv_platform_tags::TagsError;
use uv_platform_tags::TagsOptions;
use uv_pypi_types::Conflicts;
use uv_pypi_types::ResolverMarkerEnvironment;
use uv_python::Interpreter;
use uv_python::PythonVersion;
use uv_requirements::LookaheadResolver;
use uv_requirements::NamedRequirementsResolver;
use uv_requirements::SourceTree;
use uv_requirements::SourceTreeResolution;
use uv_requirements::SourceTreeResolver;
use uv_resolver::DependencyMode;
use uv_resolver::Exclusions;
use uv_resolver::FlatIndex;
use uv_resolver::InMemoryIndex;
use uv_resolver::Manifest;
use uv_resolver::Options;
use uv_resolver::Preference;
use uv_resolver::Preferences;
use uv_resolver::PythonRequirement;
use uv_resolver::Resolver;
use uv_resolver::ResolverEnvironment;
use uv_resolver::ResolverOutput;
use uv_resolver::UpgradePackages;
use uv_types::BuildContext;
use uv_types::HashStrategy;

use crate::Error;
use crate::loggers::ResolveLogger;

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

    // Resolve the requirements from the provided sources.
    let requirements = {
        // Partition the requirements into named and unnamed requirements.
        let (mut requirements, unnamed): (Vec<_>, Vec<_>) =
            requirements
                .into_iter()
                .partition_map(|spec| match spec.requirement {
                    UnresolvedRequirement::Named(requirement) => {
                        itertools::Either::Left(requirement)
                    }
                    UnresolvedRequirement::Unnamed(requirement) => {
                        itertools::Either::Right(requirement)
                    }
                });

        // Resolve any unnamed requirements.
        if !unnamed.is_empty() {
            requirements.extend(
                NamedRequirementsResolver::new(
                    hasher,
                    index,
                    DistributionDatabase::new(
                        client,
                        build_dispatch,
                        concurrency.downloads_semaphore.clone(),
                    )
                    .with_recorder(recorder.clone()),
                )
                .with_reporter(Arc::new(ResolverReporter::from(printer)))
                .resolve(unnamed.into_iter())
                .await?,
            );
        }

        // Resolve any source trees into requirements.
        if !source_trees.is_empty() {
            let resolutions = SourceTreeResolver::new(
                extras,
                hasher,
                index,
                DistributionDatabase::new(
                    client,
                    build_dispatch,
                    concurrency.downloads_semaphore.clone(),
                )
                .with_recorder(recorder.clone()),
            )
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
                let s = if unused_extras.len() == 1 { "" } else { "s" };
                return Err(anyhow!(
                    "Requested extra{s} not found: {}",
                    unused_extras.iter().join(", ")
                )
                .into());
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
            .with_context(|| {
                format!(
                    "Failed to read dependency groups from: {}",
                    pyproject_path.display()
                )
            })?;

            // Complain if dependency groups are named that don't appear.
            for name in groups.explicit_names() {
                if !metadata.dependency_groups.contains_key(name) {
                    Err(anyhow!(
                        "The dependency group '{name}' was not found in the project: {}",
                        pyproject_path.user_display()
                    ))?;
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
    let mut hasher = hasher
        .clone()
        .augment_with_requirements(requirements.iter())?;

    // Resolve the overrides from the provided sources.
    let overrides = {
        // Partition the overrides into named and unnamed requirements.
        let (mut overrides, unnamed): (Vec<_>, Vec<_>) =
            overrides
                .into_iter()
                .partition_map(|spec| match spec.requirement {
                    UnresolvedRequirement::Named(requirement) => {
                        itertools::Either::Left(requirement)
                    }
                    UnresolvedRequirement::Unnamed(requirement) => {
                        itertools::Either::Right(requirement)
                    }
                });

        // Resolve any unnamed overrides.
        if !unnamed.is_empty() {
            overrides.extend(
                NamedRequirementsResolver::new(
                    &hasher,
                    index,
                    DistributionDatabase::new(
                        client,
                        build_dispatch,
                        concurrency.downloads_semaphore.clone(),
                    )
                    .with_recorder(recorder.clone()),
                )
                .with_reporter(Arc::new(ResolverReporter::from(printer)))
                .resolve(unnamed.into_iter())
                .await?,
            );
        }

        overrides
    };

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
    )
    .map_err(anyhow::Error::from)?;
    let excludes = Excludes::from_entries(excludes);
    let modifiers = DependencyModifiers::new(overrides, excludes);
    let preferences = Preferences::from_iter(preferences, &resolver_env);

    // Determine any lookahead requirements.
    let lookaheads = match options.dependency_mode {
        DependencyMode::Transitive => {
            let constraints = constraints.clone().with_recorder(recorder.clone());
            let modifiers = modifiers.clone().with_recorder(recorder.clone());
            let (lookaheads, updated_hasher) = LookaheadResolver::new(
                &requirements,
                &constraints,
                &modifiers,
                &hasher,
                index,
                DistributionDatabase::new(
                    client,
                    build_dispatch,
                    concurrency.downloads_semaphore.clone(),
                )
                .with_recorder(recorder.clone()),
            )
            .with_reporter(Arc::new(ResolverReporter::from(printer)))
            .resolve(&resolver_env)
            .await?;
            hasher = updated_hasher;
            lookaheads
        }
        DependencyMode::Direct => Vec::new(),
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
            DistributionDatabase::new(
                client,
                build_dispatch,
                concurrency.downloads_semaphore.clone(),
            )
            .with_recorder(recorder.clone()),
        )?
        .with_reporter(Arc::new(reporter));

        resolver.resolve().await?
    };

    logger.on_complete(resolution.len(), start, printer)?;

    Ok((resolution, hasher))
}

pub fn resolution_markers(
    python_version: Option<&PythonVersion>,
    python_platform: Option<&TargetTriple>,
    interpreter: &Interpreter,
) -> ResolverMarkerEnvironment {
    match (python_platform, python_version) {
        (Some(python_platform), Some(python_version)) => ResolverMarkerEnvironment::from(
            python_version.markers(python_platform.markers(interpreter.markers().clone())),
        ),
        (Some(python_platform), None) => {
            ResolverMarkerEnvironment::from(python_platform.markers(interpreter.markers().clone()))
        }
        (None, Some(python_version)) => {
            ResolverMarkerEnvironment::from(python_version.markers(interpreter.markers().clone()))
        }
        (None, None) => interpreter.to_resolver_marker_environment(),
    }
}

pub fn resolution_tags<'env>(
    python_version: Option<&PythonVersion>,
    python_platform: Option<&TargetTriple>,
    interpreter: &'env Interpreter,
) -> Result<Cow<'env, Tags>, TagsError> {
    if python_platform.is_none() && python_version.is_none() {
        return Ok(Cow::Borrowed(interpreter.tags()?));
    }

    let (platform, manylinux_compatible) = if let Some(python_platform) = python_platform {
        (
            python_platform.platform(),
            python_platform.manylinux_compatible(),
        )
    } else {
        (
            interpreter.platform().clone(),
            interpreter.manylinux_compatible(),
        )
    };

    let version_tuple = if let Some(python_version) = python_version {
        (python_version.major(), python_version.minor())
    } else {
        interpreter.python_tuple()
    };

    let tags = Tags::from_env(
        platform,
        version_tuple,
        interpreter.implementation_name(),
        interpreter.implementation_tuple(),
        TagsOptions {
            manylinux_compatible,
            gil_disabled: interpreter.gil_disabled(),
            debug_enabled: interpreter.debug_enabled(),
            is_cross: true,
        },
    )?;
    Ok(Cow::Owned(tags))
}
