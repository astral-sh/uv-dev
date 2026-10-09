//! Collecting the requirements to compile, sync or install.
//!
//! # `requirements.txt` format
//!
//! The `requirements.txt` format (also known as `requirements.in`) is static except for the
//! possibility of making network requests.
//!
//! All entries are stored as `requirements` and `editables` or `constraints`  depending on the kind
//! of inclusion (`uv pip install -r` and `uv pip compile` vs. `uv pip install -c` and
//! `uv pip compile -c`).
//!
//! # `pyproject.toml` and directory source.
//!
//! `pyproject.toml` files come in two forms: PEP 621 compliant with static dependencies and non-PEP 621
//! compliant or PEP 621 compliant with dynamic metadata. There are different ways how the requirements are evaluated:
//! * `uv pip install -r pyproject.toml` or `uv pip compile requirements.in`: The `pyproject.toml`
//!   must be valid (in other circumstances we allow invalid `dependencies` e.g. for hatch's
//!   relative path support), but it can be dynamic. We set the `project` from the `name` entry. If it is static, we add
//!   all `dependencies` from the pyproject.toml as `requirements` (and drop the directory). If it
//!   is dynamic, we add the directory to `source_trees`.
//! * `uv pip install .` in a directory with `pyproject.toml` or `uv pip compile requirements.in`
//!   where the `requirements.in` points to that directory: The directory is listed in
//!   `requirements`. The lookahead resolver reads the static metadata from `pyproject.toml` if
//!   available, otherwise it calls PEP 517 to resolve.
//! * `uv pip install -e`: We add the directory in `editables` instead of `requirements`. The
//!   lookahead resolver resolves it the same.
//! * `setup.py` or `setup.cfg` instead of `pyproject.toml`: Directory is an entry in
//!   `source_trees`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rustc_hash::FxHashSet;
use tracing::instrument;
use url::Url;

use uv_auth::CredentialsCache;
use uv_cache::Cache;
use uv_cache_key::CanonicalUrl;
use uv_client::BaseClientBuilder;
use uv_configuration::{
    DependencyGroups, ExcludeDependency, NoBinary, NoBuild, NoSources, Override, RequirementsInput,
};
use uv_distribution_types::{Index, Requirement, RequirementSource};
use uv_distribution_types::{
    IndexLocations, IndexUrl, NameRequirementSpecification, UnresolvedRequirement,
    UnresolvedRequirementSpecification,
};
use uv_fs::{CWD, Simplified};
use uv_normalize::{ExtraName, PackageName, PipGroupName};
use uv_pep508::VerbatimUrl;
use uv_pypi_types::PyProjectToml;
use uv_requirements_txt::{RequirementsTxt, RequirementsTxtRequirement, SourceCache};
use uv_scripts::Pep723Metadata;
use uv_warnings::warn_user;
use uv_workspace::WorkspaceCache;

use crate::script::script_metadata_specification;
use crate::{RequirementsSource, SourceTree};

/// The sections of inline metadata consumed by a requirements-file input.
#[derive(Debug, Clone, Copy)]
enum InputRole {
    Requirements,
    Constraints,
    Overrides,
    Excludes,
}

/// Settings and shared state used to lower requirements from inline script metadata.
#[derive(Debug, Clone, Copy)]
pub struct LoweringContext<'a> {
    role: InputRole,
    sources: &'a NoSources,
    index_locations: &'a IndexLocations,
    cache: &'a Cache,
    workspace_cache: &'a WorkspaceCache,
    credentials_cache: &'a CredentialsCache,
}

impl<'a> LoweringContext<'a> {
    pub fn new(
        sources: &'a NoSources,
        index_locations: &'a IndexLocations,
        cache: &'a Cache,
        workspace_cache: &'a WorkspaceCache,
        credentials_cache: &'a CredentialsCache,
    ) -> Self {
        Self {
            role: InputRole::Requirements,
            sources,
            index_locations,
            cache,
            workspace_cache,
            credentials_cache,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct RequirementsSpecification {
    /// The name of the project specifying requirements.
    pub project: Option<PackageName>,
    /// The requirements for the project.
    pub requirements: Vec<UnresolvedRequirementSpecification>,
    /// The constraints for the project.
    pub constraints: Vec<NameRequirementSpecification>,
    /// The overrides for the project.
    pub overrides: Vec<UnresolvedRequirementSpecification>,
    /// The overrides that have already been lowered to named requirements.
    pub override_dependencies: Vec<Override<Requirement>>,
    /// The excludes for the project.
    pub excludes: Vec<ExcludeDependency>,
    /// The `pylock.toml` file from which to extract the resolution.
    pub pylock: Option<RequirementsInput>,
    /// The dependency groups to use for a `pylock.toml` input.
    pub pylock_groups: DependencyGroups,
    /// The source trees from which to extract requirements.
    pub source_trees: Vec<SourceTree>,
    /// The groups to use for `source_trees`
    pub groups: BTreeMap<PathBuf, DependencyGroups>,
    /// The extras used to collect requirements.
    pub extras: FxHashSet<ExtraName>,
    /// Full definitions of indexes selected by lowered script sources.
    pub indexes: Vec<Index>,
    /// The index URL to use for fetching packages.
    pub index_url: Option<IndexUrl>,
    /// The extra index URLs to use for fetching packages.
    pub extra_index_urls: Vec<IndexUrl>,
    /// Whether to disallow index usage.
    pub no_index: bool,
    /// Whether all requirements must be hashed.
    pub require_hashes: bool,
    /// The `--find-links` locations to use for fetching packages.
    pub find_links: Vec<IndexUrl>,
    /// The `--no-binary` flags to enforce when selecting distributions.
    pub no_binary: NoBinary,
    /// The `--no-build` flags to enforce when selecting distributions.
    pub no_build: NoBuild,
}

impl RequirementsSpecification {
    /// Merge source indexes without losing policies to name-based client deduplication.
    fn extend_indexes(&mut self, indexes: impl IntoIterator<Item = Index>) -> Result<()> {
        for index in indexes {
            if let Some(name) = index.name.as_ref()
                && let Some(existing) = self
                    .indexes
                    .iter()
                    .find(|existing| existing.name.as_ref() == Some(name))
            {
                if existing != &index {
                    return Err(anyhow::anyhow!(
                        "Conflicting definitions for index `{name}` in requirements sources"
                    ));
                }
                continue;
            }
            if index.default && self.indexes.iter().any(|existing| existing.default) {
                return Err(anyhow::anyhow!(
                    "Multiple default indexes in requirements sources"
                ));
            }
            self.indexes.push(index);
        }
        Ok(())
    }

    /// Read the requirements and constraints from a source.
    #[instrument(skip_all, level = tracing::Level::DEBUG, fields(source = % source))]
    pub async fn from_source(
        source: &RequirementsSource,
        client_builder: &BaseClientBuilder<'_>,
        lowering_context: LoweringContext<'_>,
    ) -> Result<Self> {
        Self::from_source_with_cache(
            source,
            client_builder,
            lowering_context,
            &mut SourceCache::default(),
        )
        .await
    }

    /// Create a [`RequirementsSpecification`] from PEP 723 script metadata.
    async fn from_pep723_metadata(
        mut metadata: Pep723Metadata,
        input: &RequirementsInput,
        lowering_context: LoweringContext<'_>,
    ) -> Result<Self> {
        // Discard ignored sections before lowering can discover workspaces or select index policies.
        if let Some(tool_uv) = metadata.tool.as_mut().and_then(|tool| tool.uv.as_mut()) {
            match lowering_context.role {
                InputRole::Requirements => {}
                InputRole::Constraints => {
                    tool_uv.override_dependencies = None;
                    tool_uv.exclude_dependencies = None;
                }
                InputRole::Overrides => {
                    tool_uv.constraint_dependencies = None;
                    tool_uv.exclude_dependencies = None;
                }
                InputRole::Excludes => {
                    tool_uv.constraint_dependencies = None;
                    tool_uv.override_dependencies = None;
                }
            }
        }
        let tool_uv = metadata.tool.as_ref().and_then(|tool| tool.uv.as_ref());
        let script_dir = match input {
            RequirementsInput::Stdin | RequirementsInput::Remote(_) => CWD.to_path_buf(),
            RequirementsInput::Local(path) => std::path::absolute(path)?
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| CWD.to_path_buf()),
        };

        let (mut specification, indexes) = script_metadata_specification(
            &metadata,
            &script_dir,
            lowering_context.sources,
            lowering_context.index_locations,
            lowering_context.cache,
            lowering_context.workspace_cache,
            lowering_context.credentials_cache,
        )
        .await?;
        // Requirements-file inputs add their selected definitions to the caller's settings.
        specification.indexes = indexes;

        // Requirements files are consumed relative to the invoking directory. Script sources
        // instead use the script's directory, so emit their resolved paths when compiling them.
        let absolute_path = |requirement: &mut Requirement| match &mut requirement.source {
            RequirementSource::Path { url, .. } | RequirementSource::Directory { url, .. } => {
                if url.prefers_relative() {
                    *url = VerbatimUrl::from_url(url.to_url());
                }
            }
            RequirementSource::Registry { .. }
            | RequirementSource::Url { .. }
            | RequirementSource::GitDirectory { .. }
            | RequirementSource::GitPath { .. } => {}
        };
        for requirement in &mut specification.requirements {
            if let UnresolvedRequirement::Named(requirement) = &mut requirement.requirement {
                absolute_path(requirement);
            }
        }
        for constraint in &mut specification.constraints {
            absolute_path(&mut constraint.requirement);
        }
        for entry in &mut specification.override_dependencies {
            match entry {
                Override::Requirement(requirement) => absolute_path(requirement),
                Override::Package(package) => {
                    for requirement in &mut package.dependencies {
                        absolute_path(requirement);
                    }
                }
            }
        }

        if let Some(tool_uv) = tool_uv {
            Ok(Self {
                index_url: tool_uv
                    .top_level
                    .index_url
                    .as_ref()
                    .map(|index| Index::from(index.clone()).url),
                extra_index_urls: tool_uv
                    .top_level
                    .extra_index_url
                    .as_ref()
                    .into_iter()
                    .flat_map(|urls| urls.iter().map(|index| Index::from(index.clone()).url))
                    .collect(),
                no_index: tool_uv.top_level.no_index.unwrap_or_default(),
                find_links: tool_uv
                    .top_level
                    .find_links
                    .as_ref()
                    .into_iter()
                    .flat_map(|urls| urls.iter().map(|index| Index::from(index.clone()).url))
                    .collect(),
                no_binary: NoBinary::from_args(
                    tool_uv.top_level.no_binary,
                    tool_uv
                        .top_level
                        .no_binary_package
                        .clone()
                        .unwrap_or_default(),
                ),
                no_build: NoBuild::from_args(
                    tool_uv.top_level.no_build,
                    tool_uv
                        .top_level
                        .no_build_package
                        .clone()
                        .unwrap_or_default(),
                ),
                ..specification
            })
        } else {
            Ok(specification)
        }
    }

    /// Create a [`RequirementsSpecification`] from a parsed `requirements.txt` file.
    fn from_requirements_txt(requirements_txt: RequirementsTxt) -> Self {
        Self {
            requirements: requirements_txt
                .requirements
                .into_iter()
                .map(UnresolvedRequirementSpecification::from)
                .chain(
                    requirements_txt
                        .editables
                        .into_iter()
                        .map(UnresolvedRequirementSpecification::from),
                )
                .collect(),
            constraints: requirements_txt
                .constraints
                .into_iter()
                .map(Requirement::from)
                .map(NameRequirementSpecification::from)
                .collect(),
            index_url: requirements_txt.index_url.map(IndexUrl::from),
            extra_index_urls: requirements_txt
                .extra_index_urls
                .into_iter()
                .map(IndexUrl::from)
                .collect(),
            no_index: requirements_txt.no_index,
            find_links: requirements_txt
                .find_links
                .into_iter()
                .map(IndexUrl::from)
                .collect(),
            no_binary: requirements_txt.no_binary,
            no_build: requirements_txt.only_binary,
            require_hashes: requirements_txt.require_hashes,
            ..Self::default()
        }
    }

    /// Read the requirements and constraints from a source, using a cache for file contents.
    #[instrument(skip_all, level = tracing::Level::DEBUG, fields(source = % source))]
    async fn from_source_with_cache(
        source: &RequirementsSource,
        client_builder: &BaseClientBuilder<'_>,
        lowering_context: LoweringContext<'_>,
        cache: &mut SourceCache,
    ) -> Result<Self> {
        Ok(match source {
            RequirementsSource::Package(requirement) => Self {
                requirements: vec![UnresolvedRequirementSpecification::from(
                    requirement.clone(),
                )],
                ..Self::default()
            },
            RequirementsSource::Editable(requirement) => {
                let mut requirement = requirement.clone();
                requirement.make_editable().with_context(|| {
                    format!("Unsupported editable requirement: `{requirement}`")
                })?;
                Self {
                    requirements: vec![UnresolvedRequirementSpecification::from(requirement)],
                    ..Self::default()
                }
            }
            RequirementsSource::RequirementsTxt(input) => {
                if let RequirementsInput::Local(path) = input
                    && !path.exists()
                {
                    return Err(anyhow::anyhow!("File not found: {}", path.user_display()));
                }

                let requirements_txt =
                    RequirementsTxt::parse_with_cache(input.clone(), &*CWD, client_builder, cache)
                        .await?;

                if requirements_txt == RequirementsTxt::default() {
                    warn_user!(
                        "Requirements file `{}` does not contain any dependencies",
                        input.user_display()
                    );
                }

                Self::from_requirements_txt(requirements_txt)
            }
            RequirementsSource::PyprojectToml(path) => {
                let content = match fs_err::tokio::read_to_string(&path).await {
                    Ok(content) => content,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        return Err(anyhow::anyhow!("File not found: {}", path.user_display()));
                    }
                    Err(err) => {
                        return Err(anyhow::anyhow!(
                            "Failed to read `{}`: {}",
                            path.user_display(),
                            err
                        ));
                    }
                };
                let pyproject_toml = PyProjectToml::from_toml(&content, path.user_display())
                    .with_context(|| format!("Failed to parse: {}", path.user_display()))?;

                Self {
                    source_trees: vec![SourceTree::PyProjectToml(path.clone(), pyproject_toml)],
                    ..Self::default()
                }
            }
            RequirementsSource::Pep723Script(input) => {
                let content = if let Some(content) = cache.get(input) {
                    content.clone()
                } else {
                    let content = read_file(input, client_builder).await?;
                    cache.insert(input.clone(), content.clone());
                    content
                };

                let metadata = match Pep723Metadata::parse(content.as_bytes()) {
                    Ok(Some(script)) => script,
                    Ok(None) => {
                        return Err(anyhow::anyhow!(
                            "`{}` does not contain inline script metadata",
                            input.user_display(),
                        ));
                    }
                    Err(err) => return Err(err.into()),
                };

                Self::from_pep723_metadata(metadata, input, lowering_context).await?
            }
            RequirementsSource::SetupPy(path) => {
                if !path.is_file() {
                    return Err(anyhow::anyhow!("File not found: {}", path.user_display()));
                }

                Self {
                    source_trees: vec![SourceTree::SetupPy(path.clone())],
                    ..Self::default()
                }
            }
            RequirementsSource::SetupCfg(path) => {
                if !path.is_file() {
                    return Err(anyhow::anyhow!("File not found: {}", path.user_display()));
                }

                Self {
                    source_trees: vec![SourceTree::SetupCfg(path.clone())],
                    ..Self::default()
                }
            }
            RequirementsSource::PylockToml(input) => {
                if let RequirementsInput::Local(path) = input
                    && !path.exists()
                {
                    return Err(anyhow::anyhow!("File not found: {}", path.user_display()));
                }

                Self {
                    pylock: Some(input.clone()),
                    ..Self::default()
                }
            }
            RequirementsSource::EnvironmentYml(input) => {
                return Err(anyhow::anyhow!(
                    "Conda environment files (i.e., `{}`) are not supported",
                    input.user_display()
                ));
            }
            RequirementsSource::Extensionless(input) => {
                let content = if let Some(content) = cache.get(input) {
                    content.clone()
                } else {
                    let content = read_file(input, client_builder).await?;
                    cache.insert(input.clone(), content.clone());
                    content
                };

                // Detect if it's a PEP 723 script.
                if let Some(metadata) = Pep723Metadata::parse(content.as_bytes())? {
                    Self::from_pep723_metadata(metadata, input, lowering_context).await?
                } else {
                    // If it's not a PEP 723 script, assume it's a `requirements.txt` file.
                    let requirements_txt = RequirementsTxt::parse_str(
                        &content,
                        input.clone(),
                        &*CWD,
                        client_builder,
                        cache,
                    )
                    .await?;

                    if requirements_txt == RequirementsTxt::default() {
                        match input {
                            RequirementsInput::Stdin => {
                                warn_user!("No dependencies found in stdin");
                            }
                            RequirementsInput::Local(_) | RequirementsInput::Remote(_) => {
                                warn_user!(
                                    "Requirements file `{}` does not contain any dependencies",
                                    input.user_display()
                                );
                            }
                        }
                    }

                    Self::from_requirements_txt(requirements_txt)
                }
            }
        })
    }

    /// Read the combined requirements and constraints from a set of sources.
    pub async fn from_sources(
        requirements: &[RequirementsSource],
        constraints: &[RequirementsSource],
        overrides: &[RequirementsSource],
        excludes: &[RequirementsSource],
        groups: Option<&GroupsSpecification>,
        client_builder: &BaseClientBuilder<'_>,
        lowering_context: LoweringContext<'_>,
    ) -> Result<Self> {
        let mut spec = Self::default();
        let mut cache = SourceCache::default();

        // Disallow `pylock.toml` files as constraints.
        if let Some(pylock_toml) = constraints.iter().find_map(|source| {
            if let RequirementsSource::PylockToml(path) = source {
                Some(path)
            } else {
                None
            }
        }) {
            return Err(anyhow::anyhow!(
                "Cannot use `{}` as a constraint file",
                pylock_toml.user_display()
            ));
        }

        // Disallow `pylock.toml` files as overrides.
        if let Some(pylock_toml) = overrides.iter().find_map(|source| {
            if let RequirementsSource::PylockToml(path) = source {
                Some(path)
            } else {
                None
            }
        }) {
            return Err(anyhow::anyhow!(
                "Cannot use `{}` as an override file",
                pylock_toml.user_display()
            ));
        }

        // Disallow `pylock.toml` files as excludes.
        if let Some(pylock_toml) = excludes.iter().find_map(|source| {
            if let RequirementsSource::PylockToml(path) = source {
                Some(path)
            } else {
                None
            }
        }) {
            return Err(anyhow::anyhow!(
                "Cannot use `{}` as an exclude file",
                pylock_toml.user_display()
            ));
        }

        // If we have a `pylock.toml`, don't allow additional requirements, constraints, or
        // overrides.
        if requirements
            .iter()
            .any(|source| matches!(source, RequirementsSource::PylockToml(_)))
        {
            if requirements
                .iter()
                .any(|source| !matches!(source, RequirementsSource::PylockToml(..)))
            {
                return Err(anyhow::anyhow!(
                    "Cannot specify additional requirements alongside a `pylock.toml` file",
                ));
            }
            if !constraints.is_empty() {
                return Err(anyhow::anyhow!(
                    "Cannot specify constraints with a `pylock.toml` file"
                ));
            }
            if !overrides.is_empty() {
                return Err(anyhow::anyhow!(
                    "Cannot specify overrides with a `pylock.toml` file"
                ));
            }

            // If we have a `pylock.toml`, disallow specifying paths for groups; instead, require
            // that all groups refer to the `pylock.toml` file.
            if let Some(groups) = groups {
                let mut names = Vec::new();
                for group in &groups.groups {
                    if group.path.is_some() {
                        return Err(anyhow::anyhow!(
                            "Cannot specify paths for groups with a `pylock.toml` file; all groups must refer to the `pylock.toml` file"
                        ));
                    }
                    names.push(group.name.clone());
                }

                if !names.is_empty() {
                    spec.pylock_groups = DependencyGroups::from_args(
                        None,
                        Vec::new(),
                        Vec::new(),
                        false,
                        names,
                        false,
                    );
                }
            }
        } else if let Some(groups) = groups {
            // pip `--group` flags specify their own sources, which we need to process here.
            // First, we collect all groups by their path.
            let mut groups_by_path = BTreeMap::new();
            for group in &groups.groups {
                // If there's no path provided, expect a pyproject.toml in the project-dir
                // (Which is typically the current working directory, matching pip's behaviour)
                let pyproject_path = group
                    .path
                    .clone()
                    .unwrap_or_else(|| groups.root.join("pyproject.toml"));
                groups_by_path
                    .entry(pyproject_path)
                    .or_insert_with(Vec::new)
                    .push(group.name.clone());
            }

            let mut group_specs = BTreeMap::new();
            for (path, groups) in groups_by_path {
                let group_spec =
                    DependencyGroups::from_args(None, Vec::new(), Vec::new(), false, groups, false);
                group_specs.insert(path, group_spec);
            }
            spec.groups = group_specs;
        }

        // Resolve sources into specifications so we know their `source_tree`.
        let mut requirement_sources = Vec::new();
        for source in requirements {
            let source =
                Self::from_source_with_cache(source, client_builder, lowering_context, &mut cache)
                    .await?;
            requirement_sources.push(source);
        }

        // Read all requirements, and keep track of all requirements _and_ constraints.
        // A `requirements.txt` can contain a `-c constraints.txt` directive within it, so reading
        // a requirements file can also add constraints.
        for source in requirement_sources {
            spec.requirements.extend(source.requirements);
            spec.constraints.extend(source.constraints);
            spec.overrides.extend(source.overrides);
            spec.override_dependencies
                .extend(source.override_dependencies);
            spec.excludes.extend(source.excludes);
            spec.extras.extend(source.extras);
            spec.source_trees.extend(source.source_trees);

            // Allow at most one `pylock.toml`.
            if let Some(pylock) = source.pylock {
                if let Some(existing) = spec.pylock {
                    return Err(anyhow::anyhow!(
                        "Multiple `pylock.toml` files specified: `{}` vs. `{}`",
                        existing.user_display(),
                        pylock.user_display(),
                    ));
                }
                spec.pylock = Some(pylock);
            }

            // Use the first project name discovered.
            if spec.project.is_none() {
                spec.project = source.project;
            }

            if let Some(index_url) = source.index_url {
                if let Some(existing) = spec.index_url
                    && CanonicalUrl::new(index_url.url().clone())
                        != CanonicalUrl::new(existing.url().clone())
                {
                    return Err(anyhow::anyhow!(
                        "Multiple index URLs specified: `{existing}` vs. `{index_url}`",
                    ));
                }
                spec.index_url = Some(index_url);
            }
            spec.no_index |= source.no_index;
            spec.extend_indexes(source.indexes)?;
            spec.extra_index_urls.extend(source.extra_index_urls);
            spec.find_links.extend(source.find_links);
            spec.no_binary.extend(source.no_binary);
            spec.no_build.extend(source.no_build);
            spec.require_hashes |= source.require_hashes;
        }

        // Read all constraints, treating both requirements _and_ constraints as constraints.
        // Overrides are ignored.
        for source in constraints {
            let source = Self::from_source_with_cache(
                source,
                client_builder,
                LoweringContext {
                    role: InputRole::Constraints,
                    ..lowering_context
                },
                &mut cache,
            )
            .await?;
            for entry in source.requirements {
                match entry.requirement {
                    UnresolvedRequirement::Named(requirement) => {
                        spec.constraints.push(NameRequirementSpecification {
                            requirement,
                            hashes: entry.hashes,
                        });
                    }
                    UnresolvedRequirement::Unnamed(requirement) => {
                        return Err(anyhow::anyhow!(
                            "Unnamed requirements are not allowed as constraints (found: `{requirement}`)"
                        ));
                    }
                }
            }
            spec.constraints.extend(source.constraints);

            if let Some(index_url) = source.index_url {
                if let Some(existing) = spec.index_url
                    && CanonicalUrl::new(index_url.url().clone())
                        != CanonicalUrl::new(existing.url().clone())
                {
                    return Err(anyhow::anyhow!(
                        "Multiple index URLs specified: `{existing}` vs. `{index_url}`",
                    ));
                }
                spec.index_url = Some(index_url);
            }
            spec.no_index |= source.no_index;
            spec.extend_indexes(source.indexes)?;
            spec.extra_index_urls.extend(source.extra_index_urls);
            spec.find_links.extend(source.find_links);
            spec.no_binary.extend(source.no_binary);
            spec.no_build.extend(source.no_build);
            spec.require_hashes |= source.require_hashes;
        }

        // Read all overrides, treating both requirements _and_ overrides as overrides.
        // Constraints are ignored.
        for source in overrides {
            let source = Self::from_source_with_cache(
                source,
                client_builder,
                LoweringContext {
                    role: InputRole::Overrides,
                    ..lowering_context
                },
                &mut cache,
            )
            .await?;
            spec.overrides.extend(source.requirements);
            spec.overrides.extend(source.overrides);
            spec.override_dependencies
                .extend(source.override_dependencies);

            if let Some(index_url) = source.index_url {
                if let Some(existing) = spec.index_url
                    && CanonicalUrl::new(index_url.url().clone())
                        != CanonicalUrl::new(existing.url().clone())
                {
                    return Err(anyhow::anyhow!(
                        "Multiple index URLs specified: `{existing}` vs. `{index_url}`",
                    ));
                }
                spec.index_url = Some(index_url);
            }
            spec.no_index |= source.no_index;
            spec.extend_indexes(source.indexes)?;
            spec.extra_index_urls.extend(source.extra_index_urls);
            spec.find_links.extend(source.find_links);
            spec.no_binary.extend(source.no_binary);
            spec.no_build.extend(source.no_build);
            spec.require_hashes |= source.require_hashes;
        }

        // Collect excludes.
        for source in excludes {
            let source = Self::from_source_with_cache(
                source,
                client_builder,
                LoweringContext {
                    role: InputRole::Excludes,
                    sources: &NoSources::All,
                    ..lowering_context
                },
                &mut cache,
            )
            .await?;
            for req_spec in source.requirements {
                match req_spec.requirement {
                    UnresolvedRequirement::Named(requirement) => {
                        spec.excludes
                            .push(ExcludeDependency::Dependency(requirement.name));
                    }
                    UnresolvedRequirement::Unnamed(requirement) => {
                        return Err(anyhow::anyhow!(
                            "Unnamed requirements are not allowed as exclusions (found: `{requirement}`)"
                        ));
                    }
                }
            }
            spec.excludes.extend(source.excludes);
        }

        Ok(spec)
    }

    /// Parse an individual package requirement.
    pub fn parse_package(name: &str) -> Result<UnresolvedRequirementSpecification> {
        let requirement = RequirementsTxtRequirement::parse(name, &*CWD, false)
            .with_context(|| format!("Failed to parse: `{name}`"))?;
        Ok(UnresolvedRequirementSpecification::from(requirement))
    }

    /// Read the requirements from a set of sources.
    pub async fn from_simple_sources(
        requirements: &[RequirementsSource],
        client_builder: &BaseClientBuilder<'_>,
        lowering_context: LoweringContext<'_>,
    ) -> Result<Self> {
        Self::from_sources(
            requirements,
            &[],
            &[],
            &[],
            None,
            client_builder,
            lowering_context,
        )
        .await
    }

    /// Initialize a [`RequirementsSpecification`] from a list of [`Requirement`], including
    /// constraints, overrides, and excludes.
    pub fn from_excludes(
        requirements: Vec<Requirement>,
        constraints: Vec<Requirement>,
        overrides: Vec<Requirement>,
        excludes: Vec<ExcludeDependency>,
    ) -> Self {
        Self {
            requirements: requirements
                .into_iter()
                .map(UnresolvedRequirementSpecification::from)
                .collect(),
            constraints: constraints
                .into_iter()
                .map(NameRequirementSpecification::from)
                .collect(),
            overrides: overrides
                .into_iter()
                .map(UnresolvedRequirementSpecification::from)
                .collect(),
            excludes,
            ..Self::default()
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct GroupsSpecification {
    /// The path to the project root, relative to which the default `pyproject.toml` file is
    /// located.
    pub root: PathBuf,
    /// The enabled groups.
    pub groups: Vec<PipGroupName>,
}

/// Read the contents of a requirements input.
async fn read_file(
    input: &RequirementsInput,
    client_builder: &BaseClientBuilder<'_>,
) -> Result<String> {
    match input {
        RequirementsInput::Stdin => Ok(uv_fs::read_stdin_to_string_transcode()?),
        RequirementsInput::Remote(url) => {
            let client = client_builder.build()?;
            let response = client
                .for_host(url)
                .get(Url::from(url.clone()))
                .send()
                .await?;

            response.error_for_status_ref()?;

            Ok(response.text().await?)
        }
        RequirementsInput::Local(path) => Ok(uv_fs::read_to_string_transcode(path).await?),
    }
}
