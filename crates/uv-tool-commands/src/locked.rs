//! Select a tool before reading its packaged lock, without resolving its dependencies.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;

use anyhow::{Context, bail};
use futures::stream::FuturesUnordered;
use futures::{StreamExt, TryStreamExt};
use petgraph::visit::Dfs;
use tracing::debug;

use uv_cache::Cache;
use uv_client::{
    BaseClientBuilder, Connectivity, MetadataFormat, RegistryClient, RegistryClientBuilder,
    VersionFiles,
};
use uv_configuration::{
    Concurrency, Constraints, DependencyModifiers, ExcludeDependency, Excludes, HashCheckingMode,
    NoBinary, NoBuild, Override, Overrides, TargetTriple,
};
use uv_dispatch::{BuildDispatch, PlatformState};
use uv_distribution::{DistributionDatabase, LocalWheel, LoweredExtraBuildDependencies};
use uv_distribution_filename::{DistExtension, DistFilename};
use uv_distribution_types::{
    ArchiveHashGroups, ArchiveHashPolicy, BuiltDist, CachedDist, DependencyMetadata, Dist, Edge,
    Hashed, Index, IndexCapabilities, IndexLocations, IndexUrl, Name, NameRequirementSpecification,
    Node, PYPI_URL, Requirement, RequirementScope, RequirementSource, Resolution, ResolvedDist,
    SourceDist, StaticMetadata, UnresolvedRequirement, UnresolvedRequirementSpecification,
    parse_all_url_hashes,
};
use uv_lock::PylockToml;
use uv_metadata::{find_flat_dist_info, read_flat_wheel_metadata};
use uv_normalize::{ExtraName, PackageName};
use uv_pep440::{Operator, Version, VersionSpecifier, VersionSpecifiers};
use uv_pep508::{MarkerEnvironment, MarkerTree, VersionOrUrl};
use uv_platform_tags::Tags;
use uv_preview::{Preview, PreviewFeature};
use uv_pypi_types::{HashAlgorithm, HashDigest, HashDigests, Hashes, ResolutionMetadata};
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_python_types::{PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest};
use uv_redacted::SafeUrl;
use uv_redacted::policy::Stripped;
use uv_requirements::{RequirementsSource, RequirementsSpecification};
use uv_resolver::{FlatIndex, ResolutionGraphNode};
use uv_settings::PythonInstallMirrors;
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy, requirement_hash_groups};
use uv_workspace::WorkspaceCache;

use uv_command_support::{Printer, UvError};
use uv_environment_operations::{
    EnvironmentError, EnvironmentResolution, EnvironmentSpecification, resolve_environment,
    resolve_environment_with_hashes,
};
use uv_python_discovery::PythonDownloadReporter;
use uv_resolve_operations::{
    loggers::SummaryResolveLogger, resolution_markers, resolution_tags, resolve_pylock_toml,
};
use uv_settings::ResolverSettings;

use crate::common::refine_interpreter;

/// A failure to select the tool wheel, before inspecting its packaged lock.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
struct RootSelectionError {
    #[source]
    error: EnvironmentError,
    incompatible_python: Vec<VersionSpecifiers>,
}

pub(super) fn root_selection_error(err: anyhow::Error) -> Result<EnvironmentError, anyhow::Error> {
    err.downcast::<RootSelectionError>()
        .map(|selection| selection.error)
}

pub(super) fn unwrap_root_selection_error(err: anyhow::Error) -> anyhow::Error {
    match root_selection_error(err) {
        Ok(err) => err.into(),
        Err(err) => err,
    }
}

/// Resolve the tool, retrying root selection with a compatible interpreter when possible.
pub(super) async fn resolve_with_interpreter(
    spec: RequirementsSpecification,
    interpreter: Interpreter,
    refine: bool,
    python_request: Option<&PythonRequest>,
    python_platform: Option<&TargetTriple>,
    build_constraints: &Constraints,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    reporter: &PythonDownloadReporter,
    install_mirrors: &PythonInstallMirrors,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    state: &PlatformState,
    concurrency: &Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> anyhow::Result<(Resolution, Interpreter)> {
    let result = Box::pin(resolve(
        spec.clone(),
        &interpreter,
        python_platform,
        build_constraints,
        settings,
        client_builder,
        state,
        concurrency,
        cache,
        workspace_cache,
        printer,
        preview,
    ))
    .await;
    let err = match result {
        Ok(resolution) => return Ok((resolution, interpreter)),
        Err(err) => err,
    };
    let Some(RootSelectionError {
        error: EnvironmentError::Resolve(operation),
        incompatible_python,
    }) = err.downcast_ref::<RootSelectionError>()
    else {
        return Err(err);
    };
    if !refine {
        return Err(err);
    }
    let Some(interpreter) = refine_interpreter(
        &interpreter,
        python_request,
        operation,
        incompatible_python,
        client_builder,
        reporter,
        install_mirrors,
        python_preference,
        python_arch,
        python_downloads,
        cache,
    )
    .await
    .ok()
    .flatten() else {
        return Err(err);
    };
    debug!(
        "Re-resolving with Python {} (`{}`)",
        interpreter.python_version(),
        interpreter.sys_executable().display()
    );
    let resolution = Box::pin(resolve(
        spec,
        &interpreter,
        python_platform,
        build_constraints,
        settings,
        client_builder,
        state,
        concurrency,
        cache,
        workspace_cache,
        printer,
        preview,
    ))
    .await?;
    Ok((resolution, interpreter))
}

pub(super) fn check_preview(locked: bool, preview: Preview) -> anyhow::Result<()> {
    if locked && !preview.is_enabled(PreviewFeature::LockedTools) {
        bail!("`--locked` for tools requires the `locked-tools` preview feature");
    }
    Ok(())
}

pub(super) fn check_tool_requirement(requirement: &UnresolvedRequirement) -> anyhow::Result<()> {
    if let Some(url) = requirement.source().to_verbatim_parsed_url() {
        parse_all_url_hashes(&url.verbatim)?;
    }
    match requirement.source().as_ref() {
        RequirementSource::Registry { .. }
        | RequirementSource::Url {
            ext: DistExtension::Wheel,
            ..
        }
        | RequirementSource::Path {
            ext: DistExtension::Wheel,
            ..
        } => Ok(()),
        RequirementSource::Url {
            ext: DistExtension::Source(_),
            ..
        }
        | RequirementSource::Path {
            ext: DistExtension::Source(_),
            ..
        }
        | RequirementSource::GitDirectory { .. }
        | RequirementSource::GitPath { .. }
        | RequirementSource::Directory { .. } => {
            bail!("A wheel is required to install a tool with `--locked`")
        }
    }
}

pub(super) fn check_arguments(locked: bool, with: &[RequirementsSource]) -> anyhow::Result<()> {
    if locked && !with.is_empty() {
        bail!("`--locked` requires a single tool package and cannot be combined with `--with`");
    }
    Ok(())
}

/// Keep hashes attached to named overrides while resolving any unnamed requirements.
pub(super) fn check_override_inputs(
    overrides: &[UnresolvedRequirementSpecification],
    constraints: &[NameRequirementSpecification],
    override_dependencies: &[Override<Requirement>],
) -> anyhow::Result<()> {
    let mut hashed_inputs = false;
    for entry in constraints {
        requirement_url_hashes(&entry.requirement)?;
        check_unsupported_hash_fragment(&entry.requirement)?;
        let has_hashes = !entry.hashes.is_empty() || entry.requirement.hashes()?.is_some();
        let mut extra_marker = false;
        entry
            .requirement
            .marker
            .visit_extras(|_, _| extra_marker = true);
        if extra_marker {
            bail!("Constraints with extra markers are not supported with `--locked`");
        }
        hashed_inputs |= has_hashes;
    }
    for entry in overrides {
        if let UnresolvedRequirement::Named(requirement) = &entry.requirement {
            requirement_url_hashes(requirement)?;
            check_unsupported_hash_fragment(requirement)?;
        }
        hashed_inputs |= !entry.hashes.is_empty() || entry.requirement.hashes()?.is_some();
    }
    for entry in override_dependencies {
        let requirements = match entry {
            Override::Requirement(requirement) => std::slice::from_ref(requirement),
            Override::Package(package) => &package.dependencies,
        };
        for requirement in requirements {
            requirement_url_hashes(requirement)?;
            check_unsupported_hash_fragment(requirement)?;
            let mut extra_marker = false;
            requirement.marker.visit_extras(|_, _| extra_marker = true);
            if extra_marker {
                bail!("Overrides with extra markers are not supported with `--locked`");
            }
            hashed_inputs |= requirement.hashes()?.is_some();
        }
    }
    for entry in overrides {
        match &entry.requirement {
            UnresolvedRequirement::Named(requirement) => {
                let mut extra_marker = false;
                requirement.marker.visit_extras(|_, _| extra_marker = true);
                if extra_marker {
                    bail!("Overrides with extra markers are not supported with `--locked`");
                }
                if !entry.hashes.is_empty() || requirement.hashes()?.is_some() {
                    check_hashed_source(requirement)?;
                }
            }
            UnresolvedRequirement::Unnamed(_)
                if !entry.hashes.is_empty() || entry.requirement.hashes()?.is_some() =>
            {
                bail!("An override with hashes must include its package name with `--locked`");
            }
            UnresolvedRequirement::Unnamed(_) if hashed_inputs => {
                bail!(
                    "Overrides must include their package names when using hashes with `--locked`"
                );
            }
            UnresolvedRequirement::Unnamed(_) => {}
        }
    }
    Ok(())
}

fn check_unsupported_hash_fragment(requirement: &Requirement) -> anyhow::Result<()> {
    match &requirement.source {
        RequirementSource::GitDirectory { url, .. }
        | RequirementSource::GitPath { url, .. }
        | RequirementSource::Directory { url, .. } => {
            if let Some(fragment) = url.fragment()
                && Hashes::parse_url_fragment(fragment)?.is_some()
            {
                bail!("Cannot verify archive hashes for `{requirement}`");
            }
        }
        RequirementSource::Registry { .. }
        | RequirementSource::Url { .. }
        | RequirementSource::Path { .. } => {}
    }
    Ok(())
}

fn requirement_url_hashes(requirement: &Requirement) -> anyhow::Result<Vec<HashDigest>> {
    if let Some(url) = requirement.source.to_verbatim_parsed_url() {
        Ok(parse_all_url_hashes(&url.verbatim)?)
    } else {
        Ok(Vec::new())
    }
}

fn check_hashed_source(requirement: &Requirement) -> anyhow::Result<()> {
    let supported = match &requirement.source {
        RequirementSource::Registry { specifier, .. } => match specifier.as_ref() {
            [specifier] => match specifier.operator() {
                Operator::Equal | Operator::ExactEqual => true,
                Operator::EqualStar
                | Operator::NotEqual
                | Operator::NotEqualStar
                | Operator::TildeEqual
                | Operator::LessThan
                | Operator::LessThanEqual
                | Operator::GreaterThan
                | Operator::GreaterThanEqual => false,
            },
            _ => false,
        },
        RequirementSource::Url { .. } | RequirementSource::Path { .. } => true,
        RequirementSource::GitDirectory { .. }
        | RequirementSource::GitPath { .. }
        | RequirementSource::Directory { .. } => false,
    };
    if !supported {
        bail!(
            "A hashed requirement must identify a pinned version or archive with `--locked`: `{requirement}`"
        );
    }
    Ok(())
}

pub(super) async fn resolve(
    spec: RequirementsSpecification,
    interpreter: &Interpreter,
    python_platform: Option<&TargetTriple>,
    build_constraints: &Constraints,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    state: &PlatformState,
    concurrency: &Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> anyhow::Result<Resolution> {
    if spec.pylock.is_some() || !spec.source_trees.is_empty() {
        bail!("`--locked` cannot be combined with additional requirements files");
    }
    let [requirement] = spec.requirements.as_slice() else {
        bail!("`--locked` requires a single tool package and cannot be combined with `--with`");
    };
    check_tool_requirement(&requirement.requirement)?;
    let UnresolvedRequirement::Named(requirement) = &requirement.requirement else {
        bail!("Expected a named tool requirement");
    };
    if !requirement.extras.is_empty() {
        bail!("Extras are not supported with `--locked`");
    }
    let requirement = requirement.clone();
    check_override_inputs(
        &spec.overrides,
        &spec.constraints,
        &spec.override_dependencies,
    )?;
    if !spec.overrides.is_empty()
        || !spec.override_dependencies.is_empty()
        || !spec.excludes.is_empty()
    {
        for constraint in &spec.constraints {
            // Registry constraint hashes apply to the selected package regardless of the range.
            if let RequirementSource::Registry { .. } = constraint.requirement.source {
                continue;
            }
            if !constraint.hashes.is_empty() || constraint.requirement.hashes()?.is_some() {
                check_hashed_source(&constraint.requirement)?;
            }
        }
    }
    if settings.build_options.no_binary_package(&requirement.name) {
        bail!("`--locked` requires a wheel for the tool, but binary distributions are disabled");
    }
    let mut selection_settings = settings.clone();
    selection_settings.build_options = selection_settings.build_options.combine(
        NoBinary::None,
        NoBuild::Packages(vec![requirement.name.clone()]),
    );
    let client = RegistryClientBuilder::new(
        client_builder.clone().keyring(settings.keyring_provider),
        cache.clone(),
    )
    .index_locations(settings.index_locations.clone())
    .index_strategy(settings.index_strategy)
    .markers(interpreter.markers())
    .platform(interpreter.platform())
    .build()?;
    let cached_client = RegistryClientBuilder::new(
        client_builder
            .clone()
            .connectivity(Connectivity::Offline)
            .keyring(settings.keyring_provider),
        cache.clone(),
    )
    .index_locations(settings.index_locations.clone())
    .index_strategy(settings.index_strategy)
    .markers(interpreter.markers())
    .platform(interpreter.platform())
    .build()?;
    let flat_index = FlatIndex::load(&client, cache, &settings.index_locations).await?;
    let environment;
    let build_isolation = match &settings.build_isolation {
        uv_configuration::BuildIsolation::Isolate => BuildIsolation::Isolated,
        uv_configuration::BuildIsolation::Shared => {
            environment = PythonEnvironment::from_interpreter(interpreter.clone());
            BuildIsolation::Shared(&environment)
        }
        uv_configuration::BuildIsolation::SharedPackage(packages) => {
            environment = PythonEnvironment::from_interpreter(interpreter.clone());
            BuildIsolation::SharedPackage(&environment, packages)
        }
    };
    let extra_build_requires =
        LoweredExtraBuildDependencies::from_non_lowered(settings.extra_build_dependencies.clone())
            .into_inner();
    let build_hasher = HashStrategy::from_constraints(
        build_constraints,
        Some(&interpreter.to_resolver_marker_environment()),
        settings.build_hash_checking,
    )?;
    let dispatch = BuildDispatch::new(
        &client,
        cache,
        build_constraints,
        interpreter,
        &settings.index_locations,
        &flat_index,
        &settings.dependency_metadata,
        state.clone().into_inner(),
        settings.index_strategy,
        &settings.config_setting,
        &settings.config_settings_package,
        build_isolation,
        &extra_build_requires,
        &settings.extra_build_variables,
        settings.link_mode,
        &settings.build_options,
        &build_hasher,
        settings.exclude_newer.clone(),
        settings.sources.clone(),
        SourceTreeEditablePolicy::Tool,
        workspace_cache.clone(),
        concurrency.clone(),
        preview,
    );
    let tags = resolution_tags(None, python_platform, interpreter)?;
    let markers = resolution_markers(None, python_platform, interpreter);
    let mut root_hashes = requirement_hash_groups(&requirement, &[])?;
    for constraint in &spec.constraints {
        if constraint.requirement.name == requirement.name
            && constraint
                .requirement
                .evaluate_markers(Some(markers.markers()), &[])
        {
            root_hashes.extend(requirement_hash_groups(
                &constraint.requirement,
                &constraint.hashes,
            )?);
        }
    }
    for entry in &spec.overrides {
        if let UnresolvedRequirement::Named(override_requirement) = &entry.requirement
            && override_requirement.name == requirement.name
            && override_requirement.evaluate_markers(Some(markers.markers()), &[])
        {
            root_hashes.extend(requirement_hash_groups(
                override_requirement,
                &entry.hashes,
            )?);
        }
    }
    for entry in &spec.override_dependencies {
        if let Override::Requirement(override_requirement) = entry
            && override_requirement.name == requirement.name
            && override_requirement.evaluate_markers(Some(markers.markers()), &[])
        {
            root_hashes.extend(requirement_hash_groups(override_requirement, &[])?);
        }
    }
    let mut spec = spec;
    let database =
        DistributionDatabase::new(&client, &dispatch, concurrency.downloads_semaphore.clone());
    let mut incompatible_python = Vec::new();
    let (selected, wheel, root_hash) = loop {
        let selected = Resolution::from(
            resolve_environment(
                spec.clone().into(),
                EnvironmentResolution::Direct,
                interpreter,
                python_platform,
                SourceTreeEditablePolicy::Tool,
                build_constraints.clone(),
                &selection_settings,
                client_builder,
                state,
                Box::new(SummaryResolveLogger),
                concurrency,
                cache,
                workspace_cache,
                printer,
                preview,
            )
            .await
            .map_err(|error| RootSelectionError {
                error,
                incompatible_python: std::mem::take(&mut incompatible_python),
            })?,
        );
        let selected_dist = selected
            .distributions()
            .next()
            .context("No compatible tool was selected")?;
        let ResolvedDist::Installable { dist, .. } = selected_dist else {
            bail!("Expected an installable tool distribution");
        };
        if let Dist::Source(_) = dist.as_ref() {
            bail!("A wheel is required to install a tool with `--locked`");
        }

        let mut artifact_hashes = root_hashes.clone();
        if let Dist::Built(BuiltDist::Registry(dist)) = dist.as_ref() {
            let file = &dist.best_wheel().file;
            artifact_hashes.extend(
                file.hashes
                    .iter()
                    .filter(|hash| hash.algorithm() != HashAlgorithm::Md5)
                    .cloned()
                    .map(|hash| vec![hash]),
            );
            artifact_hashes.extend(
                parse_all_url_hashes(&file.url.to_url()?)?
                    .into_iter()
                    .map(|hash| vec![hash]),
            );
        }
        let hasher = HashStrategy::from_resolution(&selected, HashCheckingMode::Verify)?;
        let (wheel, root_hash) = verify_hash_constraints(
            &database,
            dist,
            &tags,
            hasher.archive_policy(dist.as_ref()),
            &artifact_hashes,
        )
        .await?;
        let wheel = CachedDist::from(wheel);

        let metadata = read_flat_wheel_metadata(wheel.filename(), wheel.path())?;
        if let Some(requires_python) = metadata.requires_python.as_ref()
            && !requires_python.contains(interpreter.python_version())
        {
            // Index metadata may omit Requires-Python, particularly for --find-links.
            // Eliminate incompatible releases before considering their packaged locks.
            let mut excluded = requirement.clone();
            if let RequirementSource::Registry { specifier, .. } = &mut excluded.source
                && dist.index().is_some()
            {
                *specifier =
                    VersionSpecifier::not_equals_version(wheel.filename().version.clone()).into();
                incompatible_python.push(requires_python.clone());
                spec.constraints
                    .push(NameRequirementSpecification::from(excluded));
                continue;
            }
            bail!(
                "`{selected_dist}` requires Python `{requires_python}`, but the selected interpreter is Python {}",
                interpreter.python_version()
            );
        }
        break (selected, wheel, root_hash);
    };
    let selected_dist = selected
        .distributions()
        .next()
        .context("No compatible tool was selected")?;
    let dist_info = find_flat_dist_info(wheel.filename(), wheel.path())?;
    let dist_info = wheel.path().join(format!("{dist_info}.dist-info"));
    let contents = match fs_err::read_to_string(dist_info.join("pylock.toml")) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            bail!(
                "`{selected_dist}` does not contain `pylock.toml` in its `.dist-info` directory; `--locked` requires a packaged lock"
            );
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("Failed to read the packaged lock for `{selected_dist}`")
            });
        }
    };
    let mut lock: PylockToml = toml::from_str(&contents)
        .with_context(|| format!("`{selected_dist}` contains an invalid `pylock.toml`"))?;
    for group in &requirement.groups {
        if !lock.dependency_groups.contains(group) {
            bail!(
                "The packaged lock for `{}` does not support the dependency group `{group}`",
                requirement.name
            );
        }
    }
    let mut groups = lock.default_groups.clone();
    groups.extend(requirement.groups.iter().cloned());
    lock.validate_environment(markers.markers())?;
    groups.sort_unstable();
    groups.dedup();
    // Only packages selected for this environment can contribute artifacts to the installation.
    lock.packages
        .retain(|package| package.is_active(markers.markers(), &requirement.extras, &groups));
    let mut override_names = HashSet::new();
    let mut hash_overrides = spec.overrides.clone();
    for entry in &spec.overrides {
        let UnresolvedRequirement::Named(override_requirement) = &entry.requirement else {
            bail!("Expected a named dependency override");
        };
        if override_requirement.evaluate_markers(Some(markers.markers()), &[]) {
            override_names.insert(override_requirement.name.clone());
        }
    }
    for entry in &spec.override_dependencies {
        match entry {
            Override::Requirement(override_requirement) => {
                if override_requirement.hashes()?.is_some() {
                    check_hashed_source(override_requirement)?;
                }
                if override_requirement.evaluate_markers(Some(markers.markers()), &[]) {
                    override_names.insert(override_requirement.name.clone());
                }
                hash_overrides.push(override_requirement.clone().into());
            }
            Override::Package(_) => {}
        }
    }
    let mut excluded_names = HashSet::new();
    for entry in &spec.excludes {
        match entry {
            ExcludeDependency::Dependency(name) => {
                excluded_names.insert(name.clone());
            }
            ExcludeDependency::Package(_) => {
                bail!("Scoped dependency exclusions are not supported with `--locked`");
            }
        }
    }
    let modifiers = DependencyModifiers::new(
        Overrides::from_entries(spec.override_dependencies.clone())?,
        Excludes::from_entries(spec.excludes.clone()),
    );
    let selected_version = selected_dist
        .version()
        .context("The selected tool has no version")?;
    let scopes = std::iter::once((&requirement.name, selected_version)).chain(
        lock.packages
            .iter()
            .filter(|package| {
                !excluded_names.contains(&package.name)
                    && package.is_active(markers.markers(), &requirement.extras, &groups)
            })
            .filter_map(|package| {
                package
                    .version
                    .as_ref()
                    .map(|version| (&package.name, version))
            }),
    );
    let mut scoped_names = HashSet::new();
    for (name, version) in scopes {
        for override_requirement in modifiers.scoped_overrides_for(name, version) {
            if override_requirement.hashes()?.is_some() {
                check_hashed_source(override_requirement)?;
            }
            if override_requirement.evaluate_markers(Some(markers.markers()), &[]) {
                scoped_names.insert(override_requirement.name.clone());
                hash_overrides.push(override_requirement.clone().into());
            }
        }
    }
    override_names.extend(scoped_names);
    if spec.override_dependencies.iter().any(|entry| match entry {
        Override::Package(package) => {
            package.package.name() != &requirement.name
                && override_names.contains(package.package.name())
        }
        Override::Requirement(_) => false,
    }) {
        bail!("Scoped overrides for a replaced package are not supported with `--locked`");
    }
    let mut replaced_names = HashSet::new();
    for package in &lock.packages {
        if package.name != requirement.name
            && !excluded_names.contains(&package.name)
            && override_names.contains(&package.name)
            && package.is_active(markers.markers(), &requirement.extras, &groups)
            && !replaced_names.insert(package.name.clone())
        {
            bail!(
                "The packaged lock selects `{}` more than once",
                package.name
            );
        }
    }
    lock.packages.retain(|package| {
        !excluded_names.contains(&package.name)
            && (package.name == requirement.name || !override_names.contains(&package.name))
    });
    if lock.has_missing_hashes() {
        bail!(
            "The packaged lock for `{selected_dist}` is missing artifact hashes; regenerate the lock before publishing the package"
        );
    }
    let indexed_artifacts = validate_artifact_urls(
        &lock,
        &client,
        &cached_client,
        &settings.index_locations,
        cache,
        concurrency,
    )
    .await
    .with_context(|| {
        format!("The packaged lock for `{selected_dist}` contains unverified artifacts")
    })?;
    // Use the user's configured index, not the publisher-supplied URL, for cache identity.
    for package in &mut lock.packages {
        package.index = Some(
            configured_index(
                package.index.as_ref().map(SafeUrl::as_url),
                &settings.index_locations,
            )?
            .without_credentials()
            .into_owned()
            .with_policy(Stripped),
        );
    }
    let (dependencies, _) = resolve_pylock_toml(
        lock,
        &dist_info,
        interpreter,
        None,
        python_platform,
        &requirement.extras,
        &groups,
        &settings.build_options,
        Some(HashCheckingMode::Verify),
    )?;
    if dependencies
        .distributions()
        .any(|dist| dist.name() == &requirement.name)
    {
        bail!(
            "The packaged lock for `{}` must not include the tool itself",
            requirement.name
        );
    }

    let mut graph = dependencies.graph().clone();
    for node in graph.node_weights_mut() {
        let Node::Dist {
            dist,
            hashes,
            install: true,
            ..
        } = node
        else {
            continue;
        };
        let secure_hashes: Vec<_> = hashes
            .iter()
            .filter(|hash| hash.algorithm() != HashAlgorithm::Md5)
            .cloned()
            .collect();
        if secure_hashes.is_empty() {
            bail!("The packaged lock for `{selected_dist}` has no secure hash for `{dist}`");
        }
        *hashes = secure_hashes.into();
    }
    let mut dependency_hashes = HashMap::<_, Vec<_>>::new();
    for constraint in &spec.constraints {
        let requirement = &constraint.requirement;
        if !requirement.evaluate_markers(Some(markers.markers()), &[]) {
            continue;
        }
        dependency_hashes
            .entry(requirement.name.clone())
            .or_default()
            .extend(requirement_hash_groups(requirement, &constraint.hashes)?);
    }
    let verify_node = async |(node_index, dist, hashes): (_, ResolvedDist, HashDigests),
                             hash_strategy: Option<&HashStrategy>| {
        let mut constraints = dependency_hashes
            .get(dist.name())
            .cloned()
            .unwrap_or_default();
        let ResolvedDist::Installable { dist, .. } = &dist else {
            bail!("Expected an installable locked dependency");
        };
        let (index, filename, file) = match dist.as_ref() {
            Dist::Built(BuiltDist::Registry(wheels)) => {
                let wheel = wheels.best_wheel();
                (
                    &wheel.index,
                    DistFilename::WheelFilename(wheel.filename.clone()),
                    &wheel.file,
                )
            }
            Dist::Source(SourceDist::Registry(sdist)) => {
                let filename = DistFilename::try_from_filename(&sdist.file.filename, &sdist.name)
                    .context("Invalid locked source distribution filename")?;
                (&sdist.index, filename, &sdist.file)
            }
            Dist::Built(BuiltDist::DirectUrl(_) | BuiltDist::Path(_) | BuiltDist::GitPath(_))
            | Dist::Source(
                SourceDist::DirectUrl(_)
                | SourceDist::GitDirectory(_)
                | SourceDist::GitPath(_)
                | SourceDist::Path(_)
                | SourceDist::Directory(_),
            ) => bail!("Expected a registry distribution for a locked dependency"),
        };
        let mut url = file.url.to_url()?;
        constraints.extend(
            parse_all_url_hashes(&url)?
                .into_iter()
                .map(|hash| vec![hash]),
        );
        if let Some(strategy) = hash_strategy {
            constraints.extend(
                strategy
                    .archive_policy_for_url(&url)
                    .digests()
                    .cloned()
                    .map(|hash| vec![hash]),
            );
        }
        url.set_fragment(None);
        let mut matching = indexed_artifacts
            .iter()
            .filter(|artifact| {
                artifact.index.as_str().trim_end_matches('/')
                    == index.without_credentials().as_str().trim_end_matches('/')
                    && artifact.location() == url.as_str()
                    && artifact.filename == filename
            })
            .peekable();
        if matching.peek().is_none() {
            bail!("The selected artifact for `{dist}` is not listed by {index}");
        }
        for artifact in matching {
            if let Some(requires_python) = &artifact.requires_python
                && !requires_python.contains(&markers.markers().python_full_version().version)
            {
                bail!(
                    "`{dist}` requires Python `{requires_python}`, but the selected interpreter is Python {}",
                    markers.markers().python_full_version().version
                );
            }
            let index_hashes: Vec<_> = artifact
                .hashes
                .iter()
                .filter(|hash| hash.algorithm() != HashAlgorithm::Md5)
                .cloned()
                .collect();
            constraints.extend(index_hashes.into_iter().map(|hash| vec![hash]));
            constraints.extend(
                parse_all_url_hashes(&artifact.url)?
                    .into_iter()
                    .map(|hash| vec![hash]),
            );
        }
        let (wheel, verified) = verify_hash_constraints(
            &database,
            dist,
            &tags,
            ArchiveHashPolicy::All(hashes.as_slice()),
            &constraints,
        )
        .await?;
        let wheel = CachedDist::from(wheel);
        let metadata = read_flat_wheel_metadata(wheel.filename(), wheel.path())?;
        if let Some(requires_python) = &metadata.requires_python
            && !requires_python.contains(&markers.markers().python_full_version().version)
        {
            bail!(
                "`{dist}` requires Python `{requires_python}`, but the selected interpreter is Python {}",
                markers.markers().python_full_version().version
            );
        }
        Ok::<_, anyhow::Error>((node_index, HashDigests::from(vec![verified]), metadata))
    };
    let locked_names = dependencies
        .distributions()
        .filter(|dist| !override_names.contains(dist.name()))
        .map(|dist| dist.name().clone())
        .collect::<HashSet<_>>();
    for constraint in &spec.constraints {
        let constraint_requirement = &constraint.requirement;
        if !locked_names.contains(&constraint_requirement.name)
            || !constraint_requirement.evaluate_markers(Some(markers.markers()), &[])
        {
            continue;
        }
        let Some(dist) = dependencies
            .distributions()
            .find(|dist| dist.name() == &constraint_requirement.name)
        else {
            continue;
        };
        let matches = match &constraint_requirement.source {
            RequirementSource::Registry {
                specifier, index, ..
            } => {
                index.is_none()
                    && dist
                        .version()
                        .is_some_and(|version| specifier.contains(version))
            }
            RequirementSource::Url {
                location,
                subdirectory: None,
                ..
            } => {
                let ResolvedDist::Installable { dist, .. } = dist else {
                    bail!("Expected an installable locked dependency");
                };
                let file = match dist.as_ref() {
                    Dist::Built(BuiltDist::Registry(wheels)) => &wheels.best_wheel().file,
                    Dist::Source(SourceDist::Registry(sdist)) => &sdist.file,
                    Dist::Built(
                        BuiltDist::DirectUrl(_) | BuiltDist::Path(_) | BuiltDist::GitPath(_),
                    )
                    | Dist::Source(
                        SourceDist::DirectUrl(_)
                        | SourceDist::GitDirectory(_)
                        | SourceDist::GitPath(_)
                        | SourceDist::Path(_)
                        | SourceDist::Directory(_),
                    ) => bail!("Expected a registry distribution for a locked dependency"),
                };
                let mut artifact_url = file.url.to_url()?;
                artifact_url.set_fragment(None);
                let mut constraint_url = location.clone();
                constraint_url.set_fragment(None);
                artifact_url.as_str() == constraint_url.as_str()
            }
            RequirementSource::Url { .. }
            | RequirementSource::GitDirectory { .. }
            | RequirementSource::GitPath { .. }
            | RequirementSource::Directory { .. }
            | RequirementSource::Path { .. } => false,
        };
        if !matches {
            bail!(
                "The packaged lock selects `{dist}`, which is incompatible with constraint `{constraint_requirement}`"
            );
        }
    }
    if !spec.overrides.is_empty()
        || !spec.override_dependencies.is_empty()
        || !spec.excludes.is_empty()
    {
        let hash_strategy = HashStrategy::from_independent_requirements(
            hash_overrides
                .iter()
                .map(|entry| (&entry.requirement, entry.hashes.as_slice())),
            spec.constraints
                .iter()
                .map(|entry| (&entry.requirement, entry.hashes.as_slice())),
            Some(&markers),
        )?;
        let locked_nodes = graph
            .node_indices()
            .filter_map(|index| match &graph[index] {
                Node::Dist {
                    dist,
                    hashes,
                    install: true,
                } if !override_names.contains(dist.name()) => {
                    Some((dist.name().clone(), (index, dist.clone(), hashes.clone())))
                }
                Node::Dist { .. } | Node::Root => None,
            })
            .collect::<HashMap<_, _>>();
        let locked_versions = locked_nodes
            .iter()
            .filter_map(|(name, (_, dist, _))| {
                dist.version()
                    .map(|version| (name.clone(), version.clone()))
            })
            .collect::<HashMap<_, _>>();
        let root_metadata = read_flat_wheel_metadata(wheel.filename(), wheel.path())?;
        let mut verified_metadata = HashMap::new();
        verified_metadata.insert(requirement.name.clone(), root_metadata);
        let mut override_extras = HashMap::<PackageName, HashSet<ExtraName>>::new();
        let mut modified_spec = spec.clone();
        // A URL constraint already matched the locked artifact. Keep the package's index source
        // in the resolver and check the constraint's hashes when verifying the locked archive.
        modified_spec.constraints.retain(|constraint| {
            !locked_names.contains(&constraint.requirement.name)
                || !matches!(constraint.requirement.source, RequirementSource::Url { .. })
        });
        let mut selected_requirement = requirement.clone();
        // Group membership has already selected the packages from the packaged lock.
        selected_requirement.groups = Box::new([]);
        selected_requirement.source = RequirementSource::from(selected_dist);
        if let RequirementSource::Registry { index, .. } = &mut selected_requirement.source {
            // The selected wheel can come from a find-links location, which is not a package index.
            *index = None;
        }
        modified_spec.requirements = vec![selected_requirement.into()];
        // A standard lock does not provide an installation dependency graph. Keep every selected
        // package unless it was explicitly excluded, even if it becomes unnecessary.
        for dist in dependencies.distributions() {
            modified_spec.requirements.push(
                Requirement {
                    name: dist.name().clone(),
                    extras: Box::new([]),
                    groups: Box::new([]),
                    marker: MarkerTree::default(),
                    source: RequirementSource::from(dist),
                    scope: RequirementScope::Global,
                    origin: None,
                }
                .into(),
            );
        }
        for name in &replaced_names {
            modified_spec.requirements.push(
                Requirement {
                    name: name.clone(),
                    extras: Box::new([]),
                    groups: Box::new([]),
                    marker: MarkerTree::default(),
                    source: RequirementSource::Registry {
                        specifier: VersionSpecifiers::empty(),
                        index: None,
                        conflict: None,
                    },
                    scope: RequirementScope::Global,
                    origin: None,
                }
                .into(),
            );
        }
        modified_spec
            .overrides
            .retain(|entry| match &entry.requirement {
                UnresolvedRequirement::Named(override_requirement) => {
                    override_requirement.name != requirement.name
                }
                UnresolvedRequirement::Unnamed(_) => true,
            });
        modified_spec
            .override_dependencies
            .retain(|entry| match entry {
                Override::Requirement(override_requirement) => {
                    override_requirement.name != requirement.name
                }
                Override::Package(_) => true,
            });
        for (name, (_, dist, _)) in &locked_nodes {
            modified_spec
                .constraints
                .push(NameRequirementSpecification::from(Requirement {
                    name: name.clone(),
                    extras: Box::new([]),
                    groups: Box::new([]),
                    marker: MarkerTree::default(),
                    source: RequirementSource::from(dist),
                    scope: RequirementScope::Global,
                    origin: None,
                }));
        }
        let (resolved, hash_strategy) = loop {
            let mut metadata = settings
                .dependency_metadata
                .values()
                .filter(|entry| {
                    entry.name != requirement.name && !locked_nodes.contains_key(&entry.name)
                })
                .cloned()
                .collect::<Vec<_>>();
            metadata.extend(verified_metadata.values().cloned().map(|entry| {
                let extras = override_extras.get(&entry.name);
                locked_metadata(
                    entry,
                    &locked_versions,
                    &override_names,
                    extras,
                    markers.markers(),
                )
            }));
            for (name, (_, dist, _)) in &locked_nodes {
                if !verified_metadata.contains_key(name) {
                    metadata.push(StaticMetadata {
                        name: name.clone(),
                        version: dist.version().cloned(),
                        requires_dist: Box::new([]),
                        requires_python: None,
                        provides_extra: Box::new([]),
                    });
                }
            }
            let mut modified_settings = settings.clone();
            modified_settings.dependency_metadata = DependencyMetadata::from_entries(metadata);
            let modified_state = PlatformState::default();
            let (output, updated_hasher) = resolve_environment_with_hashes(
                EnvironmentSpecification::from(modified_spec.clone())
                    .with_hash_strategy(hash_strategy.clone()),
                EnvironmentResolution::Specific,
                interpreter,
                python_platform,
                SourceTreeEditablePolicy::Tool,
                build_constraints.clone(),
                &modified_settings,
                client_builder,
                &modified_state,
                Box::new(SummaryResolveLogger),
                concurrency,
                cache,
                workspace_cache,
                printer,
                preview,
            )
            .await
            .map_err(UvError::from)?;
            let mut added = false;
            for index in output.graph.node_indices() {
                let ResolutionGraphNode::Dist(dist) = &output.graph[index] else {
                    continue;
                };
                for override_requirement in
                    modifiers.scoped_overrides_for(&dist.name, &dist.version)
                {
                    if override_requirement.evaluate_markers(Some(markers.markers()), &[])
                        && locked_nodes.contains_key(&override_requirement.name)
                        && !override_names.contains(&override_requirement.name)
                    {
                        bail!(
                            "Scoped overrides introduced by an override cannot replace locked packages"
                        );
                    }
                }
                if !override_names.contains(&dist.name) {
                    continue;
                }
                let mut traversal = Dfs::new(&output.graph, index);
                while let Some(index) = traversal.next(&output.graph) {
                    let ResolutionGraphNode::Dist(dist) = &output.graph[index] else {
                        continue;
                    };
                    if let Some(extra) = dist.kind.extra()
                        && (dist.name == requirement.name || locked_nodes.contains_key(&dist.name))
                    {
                        if dist.name == requirement.name {
                            bail!(
                                "An override requires the extra `{extra}` of locked package `{}`, but extras are not supported with `--locked`",
                                dist.name
                            );
                        }
                        added |= override_extras
                            .entry(dist.name.clone())
                            .or_default()
                            .insert(extra.clone());
                    }
                }
            }
            let resolved = Resolution::from(output);
            for dist in resolved.distributions() {
                let Some((index, locked_dist, hashes)) = locked_nodes.get(dist.name()) else {
                    continue;
                };
                if verified_metadata.contains_key(dist.name()) {
                    continue;
                }
                let (_, verified_hashes, metadata) = verify_node(
                    (*index, locked_dist.clone(), hashes.clone()),
                    Some(&updated_hasher),
                )
                .await?;
                if let Node::Dist { hashes, .. } = &mut graph[*index] {
                    *hashes = verified_hashes;
                }
                verified_metadata.insert(dist.name().clone(), metadata);
                added = true;
            }
            if !added {
                break (resolved, updated_hasher);
            }
        };
        let mut modified_graph = resolved.graph().clone();
        for node in modified_graph.node_weights_mut() {
            let Node::Dist { dist, .. } = node else {
                continue;
            };
            if dist.name() == &requirement.name {
                if let Some(selected) = selected.graph().node_weights().find(|node| match node {
                    Node::Dist { dist, .. } => dist.name() == &requirement.name,
                    Node::Root => false,
                }) {
                    *node = selected.clone();
                }
            } else if let Some((index, _, _)) = locked_nodes.get(dist.name()) {
                *node = graph[*index].clone();
            }
        }

        for node in modified_graph.node_weights_mut() {
            let Node::Dist {
                dist,
                hashes,
                install: true,
            } = node
            else {
                continue;
            };
            if dist.name() == &requirement.name {
                *hashes = vec![root_hash.clone()].into();
                continue;
            }
            if locked_names.contains(dist.name()) {
                continue;
            }
            let ResolvedDist::Installable { dist, .. } = dist else {
                bail!("Expected an installable dependency");
            };
            let mut constraints = dependency_hashes
                .get(dist.name())
                .cloned()
                .unwrap_or_default();
            let index_file = match dist.as_ref() {
                Dist::Built(BuiltDist::Registry(dist)) => Some(&dist.best_wheel().file),
                Dist::Source(SourceDist::Registry(dist)) => Some(&dist.file),
                Dist::Built(
                    BuiltDist::DirectUrl(_) | BuiltDist::Path(_) | BuiltDist::GitPath(_),
                )
                | Dist::Source(
                    SourceDist::DirectUrl(_)
                    | SourceDist::Path(_)
                    | SourceDist::GitDirectory(_)
                    | SourceDist::GitPath(_)
                    | SourceDist::Directory(_),
                ) => None,
            };
            if let Some(file) = index_file {
                constraints.extend(
                    file.hashes
                        .iter()
                        .filter(|hash| hash.algorithm() != HashAlgorithm::Md5)
                        .cloned()
                        .map(|hash| vec![hash]),
                );
                constraints.extend(
                    parse_all_url_hashes(&file.url.to_url()?)?
                        .into_iter()
                        .map(|hash| vec![hash]),
                );
            }
            let direct_url = match dist.as_ref() {
                Dist::Built(BuiltDist::DirectUrl(dist)) => Some(&*dist.url),
                Dist::Built(BuiltDist::Path(dist)) => Some(&*dist.url),
                Dist::Source(SourceDist::DirectUrl(dist)) => Some(&*dist.url),
                Dist::Source(SourceDist::Path(dist)) => Some(&*dist.url),
                Dist::Built(BuiltDist::Registry(_) | BuiltDist::GitPath(_))
                | Dist::Source(
                    SourceDist::Registry(_)
                    | SourceDist::GitDirectory(_)
                    | SourceDist::GitPath(_)
                    | SourceDist::Directory(_),
                ) => None,
            };
            if let Some(url) = direct_url {
                constraints.extend(
                    parse_all_url_hashes(url)?
                        .into_iter()
                        .map(|hash| vec![hash]),
                );
            }
            let policy = hash_strategy.archive_policy(dist.as_ref());
            match dist.as_ref() {
                Dist::Source(
                    SourceDist::Directory(_) | SourceDist::GitDirectory(_) | SourceDist::GitPath(_),
                )
                | Dist::Built(BuiltDist::GitPath(_)) => {
                    if !constraints.is_empty() || policy.requires_validation() {
                        bail!("Cannot verify archive hashes for `{dist}`");
                    }
                    // Git commits and source directories identify these sources independently
                    // of the cached metadata.
                    *hashes = HashDigests::empty();
                }
                Dist::Built(_) | Dist::Source(_) => {
                    let resolution_hashes = hashes
                        .iter()
                        .filter(|hash| hash.algorithm() != HashAlgorithm::Md5)
                        .cloned()
                        .collect::<Vec<_>>();
                    if !resolution_hashes.is_empty() {
                        constraints.push(resolution_hashes);
                    }
                    let (_, hash) =
                        verify_hash_constraints(&database, dist, &tags, policy, &constraints)
                            .await?;
                    *hashes = vec![hash].into();
                }
            }
        }
        return Ok(Resolution::new(modified_graph));
    }

    let nodes = graph
        .node_indices()
        .filter_map(|index| match &graph[index] {
            Node::Dist {
                dist,
                hashes,
                install: true,
            } => Some((index, dist.clone(), hashes.clone())),
            Node::Dist { .. } | Node::Root => None,
        })
        .collect::<Vec<_>>();
    let mut nodes = nodes.into_iter();
    let mut pending = FuturesUnordered::new();
    for node in nodes.by_ref().take(concurrency.downloads) {
        pending.push(verify_node(node, None));
    }
    let mut verified_nodes = Vec::new();
    let mut failure = None;
    while let Some(result) = pending.next().await {
        match result {
            Ok(node) => verified_nodes.push(node),
            Err(err) if failure.is_none() => failure = Some(err),
            Err(_) => {}
        }
        // Stop starting new checks after a failure, but let pending checks finish before returning.
        if failure.is_none()
            && let Some(node) = nodes.next()
        {
            pending.push(verify_node(node, None));
        }
    }
    if let Some(err) = failure {
        return Err(err);
    }
    for (index, hashes, _) in verified_nodes {
        let Node::Dist {
            hashes: destination,
            ..
        } = &mut graph[index]
        else {
            bail!("Expected a locked dependency");
        };
        *destination = hashes;
    }

    let root = graph
        .node_indices()
        .find(|&index| match graph[index] {
            Node::Root => true,
            Node::Dist { .. } => false,
        })
        .context("Expected a root in the packaged lock")?;
    for node in selected.graph().node_weights() {
        if let Node::Dist { .. } = node {
            let mut selected = node.clone();
            if let Node::Dist { hashes, .. } = &mut selected {
                *hashes = vec![root_hash.clone()].into();
            }
            let node = graph.add_node(selected);
            graph.add_edge(root, node, Edge::Prod);
        }
    }
    Ok(Resolution::new(graph))
}

/// Use only dependencies represented in the lock or explicitly overridden by the user.
fn locked_metadata(
    metadata: ResolutionMetadata,
    locked_versions: &HashMap<PackageName, Version>,
    override_names: &HashSet<PackageName>,
    override_extras: Option<&HashSet<ExtraName>>,
    markers: &MarkerEnvironment,
) -> StaticMetadata {
    StaticMetadata {
        name: metadata.name,
        version: Some(metadata.version),
        requires_dist: metadata
            .requires_dist
            .into_vec()
            .into_iter()
            .filter_map(|mut requirement| {
                if let Some(version) = locked_versions.get(&requirement.name) {
                    // A packaged lock may reflect the publisher's overrides, so its versions
                    // need not satisfy the original metadata's version specifiers.
                    requirement.version_or_url = Some(VersionOrUrl::VersionSpecifier(
                        VersionSpecifier::equals_version(version.clone()).into(),
                    ));
                    Some(requirement)
                } else if override_names.contains(&requirement.name)
                    || override_extras.is_some_and(|extras| {
                        !requirement.evaluate_markers(markers, &[])
                            && extras.iter().any(|extra| {
                                requirement.evaluate_markers(markers, std::slice::from_ref(extra))
                            })
                    })
                {
                    Some(requirement)
                } else {
                    None
                }
            })
            .collect(),
        requires_python: metadata.requires_python,
        provides_extra: metadata.provides_extra,
    }
}

/// Check each source archive hash policy before its build backend can run.
/// Check each set of trusted digests against the same archive and return its SHA-256.
///
/// The caller uses that digest to bind installation to the bytes that passed these checks.
async fn verify_hash_constraints(
    database: &DistributionDatabase<'_, BuildDispatch<'_>>,
    dist: &Dist,
    tags: &Tags,
    policy: ArchiveHashPolicy<'_>,
    constraints: &[Vec<HashDigest>],
) -> anyhow::Result<(LocalWheel, HashDigest)> {
    let mut groups = constraints.to_vec();
    match policy {
        ArchiveHashPolicy::None | ArchiveHashPolicy::Generate => {}
        ArchiveHashPolicy::Any(hashes) => groups.push(hashes.to_vec()),
        ArchiveHashPolicy::All([]) => groups.push(Vec::new()),
        ArchiveHashPolicy::All(hashes) => {
            groups.extend(hashes.iter().cloned().map(|hash| vec![hash]));
        }
        ArchiveHashPolicy::AllOfAny {
            groups: other,
            url_hashes,
        } => {
            if let Some(other) = other {
                groups.extend_from_slice(other.groups());
            }
            groups.extend(url_hashes.iter().cloned().map(|hash| vec![hash]));
        }
    }
    let groups = ArchiveHashGroups::new(groups);
    let policy = if groups.groups().is_empty() {
        ArchiveHashPolicy::Generate
    } else {
        ArchiveHashPolicy::all_of_any(&groups)
    };
    let wheel = database.get_or_build_wheel(dist, tags, policy).await?;
    if !wheel.satisfies(policy) {
        return Err(uv_distribution::Error::hash_mismatch(
            dist.to_string(),
            policy.digests(),
            wheel.hashes(),
        )
        .into());
    }
    let sha256 = wheel
        .hashes()
        .iter()
        .find(|hash| hash.algorithm() == HashAlgorithm::Sha256)
        .context("Missing verified archive hash")?
        .clone();
    Ok((wheel, sha256))
}

fn configured_index<'a>(
    locked: Option<&SafeUrl>,
    indexes: &'a IndexLocations,
) -> anyhow::Result<&'a IndexUrl> {
    let locked = locked.unwrap_or(&PYPI_URL);
    indexes
        .indexes()
        .chain(indexes.explicit_indexes())
        .find(|index| {
            index
                .url()
                .without_credentials()
                .as_str()
                .trim_end_matches('/')
                == locked.as_str().trim_end_matches('/')
        })
        .map(Index::url)
        .with_context(|| format!("Index `{locked}` from the packaged lock is not configured"))
}

/// Check the entire packaged lock against its configured indexes before accessing its artifacts.
async fn validate_artifact_urls(
    lock: &PylockToml,
    client: &RegistryClient,
    cached_client: &RegistryClient,
    indexes: &IndexLocations,
    cache: &Cache,
    concurrency: &Concurrency,
) -> anyhow::Result<Vec<IndexedArtifact>> {
    let capabilities = IndexCapabilities::default();
    let artifacts = futures::stream::iter(&lock.packages)
        .map(async |package| {
            let version = package.version.as_ref().with_context(|| {
                format!(
                    "`{}` must have a version to verify its files on the index",
                    package.name
                )
            })?;
            let artifacts = package.registry_artifacts()?.with_context(|| {
                format!(
                    "`{}=={version}` must use wheel or source distribution URLs from an index",
                    package.name
                )
            })?;

            let index = configured_index(package.index.as_ref().map(SafeUrl::as_url), indexes)?;
            // A cache miss for a higher-priority index is not evidence that it lacks the package.
            let first_index = indexes.fetch_indexes().next().is_some_and(|first| same_index(first.url(), index));
            if (client.connectivity().is_offline()
                || (first_index && !cache.must_revalidate_package(&package.name)))
                && let Ok(metadata) = cached_client
                .simple_detail(
                    &package.name,
                    None,
                    &capabilities,
                    &concurrency.downloads_semaphore,
                )
                .await
                && let Ok(expected) = index_artifacts(metadata, index, &package.name, version)
                && artifacts.iter().all(|(url, filename)| {
                    let mut location = (*url).clone();
                    location.set_fragment(None);
                    expected.iter().any(|artifact| artifact.location() == location.as_str() && artifact.filename == *filename)
                })
            {
                return Ok(expected);
            }
            let metadata = client
                .simple_detail(
                    &package.name,
                    None,
                    &capabilities,
                    &concurrency.downloads_semaphore,
                )
                .await?;
            if !metadata.iter().any(|(selected, _)| same_index(selected, index)) {
                bail!(
                    "Index `{index}` from the packaged lock is not selected for `{}` by the configured index strategy",
                    package.name
                );
            }
            let expected = index_artifacts(metadata, index, &package.name, version)?;
            for (url, filename) in artifacts {
                let mut location = url.clone();
                // Simple API HTML links may include a hash fragment, which is not sent to the server.
                location.set_fragment(None);
                if !expected.iter().any(|artifact| artifact.location() == location.as_str()) {
                    bail!(
                        "URL for `{}=={version}` is not listed by {index}: {url}",
                        package.name,
                    );
                }
                if !expected.iter().any(|artifact| artifact.location() == location.as_str() && artifact.filename == filename) {
                    bail!("Filename `{filename}` for `{}=={version}` does not match the file listed at {url} by {index}", package.name);
                }
            }
            Ok::<_, anyhow::Error>(expected)
        })
        .buffer_unordered(concurrency.downloads)
        .try_collect::<Vec<_>>()
        .await?;
    Ok(artifacts.into_iter().flatten().collect())
}

struct IndexedArtifact {
    index: SafeUrl,
    filename: DistFilename,
    url: SafeUrl,
    requires_python: Option<Arc<VersionSpecifiers>>,
    hashes: HashDigests,
}

impl IndexedArtifact {
    /// Return the URL sent to the server, excluding the fragment containing any hashes.
    fn location(&self) -> &str {
        self.url
            .as_str()
            .split_once('#')
            .map_or(self.url.as_str(), |(location, _)| location)
    }
}

fn same_index(left: &IndexUrl, right: &IndexUrl) -> bool {
    left.without_credentials().as_str().trim_end_matches('/')
        == right.without_credentials().as_str().trim_end_matches('/')
}

fn index_artifacts(
    metadata: Vec<(&IndexUrl, MetadataFormat)>,
    configured_index: &IndexUrl,
    package: &uv_normalize::PackageName,
    version: &Version,
) -> anyhow::Result<Vec<IndexedArtifact>> {
    let mut artifacts = Vec::new();
    for (index, metadata) in metadata {
        if !same_index(index, configured_index) {
            continue;
        }
        match metadata {
            MetadataFormat::Simple(metadata) => {
                for datum in metadata.iter() {
                    if rkyv::deserialize::<Version, rkyv::rancor::Error>(&datum.version)?
                        != *version
                    {
                        continue;
                    }
                    let files =
                        rkyv::deserialize::<VersionFiles, rkyv::rancor::Error>(&datum.files)?;
                    for (filename, file) in files.all(package) {
                        artifacts.push(IndexedArtifact {
                            index: configured_index.without_credentials().into_owned(),
                            filename,
                            url: file.url.to_url()?,
                            requires_python: file.requires_python,
                            hashes: file.hashes,
                        });
                    }
                }
            }
            MetadataFormat::Flat(_) => bail!("Expected Simple API metadata from {index}"),
        }
    }
    Ok(artifacts)
}
