use std::io;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::Semaphore;
use tracing::{Span, debug};

use crate::{
    EnvironmentError, EnvironmentResolution, EnvironmentSpecification, resolve_environment,
    sync_environment,
};
use uv_command_support::Printer;
use uv_configuration::{Concurrency, Constraints, HashCheckingMode, Modifications, TargetTriple};
use uv_dispatch::PlatformState;
use uv_install_operations::loggers::InstallLogger;
use uv_resolve_operations::loggers::ResolveLogger;
use uv_settings::ResolverInstallerSettings;

use uv_cache::{Cache, CacheBucket};
use uv_cache_info::CacheInfo;
use uv_cache_key::{cache_digest, hash_digest};
use uv_client::BaseClientBuilder;
use uv_distribution_types::{
    BuiltDist, Dist, Identifier, Node, Resolution, ResolvedDist, SourceDist,
};
use uv_preview::Preview;
use uv_pypi_types::HashDigests;
use uv_python_interpreter::{Interpreter, PythonEnvironment, canonicalize_executable};
use uv_settings::MalwareCheckSettings;
use uv_types::{HashStrategy, HashVerification, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

/// A [`PythonEnvironment`] stored in the cache.
#[derive(Debug)]
pub struct CachedEnvironment(PythonEnvironment);

impl From<CachedEnvironment> for PythonEnvironment {
    fn from(environment: CachedEnvironment) -> Self {
        environment.0
    }
}

#[derive(Debug, Clone, Hash)]
struct CachedEnvironmentDist {
    dist: ResolvedDist,
    hashes: uv_pypi_types::HashDigests,
    cache_info: Option<CacheInfo>,
}

/// Scan mutable local inputs and hash the resolution without blocking the async executor.
async fn resolution_cache_key(
    distributions: Vec<(ResolvedDist, HashDigests)>,
    slots: Arc<Semaphore>,
) -> Result<String, EnvironmentError> {
    if distributions
        .iter()
        .all(|(dist, _)| CachedEnvironment::cache_info_path(dist).is_none())
    {
        return Ok(hash_resolution(distributions)?);
    }
    let permit = slots.acquire_owned().await.map_err(io::Error::other)?;
    let span = Span::current();
    Ok(tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _entered = span.enter();
        hash_resolution(distributions)
    })
    .await
    .map_err(io::Error::other)??)
}

fn hash_resolution(
    distributions: Vec<(ResolvedDist, HashDigests)>,
) -> Result<String, uv_cache_info::CacheInfoError> {
    let mut distributions = distributions
        .into_iter()
        .map(|(dist, hashes)| {
            let cache_info = CachedEnvironment::cache_info_path(&dist)
                .map(CacheInfo::from_path)
                .transpose()?;
            Ok(CachedEnvironmentDist {
                dist,
                hashes,
                cache_info,
            })
        })
        .collect::<Result<Vec<_>, uv_cache_info::CacheInfoError>>()?;
    distributions.sort_unstable_by(|left, right| {
        left.dist
            .distribution_id()
            .cmp(&right.dist.distribution_id())
    });
    Ok(hash_digest(&distributions))
}

fn cached_environment_resolution_hash(
    resolution_hash: String,
    hash_strategy: &HashStrategy,
) -> String {
    match hash_strategy.verification() {
        // Preserve existing cache identities for environments materialized without verification.
        HashVerification::None => resolution_hash,
        // Never reuse an environment materialized without hash verification for a lock-backed
        // resolution with the same distributions and expected hashes.
        HashVerification::IfPresent(_) | HashVerification::Required(_) => {
            hash_digest(&("verify", resolution_hash))
        }
    }
}

impl CachedEnvironment {
    /// Get or create an [`CachedEnvironment`] based on a given set of requirements.
    pub async fn from_spec(
        spec: EnvironmentSpecification<'_>,
        build_constraints: Constraints,
        interpreter: &Interpreter,
        python_platform: Option<&TargetTriple>,
        settings: &ResolverInstallerSettings,
        client_builder: &BaseClientBuilder<'_>,
        state: &PlatformState,
        resolve: Box<dyn ResolveLogger>,
        install: Box<dyn InstallLogger>,
        installer_metadata: bool,
        concurrency: &Concurrency,
        cache: &Cache,
        workspace_cache: &WorkspaceCache,
        printer: Printer,
        preview: Preview,
    ) -> Result<Self, EnvironmentError> {
        let interpreter = Self::base_interpreter(interpreter, cache)?;

        // Resolve the requirements with the interpreter.
        let resolution = Resolution::from(
            resolve_environment(
                spec,
                EnvironmentResolution::Specific,
                &interpreter,
                python_platform,
                SourceTreeEditablePolicy::Project,
                build_constraints.clone(),
                &settings.resolver,
                client_builder,
                state,
                resolve,
                concurrency,
                cache,
                workspace_cache,
                printer,
                preview,
            )
            .await?,
        );

        Self::from_resolution(
            &resolution,
            HashStrategy::default(),
            build_constraints,
            &interpreter,
            settings,
            client_builder,
            state,
            install,
            installer_metadata,
            concurrency,
            cache,
            printer,
            preview,
        )
        .await
    }

    /// Get or create a [`CachedEnvironment`] from a lock-backed [`Resolution`].
    ///
    /// Prefer [`Self::from_spec`] when starting from unresolved requirements; it selects the base
    /// interpreter and resolves the requirements for that interpreter before delegating here.
    ///
    /// This method checks `resolution` for malware when enabled and verifies its recorded hashes.
    /// Both checks run before cache lookup. `interpreter` must be the base interpreter for which
    /// `resolution` was produced. In particular, callers materializing a universal lock must derive
    /// its markers and tags from the same interpreter.
    pub async fn from_locked_resolution(
        resolution: &Resolution,
        build_constraints: Constraints,
        interpreter: &Interpreter,
        settings: &ResolverInstallerSettings,
        malware_settings: &MalwareCheckSettings,
        client_builder: &BaseClientBuilder<'_>,
        state: &PlatformState,
        install: Box<dyn InstallLogger>,
        installer_metadata: bool,
        concurrency: &Concurrency,
        cache: &Cache,
        printer: Printer,
        preview: Preview,
    ) -> Result<Self, EnvironmentError> {
        let malware_check_client_builder = client_builder
            .clone()
            .keyring(settings.resolver.keyring_provider);
        crate::malware::check_resolution_malware(
            resolution,
            &malware_check_client_builder,
            concurrency,
            malware_settings,
            cache,
            preview,
        )
        .await?;

        let hash_strategy = HashStrategy::from_resolution(resolution, HashCheckingMode::Verify)?;
        Self::from_resolution(
            resolution,
            hash_strategy,
            build_constraints,
            interpreter,
            settings,
            client_builder,
            state,
            install,
            installer_metadata,
            concurrency,
            cache,
            printer,
            preview,
        )
        .await
    }

    async fn from_resolution(
        resolution: &Resolution,
        hash_strategy: HashStrategy,
        build_constraints: Constraints,
        interpreter: &Interpreter,
        settings: &ResolverInstallerSettings,
        client_builder: &BaseClientBuilder<'_>,
        state: &PlatformState,
        install: Box<dyn InstallLogger>,
        installer_metadata: bool,
        concurrency: &Concurrency,
        cache: &Cache,
        printer: Printer,
        preview: Preview,
    ) -> Result<Self, EnvironmentError> {
        // Compute mutable source cache keys in one ordered batch before hashing the resolution.
        let distributions = resolution
            .graph()
            .node_weights()
            .filter_map(|node| match node {
                Node::Dist {
                    dist,
                    hashes,
                    install: true,
                } => Some((dist.clone(), hashes.clone())),
                Node::Dist { install: false, .. } | Node::Root => None,
            })
            .collect();
        let resolution_hash = cached_environment_resolution_hash(
            resolution_cache_key(distributions, concurrency.downloads_semaphore.clone()).await?,
            &hash_strategy,
        );

        // Construct a hash for the environment.
        //
        // Use the canonicalized base interpreter path since that's the interpreter we performed the
        // resolution with and the interpreter the environment will be created with.
        //
        // We cache environments independent of the environment they'd be layered on top of. The
        // assumption is such that the environment will _not_ be modified by the user or uv;
        // otherwise, we risk cache poisoning. For example, if we were to write a `.pth` file to
        // the cached environment, it would be shared across all projects that use the same
        // interpreter and the same cached dependencies.
        //
        // TODO(zanieb): We should include the version of the base interpreter in the hash, so if
        // the interpreter at the canonicalized path changes versions we construct a new
        // environment.
        let interpreter_hash =
            cache_digest(&canonicalize_executable(interpreter.sys_executable())?);

        // Search in the content-addressed cache.
        let cache_entry = cache.entry(CacheBucket::Environments, interpreter_hash, resolution_hash);

        if let Ok(root) = cache.resolve_link(cache_entry.path()) {
            if let Ok(environment) = PythonEnvironment::from_root(root, cache) {
                return Ok(Self(environment));
            }
        }

        // Create the environment in the cache, then relocate it to its content-addressed location.
        let temp_dir = cache.venv_dir()?;
        let venv = uv_virtualenv::create_venv(
            temp_dir.path(),
            interpreter.clone(),
            uv_virtualenv::Prompt::None,
            false,
            uv_virtualenv::OnExisting::Remove(uv_virtualenv::RemovalReason::TemporaryEnvironment),
            true,
            uv_virtualenv::Seed::Disabled,
            false,
        )?;

        sync_environment(
            venv,
            resolution,
            hash_strategy,
            Modifications::Exact,
            build_constraints,
            settings.into(),
            client_builder,
            state,
            install,
            installer_metadata,
            concurrency,
            cache,
            printer,
            preview,
        )
        .await?;

        // Now that the environment is complete, sync it to its content-addressed location.
        let id = cache.persist(temp_dir.keep(), cache_entry.path()).await?;
        let root = cache.archive(&id);

        Ok(Self(PythonEnvironment::from_root(root, cache)?))
    }

    /// Return the local path whose mutable cache keys can invalidate this distribution.
    fn cache_info_path(dist: &ResolvedDist) -> Option<&Path> {
        match dist {
            ResolvedDist::Installed { .. } => None,
            ResolvedDist::Installable { dist, .. } => match dist.as_ref() {
                Dist::Built(BuiltDist::Path(wheel)) => Some(&wheel.install_path),
                Dist::Source(SourceDist::Path(sdist)) => Some(&sdist.install_path),
                Dist::Source(SourceDist::Directory(directory)) => Some(&directory.install_path),
                _ => None,
            },
        }
    }

    /// Return the [`Interpreter`] to use for the cached environment, based on a given
    /// [`Interpreter`].
    ///
    /// When caching, always use the base interpreter, rather than that of the virtual
    /// environment.
    pub fn base_interpreter(
        interpreter: &Interpreter,
        cache: &Cache,
    ) -> Result<Interpreter, uv_python_discovery::Error> {
        let base_python = if cfg!(unix) {
            interpreter.find_base_python()?
        } else {
            interpreter.to_base_python()?
        };
        if base_python == interpreter.sys_executable() {
            debug!(
                "Caching via base interpreter: {}",
                interpreter.sys_executable().display()
            );
            Ok(interpreter.clone())
        } else {
            let base_interpreter = Interpreter::query(base_python, cache)?;
            debug!(
                "Caching via base interpreter: {}",
                base_interpreter.sys_executable().display()
            );
            Ok(base_interpreter)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};
    use tokio::sync::Semaphore;
    use uv_distribution_types::{Dist, ResolvedDist};
    use uv_pypi_types::HashDigests;
    use uv_redacted::DisplaySafeUrl;
    use uv_types::HashStrategy;

    use super::{cached_environment_resolution_hash, hash_digest, resolution_cache_key};

    fn poll_once<F: Future>(future: F) -> Poll<F::Output> {
        pin!(future).poll(&mut Context::from_waker(Waker::noop()))
    }

    #[tokio::test]
    async fn local_resolution_keys_wait_for_admission_and_refresh() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        fs_err::write(
            directory.path().join("pyproject.toml"),
            "[tool.uv]\ncache-keys = [{dir = 'generated'}]\n",
        )?;
        let dist = Dist::from_directory_url(
            "demo".parse()?,
            DisplaySafeUrl::from_file_path(directory.path())
                .map_err(|()| anyhow::anyhow!("invalid fixture URL"))?
                .as_str()
                .parse()?,
            directory.path(),
            None,
            None,
        )?;
        let distributions = vec![(
            ResolvedDist::Installable {
                dist: Arc::new(dist),
                version: None,
            },
            HashDigests::empty(),
        )];
        let slots = Arc::new(Semaphore::new(1));
        let reserved = slots.clone().try_acquire_owned()?;
        assert!(poll_once(resolution_cache_key(distributions.clone(), slots.clone())).is_pending());
        drop(reserved);
        let before = resolution_cache_key(distributions.clone(), slots.clone()).await?;
        fs_err::create_dir(directory.path().join("generated"))?;
        let after = resolution_cache_key(distributions, slots).await?;
        assert_ne!(before, after);
        Ok(())
    }

    #[test]
    fn resolution_without_local_inputs_does_not_need_admission() -> anyhow::Result<()> {
        let Poll::Ready(result) = poll_once(resolution_cache_key(
            Vec::new(),
            Arc::new(Semaphore::new(0)),
        )) else {
            anyhow::bail!("expected inline resolution key");
        };
        result?;
        Ok(())
    }

    #[test]
    fn verified_cached_environment_uses_separate_resolution_hash() {
        let resolution_hash = hash_digest(&["ty==0.0.17"]);
        let unverified =
            cached_environment_resolution_hash(resolution_hash.clone(), &HashStrategy::default());
        let verified = cached_environment_resolution_hash(
            resolution_hash.clone(),
            &HashStrategy::verify(Arc::default()),
        );

        assert_eq!(unverified, resolution_hash);
        assert_ne!(verified, unverified);
    }
}
