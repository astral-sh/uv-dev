//! Discover, lock and initialize project environments.

use std::fmt::Write;
use std::io;
use std::path::{Path, PathBuf};

use owo_colors::OwoColorize;
use tracing::{debug, warn};

use uv_cache::{Cache, CacheBucket};
use uv_cache_key::{cache_digest, cache_name};
use uv_client::BaseClientBuilder;
use uv_command_support::Printer;
use uv_configuration::{ActiveEnvironment, DependencyGroupsWithDefaults, DryRun};
use uv_distribution_types::RequiresPython;
use uv_fs::{LockedFile, LockedFileError, LockedFileMode, Simplified, verbatim_path};
use uv_lock::{Installable, Lock};
use uv_normalize::PackageName;
use uv_preview::PreviewFeature;
use uv_python_discovery::CompatibleProjectPython;
use uv_python_discovery::ConfigDiscovery;
use uv_python_discovery::EnvironmentIncompatibilityError;
use uv_python_discovery::EnvironmentKind;
use uv_python_discovery::ProjectPythonRequest;
use uv_python_discovery::PythonDownloadReporter;
use uv_python_discovery::PythonInstallation;
use uv_python_discovery::check_environment_compatibility;
use uv_python_interpreter::{BrokenLink, Interpreter, InvalidEnvironmentKind, PythonEnvironment};
use uv_python_managed::{PythonMinorVersionLink, UpgradePolicy};
use uv_python_types::{
    EnvironmentPreference, LenientImplementationName, PythonArchitecture, PythonDownloads,
    PythonPreference, PythonRequest,
};
use uv_settings::PythonInstallMirrors;
use uv_warnings::{warn_user, warn_user_once};
use uv_workspace::{ProjectEnvironmentSelection, Workspace};

use crate::EnvironmentError;
use crate::install_target::{InstallTarget, PackageSelection};

/// The policy for discovering and initializing a project environment.
#[derive(Debug, Clone, Copy)]
pub enum ProjectEnvironmentPolicy {
    /// An environment is unnecessary; ignore it if invalid or incompatible.
    Optional,

    /// Require a valid environment compatible with the Python requirements.
    ///
    /// Replace an existing environment if it is incompatible.
    Compatible,

    /// Preserve a valid existing environment, even if incompatible.
    ///
    /// Create an environment if none exists, or replace an invalid virtual environment.
    Preserve,
}

/// Discover an existing project environment at `root` without validating its compatibility.
fn existing_project_environment(
    root: &Path,
    centralized: bool,
    policy: ProjectEnvironmentPolicy,
    cache: &Cache,
) -> Result<Option<PythonEnvironment>, EnvironmentError> {
    let environment = match PythonEnvironment::from_root(root, cache) {
        Ok(environment) => environment,
        Err(uv_python_interpreter::PythonEnvironmentError::MissingEnvironment(_)) => {
            return Ok(None);
        }
        Err(uv_python_interpreter::PythonEnvironmentError::InvalidEnvironment(inner)) => {
            match inner.kind {
                InvalidEnvironmentKind::NotDirectory => {
                    return Err(EnvironmentError::InvalidProjectEnvironmentDir(
                        root.to_path_buf(),
                        inner.kind.to_string(),
                    ));
                }
                InvalidEnvironmentKind::MissingExecutable(_) => {
                    if !matches!(policy, ProjectEnvironmentPolicy::Optional)
                        && !centralized
                        && fs_err::read_dir(root).is_ok_and(|mut dir| dir.next().is_some())
                    {
                        if !root.join("pyvenv.cfg").try_exists().unwrap_or_default() {
                            return Err(EnvironmentError::InvalidProjectEnvironmentDir(
                                root.to_path_buf(),
                                "it is not a valid Python environment (no Python executable was found)"
                                    .to_string(),
                            ));
                        }
                    }
                }
                InvalidEnvironmentKind::Empty => {}
            }
            return Ok(None);
        }
        Err(uv_python_interpreter::PythonEnvironmentError::Query(
            uv_python_interpreter::InterpreterError::NotFound(_),
        )) => {
            return Ok(None);
        }
        Err(uv_python_interpreter::PythonEnvironmentError::Query(
            uv_python_interpreter::InterpreterError::BrokenLink(BrokenLink {
                path,
                unix,
                venv: _,
            }),
        )) => {
            if unix {
                let target_path = fs_err::read_link(&path)?;
                warn_user!(
                    "Ignoring existing virtual environment linked to non-existent Python interpreter: `{}` -> `{}`",
                    path.user_display().cyan(),
                    target_path.user_display().cyan(),
                );
            } else {
                warn_user!(
                    "Ignoring existing virtual environment linked to non-existent Python interpreter: {}",
                    path.user_display().cyan(),
                );
            }
            return Ok(None);
        }
        Err(err) => return Err(err.into()),
    };

    Ok(Some(environment))
}

/// Discover a compatible project environment at `root`.
fn discover_project_environment(
    root: &Path,
    python_request: Option<&PythonRequest>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    requires_python: Option<&RequiresPython>,
    policy: ProjectEnvironmentPolicy,
    centralized: bool,
    cache: &Cache,
) -> Result<Option<PythonEnvironment>, EnvironmentError> {
    let Some(environment) = existing_project_environment(root, centralized, policy, cache)? else {
        return Ok(None);
    };

    let compatibility = check_environment_compatibility(
        &environment,
        EnvironmentKind::Project,
        python_request,
        python_preference,
        python_arch,
        requires_python,
        cache,
    );

    // Conflicting versions for the same base interpreter indicate its cached metadata may be
    // corrupted. Clear the entry before interpreter discovery can select stale metadata.
    if matches!(
        &compatibility,
        Err(EnvironmentIncompatibilityError::PyenvVersionConflict(..))
    ) && let Ok(base_executable) = environment.interpreter().to_base_python()
        && let Ok(base_interpreter) = Interpreter::query(&base_executable, cache)
        && environment.uses(&base_interpreter)
        && environment.interpreter().python_version() != base_interpreter.python_version()
    {
        debug!(
            "Clearing cached interpreter info for `{}` after finding conflicting Python versions ({} and {})",
            base_executable.user_display(),
            base_interpreter.python_version(),
            environment.interpreter().python_version(),
        );
        Interpreter::clear_cache(&base_executable, cache)?;
    }

    match compatibility {
        Ok(()) => Ok(Some(environment)),
        Err(err) if matches!(policy, ProjectEnvironmentPolicy::Preserve) => {
            if centralized {
                let root = environment.root();
                warn_user!(
                    "Using incompatible environment (`{}`) due to `--no-sync` ({err})",
                    root.file_name()
                        .unwrap_or(root.as_os_str())
                        .to_string_lossy()
                        .cyan(),
                );
            } else {
                warn_user!(
                    "Using incompatible environment (`{}`) due to `--no-sync` ({err})",
                    environment.root().user_display().cyan(),
                );
            }
            Ok(Some(environment))
        }
        Err(err) => {
            debug!("{err}");
            Ok(None)
        }
    }
}

/// Return whether to use centralized project environments for this invocation.
pub fn centralized_environments_enabled(
    selection: &ProjectEnvironmentSelection,
    cache: &Cache,
) -> bool {
    if !selection.is_default() || !uv_preview::is_enabled(PreviewFeature::CentralizedProjectEnvs) {
        return false;
    }
    if cache.is_temporary() {
        warn_user_once!(
            "The `centralized-project-envs` feature has no effect when `--no-cache` is enabled"
        );
        return false;
    }
    true
}

/// Return whether `path` is lexically within `base`.
fn is_path_lexically_within(path: &Path, base: &Path) -> bool {
    // Normally only longer paths must be in the verbatim namespace, normalise both so the
    // comparison works correctly regardless.
    verbatim_path(path).starts_with(verbatim_path(base).as_ref())
}

/// Return whether `path` looks like a path we wrote and references our environment cache.
///
/// This isn't fully robust, and cannot be, as the path may not exist.
fn is_centralized_environment_path(path: &Path, cache: &Cache) -> bool {
    let Ok(environments) = std::path::absolute(cache.bucket(CacheBucket::Environments)) else {
        return false;
    };
    if is_path_lexically_within(path, &environments) {
        return true;
    }

    // Resolve existing relative or indirect paths; only the lexical check can handle dangling
    // paths.
    fs_err::canonicalize(path).is_ok_and(|path| {
        fs_err::canonicalize(&environments)
            .is_ok_and(|environments| is_path_lexically_within(&path, &environments))
    })
}

/// Return whether `path` appears to link into the current cache's environment bucket.
fn is_centralized_environment_link(path: &Path, cache: &Cache) -> bool {
    let Ok(target) = fs_err::read_link(path) else {
        return false;
    };
    is_centralized_environment_path(&target, cache) || is_centralized_environment_path(path, cache)
}

/// Read an environment path from a file.
fn read_environment_path_file(path: &Path) -> io::Result<PathBuf> {
    let target = PathBuf::from(fs_err::read_to_string(path)?);
    Ok(if target.is_absolute() {
        target
    } else {
        path.parent().unwrap_or(Path::new("")).join(target)
    })
}

/// Return whether `path` refers to an environment in the current cache's environment bucket.
pub fn is_centralized_environment_reference(path: &Path, cache: &Cache) -> bool {
    is_centralized_environment_link(path, cache)
        || read_environment_path_file(path)
            .is_ok_and(|target| is_centralized_environment_path(&target, cache))
}

/// Return the centralized environment path for a project and interpreter.
pub fn centralized_environment_root(
    target: ProjectEnvironmentTarget<'_>,
    interpreter: &Interpreter,
    upgrade_policy: UpgradePolicy,
    cache: &Cache,
) -> PathBuf {
    let install_path = target.install_path();
    let workspace_path =
        fs_err::canonicalize(install_path).unwrap_or_else(|_| install_path.to_path_buf());
    let interpreter_key = interpreter.key();
    // Use the workspace path to isolate projects and the interpreter key to maximize intra-project
    // environment re-use while avoiding clashes with incompatible environments. Ignoring the patch
    // version allows upgradeable managed environments to be re-used after an upgrade.
    let (digest, python_version) =
        if let Some(link) = PythonMinorVersionLink::from_interpreter(interpreter, upgrade_policy) {
            (
                cache_digest(&(&workspace_path, link.key())),
                interpreter.python_minor_version(),
            )
        } else {
            (
                cache_digest(&(&workspace_path, &interpreter_key)),
                interpreter.python_version().clone(),
            )
        };
    let name = target
        .project_name()
        .and_then(|name| cache_name(name.as_ref(), Some(100)))
        .or_else(|| {
            workspace_path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| cache_name(name, Some(100)))
        });
    let implementation = interpreter_key.implementation();
    let implementation = match implementation.as_ref() {
        LenientImplementationName::Known(implementation) => implementation
            .short_name()
            .unwrap_or_else(|| implementation.long_name()),
        LenientImplementationName::Unknown(implementation) => implementation,
    };
    let entry = name.map_or_else(
        // A virtual workspace can be nameless if its directory has no cache-safe characters.
        || format!("{implementation}{python_version}-{digest}"),
        |name| format!("{name}-{implementation}{python_version}-{digest}"),
    );
    cache
        .shard(CacheBucket::Environments, entry)
        .into_path_buf()
}

/// How to report failures updating `.venv`.
#[derive(Clone, Copy)]
pub enum LinkErrorReporting {
    /// Report failures to the user.
    User,
    /// Log failures at warning level.
    Log,
}

/// Point the project's `.venv` to the centralized environment, returning whether the link was
/// successfully updated.
pub fn update_project_environment_link(
    environment: &PythonEnvironment,
    target: ProjectEnvironmentTarget<'_>,
    link_error_reporting: LinkErrorReporting,
) -> bool {
    let link = target.install_path().join(".venv");
    let report_error = |message: std::fmt::Arguments<'_>| match link_error_reporting {
        LinkErrorReporting::User => warn_user_once!("{message}"),
        LinkErrorReporting::Log => warn!("{message}"),
    };

    if fs_err::symlink_metadata(&link).is_ok_and(|metadata| metadata.is_dir()) {
        if uv_fs::is_virtualenv_base(&link) {
            if let Err(err) = uv_fs::remove_virtualenv(&link) {
                report_error(format_args!(
                    "Failed to remove existing local virtual environment: {err}"
                ));
                return false;
            }
        } else {
            // On Windows, copying a junction can produce an empty directory.
            #[cfg(windows)]
            if let Err(err) = fs_err::remove_dir(&link) {
                report_error(format_args!(
                    "Failed to create link to project environment: {err}"
                ));
                return false;
            }
        }
    }

    // On Windows replace_symlink won't replace a file, but we want to try to upgrade to a junction
    // if possible.
    if cfg!(windows) {
        let _ = fs_err::remove_file(&link);
    }

    let Err(link_error) = uv_fs::replace_symlink(environment.root(), &link) else {
        return true;
    };
    warn!("Failed to create link to project environment: {link_error}");

    let Some(target) = environment.root().to_str() else {
        report_error(format_args!(
            "Failed to write the environment path to `{}`: the path is not valid UTF-8",
            link.simplified_display()
        ));
        return false;
    };

    if let Err(err) = uv_fs::write_atomic_sync(&link, target.as_bytes()) {
        report_error(format_args!("Failed to write the environment path: {err}"));
        return false;
    }

    report_error(format_args!(
        "Failed to create link to project environment; wrote the environment path to `{}` instead",
        link.simplified_display()
    ));
    false
}

/// The project information needed to discover and create an environment.
#[derive(Clone, Copy)]
pub enum ProjectEnvironmentTarget<'a> {
    Workspace(&'a Workspace),
    Lockfile { root: &'a Path, lock: &'a Lock },
}

impl<'a> From<&'a Workspace> for ProjectEnvironmentTarget<'a> {
    fn from(workspace: &'a Workspace) -> Self {
        Self::Workspace(workspace)
    }
}

impl<'a> ProjectEnvironmentTarget<'a> {
    /// Return the directory where the environment is installed.
    fn install_path(self) -> &'a Path {
        match self {
            Self::Workspace(workspace) => workspace.install_path(),
            Self::Lockfile { root, .. } => root,
        }
    }

    /// Return the project associated with this environment, if any.
    fn project_name(self) -> Option<&'a PackageName> {
        match self {
            Self::Workspace(workspace) => workspace
                .pyproject_toml()
                .project
                .as_ref()
                .map(|project| &project.name),
            Self::Lockfile { lock, .. } => lock.root().map(uv_lock::Package::name),
        }
    }

    /// Return the discovered workspace, if this target has one.
    fn workspace(self) -> Option<&'a Workspace> {
        match self {
            Self::Workspace(workspace) => Some(workspace),
            Self::Lockfile { .. } => None,
        }
    }
}

/// An interpreter suitable for the project.
#[derive(Debug)]
#[expect(clippy::large_enum_variant)]
pub enum ProjectInterpreter {
    /// A compatible interpreter from outside the project, to create a new virtual environment.
    Interpreter(CompatibleProjectPython),
    /// An existing project environment, which may be incompatible under `--no-sync`.
    Environment(PythonEnvironment),
}

impl ProjectInterpreter {
    /// Discover an existing project environment without selecting or downloading an interpreter.
    pub fn discover_existing(
        install_path: &Path,
        active: ActiveEnvironment,
        cache: &Cache,
    ) -> Result<Option<PythonEnvironment>, EnvironmentError> {
        let selection = ProjectEnvironmentSelection::from_install_path(install_path, active);
        let root = selection
            .explicit_path()
            .map_or_else(|| install_path.join(".venv"), Path::to_path_buf);
        let root = read_environment_path_file(&root).unwrap_or(root);
        let centralized = centralized_environments_enabled(&selection, cache)
            || is_centralized_environment_reference(&root, cache);
        let root = if centralized {
            fs_err::canonicalize(&root).unwrap_or(root)
        } else {
            root
        };

        existing_project_environment(
            &root,
            centralized,
            ProjectEnvironmentPolicy::Optional,
            cache,
        )
    }

    /// Discover an interpreter for a workspace or frozen lockfile.
    pub async fn discover(
        target: ProjectEnvironmentTarget<'_>,
        project_python: ProjectPythonRequest,
        client_builder: &BaseClientBuilder<'_>,
        python_preference: PythonPreference,
        python_arch: Option<PythonArchitecture>,
        python_downloads: PythonDownloads,
        install_mirrors: &PythonInstallMirrors,
        policy: ProjectEnvironmentPolicy,
        active: ActiveEnvironment,
        cache: &Cache,
        printer: Printer,
    ) -> Result<Self, EnvironmentError> {
        let python_request = project_python.python_request.as_ref();
        let requires_python = project_python.requires_python();
        let upgrade_policy =
            UpgradePolicy::from_request(python_request.unwrap_or(&PythonRequest::Default));

        let environment_selection =
            ProjectEnvironmentSelection::from_install_path(target.install_path(), active);
        let centralized = centralized_environments_enabled(&environment_selection, cache);

        // Prefer `.venv`'s interpreter to keep its compatible cached environment selected; derive
        // the cache root instead of trusting the link target.
        if centralized {
            let project_environment_path = target.install_path().join(".venv");
            if let Ok(candidate) = PythonEnvironment::from_root(
                read_environment_path_file(&project_environment_path)
                    .ok()
                    .as_deref()
                    .unwrap_or(&project_environment_path),
                cache,
            ) {
                let root = centralized_environment_root(
                    target,
                    candidate.interpreter(),
                    upgrade_policy,
                    cache,
                );
                if let Some(environment) = discover_project_environment(
                    &root,
                    python_request,
                    python_preference,
                    python_arch,
                    requires_python,
                    policy,
                    centralized,
                    cache,
                )? {
                    return Ok(Self::Environment(environment));
                }
            }
        } else {
            let project_environment_path = environment_selection
                .explicit_path()
                .map_or_else(|| target.install_path().join(".venv"), Path::to_path_buf);
            // TODO(tk): Revisit after PEP 832.
            // A centralized path file is not a local environment; let initialization replace it.
            if !(environment_selection.is_default()
                && read_environment_path_file(&project_environment_path)
                    .is_ok_and(|target| is_centralized_environment_path(&target, cache)))
                && let Some(environment) = discover_project_environment(
                    &project_environment_path,
                    python_request,
                    python_preference,
                    python_arch,
                    requires_python,
                    policy,
                    centralized,
                    cache,
                )?
            {
                return Ok(Self::Environment(environment));
            }
        }

        let reporter = PythonDownloadReporter::single(printer);

        // Locate the Python interpreter to use in the environment.
        let python = PythonInstallation::find_or_download(
            python_request,
            EnvironmentPreference::OnlySystem,
            python_preference,
            python_arch,
            python_downloads,
            client_builder,
            cache,
            Some(&reporter),
            install_mirrors.mirrors(),
            install_mirrors.python_downloads_json_url.as_deref(),
        )
        .await?;

        if centralized {
            let root =
                centralized_environment_root(target, python.interpreter(), upgrade_policy, cache);
            if let Some(environment) = discover_project_environment(
                &root,
                python_request,
                python_preference,
                python_arch,
                requires_python,
                policy,
                centralized,
                cache,
            )? {
                return Ok(Self::Environment(environment));
            }
        }

        let managed = python.source().is_managed();
        let implementation = python.implementation();
        let interpreter = python.into_interpreter();

        if managed {
            writeln!(
                printer.stderr(),
                "Using {} {}{}",
                implementation.pretty(),
                interpreter.python_version().cyan(),
                interpreter.variant().display_suffix().cyan(),
            )?;
        } else {
            writeln!(
                printer.stderr(),
                "Using {} {}{} interpreter at: {}",
                implementation.pretty(),
                interpreter.python_version(),
                interpreter.variant().display_suffix(),
                interpreter.sys_executable().user_display().cyan()
            )?;
        }

        Ok(Self::Interpreter(project_python.validate(interpreter)?))
    }

    /// Convert the [`ProjectInterpreter`] into an [`Interpreter`].
    pub fn into_interpreter(self) -> Interpreter {
        match self {
            Self::Interpreter(interpreter) => interpreter.into_interpreter(),
            Self::Environment(environment) => environment.into_interpreter(),
        }
    }
}

/// Grab a file lock for the project environment to prevent concurrent writes across processes.
pub async fn lock_project_environment(
    target: ProjectEnvironmentTarget<'_>,
) -> Result<LockedFile, LockedFileError> {
    let install_path = target.install_path();
    LockedFile::acquire(
        std::env::temp_dir().join(format!("uv-{}.lock", cache_digest(&install_path))),
        LockedFileMode::Exclusive,
        install_path.simplified_display(),
    )
    .await
}

/// The Python environment for a project.
#[derive(Debug)]
pub enum ProjectEnvironment {
    /// An existing [`PythonEnvironment`] was accepted by the compatibility policy.
    Existing(PythonEnvironment),
    /// An existing [`PythonEnvironment`] was discovered, but did not satisfy the project's
    /// requirements, and so was replaced.
    Replaced(PythonEnvironment),
    /// A new [`PythonEnvironment`] was created.
    Created(PythonEnvironment),
    /// An existing [`PythonEnvironment`] was discovered, but did not satisfy the project's
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

impl ProjectEnvironment {
    /// Initialize a virtual environment for the current project.
    pub async fn get_or_init(
        target: ProjectEnvironmentTarget<'_>,
        frozen_target: Option<InstallTarget<'_>>,
        groups: &DependencyGroupsWithDefaults,
        python: Option<PythonRequest>,
        install_mirrors: &PythonInstallMirrors,
        client_builder: &BaseClientBuilder<'_>,
        python_preference: PythonPreference,
        python_arch: Option<PythonArchitecture>,
        python_downloads: PythonDownloads,
        no_sync: bool,
        config_discovery: ConfigDiscovery,
        active: ActiveEnvironment,
        cache: &Cache,
        dry_run: DryRun,
        link_error_reporting: LinkErrorReporting,
        printer: Printer,
    ) -> Result<Self, EnvironmentError> {
        let environment_selection =
            ProjectEnvironmentSelection::from_install_path(target.install_path(), active);
        let centralized = centralized_environments_enabled(&environment_selection, cache);

        // Lock the project environment to avoid synchronization issues.
        let _lock = lock_project_environment(target)
            .await
            .inspect_err(|err| {
                warn!("Failed to acquire project environment lock: {err}");
            })
            .ok();

        // A selected installation target narrows group requirements. Otherwise a lockfile-only
        // environment uses the requirements of the entire workspace.
        let frozen_target = frozen_target.or_else(|| match target {
            ProjectEnvironmentTarget::Workspace(_) => None,
            ProjectEnvironmentTarget::Lockfile { root, lock } => Some(InstallTarget::Lockfile {
                root,
                project_name: lock.root().map(uv_lock::Package::name),
                selection: PackageSelection::Workspace,
                lock,
            }),
        });
        let project_python = if let Some(frozen_target) = frozen_target {
            ProjectPythonRequest::from_requirements(
                python,
                Some(frozen_target.install_path()),
                Some(frozen_target.python_requirement(groups)?),
                target.install_path(),
                config_discovery,
            )
            .await?
        } else {
            ProjectPythonRequest::from_request(
                python,
                target.workspace(),
                groups,
                target.install_path(),
                config_discovery,
            )
            .await?
        };
        match ProjectInterpreter::discover(
            target,
            project_python,
            client_builder,
            python_preference,
            python_arch,
            python_downloads,
            install_mirrors,
            if no_sync {
                ProjectEnvironmentPolicy::Preserve
            } else {
                ProjectEnvironmentPolicy::Compatible
            },
            active,
            cache,
            printer,
        )
        .await?
        {
            // Use the environment accepted by the compatibility policy.
            ProjectInterpreter::Environment(environment) => {
                if centralized && !dry_run.enabled() {
                    update_project_environment_link(&environment, target, link_error_reporting);
                }
                Ok(Self::Existing(environment))
            }

            // Otherwise, create a virtual environment with the discovered interpreter.
            ProjectInterpreter::Interpreter(interpreter) => {
                let requested = interpreter.into_requested_interpreter();
                let upgrade_policy = UpgradePolicy::from_request(requested.request());
                let interpreter = requested.into_interpreter();
                let root = if centralized {
                    centralized_environment_root(target, &interpreter, upgrade_policy, cache)
                } else {
                    environment_selection
                        .explicit_path()
                        .map_or_else(|| target.install_path().join(".venv"), Path::to_path_buf)
                };
                let centralized_environment_reference =
                    !centralized && is_centralized_environment_reference(&root, cache);

                // Avoid removing things that are not virtual environments and are outside the
                // environment cache.
                let replace_environment = if centralized_environment_reference {
                    true
                } else {
                    match (root.try_exists(), root.join("pyvenv.cfg").try_exists()) {
                        // It's a virtual environment we can remove it
                        (_, Ok(true)) => true,
                        // It doesn't exist at all, we should use it without deleting it to avoid TOCTOU bugs
                        (Ok(false), Ok(false)) => false,
                        // If it's not a virtual environment, bail
                        (Ok(true), Ok(false)) => {
                            // Unless it's empty, in which case we just ignore it
                            if root.read_dir().is_ok_and(|mut dir| dir.next().is_none()) {
                                false
                            } else if centralized {
                                // Unless it's the derived cache entry, which is uv-owned and safe to replace
                                true
                            } else {
                                return Err(EnvironmentError::InvalidProjectEnvironmentDir(
                                    root,
                                    "it is not a compatible environment but cannot be recreated because it is not a virtual environment".to_string(),
                                ));
                            }
                        }
                        // Similarly, if we can't _tell_ if it exists we should bail
                        (_, Err(err)) | (Err(err), _) => {
                            return Err(EnvironmentError::InvalidProjectEnvironmentDir(
                                root,
                                format!(
                                    "it is not a compatible environment but cannot be recreated because uv cannot determine if it is a virtual environment: {err}"
                                ),
                            ));
                        }
                    }
                };

                // Determine a prompt for the environment, in order of preference:
                //
                // 1) The name of the project
                // 2) The name of the directory at the root of the workspace
                // 3) No prompt
                let prompt = target
                    .project_name()
                    .map(ToString::to_string)
                    .or_else(|| {
                        target
                            .install_path()
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                    })
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
                        uv_preview::is_enabled(PreviewFeature::RelocatableEnvsDefault),
                        uv_virtualenv::Seed::Disabled,
                        upgrade_policy,
                    )?;
                    return Ok(if replace_environment {
                        Self::WouldReplace(root, environment, temp_dir)
                    } else {
                        Self::WouldCreate(root, environment, temp_dir)
                    });
                }

                if replace_environment {
                    // Remove centralized references directly to preserve their cached targets.
                    let removed = if centralized_environment_reference {
                        match uv_fs::remove_virtualenv(&root) {
                            Ok(()) => true,
                            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                            Err(err) => return Err(uv_virtualenv::Error::from(err).into()),
                        }
                    } else {
                        uv_fs::clear_virtualenv(&root).map_err(uv_virtualenv::Error::from)?
                    };
                    if removed {
                        let removed_entry = if centralized_environment_reference {
                            "link to project environment"
                        } else {
                            "virtual environment"
                        };
                        writeln!(
                            printer.stderr(),
                            "Removed {removed_entry} at: {}",
                            root.user_display().cyan()
                        )?;
                    }
                }

                if centralized {
                    writeln!(
                        printer.stderr(),
                        "Creating virtual environment `{}`",
                        root.file_name()
                            .unwrap_or(root.as_os_str())
                            .to_string_lossy()
                            .cyan(),
                    )?;
                } else {
                    writeln!(
                        printer.stderr(),
                        "Creating virtual environment at: {}",
                        root.user_display().cyan()
                    )?;
                }

                let environment = uv_virtualenv::create_venv(
                    &root,
                    interpreter,
                    prompt,
                    false,
                    uv_virtualenv::OnExisting::Remove(
                        uv_virtualenv::RemovalReason::ManagedEnvironment,
                    ),
                    uv_preview::is_enabled(PreviewFeature::RelocatableEnvsDefault),
                    uv_virtualenv::Seed::Disabled,
                    upgrade_policy,
                )?;
                environment.cache_virtualenv(false, cache)?;

                if centralized {
                    update_project_environment_link(&environment, target, link_error_reporting);
                }

                if replace_environment {
                    Ok(Self::Replaced(environment))
                } else {
                    Ok(Self::Created(environment))
                }
            }
        }
    }

    /// Convert the [`ProjectEnvironment`] into a [`PythonEnvironment`].
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

impl std::ops::Deref for ProjectEnvironment {
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
