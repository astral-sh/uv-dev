use anyhow::{Context, Result};
use itertools::Itertools;
use owo_colors::OwoColorize;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::str::FromStr;
use tracing::{debug, trace};

use uv_cache::Cache;
use uv_cache_key::CanonicalUrl;
use uv_client::{BaseClient, BaseClientBuilder, ClientBuildError, RegistryClientBuilder};
use uv_configuration::{Concurrency, Constraints, DryRun, HashCheckingMode, TargetTriple};
use uv_distribution::LoweredExtraBuildDependencies;
use uv_distribution_types::{ExtraBuildRequires, Index, Name, Requirement, RequirementSource};
use uv_fs::{CWD, Simplified};
use uv_installer::{InstallationStrategy, Planner, SitePackages};
use uv_normalize::PackageName;
use uv_pep440::{Operator, Version};
use uv_pep508::MarkerEnvironment;
use uv_platform_tags::Platform;
use uv_preview::{Preview, PreviewFeature};
use uv_python::{
    EnvironmentPreference, Interpreter, PythonDownloads, PythonInstallation, PythonPreference,
    PythonRequest,
};
use uv_requirements::RequirementsSpecification;
use uv_settings::{Combine, PythonInstallMirrors, ResolverInstallerOptions, ToolOptions};
use uv_tool::{InstalledTools, Tool};
use uv_types::{HashStrategy, SourceTreeEditablePolicy};
use uv_workspace::WorkspaceCache;

use crate::commands::pip::loggers::{
    DefaultInstallLogger, SummaryResolveLogger, UpgradeInstallLogger,
};
use crate::commands::pip::{operations::Modifications, resolution_tags};
use crate::commands::project::{
    EnvironmentResolution, EnvironmentUpdate, PlatformState, resolve_environment, sync_environment,
    update_environment,
};
use crate::commands::reporters::PythonDownloadReporter;
use crate::commands::tool::common::{ToolLock, remove_entrypoints, tool_environment_spec};
use crate::commands::{ExitStatus, conjunction, tool::common::finalize_tool_install};
use crate::printer::Printer;
use crate::settings::ResolverInstallerSettings;

/// Upgrade a tool.
pub(crate) async fn upgrade(
    names: Vec<String>,
    python: Option<String>,
    python_platform: Option<TargetTriple>,
    install_mirrors: PythonInstallMirrors,
    args: ResolverInstallerOptions,
    filesystem: ResolverInstallerOptions,
    client_builder: BaseClientBuilder<'_>,
    python_preference: PythonPreference,
    python_downloads: PythonDownloads,
    installer_metadata: bool,
    concurrency: Concurrency,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    let installed_tools = InstalledTools::from_settings()?.init()?;
    let _lock = installed_tools.lock().await?;

    // Collect the tools to upgrade, along with any constraints.
    let names: BTreeMap<PackageName, Vec<Requirement>> = {
        if names.is_empty() {
            installed_tools
                .tools()
                .with_context(|| {
                    format!(
                        "Failed to inspect installed tools in `{}`",
                        installed_tools.root().user_display()
                    )
                })?
                .into_iter()
                .map(|(name, _)| (name, Vec::new()))
                .collect()
        } else {
            let mut map = BTreeMap::new();
            for name in names {
                let requirement = Requirement::from(uv_pep508::Requirement::parse(&name, &*CWD)?);
                map.entry(requirement.name.clone())
                    .or_insert_with(Vec::new)
                    .push(requirement);
            }
            map
        }
    };

    if names.is_empty() {
        writeln!(printer.stderr(), "Nothing to upgrade")?;
        return Ok(ExitStatus::Success);
    }

    let reporter = PythonDownloadReporter::single(printer);

    let python_request = python.as_deref().map(PythonRequest::parse);

    let interpreter = if python_request.is_some() {
        Some(
            PythonInstallation::find_or_download(
                python_request.as_ref(),
                EnvironmentPreference::OnlySystem,
                python_preference,
                python_downloads,
                &client_builder,
                cache,
                Some(&reporter),
                install_mirrors.python_install_mirror.as_deref(),
                install_mirrors.pypy_install_mirror.as_deref(),
                install_mirrors.python_downloads_json_url.as_deref(),
            )
            .await?
            .into_interpreter(),
        )
    } else {
        None
    };

    // Determine whether we applied any upgrades.
    let mut did_upgrade_tool = vec![];

    // Determine whether we applied any upgrades.
    let mut did_upgrade_environment = vec![];

    // Constraints that caused upgrades to be skipped or altered.
    let mut collected_constraints: Vec<(PackageName, UpgradeConstraint)> = Vec::new();

    let mut registry_clients = UpgradeRegistryClients::default();
    let mut errors = Vec::new();
    for (name, constraints) in &names {
        debug!("Upgrading tool: `{name}`");
        let result = Box::pin(upgrade_tool(
            name,
            constraints,
            interpreter.as_ref(),
            python_platform.as_ref(),
            printer,
            &installed_tools,
            &args,
            &client_builder,
            &mut registry_clients,
            cache,
            workspace_cache,
            &filesystem,
            installer_metadata,
            &concurrency,
            preview,
        ))
        .await;

        match result {
            Ok(report) => {
                match report.outcome {
                    UpgradeOutcome::UpgradeEnvironment => {
                        did_upgrade_environment.push(name);
                    }
                    UpgradeOutcome::UpgradeTool | UpgradeOutcome::UpgradeDependencies => {
                        did_upgrade_tool.push(name);
                    }
                    UpgradeOutcome::NoOp => {
                        debug!("Upgrading `{name}` was a no-op");
                    }
                }

                if let Some(constraint) = report.constraint.clone() {
                    collected_constraints.push((name.clone(), constraint));
                }
            }
            Err(err) => {
                errors.push((name, err));
            }
        }
    }

    if !errors.is_empty() {
        for (name, err) in errors
            .into_iter()
            .sorted_unstable_by(|(name_a, _), (name_b, _)| name_a.cmp(name_b))
        {
            trace!("Error trace: {err:?}");
            crate::commands::diagnostics::write_error_chain(
                &err.context(format!("Failed to upgrade {}", name.green())),
                printer,
            )?;
        }
        return Ok(ExitStatus::Failure);
    }

    if did_upgrade_tool.is_empty() && did_upgrade_environment.is_empty() {
        writeln!(printer.stderr(), "Nothing to upgrade")?;
    }

    if let Some(python_request) = python_request {
        if !did_upgrade_environment.is_empty() {
            let tools = did_upgrade_environment
                .iter()
                .map(|name| format!("`{}`", name.cyan()))
                .collect::<Vec<_>>();
            let s = if tools.len() > 1 { "s" } else { "" };
            writeln!(
                printer.stderr(),
                "Upgraded tool environment{s} for {} to {}",
                conjunction(tools),
                python_request.cyan(),
            )?;
        }
    }

    if !collected_constraints.is_empty() {
        writeln!(printer.stderr())?;
    }

    for (name, constraint) in collected_constraints {
        constraint.print(&name, printer)?;
    }

    Ok(ExitStatus::Success)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpgradeOutcome {
    /// The tool itself was upgraded.
    UpgradeTool,
    /// The tool's dependencies were upgraded, but the tool itself was unchanged.
    UpgradeDependencies,
    /// The tool's environment was upgraded.
    UpgradeEnvironment,
    /// The tool was already up-to-date.
    NoOp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UpgradeConstraint {
    /// The tool remains pinned to an exact version, so an upgrade was skipped.
    PinnedVersion { version: Version },
}

impl UpgradeConstraint {
    fn print(&self, name: &PackageName, printer: Printer) -> Result<()> {
        match self {
            Self::PinnedVersion { version } => {
                let name = name.to_string();
                let reinstall_command = format!("uv tool install {name}@latest");

                writeln!(
                    printer.stderr(),
                    "hint: `{}` is pinned to `{}` (installed with an exact version pin); reinstall with `{}` to upgrade to a new version.",
                    name.cyan(),
                    version.to_string().magenta(),
                    reinstall_command.green(),
                )?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpgradeReport {
    outcome: UpgradeOutcome,
    constraint: Option<UpgradeConstraint>,
}

#[derive(Default)]
struct UpgradeRegistryClients(Vec<UpgradeRegistryClient>);

struct UpgradeRegistryClient {
    markers: MarkerEnvironment,
    platform: Platform,
    client: BaseClient,
}

impl UpgradeRegistryClients {
    fn for_interpreter(
        &mut self,
        markers: &MarkerEnvironment,
        platform: &Platform,
        builder: RegistryClientBuilder<'_>,
    ) -> Result<BaseClient, ClientBuildError> {
        if let Some(client) = self
            .0
            .iter()
            .find(|client| client.markers == *markers && client.platform == *platform)
        {
            return Ok(client.client.clone());
        }

        let client = builder.build()?.cached_client().uncached().clone();
        self.0.push(UpgradeRegistryClient {
            markers: markers.clone(),
            platform: platform.clone(),
            client: client.clone(),
        });
        Ok(client)
    }
}

/// Upgrade a specific tool.
async fn upgrade_tool(
    name: &PackageName,
    constraints: &[Requirement],
    interpreter: Option<&Interpreter>,
    python_platform: Option<&TargetTriple>,
    printer: Printer,
    installed_tools: &InstalledTools,
    args: &ResolverInstallerOptions,
    client_builder: &BaseClientBuilder<'_>,
    registry_clients: &mut UpgradeRegistryClients,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    filesystem: &ResolverInstallerOptions,
    installer_metadata: bool,
    concurrency: &Concurrency,
    preview: Preview,
) -> Result<UpgradeReport> {
    let tool_locks = preview.is_enabled(PreviewFeature::ToolInstallLocks);
    // Ensure the tool is installed.
    let existing_tool_receipt = match installed_tools.get_tool_receipt(name) {
        Ok(Some(receipt)) => receipt,
        Ok(None) => {
            let install_command = format!("uv tool install {name}");
            return Err(anyhow::anyhow!(
                "`{}` is not installed; run `{}` to install",
                name.cyan(),
                install_command.green()
            ));
        }
        Err(_) => {
            let install_command = format!("uv tool install --force {name}");
            return Err(anyhow::anyhow!(
                "`{}` is missing a valid receipt; run `{}` to reinstall",
                name.cyan(),
                install_command.green()
            ));
        }
    };

    let environment = match installed_tools.get_environment(name, cache) {
        Ok(Some(environment)) => environment,
        Ok(None) => {
            let install_command = format!("uv tool install {name}");
            return Err(anyhow::anyhow!(
                "`{}` is not installed; run `{}` to install",
                name.cyan(),
                install_command.green()
            ));
        }
        Err(_) => {
            let install_command = format!("uv tool install --force {name}");
            return Err(anyhow::anyhow!(
                "`{}` is missing a valid environment; run `{}` to reinstall",
                name.cyan(),
                install_command.green()
            ));
        }
    };

    // Restore credentials from user configuration when the receipt refers to the same index.
    // Receipts intentionally omit credentials, including usernames needed for keyring lookups.
    let mut receipt = ResolverInstallerOptions::from(existing_tool_receipt.options().clone());
    if let (Some(stored), Some(configured)) = (
        receipt.indexes.index_url.as_ref(),
        filesystem.indexes.index_url.as_ref(),
    ) {
        let stored = Index::from(stored.clone());
        let configured = Index::from(configured.clone());

        if stored.raw_url().username().is_empty()
            && stored.raw_url().password().is_none()
            && (!configured.raw_url().username().is_empty()
                || configured.raw_url().password().is_some())
            && CanonicalUrl::new(stored.raw_url().clone())
                == CanonicalUrl::new(configured.raw_url().clone())
        {
            receipt.indexes.index_url = Some(configured.into());
        }
    }

    // Resolve the appropriate settings, preferring: CLI > receipt > user.
    let options = args.clone().combine(receipt.combine(filesystem.clone()));
    let settings = ResolverInstallerSettings::from(options.clone());

    let build_constraints = existing_tool_receipt.build_constraints().to_vec();
    let manifest_constraints = existing_tool_receipt
        .constraints()
        .iter()
        .chain(constraints)
        .cloned()
        .collect::<Vec<_>>();
    let manifest_overrides = existing_tool_receipt.overrides().to_vec();
    let manifest_excludes = existing_tool_receipt.excludes().to_vec();
    let lock_manifest = ToolLock::manifest(
        existing_tool_receipt.requirements(),
        &manifest_constraints,
        &manifest_overrides,
        &manifest_excludes,
        &build_constraints,
        &settings.resolver.dependency_metadata,
    );
    let build_constraints = Constraints::from_specifications(build_constraints);

    // Resolve the requirements.
    let spec = RequirementsSpecification::from_excludes(
        existing_tool_receipt.requirements().to_vec(),
        manifest_constraints,
        manifest_overrides,
        manifest_excludes,
    );
    // Initialize any shared state.
    let state = PlatformState::default();
    // Check if we need to create a new environment — if so, resolve it first, then install the
    // requested tool.
    let requested_interpreter =
        interpreter.filter(|interpreter| !environment.environment().uses(interpreter));
    // Transport settings are shared by the invocation, while interpreter identity is part of
    // the user agent. Each resolution still builds middleware for its own indexes and keyring.
    let target_interpreter =
        requested_interpreter.unwrap_or_else(|| environment.environment().interpreter());
    let transport = registry_clients.for_interpreter(
        target_interpreter.markers(),
        target_interpreter.platform(),
        RegistryClientBuilder::new(client_builder.clone(), cache.clone())
            .markers(target_interpreter.markers())
            .platform(target_interpreter.platform()),
    )?;
    let shared_client_builder = client_builder.clone().reuse_client(&transport);
    let client_builder = &shared_client_builder;
    let tool_dir = installed_tools.tool_dir(name);
    // TODO(zanieb): When updating an existing environment, build it in the cache directory then
    // copy it into the tool directory.
    let (environment, outcome, tool_lock) = if tool_locks {
        let target_interpreter =
            requested_interpreter.unwrap_or_else(|| environment.environment().interpreter());
        let site_packages = SitePackages::from_environment(environment.environment())?;
        let universal_resolution = resolve_environment(
            tool_environment_spec(spec, None, Some(&site_packages)),
            EnvironmentResolution::Universal,
            target_interpreter,
            python_platform,
            SourceTreeEditablePolicy::Tool,
            build_constraints.clone(),
            &settings.resolver,
            client_builder,
            &state,
            Box::new(SummaryResolveLogger),
            concurrency,
            cache,
            workspace_cache,
            printer,
            preview,
        )
        .await?;
        let tool_lock = ToolLock::from_resolution(
            &tool_dir,
            &universal_resolution,
            &lock_manifest,
            &settings.resolver.index_locations,
        )?;
        let resolution = tool_lock.to_resolution(
            Some(name),
            target_interpreter,
            python_platform,
            &settings.resolver.build_options,
        )?;
        let hash_strategy = HashStrategy::from_resolution(&resolution, HashCheckingMode::Verify)?;

        if requested_interpreter.is_some() {
            let environment =
                installed_tools.create_environment(name, target_interpreter.clone())?;
            let environment = sync_environment(
                environment,
                &resolution,
                hash_strategy,
                Modifications::Exact,
                build_constraints,
                (&settings).into(),
                client_builder,
                &state,
                Box::new(DefaultInstallLogger),
                installer_metadata,
                concurrency,
                cache,
                printer,
                preview,
            )
            .await?;
            (
                environment,
                UpgradeOutcome::UpgradeEnvironment,
                Some(tool_lock),
            )
        } else {
            // Otherwise, upgrade the existing environment.
            let ResolverInstallerSettings {
                resolver:
                    crate::settings::ResolverSettings {
                        config_setting,
                        config_settings_package,
                        extra_build_dependencies,
                        extra_build_variables,
                        ..
                    },
                ..
            } = &settings;
            let extra_build_requires =
                LoweredExtraBuildDependencies::from_non_lowered(extra_build_dependencies.clone())
                    .into_inner();
            let tags = resolution_tags(
                None,
                python_platform,
                environment.environment().interpreter(),
            )?;
            let plan = Planner::new(&resolution).build(
                site_packages,
                InstallationStrategy::Permissive,
                &settings.reinstall,
                &settings.resolver.build_options,
                &hash_strategy,
                &settings.resolver.index_locations,
                config_setting,
                config_settings_package,
                &extra_build_requires,
                extra_build_variables,
                cache,
                environment.environment(),
                &tags,
            )?;
            let plan_is_empty = plan.is_empty();
            let changes_tool = plan.cached.iter().any(|dist| dist.name() == name)
                || plan.remote.iter().any(|dist| dist.name() == name)
                || plan.reinstalls.iter().any(|dist| dist.name() == name)
                || plan.extraneous.iter().any(|dist| dist.name() == name);
            let outcome = if plan_is_empty {
                UpgradeOutcome::NoOp
            } else if changes_tool {
                UpgradeOutcome::UpgradeTool
            } else {
                UpgradeOutcome::UpgradeDependencies
            };
            let environment = if plan_is_empty && !settings.compile_bytecode {
                environment.into_environment()
            } else {
                sync_environment(
                    environment.into_environment(),
                    &resolution,
                    hash_strategy,
                    Modifications::Exact,
                    build_constraints,
                    (&settings).into(),
                    client_builder,
                    &state,
                    Box::new(UpgradeInstallLogger::new(name.clone())),
                    installer_metadata,
                    concurrency,
                    cache,
                    printer,
                    preview,
                )
                .await?
            };
            (environment, outcome, Some(tool_lock))
        }
    } else if let Some(interpreter) = requested_interpreter {
        let resolution = resolve_environment(
            spec.into(),
            EnvironmentResolution::Specific,
            interpreter,
            python_platform,
            SourceTreeEditablePolicy::Tool,
            build_constraints.clone(),
            &settings.resolver,
            client_builder,
            &state,
            Box::new(SummaryResolveLogger),
            concurrency,
            cache,
            workspace_cache,
            printer,
            preview,
        )
        .await?;
        let environment = installed_tools.create_environment(name, interpreter.clone())?;
        let environment = sync_environment(
            environment,
            &resolution.into(),
            HashStrategy::default(),
            Modifications::Exact,
            build_constraints,
            (&settings).into(),
            client_builder,
            &state,
            Box::new(DefaultInstallLogger),
            installer_metadata,
            concurrency,
            cache,
            printer,
            preview,
        )
        .await?;
        (environment, UpgradeOutcome::UpgradeEnvironment, None)
    } else {
        // Otherwise, upgrade the existing environment.
        let EnvironmentUpdate {
            environment,
            changelog,
        } = update_environment(
            environment.into_environment(),
            spec,
            Modifications::Exact,
            python_platform,
            SourceTreeEditablePolicy::Tool,
            build_constraints,
            ExtraBuildRequires::default(),
            &settings,
            client_builder,
            &state,
            Box::new(SummaryResolveLogger),
            Box::new(UpgradeInstallLogger::new(name.clone())),
            installer_metadata,
            concurrency,
            cache,
            workspace_cache,
            DryRun::Disabled,
            printer,
            preview,
        )
        .await?;

        let outcome = if changelog.includes(name) {
            UpgradeOutcome::UpgradeTool
        } else if changelog.is_empty() {
            UpgradeOutcome::NoOp
        } else {
            UpgradeOutcome::UpgradeDependencies
        };

        (environment, outcome, None)
    };

    if matches!(
        outcome,
        UpgradeOutcome::UpgradeEnvironment | UpgradeOutcome::UpgradeTool
    ) {
        // At this point, we updated the existing environment, so we should remove any of its
        // existing executables.
        remove_entrypoints(&existing_tool_receipt);

        let entrypoints: Vec<_> = existing_tool_receipt
            .entrypoints()
            .iter()
            .filter_map(|entry| PackageName::from_str(entry.from.as_ref()?).ok())
            .collect();

        // If we modified the target tool, reinstall the entrypoints.
        finalize_tool_install(
            &environment,
            name,
            &entrypoints,
            installed_tools,
            &ToolOptions::from(options),
            true,
            existing_tool_receipt.python().to_owned(),
            existing_tool_receipt.requirements().to_vec(),
            existing_tool_receipt.constraints().to_vec(),
            existing_tool_receipt.overrides().to_vec(),
            existing_tool_receipt.excludes().to_vec(),
            existing_tool_receipt.build_constraints().to_vec(),
            tool_lock.as_ref(),
            printer,
        )?;
    } else if tool_locks {
        ToolLock::write(&tool_dir, tool_lock.as_ref())?;
        installed_tools.add_tool_receipt(
            name,
            existing_tool_receipt
                .clone()
                .with_options(ToolOptions::from(options)),
        )?;
    }

    let constraint = match &outcome {
        UpgradeOutcome::UpgradeDependencies | UpgradeOutcome::NoOp => {
            pinned_requirement_version(&existing_tool_receipt, name)
                .map(|version| UpgradeConstraint::PinnedVersion { version })
        }
        UpgradeOutcome::UpgradeTool | UpgradeOutcome::UpgradeEnvironment => None,
    };

    Ok(UpgradeReport {
        outcome,
        constraint,
    })
}

fn pinned_requirement_version(tool: &Tool, name: &PackageName) -> Option<Version> {
    pinned_version_from(tool.requirements(), name)
        .or_else(|| pinned_version_from(tool.constraints(), name))
}

fn pinned_version_from(requirements: &[Requirement], name: &PackageName) -> Option<Version> {
    requirements
        .iter()
        .filter(|requirement| requirement.name == *name)
        .find_map(|requirement| match &requirement.source {
            RequirementSource::Registry { specifier, .. } => {
                specifier
                    .iter()
                    .find_map(|specifier| match specifier.operator() {
                        Operator::Equal | Operator::ExactEqual => Some(specifier.version().clone()),
                        _ => None,
                    })
            }
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use anyhow::Result;
    use bytes::Bytes;
    use http::header::USER_AGENT;
    use http_body_util::Full;
    use hyper::body::Incoming;
    use hyper::service::service_fn;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use uv_cache::Cache;
    use uv_client::{BaseClientBuilder, RegistryClient, RegistryClientBuilder};
    use uv_pep440::Version;
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};
    use uv_platform_tags::{Arch, Os, Platform};
    use uv_redacted::DisplaySafeUrl;

    use super::UpgradeRegistryClients;

    async fn observe(client: &RegistryClient, url: &DisplaySafeUrl) -> Result<Value> {
        Ok(client
            .uncached_client(url)
            .get(url.as_str())
            .send()
            .await?
            .json()
            .await?)
    }

    #[tokio::test]
    async fn tool_upgrade_connections_follow_interpreter_identity() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = DisplaySafeUrl::parse(&format!("http://{}", listener.local_addr()?))?;
        let server = tokio::spawn(async move {
            let mut connection = 0;
            while let Ok((stream, _)) = listener.accept().await {
                connection += 1;
                tokio::spawn(async move {
                    let service = service_fn(move |request: hyper::Request<Incoming>| async move {
                        let user_agent = request
                            .headers()
                            .get(USER_AGENT)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default();
                        let body = json!({"connection": connection, "user_agent": user_agent});
                        Ok::<_, Infallible>(hyper::Response::new(Full::new(Bytes::from(
                            body.to_string(),
                        ))))
                    });
                    let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });

        let markers = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: "3.12.0",
            os_name: "posix",
            platform_machine: "x86_64",
            platform_python_implementation: "CPython",
            platform_release: "6.0.0",
            platform_system: "Linux",
            platform_version: "6.0.0",
            python_full_version: "3.12.0",
            python_version: "3.12",
            sys_platform: "linux",
        })?;
        let other_markers = markers
            .clone()
            .with_python_full_version(Version::new([3, 12, 1]));
        let platform = Platform::new(
            Os::Manylinux {
                major: 2,
                minor: 17,
            },
            Arch::X86_64,
        );
        let other_platform = Platform::new(
            Os::Manylinux {
                major: 2,
                minor: 17,
            },
            Arch::Aarch64,
        );
        let cache = Cache::temp()?;
        let mut clients = UpgradeRegistryClients::default();
        let mut observations = Vec::new();
        for (markers, platform) in [
            (&markers, &platform),
            (&markers, &platform),
            (&other_markers, &platform),
            (&markers, &other_platform),
            (&markers, &platform),
        ] {
            let transport = clients.for_interpreter(
                markers,
                platform,
                RegistryClientBuilder::new(BaseClientBuilder::default(), cache.clone())
                    .markers(markers)
                    .platform(platform),
            )?;
            let client = RegistryClientBuilder::new(
                BaseClientBuilder::default().reuse_client(&transport),
                cache.clone(),
            )
            .markers(markers)
            .platform(platform)
            .build()?;
            observations.push(observe(&client, &url).await?);
        }
        let separate = RegistryClientBuilder::new(BaseClientBuilder::default(), cache)
            .markers(&markers)
            .platform(&other_platform)
            .build()?;
        let separate = observe(&separate, &url).await?;
        server.abort();

        assert_eq!(observations[0], observations[1]);
        assert_eq!(observations[0], observations[4]);
        assert_ne!(observations[0]["connection"], observations[2]["connection"]);
        assert_ne!(observations[0]["user_agent"], observations[2]["user_agent"]);
        assert_ne!(observations[0]["connection"], observations[3]["connection"]);
        assert_eq!(observations[3]["user_agent"], separate["user_agent"]);
        Ok(())
    }
}
