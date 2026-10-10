//! Inputs shared by lock resolution and dynamic workspace metadata discovery.

use rustc_hash::FxHashSet;

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, RegistryClient, RegistryClientBuilder};
use uv_configuration::{Concurrency, Constraints, Upgrade};
use uv_dispatch::{BuildDispatch, UniversalState};
use uv_distribution::LoweredExtraBuildDependencies;
use uv_distribution_types::{ExtraBuildRequires, HashCollection};
use uv_lock::Lock;
use uv_normalize::PackageName;
use uv_preview::Preview;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_requirements::script_extra_build_requires;
use uv_resolver::FlatIndex;
use uv_settings::ResolverSettings;
use uv_types::{BuildIsolation, HashStrategy, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

use crate::{LockError, LockMode, LockTarget};

pub(crate) struct PreparedBuildContext {
    pub(crate) client: RegistryClient,
    pub(crate) flat_index: FlatIndex,
    pub(crate) build_constraints: Constraints,
    pub(crate) hasher: HashStrategy,
    pub(crate) locked_build_hasher: HashStrategy,
    pub(crate) probe_build_hasher: HashStrategy,
    resolution_build_hasher: HashStrategy,
    extra_build_requires: ExtraBuildRequires,
    environment: PreparedIsolation,
}

enum PreparedIsolation {
    Isolated,
    Shared(PythonEnvironment),
    SharedPackage(PythonEnvironment, Vec<PackageName>),
}

impl PreparedIsolation {
    fn as_borrowed(&self) -> BuildIsolation<'_> {
        match self {
            Self::Isolated => BuildIsolation::Isolated,
            Self::Shared(environment) => BuildIsolation::Shared(environment),
            Self::SharedPackage(environment, packages) => {
                BuildIsolation::SharedPackage(environment, packages)
            }
        }
    }
}

impl PreparedBuildContext {
    /// Register configured credentials before lowering inputs that may access those sources.
    pub(crate) fn build_client(
        target: LockTarget<'_>,
        interpreter: &Interpreter,
        settings: &ResolverSettings,
        client_builder: &BaseClientBuilder<'_>,
        cache: &Cache,
    ) -> Result<RegistryClient, LockError> {
        // Initialize the client.
        let client_builder = client_builder.clone().keyring(settings.keyring_provider);

        for index in target.indexes() {
            if let Some(credentials) = index.credentials()? {
                if let Some(root_url) = index.root_url() {
                    client_builder.store_credentials(&root_url, credentials.clone());
                }
                client_builder.store_credentials(index.raw_url(), credentials);
            }
        }

        // Initialize the registry client.
        Ok(RegistryClientBuilder::new(client_builder, cache.clone())
            .index_locations(settings.index_locations.clone())
            .index_strategy(settings.index_strategy)
            .markers(interpreter.markers())
            .platform(interpreter.platform())
            .build()?)
    }

    pub(crate) async fn new(
        client: RegistryClient,
        target: LockTarget<'_>,
        interpreter: &Interpreter,
        existing_lock: Option<&Lock>,
        mode: LockMode<'_>,
        build_constraints: Constraints,
        settings: &ResolverSettings,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
    ) -> Result<Self, LockError> {
        let ResolverSettings {
            index_locations,
            build_isolation,
            build_hash_checking,
            extra_build_dependencies,
            upgrade,
            sources,
            ..
        } = settings;
        let environment = match build_isolation {
            uv_configuration::BuildIsolation::Isolate => PreparedIsolation::Isolated,
            uv_configuration::BuildIsolation::Shared => {
                PreparedIsolation::Shared(PythonEnvironment::from_interpreter(interpreter.clone()))
            }
            uv_configuration::BuildIsolation::SharedPackage(packages) => {
                PreparedIsolation::SharedPackage(
                    PythonEnvironment::from_interpreter(interpreter.clone()),
                    packages.clone(),
                )
            }
        };
        // Checking an existing lockfile may build metadata and install build dependencies. Verify any
        // artifacts recorded in that lockfile, including for an ordinary unlocked command.
        let (locked_hasher, locked_build_hasher) =
            if let Some(existing_lock) = existing_lock.as_ref() {
                let locked_hasher =
                    existing_lock.hash_strategy(target.install_path(), &FxHashSet::default())?;
                let build_hasher = HashStrategy::from_constraints(
                    &existing_lock.build_constraints(target.install_path()),
                    Some(&interpreter.to_resolver_marker_environment()),
                    *build_hash_checking,
                )?;
                let locked_build_hasher = locked_hasher
                    .clone()
                    .with_constraint_hashes(&build_hasher)?;
                (locked_hasher, locked_build_hasher)
            } else {
                (HashStrategy::default(), HashStrategy::default())
            };
        // Re-resolving an outdated lock does not authorize replacing known artifacts. Only an
        // explicit unlocked upgrade releases the selected packages' hashes.
        let hash_upgrade = match mode {
            LockMode::Locked(..) => &Upgrade::default(),
            LockMode::Write(_) | LockMode::DryRun(_) | LockMode::Frozen(_) => upgrade,
        };
        let resolution_hasher = if hash_upgrade.is_none() {
            locked_hasher.clone()
        } else if let Some(existing_lock) = existing_lock.as_ref() {
            // An explicit upgrade allows replacing the selected packages' files, so do not require
            // them to match the hashes recorded in the lockfile.
            let upgrade_packages = existing_lock.upgrade_packages(hash_upgrade);
            existing_lock.hash_strategy(target.install_path(), &upgrade_packages)?
        } else {
            HashStrategy::default()
        };
        let hasher = HashStrategy::collect(HashCollection::Url)
            .with_verification(resolution_hasher.verification().clone());

        let build_hasher = HashStrategy::from_constraints(
            &build_constraints,
            Some(&interpreter.to_resolver_marker_environment()),
            *build_hash_checking,
        )?;
        // Early metadata discovery must verify known artifacts before populating the shared
        // metadata cache. Current constraints apply, while explicit upgrades release the same
        // locked artifacts as resolution; obsolete constraint hashes do not restrict new inputs.
        let probe_build_hasher = resolution_hasher.with_constraint_hashes(&build_hasher)?;
        // Explicit build constraints apply even when fresh resolution can replace lockfile hashes.
        let resolution_build_hasher = match mode {
            LockMode::Locked(..) => locked_hasher.with_constraint_hashes(&build_hasher)?,
            LockMode::Write(_) | LockMode::DryRun(_) | LockMode::Frozen(_) => build_hasher,
        };

        // Resolve the flat indexes from `--find-links`.
        let flat_index = FlatIndex::load(&client, cache, index_locations).await?;

        // Lower the extra build dependencies.
        let extra_build_requires = match &target {
            LockTarget::Workspace(workspace) => {
                LoweredExtraBuildDependencies::from_workspace(
                    extra_build_dependencies.clone(),
                    workspace,
                    index_locations,
                    sources,
                    cache,
                    workspace_cache,
                    client.credentials_cache(),
                )
                .await?
            }
            LockTarget::Script(script) => {
                // Try to get extra build dependencies from the script metadata
                script_extra_build_requires(
                    (*script).into(),
                    sources,
                    index_locations,
                    cache,
                    workspace_cache,
                    client.credentials_cache(),
                )
                .await?
            }
        }
        .into_inner();

        Ok(Self {
            client,
            flat_index,
            build_constraints,
            hasher,
            locked_build_hasher,
            probe_build_hasher,
            resolution_build_hasher,
            extra_build_requires,
            environment,
        })
    }

    pub(crate) fn build_dispatch<'a>(
        &'a self,
        interpreter: &'a Interpreter,
        settings: &'a ResolverSettings,
        state: &UniversalState,
        concurrency: &Concurrency,
        cache: &'a Cache,
        workspace_cache: &WorkspaceCache,
        preview: Preview,
    ) -> BuildDispatch<'a> {
        BuildDispatch::new(
            &self.client,
            cache,
            &self.build_constraints,
            interpreter,
            &settings.index_locations,
            &self.flat_index,
            &settings.dependency_metadata,
            state.fork().into_inner(),
            settings.index_strategy,
            &settings.config_setting,
            &settings.config_settings_package,
            self.environment.as_borrowed(),
            &self.extra_build_requires,
            &settings.extra_build_variables,
            settings.link_mode,
            &settings.build_options,
            &self.resolution_build_hasher,
            settings.exclude_newer.clone(),
            settings.sources.clone(),
            SourceTreeEditablePolicy::Project,
            workspace_cache.clone(),
            concurrency.clone(),
            preview,
        )
    }
}
