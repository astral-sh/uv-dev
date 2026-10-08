//! Tool lock construction, validation, projection and resolver preferences.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use tracing::debug;
use uv_cache::{Cache, Refresh};
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_configuration::{
    BuildOptions, Concurrency, Constraints, DependencyGroupsWithDefaults, ExcludeDependency,
    ExtrasSpecification, InstallOptions, Override, TargetTriple,
};
use uv_dispatch::{BuildDispatch, PlatformState};
use uv_distribution::{DistributionDatabase, LoweredExtraBuildDependencies};
use uv_distribution_types::{
    DependencyMetadata, HashCollection, IndexLocations, NameRequirementSpecification, Requirement,
    RequiresPython, Resolution,
};
use uv_fs::Simplified;
use uv_installer::SitePackages;
use uv_lock::{Installable, Lock, ResolverManifest};
use uv_normalize::{DefaultExtras, GroupName, PackageName};
use uv_preview::Preview;
use uv_pypi_types::Conflicts;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_requirements::RequirementsSpecification;
use uv_resolver::{FlatIndex, OptionsBuilder, Preference, ResolverOutput};
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

use uv_resolve_operations::{resolution_markers, resolution_tags};

use uv_command_support::Printer;
use uv_environment_operations::{EnvironmentSpecification, PreferenceLocation};
use uv_lock_operations::ValidatedLock;
use uv_settings::ResolverSettings;

/// A failure while preparing or validating an existing tool lockfile.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ToolLockError {
    #[error(transparent)]
    Validation(#[from] uv_lock_operations::LockValidationError),
    #[error(transparent)]
    ClientBuild(#[from] uv_client::ClientBuildError),
    #[error(transparent)]
    FlatIndex(Box<uv_client::FlatIndexError>),
    #[error(transparent)]
    HashStrategy(#[from] uv_types::HashStrategyError),
}

impl From<uv_client::FlatIndexError> for ToolLockError {
    fn from(error: uv_client::FlatIndexError) -> Self {
        Self::FlatIndex(Box::new(error))
    }
}

/// A universal lock for a tool environment.
pub(super) struct ToolLock {
    root: PathBuf,
    lock: Lock,
}

/// A tool lock validated against the current resolution inputs.
pub(super) struct ValidatedToolLock {
    lock: ToolLock,
    satisfied: bool,
    usable: bool,
}

impl ValidatedToolLock {
    /// Return whether the existing lock satisfies the current resolution inputs.
    pub(super) fn is_satisfied(&self) -> bool {
        self.satisfied
    }

    /// Return the lock as a resolver preference if its versions remain usable.
    pub(super) fn preference(&self) -> Option<&ToolLock> {
        self.usable.then_some(&self.lock)
    }

    /// Return the validated lock.
    pub(super) fn into_lock(self) -> ToolLock {
        self.lock
    }
}

impl ToolLock {
    /// Build the lock manifest for a tool environment.
    pub(super) fn manifest(
        requirements: &[Requirement],
        constraints: &[Requirement],
        overrides: &[Requirement],
        excludes: &[ExcludeDependency],
        build_constraints: &[NameRequirementSpecification],
        dependency_metadata: &DependencyMetadata,
    ) -> ResolverManifest {
        ResolverManifest::new(
            std::iter::empty::<PackageName>(),
            requirements.iter().cloned(),
            constraints.iter().cloned(),
            overrides.iter().cloned().map(Override::Requirement),
            excludes.iter().cloned(),
            build_constraints.iter().cloned(),
            std::iter::empty::<(GroupName, Vec<Requirement>)>(),
            dependency_metadata.values().cloned(),
        )
    }

    /// Build the lock for a tool environment.
    pub(super) fn from_resolution(
        root: &Path,
        resolution: &ResolverOutput,
        manifest: &ResolverManifest,
        index_locations: &IndexLocations,
    ) -> anyhow::Result<Self> {
        let manifest = manifest.clone().relative_to(root)?;
        let lock = Lock::from_resolution(
            resolution,
            manifest,
            root,
            Vec::new(),
            index_locations,
            false,
        )?;
        Ok(Self {
            root: root.to_path_buf(),
            lock,
        })
    }

    /// Read the lock for a tool, if one has been generated.
    pub(super) fn read(directory: &Path) -> Option<Self> {
        let path = directory.join("uv.lock");
        match fs_err::read_to_string(&path) {
            Ok(contents) => match Lock::from_toml(&contents) {
                Ok(lock) => Some(Self {
                    root: directory.to_path_buf(),
                    lock,
                }),
                Err(err) => {
                    debug!(
                        "Ignoring invalid tool lock at `{}`: {err}",
                        path.user_display()
                    );
                    None
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => {
                debug!(
                    "Ignoring unreadable tool lock at `{}`: {err}",
                    path.user_display()
                );
                None
            }
        }
    }

    /// Write or remove the lock for a tool.
    pub(super) fn write(directory: &Path, lock: Option<&Self>) -> anyhow::Result<()> {
        let path = directory.join("uv.lock");
        if let Some(lock) = lock {
            uv_fs::write_atomic_sync(&path, lock.lock.to_toml()?)?;
        } else {
            match fs_err::remove_file(path) {
                Ok(()) => (),
                Err(err) if err.kind() == io::ErrorKind::NotFound => (),
                Err(err) => return Err(err.into()),
            }
        }
        Ok(())
    }

    /// Validate the lock against the current resolution inputs.
    #[expect(clippy::too_many_arguments)]
    pub(super) async fn validate(
        self,
        requirements: &[Requirement],
        constraints: &[Requirement],
        overrides: &[Requirement],
        excludes: &[ExcludeDependency],
        build_constraints: &Constraints,
        refresh: &Refresh,
        interpreter: &Interpreter,
        settings: &ResolverSettings,
        client_builder: &BaseClientBuilder<'_>,
        state: &PlatformState,
        concurrency: &Concurrency,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
        printer: Printer,
        preview: Preview,
    ) -> Result<ValidatedToolLock, ToolLockError> {
        let ResolverSettings {
            index_locations,
            index_strategy,
            keyring_provider,
            resolution,
            prerelease,
            fork_strategy,
            dependency_metadata,
            config_setting,
            config_settings_package,
            build_isolation,
            build_hash_checking,
            extra_build_dependencies,
            extra_build_variables,
            exclude_newer,
            link_mode,
            upgrade,
            build_options,
            sources,
            torch_backend: _,
            cuda_driver_version: _,
            amd_gpu_architecture: _,
        } = settings;

        let client = RegistryClientBuilder::new(
            client_builder.clone().keyring(*keyring_provider),
            cache.clone(),
        )
        .index_locations(index_locations.clone())
        .index_strategy(*index_strategy)
        .markers(interpreter.markers())
        .platform(interpreter.platform())
        .build()?;

        let environment;
        let build_isolation = match build_isolation {
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

        let options = OptionsBuilder::new()
            .resolution_mode(*resolution)
            .prerelease(prerelease.clone())
            .fork_strategy(*fork_strategy)
            .exclude_newer(exclude_newer.clone())
            .index_strategy(*index_strategy)
            .build_options(build_options.clone())
            .build();
        let hasher = HashStrategy::collect(HashCollection::Url);
        let build_hasher = HashStrategy::from_constraints(
            build_constraints,
            Some(&interpreter.to_resolver_marker_environment()),
            *build_hash_checking,
        )?;

        let flat_index = FlatIndex::load(&client, cache, index_locations).await?;

        let extra_build_requires =
            LoweredExtraBuildDependencies::from_non_lowered(extra_build_dependencies.clone())
                .into_inner();
        let build_dispatch = BuildDispatch::new(
            &client,
            cache,
            build_constraints,
            interpreter,
            index_locations,
            &flat_index,
            dependency_metadata,
            state.clone().into_inner(),
            *index_strategy,
            config_setting,
            config_settings_package,
            build_isolation,
            &extra_build_requires,
            extra_build_variables,
            *link_mode,
            build_options,
            &build_hasher,
            exclude_newer.clone(),
            sources.clone(),
            SourceTreeEditablePolicy::Tool,
            workspace_cache.clone(),
            concurrency.clone(),
            preview,
        );
        let database = DistributionDatabase::new(
            &client,
            &build_dispatch,
            concurrency.downloads_semaphore.clone(),
        );

        let requires_python =
            RequiresPython::greater_than_equal_version(&interpreter.python_minor_version());
        let overrides = overrides
            .iter()
            .cloned()
            .map(Override::Requirement)
            .collect::<Vec<_>>();
        let Self { root, lock } = self;
        let validated = ValidatedLock::validate(
            lock,
            &root,
            &BTreeMap::new(),
            &[],
            &BTreeMap::new(),
            requirements,
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            constraints,
            &overrides,
            excludes,
            build_constraints,
            &Conflicts::empty(),
            None,
            None,
            dependency_metadata,
            interpreter,
            &requires_python,
            index_locations,
            upgrade,
            Some(refresh),
            &options,
            &hasher,
            state.index(),
            &database,
            preview,
            printer,
        )
        .await?;
        let satisfied = validated.is_satisfied();
        let usable = validated.is_usable();

        Ok(ValidatedToolLock {
            lock: Self {
                root,
                lock: validated.into_lock(),
            },
            satisfied,
            usable,
        })
    }

    /// Project the universal lock into a specific environment.
    pub(super) fn to_resolution(
        &self,
        project_name: Option<&PackageName>,
        interpreter: &Interpreter,
        python_platform: Option<&TargetTriple>,
        build_options: &BuildOptions,
    ) -> anyhow::Result<Resolution> {
        struct ToolLockInstallTarget<'lock> {
            tool_lock: &'lock ToolLock,
            project_name: Option<&'lock PackageName>,
        }

        impl<'lock> Installable<'lock> for ToolLockInstallTarget<'lock> {
            fn install_path(&self) -> &'lock Path {
                &self.tool_lock.root
            }

            fn lock(&self) -> &'lock Lock {
                &self.tool_lock.lock
            }

            fn roots(&self) -> impl Iterator<Item = &PackageName> {
                std::iter::empty()
            }

            fn project_name(&self) -> Option<&PackageName> {
                self.project_name
            }
        }

        let markers = resolution_markers(None, python_platform, interpreter);
        let tags = resolution_tags(None, python_platform, interpreter)?;
        Ok(ToolLockInstallTarget {
            tool_lock: self,
            project_name,
        }
        .to_resolution(
            &markers,
            &tags,
            &ExtrasSpecification::default().with_defaults(DefaultExtras::default()),
            &DependencyGroupsWithDefaults::none(),
            build_options,
            &InstallOptions::default(),
        )?)
    }
}

/// Build an environment specification for a tool, preferring versions from its existing lock when
/// available, then falling back to the installed environment.
pub(super) fn tool_environment_spec<'lock>(
    requirements: RequirementsSpecification,
    lock: Option<&'lock ToolLock>,
    site_packages: Option<&SitePackages>,
) -> EnvironmentSpecification<'lock> {
    let specification = EnvironmentSpecification::from(requirements);
    if let Some(lock) = lock {
        return specification.with_preferences(PreferenceLocation::Lock {
            lock: &lock.lock,
            install_path: &lock.root,
        });
    }

    let preferences = site_packages
        .into_iter()
        .flat_map(|site_packages| site_packages.iter().filter_map(Preference::from_installed))
        .collect::<Vec<_>>();
    if preferences.is_empty() {
        return specification;
    }

    specification.with_preferences(PreferenceLocation::Entries(preferences))
}
