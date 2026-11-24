use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_command_support::Printer;
use uv_configuration::{Concurrency, DependencyGroupsWithDefaults};
use uv_dispatch::UniversalState;
use uv_install_operations::loggers::SummaryInstallLogger;
use uv_lock::Installable;
use uv_normalize::{GroupName, PackageName};
use uv_preview::Preview;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_settings::ResolverInstallerSettings;

use crate::EnvironmentError;
use crate::environment::CachedEnvironment;
use crate::install_target::InstallTarget;
use crate::malware::MalwareCheckContext;
use crate::sync::store_credentials_from_target;

/// A tool declared in a lockfile, whose version and dependencies depend on the interpreter.
pub struct LockedTool<'lock> {
    target: InstallTarget<'lock>,
    project_name: Option<&'lock PackageName>,
    name: &'lock PackageName,
    group: &'lock GroupName,
}

impl<'lock> LockedTool<'lock> {
    /// Find a direct declaration in the preferred group or production dependencies.
    pub fn find(
        target: InstallTarget<'lock>,
        project_name: Option<&'lock PackageName>,
        name: &'lock PackageName,
        group: &'lock GroupName,
    ) -> Result<Option<Self>, EnvironmentError> {
        if !target
            .lock()
            .declares_dependency(project_name, name, group)
            .map_err(anyhow::Error::msg)?
        {
            return Ok(None);
        }
        Ok(Some(Self {
            target,
            project_name,
            name,
            group,
        }))
    }

    /// Use the project environment if synchronization will install the tool there.
    /// Otherwise install the tool's subgraph in a cached environment.
    ///
    /// Returns `None` when the declared dependency is inactive for the chosen interpreter.
    pub async fn environment(
        &self,
        project_environment: Option<(&PythonEnvironment, &DependencyGroupsWithDefaults)>,
        interpreter: &Interpreter,
        settings: &ResolverInstallerSettings,
        malware_context: &mut MalwareCheckContext<'_>,
        client_builder: &BaseClientBuilder<'_>,
        state: &UniversalState,
        installer_metadata: bool,
        concurrency: &Concurrency,
        cache: &Cache,
        printer: Printer,
        preview: Preview,
    ) -> Result<Option<PythonEnvironment>, EnvironmentError> {
        if let Some((environment, groups)) = project_environment {
            let installation = self
                .target
                .for_environment(environment.interpreter(), None)?;
            let selection = installation.dependency_selection(self.project_name, self.name)?;
            let included = if selection.group(self.group).is_some() {
                groups.contains(self.group)
            } else if selection.production().is_some() {
                groups.prod()
            } else {
                return Ok(None);
            };
            if included {
                return Ok(Some(environment.clone()));
            }
        }

        let interpreter = CachedEnvironment::base_interpreter(interpreter, cache)?;
        let installation = self.target.for_environment(&interpreter, None)?;
        let Some(resolution) = installation.resolve_tool(
            self.project_name,
            self.name,
            self.group,
            &settings.resolver.build_options,
        )?
        else {
            return Ok(None);
        };
        store_credentials_from_target(self.target, client_builder)?;
        let environment = CachedEnvironment::from_locked_resolution(
            &resolution,
            self.target.build_constraints(),
            &interpreter,
            settings,
            malware_context,
            client_builder,
            &state.fork(),
            Box::new(SummaryInstallLogger),
            installer_metadata,
            concurrency,
            cache,
            printer,
            preview,
        )
        .await?;
        Ok(Some(environment.into()))
    }
}
