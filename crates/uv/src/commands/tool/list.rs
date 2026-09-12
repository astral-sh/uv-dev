use std::fmt::{self, Write};

use anyhow::Result;
use futures::StreamExt;
use itertools::Itertools;
use owo_colors::OwoColorize;
use rustc_hash::FxHashMap;
use serde::Serialize;

use uv_cache::{Cache, Refresh};
use uv_cache_info::Timestamp;
use uv_cli::ToolListFormat;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_configuration::Concurrency;
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{IndexCapabilities, RequiresPython};
use uv_fs::{PortablePathBuf, Simplified};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_preview::{Preview, PreviewFeature};
use uv_settings::{Combine, ResolverInstallerOptions};
use uv_tool::InstalledTools;
use uv_warnings::warn_user;

use crate::commands::ExitStatus;
use crate::commands::pip::latest::LatestClient;
use crate::commands::report::{EnvironmentReport, SchemaReport};
use crate::commands::reporters::LatestVersionReporter;
use crate::printer::Printer;
use crate::settings::ResolverInstallerSettings;

/// Whether to list all installed tools or only those with available updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolListMode {
    /// List all installed tools.
    All,
    /// List only tools with available updates.
    Outdated,
}

impl From<bool> for ToolListMode {
    fn from(outdated: bool) -> Self {
        if outdated { Self::Outdated } else { Self::All }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub(crate) struct ToolListOutput: u8 {
        const PATHS = 1 << 0;
        const VERSION_SPECIFIERS = 1 << 1;
        const WITH = 1 << 2;
        const EXTRAS = 1 << 3;
        const PYTHON = 1 << 4;
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct CommandReport {
    /// The installed command name.
    name: String,
    /// Absolute path to the installed command.
    path: PortablePathBuf,
}

impl fmt::Display for CommandReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        cfg_select! {
            windows => {
                write!(
                    formatter,
                    "{} ({})",
                    self.name,
                    self.path.as_ref().simplified_display().to_string().replace('/', "\\")
                )
            },
            unix => {
                write!(formatter, "{} ({})", self.name, self.path.as_ref().simplified_display())
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct ToolReport {
    /// Normalized name of the installed tool.
    name: PackageName,
    /// Installed version of the tool.
    #[cfg_attr(feature = "schemars", schemars(with = "String"))]
    version: Version,
    /// Latest available version when `--outdated` is requested, or null otherwise.
    #[cfg_attr(feature = "schemars", schemars(with = "Option<String>"))]
    latest_version: Option<Version>,
    #[serde(flatten)]
    environment: EnvironmentReport,
    /// Commands installed from the tool environment.
    commands: Vec<CommandReport>,
    /// Extras requested for the primary tool package.
    extras: Vec<String>,
    /// Recorded version or source constraints for the primary tool.
    version_specifiers: String,
    /// Additional recorded installation requirements, formatted for display.
    with: Vec<String>,
}

/// The preview `uv tool list` JSON report.
#[derive(Debug, Default, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(title = "uv tool list (preview)"))]
struct ToolListReport {
    /// Format information.
    schema: SchemaReport,
    /// Installed tools, sorted by normalized name.
    tools: Vec<ToolReport>,
}

/// Generate the preview `uv tool list` output schema for repository development tools.
#[cfg(feature = "schemars")]
pub fn json_schema() -> schemars::Schema {
    schemars::generate::SchemaSettings::draft07()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<ToolListReport>()
}

impl ToolListReport {
    fn render(
        &self,
        format: ToolListFormat,
        output: ToolListOutput,
        printer: Printer,
    ) -> Result<()> {
        match format {
            ToolListFormat::Text => self.render_text(output, printer),
            ToolListFormat::Json => {
                writeln!(
                    printer.stdout_important(),
                    "{}",
                    serde_json::to_string_pretty(self)?
                )?;
                Ok(())
            }
        }
    }

    fn render_text(&self, output: ToolListOutput, printer: Printer) -> Result<()> {
        for tool in &self.tools {
            let name = &tool.name;
            let version = &tool.version;

            let version_specifier = if output.contains(ToolListOutput::VERSION_SPECIFIERS)
                && !tool.version_specifiers.is_empty()
            {
                format!(" [required: {}]", tool.version_specifiers)
            } else {
                String::new()
            };

            let extra_requirements =
                if output.contains(ToolListOutput::EXTRAS) && !tool.extras.is_empty() {
                    format!(" [extras: {}]", tool.extras.join(", "))
                } else {
                    String::new()
                };

            let python_version = if output.contains(ToolListOutput::PYTHON) {
                let python = tool.environment.python();
                format!(
                    " [{} {}]",
                    python.implementation().pretty(),
                    python.version()
                )
            } else {
                String::new()
            };

            let with_requirements =
                if output.contains(ToolListOutput::WITH) && !tool.with.is_empty() {
                    format!(" [with: {}]", tool.with.join(", "))
                } else {
                    String::new()
                };

            let latest_version = tool
                .latest_version
                .as_ref()
                .map(|version| format!(" [latest: {version}]"))
                .unwrap_or_default();

            let heading = format!(
                "{name} v{version}{version_specifier}{extra_requirements}{with_requirements}{python_version}{latest_version}"
            );
            if output.contains(ToolListOutput::PATHS) {
                writeln!(
                    printer.stdout(),
                    "{} ({})",
                    heading.bold(),
                    tool.environment.path().simplified_display().cyan(),
                )?;
            } else {
                writeln!(printer.stdout(), "{}", heading.bold())?;
            }

            for command in &tool.commands {
                if output.contains(ToolListOutput::PATHS) {
                    writeln!(printer.stdout(), "- {}", command.to_string().cyan())?;
                } else {
                    writeln!(printer.stdout(), "- {}", command.name)?;
                }
            }
        }
        Ok(())
    }
}

/// List installed tools.
pub(crate) async fn list(
    output: ToolListOutput,
    mode: ToolListMode,
    output_format: ToolListFormat,
    args: ResolverInstallerOptions,
    filesystem: ResolverInstallerOptions,
    client_builder: BaseClientBuilder<'_>,
    concurrency: Concurrency,
    cache: &Cache,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    if output_format == ToolListFormat::Json && !preview.is_enabled(PreviewFeature::JsonOutput) {
        warn_user!(
            "The `--output-format json` option is experimental and the schema may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::JsonOutput
        );
    }

    let installed_tools = InstalledTools::from_settings()?;
    let _lock = match installed_tools.lock().await {
        Ok(lock) => lock,
        Err(err)
            if err
                .as_io_error()
                .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound) =>
        {
            return render_no_tools(output_format, printer);
        }
        Err(err) => return Err(err.into()),
    };

    let mut tools = installed_tools.tools()?.into_iter().collect::<Vec<_>>();
    tools.sort_by_key(|(name, _)| name.clone());

    if tools.is_empty() {
        return render_no_tools(output_format, printer);
    }

    // Collect valid tools before checking for outdated versions.
    let mut valid_tools = Vec::new();
    for (name, tool) in tools {
        let Ok(tool) = tool else {
            warn_user!(
                "Ignoring malformed tool `{name}` (run `{}` to remove)",
                format!("uv tool uninstall {name}").green()
            );
            continue;
        };

        let tool_env = match installed_tools.get_environment(&name, cache) {
            Ok(Some(env)) => env,
            Ok(None) => {
                warn_user!(
                    "Tool `{name}` environment not found (run `{}` to reinstall)",
                    format!("uv tool install {name} --reinstall").green()
                );
                continue;
            }
            Err(error) => {
                warn_user!(
                    "{error} (run `{}` to reinstall)",
                    format!("uv tool install {name} --reinstall").green()
                );
                continue;
            }
        };

        let version = match tool_env.version() {
            Ok(version) => version,
            Err(error) => {
                if let uv_tool::Error::EnvironmentError(error) = error {
                    warn_user!(
                        "{error} (run `{}` to reinstall)",
                        format!("uv tool install {name} --reinstall").green()
                    );
                } else {
                    writeln!(printer.stderr(), "{error}")?;
                }
                continue;
            }
        };

        valid_tools.push((name, tool, tool_env, version));
    }

    // Determine the latest version for each tool when `--outdated` is requested.
    let latest: FxHashMap<PackageName, Option<DistFilename>> = if mode == ToolListMode::Outdated
        && !valid_tools.is_empty()
    {
        let download_concurrency = concurrency.downloads_semaphore.clone();

        let reporter = LatestVersionReporter::from(printer).with_length(valid_tools.len() as u64);

        let mut fetches = futures::stream::iter(&valid_tools)
            .map(|(name, tool, tool_env, _version)| {
                let client_builder = client_builder.clone();
                let download_concurrency = download_concurrency.clone();
                let args = args.clone();
                let filesystem = filesystem.clone();
                async move {
                    let capabilities = IndexCapabilities::default();
                    let settings = ResolverInstallerSettings::from(args.combine(
                        ResolverInstallerOptions::from(tool.options().clone()).combine(filesystem),
                    ));
                    let interpreter = tool_env.environment().interpreter();

                    let client = RegistryClientBuilder::new(
                        client_builder
                            .clone()
                            .keyring(settings.resolver.keyring_provider),
                        cache.clone().with_refresh(Refresh::All(Timestamp::now())),
                    )
                    .index_locations(settings.resolver.index_locations.clone())
                    .index_strategy(settings.resolver.index_strategy)
                    .markers(interpreter.markers())
                    .platform(interpreter.platform())
                    .build()?;

                    let requires_python = RequiresPython::greater_than_equal_version(
                        interpreter.python_full_version(),
                    );
                    let latest_client = LatestClient {
                        client: &client,
                        capabilities: &capabilities,
                        prerelease: &settings.resolver.prerelease,
                        exclude_newer: &settings.resolver.exclude_newer,
                        index_locations: &settings.resolver.index_locations,
                        tags: None,
                        requires_python: Some(&requires_python),
                    };

                    let latest = latest_client
                        .find_latest(name, None, &download_concurrency)
                        .await?;
                    Ok::<(&PackageName, Option<DistFilename>), anyhow::Error>((name, latest))
                }
            })
            .buffer_unordered(concurrency.downloads);

        let mut map = FxHashMap::default();
        while let Some((name, version)) = fetches.next().await.transpose()? {
            if let Some(version) = version.as_ref() {
                reporter.on_fetch_version(name, version.version());
            } else {
                reporter.on_fetch_progress();
            }
            map.insert(name.clone(), version);
        }
        reporter.on_fetch_complete();
        map
    } else {
        FxHashMap::default()
    };

    let tools = valid_tools
        .into_iter()
        .filter_map(|(name, tool, tool_env, version)| {
            let latest_version = latest
                .get(&name)
                .and_then(Option::as_ref)
                .map(|filename| filename.version().clone());
            if mode == ToolListMode::Outdated
                && latest_version
                    .as_ref()
                    .is_none_or(|latest_version| latest_version <= &version)
            {
                return None;
            }

            let commands = tool
                .entrypoints()
                .iter()
                .map(|entrypoint| CommandReport {
                    name: entrypoint.name.clone(),
                    path: entrypoint.install_path.as_path().into(),
                })
                .collect();
            let extras = tool
                .requirements()
                .iter()
                .filter(|requirement| requirement.name == name)
                .flat_map(|requirement| requirement.extras.iter())
                .map(ToString::to_string)
                .collect();
            let version_specifiers = tool
                .requirements()
                .iter()
                .filter(|requirement| requirement.name == name)
                .map(|requirement| requirement.source.to_string())
                .filter(|specifier| !specifier.is_empty())
                .join(", ");
            let with = tool
                .requirements()
                .iter()
                .filter(|requirement| requirement.name != name)
                .map(|requirement| format!("{}{}", requirement.name, requirement.source))
                .collect();
            let environment = EnvironmentReport::from(tool_env.environment())
                .with_path(installed_tools.tool_dir(&name).as_path().into());

            Some(ToolReport {
                name,
                version,
                latest_version,
                environment,
                commands,
                extras,
                version_specifiers,
                with,
            })
        })
        .collect();

    let report = ToolListReport {
        schema: SchemaReport::default(),
        tools,
    };
    report.render(output_format, output, printer)?;
    Ok(ExitStatus::Success)
}

fn render_no_tools(format: ToolListFormat, printer: Printer) -> Result<ExitStatus> {
    match format {
        ToolListFormat::Text => writeln!(printer.stderr(), "No tools installed")?,
        ToolListFormat::Json => {
            ToolListReport::default().render(format, ToolListOutput::empty(), printer)?;
        }
    }
    Ok(ExitStatus::Success)
}
