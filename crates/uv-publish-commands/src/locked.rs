use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};

use uv_auth::CredentialsCache;
use uv_cache::Cache;
use uv_configuration::DependencyGroupsWithDefaults;
use uv_distribution::RequiresDist;
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{File, IndexLocations, IndexUrl, Requirement, RequiresPython};
use uv_lock::{Lock, PylockToml};
use uv_lock_operations::LockTarget;
use uv_normalize::{ExtraName, PackageName};
use uv_pep440::{Version, VersionSpecifiers, release_specifiers_to_ranges};
use uv_publish::PreparedDistribution;
use uv_pypi_types::{
    LenientRequirement, LenientVersionSpecifiers, PyProjectToml, ResolutionMetadata,
};
use uv_python_discovery::find_requires_python;
use uv_static::TarBackend;
use uv_workspace::{DiscoveryOptions, Workspace, WorkspaceCache};

/// Files from releases this invocation uploaded or verified on the publishing index.
pub(crate) type PublishedArtifacts = BTreeMap<PackageName, Vec<(DistFilename, File)>>;

/// Preflighted runtime locks for the packages selected for publication.
pub(crate) struct PublishLocks {
    order: Vec<PackageName>,
    locks: BTreeMap<PackageName, PublishLock>,
    needed: BTreeSet<PackageName>,
    index: Option<IndexUrl>,
    allow_non_pypi: bool,
}

struct PublishLock {
    pylock: PylockToml,
    workspace_dependencies: BTreeMap<PackageName, Version>,
}

impl PublishLocks {
    /// Validate local metadata and compute the upload order before uploading any artifacts.
    pub(crate) async fn load(
        project_dir: &Path,
        distributions: &[PreparedDistribution],
        index: Option<&IndexUrl>,
        allow_non_pypi: bool,
        cache: &Cache,
        index_locations: &IndexLocations,
        credentials_cache: &CredentialsCache,
    ) -> Result<Self> {
        let workspace_cache = WorkspaceCache::default();
        let workspace = Workspace::discover(
            project_dir,
            &DiscoveryOptions::default(),
            cache,
            &workspace_cache,
        )
        .await
        .context("Cannot discover a workspace for locked publishing")?;
        let root = workspace.install_path();
        let lock = read_lock(root)?;
        let locked_members = lock.workspace_member_paths().collect::<BTreeMap<_, _>>();
        let current_members = workspace.packages();
        if current_members.len() != locked_members.len()
            || current_members.iter().any(|(name, member)| {
                locked_members
                    .get(name)
                    .is_none_or(|path| root.join(path).components().ne(member.root().components()))
            })
        {
            bail!("The workspace members have changed; run `uv lock` before publishing");
        }
        if let Some(requires_python) =
            find_requires_python(&workspace, &DependencyGroupsWithDefaults::none())?
            && !same_python(
                Some(requires_python.specifiers()),
                Some(lock.requires_python().specifiers()),
            )
        {
            bail!("The workspace Python requirement has changed; run `uv lock` before publishing");
        }
        let environments = workspace
            .environments()
            .into_iter()
            .flatten()
            .map(|&marker| lock.simplify_environment(marker));
        if environments.ne(lock.simplified_supported_environments()) {
            bail!(
                "The supported workspace environments have changed; run `uv lock` before publishing"
            );
        }
        if !LockTarget::from(workspace.as_ref())
            .matches_runtime_resolution_inputs(
                &lock,
                index_locations,
                cache,
                &workspace_cache,
                credentials_cache,
            )
            .await?
        {
            bail!(
                "The workspace constraints, overrides, or excludes have changed; run `uv lock` before publishing"
            );
        }

        let mut releases = BTreeMap::new();
        for distribution in distributions {
            let filename = distribution.filename();
            let entry = releases
                .entry(filename.name().clone())
                .or_insert_with(|| (filename.version().clone(), Vec::new()));
            if &entry.0 != filename.version() {
                bail!(
                    "Cannot publish multiple versions of `{}` from one lockfile",
                    filename.name()
                );
            }
            entry.1.push(distribution);
        }

        let mut locks = BTreeMap::new();
        let mut needed = BTreeSet::new();
        for (name, (version, distributions)) in &releases {
            let member = current_members.get(name).with_context(|| {
                format!("Cannot publish `{name}` locked: it is not a workspace member")
            })?;
            let package = lock
                .find_by_name(name)
                .map_err(anyhow::Error::msg)?
                .with_context(|| {
                    format!("`uv.lock` has no entry for `{name}`; run `uv lock` before publishing")
                })?;
            if package.version() != Some(version) {
                bail!(
                    "`uv.lock` does not match `{name}=={version}`; run `uv lock` before publishing"
                );
            }

            let path = member.root().join("pyproject.toml");
            let contents = std::str::from_utf8(member.pyproject_toml().as_ref())?;
            let pyproject = PyProjectToml::from_toml(contents, path.display())?;
            let project = ResolutionMetadata::parse_pyproject_toml(pyproject, Some(version))
                .with_context(|| format!("Cannot verify project metadata for `{name}`"))?;
            if &project.version != version {
                bail!(
                    "Distribution version `{version}` does not match the current version of `{name}`"
                );
            }

            let requirements = RequiresDist::from_workspace_member(
                uv_pypi_types::RequiresDist {
                    name: project.name.clone(),
                    requires_dist: project.requires_dist.clone(),
                    provides_extra: project.provides_extra.clone(),
                    dynamic: project.dynamic,
                },
                member.root(),
                cache,
                &workspace_cache,
            )
            .await
            .with_context(|| format!("Cannot verify dependency sources for `{name}`"))?;
            if !lock.matches_publish_requirements(
                root,
                package,
                &requirements.requires_dist,
                &requirements.provides_extra,
            )? {
                bail!(
                    "`uv.lock` does not match the dependencies of `{name}`; run `uv lock` before publishing"
                );
            }

            let (pylock, workspace_dependencies) =
                PylockToml::from_publish_lock(&lock, root, &project, allow_non_pypi)?;
            for (dependency, version) in &workspace_dependencies {
                let Some((selected, artifacts)) = releases.get(dependency) else {
                    bail!(
                        "Cannot publish `{name}` locked: include distributions for its workspace dependency `{dependency}=={version}`"
                    );
                };
                if version != selected {
                    bail!(
                        "Cannot publish `{name}` locked: it requires `{dependency}=={version}`, but the selected distributions have version `{selected}`"
                    );
                }
                if artifacts.iter().any(|artifact| {
                    matches!(artifact.filename(), DistFilename::WheelFilename(wheel) if wheel.variant().is_some())
                }) {
                    bail!(
                        "Cannot publish `{name}` locked: workspace dependency `{dependency}` includes a wheel variant"
                    );
                }
            }

            for distribution in distributions {
                validate_metadata(
                    distribution,
                    &project,
                    member.root(),
                    lock.requires_python(),
                )
                .await
                .with_context(|| {
                    format!("Cannot publish `{}` locked", distribution.raw_filename())
                })?;
            }
            needed.extend(workspace_dependencies.keys().cloned());
            locks.insert(
                name.clone(),
                PublishLock {
                    pylock,
                    workspace_dependencies,
                },
            );
        }

        if !needed.is_empty() {
            let Some(index) = index else {
                bail!(
                    "Locked workspace publishing requires a package index to look up uploaded files; pass `--check-url` or `--index`"
                );
            };
            PylockToml::check_publish_index(index, allow_non_pypi)?;
        }

        let order = publish_order(&locks)?;
        Ok(Self {
            order,
            locks,
            needed,
            index: index.cloned(),
            allow_non_pypi,
        })
    }

    pub(crate) fn order(&self) -> &[PackageName] {
        &self.order
    }

    pub(crate) fn needed_as_dependency(&self, package: &PackageName) -> bool {
        self.needed.contains(package)
    }

    pub(crate) fn has_workspace_dependencies(&self) -> bool {
        !self.needed.is_empty()
    }

    /// Materialize a package lock once all of its workspace dependencies have been uploaded.
    pub(crate) fn export(
        &mut self,
        package: &PackageName,
        published: &PublishedArtifacts,
    ) -> Result<String> {
        let PublishLock {
            mut pylock,
            workspace_dependencies,
        } = self
            .locks
            .remove(package)
            .with_context(|| format!("No prepared lock for `{package}`"))?;
        if !workspace_dependencies.is_empty() {
            let index = self
                .index
                .as_ref()
                .context("Missing index for locked workspace publishing")?;
            for (name, version) in workspace_dependencies {
                let files = published
                    .get(&name)
                    .filter(|files| !files.is_empty())
                    .with_context(|| format!("No published files found for `{name}=={version}`"))?;
                pylock.replace_publish_dependency(
                    &name,
                    &version,
                    index,
                    files,
                    self.allow_non_pypi,
                )?;
            }
        }
        Ok(pylock.to_toml()?)
    }
}

fn read_lock(root: &Path) -> Result<Lock> {
    let path = root.join("uv.lock");
    let contents = fs_err::read_to_string(&path).with_context(|| {
        format!(
            "Cannot read `{}`; run `uv lock` before publishing",
            path.display()
        )
    })?;
    Lock::from_toml(&contents).context("Failed to parse `uv.lock`")
}

/// Compute a deterministic topological order over the selected workspace packages.
fn publish_order(locks: &BTreeMap<PackageName, PublishLock>) -> Result<Vec<PackageName>> {
    let mut pending = locks.iter().collect::<BTreeMap<_, _>>();
    let mut order = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let ready = pending
            .iter()
            .filter(|(_, lock)| {
                lock.workspace_dependencies
                    .keys()
                    .all(|dependency| !pending.contains_key(dependency))
            })
            .map(|(&name, _)| name)
            .collect::<Vec<_>>();
        if ready.is_empty() {
            let packages = pending.keys().map(ToString::to_string).collect::<Vec<_>>();
            bail!(
                "Cannot publish workspace packages in dependency order because of a dependency cycle: {}",
                packages.join(", ")
            );
        }
        for name in ready {
            pending.remove(name);
            order.push(name.clone());
        }
    }
    Ok(order)
}

async fn validate_metadata(
    distribution: &PreparedDistribution,
    project: &ResolutionMetadata,
    project_root: &Path,
    requires_python: &RequiresPython,
) -> Result<()> {
    let metadata = distribution.metadata(TarBackend::from_env()).await?;
    let name: PackageName = metadata.name.parse()?;
    let version: Version = metadata.version.parse()?;
    if name != project.name || version != project.version {
        bail!("Archive metadata has a different name or version from the project");
    }
    if matches!(distribution.filename(), DistFilename::SourceDistFilename(_))
        && metadata.dynamic.iter().any(|field| {
            ["requires-dist", "requires-python", "provides-extra"]
                .iter()
                .any(|expected| field.eq_ignore_ascii_case(expected))
        })
    {
        bail!("Archive dependency metadata is dynamic");
    }
    let archive_python = metadata
        .requires_python
        .as_deref()
        .map(str::parse::<LenientVersionSpecifiers>)
        .transpose()?
        .map(VersionSpecifiers::from);
    if !same_python(project.requires_python.as_ref(), archive_python.as_ref()) {
        bail!("Archive Python requirement does not match the project");
    }
    let archive_extras = metadata
        .provides_extra
        .iter()
        .map(|extra| extra.parse::<ExtraName>())
        .collect::<Result<BTreeSet<_>, _>>()?;
    if archive_extras
        .iter()
        .ne(project.provides_extra.iter().collect::<BTreeSet<_>>())
    {
        bail!("Archive extras do not match the project");
    }
    let archive_requirements = metadata
        .requires_dist
        .iter()
        .map(|requirement| {
            requirement
                .parse::<LenientRequirement>()
                .map(uv_pep508::Requirement::from)
                .map(Requirement::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !Lock::matches_publish_declarations(
        project_root,
        requires_python,
        project.requires_dist.iter().cloned().map(Requirement::from),
        archive_requirements,
    )? {
        bail!(
            "Archive dependencies do not match the project; rebuild the distribution before publishing"
        );
    }
    Ok(())
}

fn same_python(expected: Option<&VersionSpecifiers>, actual: Option<&VersionSpecifiers>) -> bool {
    expected.cloned().map(release_specifiers_to_ranges)
        == actual.cloned().map(release_specifiers_to_ranges)
}
