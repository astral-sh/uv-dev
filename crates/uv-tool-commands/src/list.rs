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
use uv_python_types::LenientImplementationName;
use uv_settings::{Combine, ResolverInstallerOptions};
use uv_tool::InstalledTools;
use uv_warnings::warn_user;

use uv_command_support::ExitStatus;
use uv_command_support::Printer;
use uv_resolve_operations::latest::LatestClient;
use uv_resolve_operations::reporters::LatestVersionReporter;
use uv_settings::ResolverInstallerSettings;

/// List installed tools.
#[expect(clippy::fn_params_excessive_bools)]
pub async fn list(
    show_paths: bool,
    show_version_specifiers: bool,
    show_with: bool,
    show_extras: bool,
    show_python: bool,
    outdated: bool,
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

    let mut tools = installed_tools.tools()?;
    tools.sort_by(|(left, _), (right, _)| left.cmp(right));

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
    let latest: FxHashMap<PackageName, DistFilename> = if outdated && !valid_tools.is_empty() {
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
                        client_builder.keyring(settings.resolver.keyring_provider),
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
            if let Some(version) = version {
                reporter.on_fetch_version(name, version.version());
                map.insert(name.clone(), version);
            } else {
                reporter.on_fetch_progress();
                // A later result supersedes an earlier one for the same normalized tool name.
                map.remove(name);
            }
        }
        reporter.on_fetch_complete();
        map
    } else {
        FxHashMap::default()
    };

    for (name, tool, tool_env, version) in valid_tools {
        // If `--outdated` is set, skip tools that are up-to-date.
        let latest_version = if outdated {
            let Some(latest) = latest
                .get(&name)
                .filter(|latest| latest.version() > &version)
            else {
                continue;
            };
            format!(" [latest: {}]", latest.version())
        } else {
            String::new()
        };

        let version_specifier = if show_version_specifiers {
            format_annotation(
                "required",
                tool.requirements()
                    .iter()
                    .filter(|req| req.name == name)
                    .map(|req| req.source.to_string())
                    .filter(|source| !source.is_empty()),
            )
        } else {
            String::new()
        };

        let extra_requirements = if show_extras {
            format_annotation(
                "extras",
                tool.requirements()
                    .iter()
                    .filter(|req| req.name == name)
                    .flat_map(|req| req.extras.iter()),
            )
        } else {
            String::new()
        };

        let python_version = if show_python {
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

        let with_requirements = if show_with {
            format_annotation(
                "with",
                tool.requirements()
                    .iter()
                    .filter(|req| req.name != name)
                    .map(|req| format!("{}{}", req.name, req.source)),
            )
        } else {
            String::new()
        };

        let summary = format!(
            "{name} v{version}{version_specifier}{extra_requirements}{with_requirements}{python_version}{latest_version}"
        );
        if show_paths {
            writeln!(
                printer.stdout(),
                "{} ({})",
                summary.bold(),
                installed_tools.tool_dir(&name).simplified_display().cyan(),
            )?;
        } else {
            writeln!(printer.stdout(), "{}", summary.bold())?;
        }

        // Output tool entrypoints
        for entrypoint in tool.entrypoints() {
            if show_paths {
                writeln!(printer.stdout(), "- {}", entrypoint.to_string().cyan())?;
            } else {
                writeln!(printer.stdout(), "- {}", entrypoint.name)?;
            }
        }
    }

    Ok(ExitStatus::Success)
}

/// Format a nonempty list of values as a tool annotation.
fn format_annotation<T: std::fmt::Display>(label: &str, values: impl Iterator<Item = T>) -> String {
    let mut values = values.peekable();
    if values.peek().is_none() {
        String::new()
    } else {
        format!(" [{label}: {}]", values.format(", "))
    }
}
