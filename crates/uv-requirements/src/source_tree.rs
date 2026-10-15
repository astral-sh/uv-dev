use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::TryStreamExt;
use futures::stream::FuturesOrdered;
use tempfile::TempDir;
use url::Url;

use uv_auth::CredentialsCache;
use uv_cache::Cache;
use uv_configuration::{ExtrasSpecification, NoSources};
use uv_distribution::{DistributionDatabase, FlatRequiresDist, Reporter, RequiresDist};
use uv_distribution_types::{
    BuildableSource, DirectorySourceUrl, HashCollection, HashValidation, Identifier,
    MetadataHashPolicy, SourceUrl,
};
use uv_distribution_types::{IndexLocations, Requirement};
use uv_fs::Simplified;
use uv_normalize::{ExtraName, PackageName};
use uv_pep508::RequirementOrigin;
use uv_pypi_types::PyProjectToml;
use uv_redacted::DisplaySafeUrl;
use uv_resolver::{InMemoryIndex, MetadataResponse};
use uv_types::{BuildContext, HashStrategy, HashVerification};
use uv_workspace::WorkspaceCache;

#[derive(Debug, Clone)]
pub enum SourceTree {
    PyProjectToml(PathBuf, PyProjectToml),
    SetupPy(PathBuf),
    SetupCfg(PathBuf),
    Stdin {
        source: Box<Self>,
        _directory: Arc<TempDir>,
    },
}

impl SourceTree {
    /// Materialize standalone project metadata so build backends can read its expected filename.
    pub(crate) fn from_stdin(
        content: &str,
        filename: &str,
        source: impl FnOnce(PathBuf) -> Self,
        cache: &Cache,
    ) -> Result<Self> {
        // Workspace discovery stops at the cache boundary, even when the cache is in a workspace.
        let directory = cache.build_dir()?;
        let path = directory.path().join(filename);
        fs_err::write(&path, content)?;
        if filename == "setup.cfg" {
            fs_err::write(
                directory.path().join("setup.py"),
                "from setuptools import setup\nsetup()\n",
            )?;
        }
        Ok(Self::Stdin {
            source: Box::new(source(path)),
            _directory: Arc::new(directory),
        })
    }

    /// Extract statically available requirements without initializing a build backend.
    pub async fn resolve_static(
        &self,
        extras: &ExtrasSpecification,
        locations: &IndexLocations,
        sources: NoSources,
        editable: bool,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
        credentials_cache: &CredentialsCache,
    ) -> Result<Option<SourceTreeResolution>> {
        let Some(pyproject_toml) = self.pyproject_toml() else {
            return Ok(None);
        };
        let path = self.root()?;
        Ok(RequiresDist::from_pyproject_toml(
            &path,
            pyproject_toml,
            locations,
            sources,
            editable,
            cache,
            workspace_cache,
            credentials_cache,
        )
        .await?
        .map(|metadata| SourceTreeResolution::from_requires_dist(self, metadata, extras)))
    }

    fn root(&self) -> Result<PathBuf> {
        let path = fs_err::canonicalize(self.path()).with_context(|| {
            format!(
                "Failed to canonicalize path to source tree: {}",
                self.path().user_display()
            )
        })?;
        path.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!(
                "The file `{}` appears to be a `pyproject.toml`, `setup.py`, or `setup.cfg` file, which must be in a directory",
                path.user_display()
            ))
    }

    /// Return the [`Path`] to the file representing the source tree (e.g., the `pyproject.toml`).
    fn path(&self) -> &Path {
        match self {
            Self::PyProjectToml(path, ..) => path,
            Self::SetupPy(path) => path,
            Self::SetupCfg(path) => path,
            Self::Stdin { source, .. } => source.path(),
        }
    }

    /// Return the [`PyProjectToml`] if this is a `pyproject.toml`-based source tree.
    fn pyproject_toml(&self) -> Option<&PyProjectToml> {
        match self {
            Self::PyProjectToml(.., toml) => Some(toml),
            Self::SetupPy(_) | Self::SetupCfg(_) => None,
            Self::Stdin { source, .. } => source.pyproject_toml(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceTreeResolution {
    /// The requirements sourced from the source trees.
    requirements: Box<[Requirement]>,
    /// The names of the projects that were resolved.
    project: PackageName,
    /// The extras used when resolving the requirements.
    extras: Box<[ExtraName]>,
}

impl SourceTreeResolution {
    fn from_requires_dist(
        source_tree: &SourceTree,
        metadata: RequiresDist,
        extras: &ExtrasSpecification,
    ) -> Self {
        let origin_path = match source_tree {
            SourceTree::Stdin { .. } => PathBuf::from("-"),
            SourceTree::PyProjectToml(..) | SourceTree::SetupPy(_) | SourceTree::SetupCfg(_) => {
                source_tree.path().to_path_buf()
            }
        };
        let origin = RequirementOrigin::Project(origin_path, metadata.name.clone());

        // Determine the extras to include when resolving the requirements.
        let extras = extras
            .extra_names(metadata.provides_extra.iter())
            .cloned()
            .collect::<Vec<_>>();

        let mut requirements = Vec::new();

        // Flatten transitive extras and include dependencies.
        requirements.extend(
            FlatRequiresDist::from_requirements(metadata.requires_dist, &metadata.name)
                .into_iter()
                .map(|requirement| Requirement {
                    origin: Some(origin.clone()),
                    marker: requirement.marker.simplify_extras(&extras),
                    ..requirement
                }),
        );

        let requirements = requirements.into_boxed_slice();
        let project = metadata.name;
        let extras = metadata.provides_extra;

        Self {
            requirements,
            project,
            extras,
        }
    }

    /// Return the name of the project that was resolved.
    pub fn project(&self) -> &PackageName {
        &self.project
    }

    /// Return the extras used when resolving the requirements.
    pub fn extras(&self) -> &[ExtraName] {
        &self.extras
    }

    /// Return the requirements sourced from the source tree.
    pub fn into_requirements(self) -> Box<[Requirement]> {
        self.requirements
    }
}

/// A resolver for requirements specified via source trees.
///
/// Used, e.g., to determine the input requirements when a user specifies a `pyproject.toml`
/// file, which may require running PEP 517 build hooks to extract metadata.
pub struct SourceTreeResolver<'a, Context: BuildContext> {
    /// The extras to include when resolving requirements.
    extras: &'a ExtrasSpecification,
    /// The hash policy to enforce.
    hasher: &'a HashStrategy,
    /// The in-memory index for resolving dependencies.
    index: &'a InMemoryIndex,
    /// The database for fetching and building distributions.
    database: DistributionDatabase<'a, Context>,
}

impl<'a, Context: BuildContext> SourceTreeResolver<'a, Context> {
    /// Instantiate a new [`SourceTreeResolver`] for a given set of `source_trees`.
    pub fn new(
        extras: &'a ExtrasSpecification,
        hasher: &'a HashStrategy,
        index: &'a InMemoryIndex,
        database: DistributionDatabase<'a, Context>,
    ) -> Self {
        Self {
            extras,
            hasher,
            index,
            database,
        }
    }

    /// Set the [`Reporter`] to use for this resolver.
    #[must_use]
    pub fn with_reporter(self, reporter: Arc<dyn Reporter>) -> Self {
        Self {
            database: self.database.with_reporter(reporter),
            ..self
        }
    }

    /// Resolve the requirements from the provided source trees.
    pub async fn resolve(
        self,
        source_trees: impl Iterator<Item = &SourceTree>,
    ) -> Result<Vec<SourceTreeResolution>> {
        let resolutions: Vec<_> = source_trees
            .map(async |source_tree| self.resolve_source_tree(source_tree).await)
            .collect::<FuturesOrdered<_>>()
            .try_collect()
            .await?;
        Ok(resolutions)
    }

    /// Infer the dependencies for a directory dependency.
    async fn resolve_source_tree(&self, source_tree: &SourceTree) -> Result<SourceTreeResolution> {
        let metadata = self.resolve_requires_dist(source_tree).await?;
        Ok(SourceTreeResolution::from_requires_dist(
            source_tree,
            metadata,
            self.extras,
        ))
    }

    /// Resolve the [`RequiresDist`] metadata for a given source tree. Attempts to resolve the
    /// requirements without building the distribution, even if the project contains (e.g.) a
    /// dynamic version since, critically, we don't need to install the package itself; only its
    /// dependencies.
    async fn resolve_requires_dist(&self, source_tree: &SourceTree) -> Result<RequiresDist> {
        // Convert to a buildable source.
        let path = fs_err::canonicalize(source_tree.path()).with_context(|| {
            format!(
                "Failed to canonicalize path to source tree: {}",
                source_tree.path().user_display()
            )
        })?;
        let path = path.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "The file `{}` appears to be a `pyproject.toml`, `setup.py`, or `setup.cfg` file, which must be in a directory",
                path.user_display()
            )
        })?;
        let path = path.simplified();

        // If the path is a `pyproject.toml`, attempt to extract the requirements statically. The
        // distribution database will do this too, but we can be even more aggressive here since we
        // _only_ need the requirements. So, for example, even if the version is dynamic, we can
        // still extract the requirements without performing a build, unlike in the database where
        // we typically construct a "complete" metadata object.
        if let Some(pyproject_toml) = source_tree.pyproject_toml() {
            if let Some(metadata) = self.database.requires_dist(path, pyproject_toml).await? {
                return Ok(metadata);
            }
        }

        let Ok(url) = Url::from_directory_path(path).map(DisplaySafeUrl::from_url) else {
            return Err(anyhow::anyhow!("Failed to convert path to URL"));
        };
        let source = SourceUrl::Directory(DirectorySourceUrl {
            url: &url,
            install_path: path,
            editable: None,
        });

        // Determine the hash policy. Since we don't have a package name, we perform a
        // manual match.
        let collection = match self.hasher.verification() {
            HashVerification::Required(_) => {
                return Err(anyhow::anyhow!(
                    "Hash-checking is not supported for local directories: {}",
                    path.user_display()
                ));
            }
            HashVerification::IfPresent(_) => match self.hasher.collection() {
                HashCollection::None => HashCollection::All,
                collection @ (HashCollection::Url | HashCollection::All) => collection,
            },
            HashVerification::None => self.hasher.collection(),
        };
        let hashes = MetadataHashPolicy {
            collection,
            validation: HashValidation::None,
        };

        // Fetch the metadata for the distribution.
        let metadata = {
            let id = source.distribution_id();
            if let Some(response) = self.index.distributions().register_or_wait(&id).await {
                let MetadataResponse::Found(archive) = &*response else {
                    panic!("Failed to find metadata for: {}", path.user_display());
                };
                archive.metadata.clone()
            } else {
                // Run the PEP 517 build process to extract metadata from the source distribution.
                let source = BuildableSource::Url(source);
                let archive = self.database.build_wheel_metadata(&source, hashes).await?;

                let metadata = archive.metadata.clone();

                // Insert the metadata into the index.
                self.index
                    .distributions()
                    .done(id, Arc::new(MetadataResponse::Found(archive)));

                metadata
            }
        };

        // This source tree was requested as an input, so preserve its authored path spelling.
        Ok(RequiresDist::from(metadata.with_force_relative(false)))
    }
}
