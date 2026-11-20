use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_command_support::Printer;
use uv_configuration::Concurrency;
use uv_dispatch::UniversalState;
use uv_install_operations::loggers::SummaryInstallLogger;
use uv_lock::{DependencyScope, Installable};
use uv_normalize::PackageName;
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
    scope: DependencyScope<'lock>,
}

impl<'lock> LockedTool<'lock> {
    /// Find a direct declaration in the given scope.
    pub fn find(
        target: InstallTarget<'lock>,
        project_name: Option<&'lock PackageName>,
        name: &'lock PackageName,
        scope: DependencyScope<'lock>,
    ) -> Result<Option<Self>, EnvironmentError> {
        if !target
            .lock()
            .declares_dependency(project_name, name, scope)
            .map_err(anyhow::Error::msg)?
        {
            return Ok(None);
        }
        Ok(Some(Self {
            target,
            project_name,
            name,
            scope,
        }))
    }

    /// Install the tool's subgraph in a cached environment when its markers apply.
    pub async fn install(
        &self,
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
        let interpreter = CachedEnvironment::base_interpreter(interpreter, cache)?;
        let installation = self.target.for_environment(&interpreter, None)?;
        let Some(resolution) = installation.resolve_tool(
            self.project_name,
            self.name,
            self.scope,
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
