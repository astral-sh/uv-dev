use uv_command_support::UvError;
use uv_environment_operations::{EnvironmentError, LockedToolInstaller};
use uv_lock::{Lock, Package};
use uv_lock_operations::{LockMode, LockOperation, LockTarget};
use uv_normalize::{DEV_DEPENDENCIES, PackageName};
use uv_pep508::Requirement;
use uv_pypi_types::{LenientRequirement, VerbatimParsedUrl};
use uv_python_interpreter::Interpreter;
use uv_resolve_operations::loggers::SummaryResolveLogger;
use uv_settings::LockedSource;
use uv_workspace::dependency_groups::FlatDependencyGroups;
use uv_workspace::{DiscoveryOptions, VirtualProject, Workspace, WorkspaceCache};

/// Under `--locked`, a new declaration can make a lock stale before it contains a checker.
pub(super) fn manifest_declares_checker(
    workspace: &Workspace,
    name: &PackageName,
) -> Result<bool, EnvironmentError> {
    let manifest = workspace.pyproject_toml();
    let declares = |req: &Requirement<VerbatimParsedUrl>| {
        &req.name == name && req.marker.evaluate_optional_environment(None, &[])
    };
    let group =
        FlatDependencyGroups::from_group(workspace.install_path(), manifest, &DEV_DEPENDENCIES)?;
    if group.is_some_and(|group| group.requirements.iter().any(declares)) {
        return Ok(true);
    }
    Ok(manifest
        .project
        .as_ref()
        .and_then(|project| project.dependencies.as_ref())
        .is_some_and(|deps| {
            deps.iter().any(|requirement| {
                requirement
                    .parse::<LenientRequirement>()
                    .is_ok_and(|req| declares(&req.into()))
            })
        }))
}

/// The locked declaration must belong to the discovered workspace root, even in frozen mode.
pub(super) fn check_root(workspace: &Workspace, lock: &Lock) -> Result<(), UvError> {
    let manifest_name = workspace
        .pyproject_toml()
        .project
        .as_ref()
        .map(|project| &project.name);
    if manifest_name != lock.root().map(Package::name) {
        return Err(UvError::user(anyhow::anyhow!(
            "The workspace root in `uv.lock` does not match the root in `pyproject.toml`. Run `uv lock` in the workspace to update the lockfile"
        )));
    }

    Ok(())
}

/// Validate the entire workspace, including members outside the script's path.
pub(super) async fn check_freshness(
    workspace: &Workspace,
    lock: &Lock,
    locked: LockedSource,
    interpreter: &Interpreter,
    installer: &LockedToolInstaller<'_>,
    workspace_cache: &WorkspaceCache,
) -> Result<(), UvError> {
    let project = VirtualProject::discover(
        workspace.install_path(),
        &DiscoveryOptions::default(),
        installer.cache,
        workspace_cache,
    )
    .await
    .map_err(EnvironmentError::from)?;
    let result = Box::pin(
        LockOperation::new(
            LockMode::Locked(interpreter, locked),
            &installer.settings.resolver,
            installer.client_builder,
            installer.state,
            Box::new(SummaryResolveLogger),
            installer.concurrency,
            installer.cache,
            workspace_cache,
            installer.printer,
            installer.preview,
        )
        .execute(LockTarget::Workspace(project.workspace())),
    )
    .await?;
    // The candidates and interpreter were selected from the first read.
    if result.lock() != lock {
        return Err(UvError::user(anyhow::anyhow!(
            "The workspace lockfile changed during checker lookup. Retry the command"
        )));
    }
    Ok(())
}
