//! Lock and initialize environments for PEP 723 scripts.

use std::path::{Path, PathBuf};

use owo_colors::OwoColorize;
use tracing::{debug, warn};

use uv_cache::Cache;
use uv_cache_key::cache_digest;
use uv_client::BaseClientBuilder;
use uv_command_support::Printer;
use uv_configuration::{ActiveEnvironment, DryRun};
use uv_fs::{LockedFile, LockedFileError, LockedFileMode, Simplified};
use uv_python_discovery::ConfigDiscovery;
use uv_python_discovery::ScriptInterpreter;
use uv_python_interpreter::PythonEnvironment;
use uv_python_managed::UpgradePolicy;
use uv_python_types::{PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest};
use uv_scripts::Pep723ItemRef;
use uv_settings::PythonInstallMirrors;

use crate::EnvironmentError;

/// Grab a file lock for the script environment to prevent concurrent writes across processes.
async fn lock_script_environment(script: Pep723ItemRef<'_>) -> Result<LockedFile, LockedFileError> {
    match script {
        Pep723ItemRef::Script(script) => {
            LockedFile::acquire(
                std::env::temp_dir().join(format!("uv-{}.lock", cache_digest(&script.path))),
                LockedFileMode::Exclusive,
                script.path.simplified_display(),
            )
            .await
        }
        Pep723ItemRef::Remote(.., url) => {
            LockedFile::acquire(
                std::env::temp_dir().join(format!("uv-{}.lock", cache_digest(url))),
                LockedFileMode::Exclusive,
                url.to_string(),
            )
            .await
        }
        Pep723ItemRef::Stdin(metadata) => {
            LockedFile::acquire(
                std::env::temp_dir().join(format!("uv-{}.lock", cache_digest(&metadata.raw))),
                LockedFileMode::Exclusive,
                "stdin".to_string(),
            )
            .await
        }
    }
}

/// The Python environment for a script.
#[derive(Debug)]
pub enum ScriptEnvironment {
    /// An existing [`PythonEnvironment`] was discovered, which satisfies the script's requirements.
    Existing(PythonEnvironment),
    /// An existing [`PythonEnvironment`] was discovered, but did not satisfy the script's
    /// requirements, and so was replaced.
    Replaced(PythonEnvironment),
    /// A new [`PythonEnvironment`] was created for the script.
    Created(PythonEnvironment),
    /// An existing [`PythonEnvironment`] was discovered, but did not satisfy the script's
    /// requirements. A new environment would've been created, but `--dry-run` mode is enabled; as
    /// such, a temporary environment was created instead.
    WouldReplace(
        PathBuf,
        PythonEnvironment,
        #[allow(unused)] tempfile::TempDir,
    ),
    /// A new [`PythonEnvironment`] would've been created, but `--dry-run` mode is enabled; as such,
    /// a temporary environment was created instead.
    WouldCreate(
        PathBuf,
        PythonEnvironment,
        #[allow(unused)] tempfile::TempDir,
    ),
}

impl ScriptEnvironment {
    /// Initialize a virtual environment for a PEP 723 script.
    pub async fn get_or_init(
        script: Pep723ItemRef<'_>,
        python_request: Option<PythonRequest>,
        client_builder: &BaseClientBuilder<'_>,
        python_preference: PythonPreference,
        python_arch: Option<PythonArchitecture>,
        python_downloads: PythonDownloads,
        install_mirrors: &PythonInstallMirrors,
        no_sync: bool,
        config_discovery: ConfigDiscovery,
        active: ActiveEnvironment,
        cache: &Cache,
        dry_run: DryRun,
        printer: Printer,
    ) -> Result<Self, EnvironmentError> {
        // Lock the script environment to avoid synchronization issues.
        let _lock = lock_script_environment(script)
            .await
            .inspect_err(|err| {
                warn!("Failed to acquire script environment lock: {err}");
            })
            .ok();

        match ScriptInterpreter::discover(
            script,
            python_request,
            client_builder,
            python_preference,
            python_arch,
            python_downloads,
            install_mirrors,
            no_sync,
            config_discovery,
            active,
            cache,
            printer,
        )
        .await?
        {
            // If we found an existing, compatible environment, use it.
            ScriptInterpreter::Environment(environment) => Ok(Self::Existing(environment)),

            // Otherwise, create a virtual environment with the discovered interpreter.
            ScriptInterpreter::Interpreter(requested) => {
                let upgrade_policy = UpgradePolicy::from_request(requested.request());
                let interpreter = requested.into_interpreter();
                let root = ScriptInterpreter::root(script, active, cache);

                // Determine a prompt for the environment, in order of preference:
                //
                // 1) The name of the script
                // 2) No prompt
                let prompt = script
                    .path()
                    .and_then(|path| path.file_name())
                    .map(|f| f.to_string_lossy().to_string())
                    .map(uv_virtualenv::Prompt::Static)
                    .unwrap_or(uv_virtualenv::Prompt::None);

                // Under `--dry-run`, avoid modifying the environment.
                if dry_run.enabled() {
                    let temp_dir = cache.venv_dir()?;
                    let environment = uv_virtualenv::create_venv(
                        temp_dir.path(),
                        interpreter,
                        prompt,
                        false,
                        uv_virtualenv::OnExisting::Remove(
                            uv_virtualenv::RemovalReason::ManagedEnvironment,
                        ),
                        false,
                        uv_virtualenv::Seed::Disabled,
                        upgrade_policy,
                    )?;
                    return Ok(if root.exists() {
                        Self::WouldReplace(root, environment, temp_dir)
                    } else {
                        Self::WouldCreate(root, environment, temp_dir)
                    });
                }

                // Remove the existing virtual environment.
                let replaced = match uv_fs::remove_virtualenv(&root) {
                    Ok(()) => {
                        debug!(
                            "Removed virtual environment at: {}",
                            root.user_display().cyan()
                        );
                        true
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                    Err(err) => return Err(uv_virtualenv::Error::from(err).into()),
                };

                debug!(
                    "Creating script environment at: {}",
                    root.user_display().cyan()
                );

                let environment = uv_virtualenv::create_venv(
                    &root,
                    interpreter,
                    prompt,
                    false,
                    uv_virtualenv::OnExisting::Remove(
                        uv_virtualenv::RemovalReason::ManagedEnvironment,
                    ),
                    false,
                    uv_virtualenv::Seed::Disabled,
                    upgrade_policy,
                )?;
                environment.cache_virtualenv(false, cache)?;

                Ok(if replaced {
                    Self::Replaced(environment)
                } else {
                    Self::Created(environment)
                })
            }
        }
    }

    /// Convert the [`ScriptEnvironment`] into a [`PythonEnvironment`].
    ///
    /// Returns an error if the environment was created in `--dry-run` mode, as dropping the
    /// associated temporary directory could lead to errors downstream.
    pub fn into_environment(self) -> Result<PythonEnvironment, EnvironmentError> {
        match self {
            Self::Existing(environment) => Ok(environment),
            Self::Replaced(environment) => Ok(environment),
            Self::Created(environment) => Ok(environment),
            Self::WouldReplace(..) => Err(EnvironmentError::DroppedEnvironment),
            Self::WouldCreate(..) => Err(EnvironmentError::DroppedEnvironment),
        }
    }

    /// Return the path to the actual target, if this was a dry run environment.
    pub fn dry_run_target(&self) -> Option<&Path> {
        match self {
            Self::WouldReplace(path, _, _) | Self::WouldCreate(path, _, _) => Some(path),
            Self::Created(_) | Self::Existing(_) | Self::Replaced(_) => None,
        }
    }
}

impl std::ops::Deref for ScriptEnvironment {
    type Target = PythonEnvironment;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Existing(environment) => environment,
            Self::Replaced(environment) => environment,
            Self::Created(environment) => environment,
            Self::WouldReplace(_, environment, _) => environment,
            Self::WouldCreate(_, environment, _) => environment,
        }
    }
}
