//! Interpreter discovery for workspace groups whose domains depend on built metadata.

use std::collections::BTreeSet;
use std::path::Path;

use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_command_support::{Printer, UvError};
use uv_configuration::{ActiveEnvironment, Concurrency, DependencyGroupsWithDefaults};
use uv_dispatch::UniversalState;
use uv_lock_operations::{LockMode, LockOperation, workspace_groups_with_cached_metadata};
use uv_normalize::PackageName;
use uv_preview::Preview;
use uv_python_discovery::{ConfigDiscovery, ProjectPythonRequest, PythonSelectionError};
use uv_python_types::{PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest};
use uv_resolve_operations::loggers::DefaultResolveLogger;
use uv_settings::{LockCheck, PythonInstallMirrors, ResolverSettings};
use uv_workspace::{ResolvedWorkspaceGroup, Workspace, WorkspaceCache};

use crate::{
    EnvironmentError, ProjectEnvironmentPolicy, ProjectEnvironmentTarget, ProjectInterpreter,
};

/// Resolve dynamic member metadata before a group domain selects the project environment.
pub async fn discover_workspace_groups(
    workspace: &Workspace,
    project_dir: &Path,
    python: Option<&str>,
    lock_check: LockCheck,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    state: &UniversalState,
    first_party_exclusions: &BTreeSet<PackageName>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    install_mirrors: &PythonInstallMirrors,
    concurrency: &Concurrency,
    config_discovery: ConfigDiscovery,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> anyhow::Result<Vec<ResolvedWorkspaceGroup>> {
    let mut groups = workspace_groups_with_cached_metadata(workspace, &settings.sources, state)?;
    while let Some((group, member)) = groups.iter().find_map(|group| {
        group
            .pending_metadata()
            .first()
            .map(|member| (group, member))
    }) {
        // Each group may require a different preliminary interpreter. Discovery is read-only;
        // the final project environment is selected after metadata has refined every domain.
        let mut metadata_group = group.clone();
        metadata_group.narrow_environment(group.member_environments()[member])?;
        let scoped = workspace.with_workspace_groups(std::slice::from_ref(&metadata_group));
        let project_python = ProjectPythonRequest::from_request(
            python.map(PythonRequest::parse),
            Some(&scoped),
            &DependencyGroupsWithDefaults::none(),
            &settings.sources,
            project_dir,
            config_discovery,
        )
        .await?;
        let discover = |request| {
            Box::pin(ProjectInterpreter::discover(
                ProjectEnvironmentTarget::from(&scoped),
                request,
                client_builder,
                python_preference,
                python_arch,
                python_downloads,
                install_mirrors,
                ProjectEnvironmentPolicy::Optional,
                ActiveEnvironment::Ignore,
                cache,
                if printer == Printer::Verbose {
                    printer
                } else {
                    Printer::Silent
                },
            ))
        };
        let interpreter = match discover(project_python).await {
            Ok(interpreter) => interpreter,
            Err(EnvironmentError::PythonSelection(error))
                if matches!(
                    error.as_ref(),
                    PythonSelectionError::RequestedPythonProjectIncompatibility(..)
                        | PythonSelectionError::DotPythonVersionProjectIncompatibility { .. }
                ) =>
            {
                // A lock can contain groups outside the command's selected Python domain. Their
                // metadata needs a compatible build interpreter; final discovery still enforces
                // the command's explicit request or version file on the selected environment.
                let request = ProjectPythonRequest::from_request(
                    None,
                    Some(&scoped),
                    &DependencyGroupsWithDefaults::none(),
                    &settings.sources,
                    project_dir,
                    ConfigDiscovery::Disabled,
                )
                .await?;
                discover(request).await?
            }
            Err(error) => return Err(error.into()),
        }
        .into_interpreter();
        let mode = match lock_check {
            LockCheck::Enabled(source) => LockMode::Locked(&interpreter, source),
            LockCheck::Disabled => LockMode::DryRun(&interpreter),
        };
        Box::pin(
            LockOperation::new(
                mode,
                settings,
                client_builder,
                state,
                Box::new(DefaultResolveLogger),
                concurrency,
                cache,
                workspace_cache,
                printer,
                preview,
            )
            .with_first_party_exclusions(first_party_exclusions.clone())
            .resolve_workspace_group_metadata(workspace, &metadata_group, member),
        )
        .await
        .map_err(UvError::from)?;
        groups = workspace_groups_with_cached_metadata(workspace, &settings.sources, state)?;
    }
    Ok(groups)
}
