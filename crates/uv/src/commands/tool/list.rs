use std::fmt::Write;

use anyhow::Result;
use futures::StreamExt;
use itertools::Itertools;
use owo_colors::OwoColorize;
use rustc_hash::FxHashMap;

use uv_cache::{Cache, Refresh};
use uv_cache_info::Timestamp;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_configuration::Concurrency;
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{IndexCapabilities, RequiresPython};
use uv_fs::Simplified;
use uv_normalize::PackageName;
use uv_python::LenientImplementationName;
use uv_settings::{Combine, ResolverInstallerOptions};
use uv_tool::InstalledTools;
use uv_warnings::warn_user;

use crate::commands::ExitStatus;
use crate::commands::pip::latest::LatestClient;
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

/// List installed tools.
pub(crate) async fn list(
    output: ToolListOutput,
    mode: ToolListMode,
    args: ResolverInstallerOptions,
    filesystem: ResolverInstallerOptions,
    client_builder: BaseClientBuilder<'_>,
    concurrency: Concurrency,
    cache: &Cache,
    printer: Printer,
) -> Result<ExitStatus> {
    let installed_tools = InstalledTools::from_settings()?;
    let _lock = match installed_tools.lock().await {
        Ok(lock) => lock,
        Err(err)
            if err
                .as_io_error()
                .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound) =>
        {
            writeln!(printer.stderr(), "No tools installed")?;
            return Ok(ExitStatus::Success);
        }
        Err(err) => return Err(err.into()),
    };

    let mut tools = installed_tools.tools()?.into_iter().collect::<Vec<_>>();
    tools.sort_by_key(|(name, _)| name.clone());

    if tools.is_empty() {
        writeln!(printer.stderr(), "No tools installed")?;
        return Ok(ExitStatus::Success);
    }

    // Collect valid tools (skip invalid ones) before checking for outdated versions.
    let mut valid_tools = Vec::new();
    for (name, tool) in tools {
        // Skip invalid tools
        let Ok(tool) = tool else {
            warn_user!(
                "Ignoring malformed tool `{name}` (run `{}` to remove)",
                format!("uv tool uninstall {name}").green()
            );
            continue;
        };

        // Get the tool environment
        let tool_env = match installed_tools.get_environment(&name, cache) {
            Ok(Some(env)) => env,
            Ok(None) => {
                warn_user!(
                    "Tool `{name}` environment not found (run `{}` to reinstall)",
                    format!("uv tool install {name} --reinstall").green()
                );
                continue;
            }
            Err(e) => {
                warn_user!(
                    "{e} (run `{}` to reinstall)",
                    format!("uv tool install {name} --reinstall").green()
                );
                continue;
            }
        };

        // Get the tool version
        let version = match tool_env.version() {
            Ok(version) => version,
            Err(e) => {
                if let uv_tool::Error::EnvironmentError(e) = e {
                    warn_user!(
                        "{e} (run `{}` to reinstall)",
                        format!("uv tool install {name} --reinstall").green()
                    );
                } else {
                    writeln!(printer.stderr(), "{e}")?;
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

        // Fetch the latest version for each tool.
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
                        prerelease: settings.resolver.prerelease,
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

    for (name, tool, tool_env, version) in valid_tools {
        // If `--outdated` is set, skip tools that are up-to-date.
        if mode == ToolListMode::Outdated {
            let is_outdated = latest
                .get(&name)
                .and_then(Option::as_ref)
                .is_some_and(|filename| filename.version() > &version);
            if !is_outdated {
                continue;
            }
        }

        let version_specifier = output
            .contains(ToolListOutput::VERSION_SPECIFIERS)
            .then(|| {
                tool.requirements()
                    .iter()
                    .filter(|req| req.name == name)
                    .map(|req| req.source.to_string())
                    .filter(|s| !s.is_empty())
                    .peekable()
            })
            .take_if(|specifiers| specifiers.peek().is_some())
            .map(|mut specifiers| {
                let specifiers = specifiers.join(", ");
                format!(" [required: {specifiers}]")
            })
            .unwrap_or_default();

        let extra_requirements = output
            .contains(ToolListOutput::EXTRAS)
            .then(|| {
                tool.requirements()
                    .iter()
                    .filter(|req| req.name == name)
                    .flat_map(|req| req.extras.iter()) // Flatten the extras from all matching requirements
                    .peekable()
            })
            .take_if(|extras| extras.peek().is_some())
            .map(|extras| {
                let extras_str = extras.map(ToString::to_string).join(", ");
                format!(" [extras: {extras_str}]")
            })
            .unwrap_or_default();

        let python_version = if output.contains(ToolListOutput::PYTHON) {
            let interpreter = tool_env.environment().interpreter();
            let implementation = LenientImplementationName::from(interpreter.implementation_name());
            format!(
                " [{} {}]",
                implementation.pretty(),
                interpreter.python_full_version()
            )
        } else {
            String::new()
        };

        let with_requirements = output
            .contains(ToolListOutput::WITH)
            .then(|| {
                tool.requirements()
                    .iter()
                    .filter(|req| req.name != name)
                    .peekable()
            })
            .take_if(|requirements| requirements.peek().is_some())
            .map(|requirements| {
                let requirements = requirements
                    .map(|req| format!("{}{}", req.name, req.source))
                    .join(", ");
                format!(" [with: {requirements}]")
            })
            .unwrap_or_default();

        let latest_version = if mode == ToolListMode::Outdated {
            latest
                .get(&name)
                .and_then(Option::as_ref)
                .map(|filename| format!(" [latest: {}]", filename.version()))
                .unwrap_or_default()
        } else {
            String::new()
        };

        if output.contains(ToolListOutput::PATHS) {
            writeln!(
                printer.stdout(),
                "{} ({})",
                format!(
                    "{name} v{version}{version_specifier}{extra_requirements}{with_requirements}{python_version}{latest_version}"
                )
                .bold(),
                installed_tools.tool_dir(&name).simplified_display().cyan(),
            )?;
        } else {
            writeln!(
                printer.stdout(),
                "{}",
                format!(
                    "{name} v{version}{version_specifier}{extra_requirements}{with_requirements}{python_version}{latest_version}"
                )
                .bold()
            )?;
        }

        // Output tool entrypoints
        for entrypoint in tool.entrypoints() {
            if output.contains(ToolListOutput::PATHS) {
                writeln!(printer.stdout(), "- {}", entrypoint.to_string().cyan())?;
            } else {
                writeln!(printer.stdout(), "- {}", entrypoint.name)?;
            }
        }
    }

    Ok(ExitStatus::Success)
}
