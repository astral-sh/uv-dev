use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_command_support::Printer;
use uv_configuration::{BuildOptions, Concurrency, HashCheckingMode};
use uv_dispatch::UniversalState;
use uv_distribution_types::Resolution;
use uv_install_operations::loggers::SummaryInstallLogger;
use uv_lock::{DependencyScope, Installable};
use uv_normalize::PackageName;
use uv_preview::Preview;
use uv_python_interpreter::{Interpreter, PythonEnvironment};
use uv_settings::ResolverInstallerSettings;
use uv_types::HashStrategy;

use crate::EnvironmentError;
use crate::environment::{CachedEnvironment, LockedInputs};
use crate::install_target::InstallTarget;
use crate::malware::MalwareCheckContext;
use crate::sync::{lower_build_dependencies, store_credentials_from_target};
use uv_workspace::WorkspaceCache;

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

    /// Select the dependency and resolve its subgraph for the base interpreter
    /// that will create the cached environment.
    fn prepare(
        self,
        interpreter: &Interpreter,
        build_options: &BuildOptions,
        cache: &Cache,
    ) -> Result<Option<PreparedTool<'lock>>, EnvironmentError> {
        let interpreter = CachedEnvironment::base_interpreter(interpreter, cache)?;
        let installation = self.target.for_environment(&interpreter, None)?;
        let Some(resolution) =
            installation.resolve_tool(self.project_name, self.name, self.scope, build_options)?
        else {
            return Ok(None);
        };
        Ok(Some(PreparedTool {
            target: self.target,
            resolution,
            interpreter,
        }))
    }
}

/// A tool's installable subgraph paired with the base interpreter it was resolved for.
struct PreparedTool<'lock> {
    target: InstallTarget<'lock>,
    resolution: Resolution,
    interpreter: Interpreter,
}

/// Installs a locked tool and its dependencies into a cached environment.
pub struct LockedToolInstaller<'a> {
    pub settings: &'a ResolverInstallerSettings,
    pub client_builder: &'a BaseClientBuilder<'a>,
    pub state: &'a UniversalState,
    pub installer_metadata: bool,
    pub concurrency: &'a Concurrency,
    pub cache: &'a Cache,
    pub printer: Printer,
    pub preview: Preview,
}

impl LockedToolInstaller<'_> {
    /// Install the tool in a cached environment if its direct edge applies to the base
    /// interpreter. Preparation and installation use the same build options.
    pub async fn install(
        &self,
        tool: LockedTool<'_>,
        interpreter: &Interpreter,
        malware_context: &mut MalwareCheckContext<'_>,
    ) -> Result<Option<PythonEnvironment>, EnvironmentError> {
        let Some(tool) = tool.prepare(
            interpreter,
            &self.settings.resolver.build_options,
            self.cache,
        )?
        else {
            return Ok(None);
        };
        store_credentials_from_target(tool.target, self.client_builder)?;
        let client = self
            .client_builder
            .clone()
            .keyring(self.settings.resolver.keyring_provider);
        crate::malware::check_resolution_malware(
            &tool.resolution,
            &client,
            self.concurrency,
            malware_context,
            self.cache,
            self.preview,
        )
        .await?;
        let hash_strategy =
            HashStrategy::from_resolution(&tool.resolution, HashCheckingMode::Verify)?;
        let build_requires = lower_build_dependencies(
            tool.target,
            &self.settings.into(),
            self.cache,
            &WorkspaceCache::default(),
            self.client_builder,
        )
        .await?
        .match_runtime(&tool.resolution)?;
        let environment = CachedEnvironment::from_resolution(
            &tool.resolution,
            Some(LockedInputs {
                hashes: hash_strategy,
                build_requires,
            }),
            tool.target.build_constraints(),
            &tool.interpreter,
            self.settings,
            self.client_builder,
            &self.state.fork(),
            Box::new(SummaryInstallLogger),
            self.installer_metadata,
            self.concurrency,
            self.cache,
            self.printer,
            self.preview,
        )
        .await?;
        Ok(Some(environment.into()))
    }
}
