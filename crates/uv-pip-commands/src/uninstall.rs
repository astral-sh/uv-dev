use std::fmt::Write;

use anyhow::Result;
use itertools::{Either, Itertools};
use owo_colors::OwoColorize;
use tracing::{debug, warn};

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_configuration::{Concurrency, Constraints, DryRun, ExtrasSpecification, GitLfsSetting};
use uv_dispatch::SharedState;
use uv_distribution_types::{InstalledMetadata, Name, UnresolvedRequirement};
use uv_distribution_types::{NameRequirementSpecification, Requirement};
use uv_fs::Simplified;
use uv_pep508::UnnamedRequirement;
use uv_preview::Preview;
use uv_pypi_types::VerbatimParsedUrl;
use uv_python_discovery::find_environment;
use uv_python_types::{
    EnvironmentPreference, Prefix, PythonArchitecture, PythonPreference, PythonRequest, Target,
};
use uv_requirements::{PylockInputMode, RequirementsSource, RequirementsSpecification};
use uv_resolve_operations::pylock::{read_pylock_toml, select_pylock_packages};
use uv_settings::ResolverSettings;
use uv_workspace::WorkspaceCache;

use crate::reporters::report_target_environment;
use uv_command_support::Printer;
use uv_command_support::{ExitStatus, elapsed};

/// Uninstall packages from the current environment.
pub async fn pip_uninstall(
    python_arch: Option<PythonArchitecture>,
    sources: &[RequirementsSource],
    build_constraints: &[RequirementsSource],
    build_constraints_from_workspace: Vec<NameRequirementSpecification>,
    python: Option<String>,
    system: bool,
    break_system_packages: bool,
    target: Option<Target>,
    prefix: Option<Prefix>,
    cache: Cache,
    settings: ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    dry_run: DryRun,
    concurrency: Concurrency,
    workspace_cache: WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    let start = std::time::Instant::now();

    let client_builder = client_builder.clone().keyring(settings.keyring_provider);

    // Read all requirements from the provided sources.
    let spec = RequirementsSpecification::from_sources(
        sources,
        &[],
        &[],
        &[],
        None,
        PylockInputMode::Import,
        &client_builder,
        &cache,
    )
    .await?;

    // Detect the current Python interpreter.
    let environment = find_environment(
        &python
            .as_deref()
            .map(PythonRequest::parse)
            .unwrap_or_default(),
        EnvironmentPreference::from_system_flag(system, true),
        PythonPreference::default().with_system_flag(system),
        python_arch,
        &cache,
    )?;

    report_target_environment(&environment, &cache, printer)?;

    // Apply any `--target` or `--prefix` directories.
    let environment = if let Some(target) = target {
        debug!(
            "Using `--target` directory at `{}`",
            target.root().user_display()
        );
        environment.with_target(target)?
    } else if let Some(prefix) = prefix {
        debug!(
            "Using `--prefix` directory at `{}`",
            prefix.root().user_display()
        );
        environment.with_prefix(prefix)?
    } else {
        environment
    };

    // If the environment is externally managed, abort.
    if let Some(externally_managed) = environment.interpreter().is_externally_managed() {
        if break_system_packages {
            debug!("Ignoring externally managed environment due to `--break-system-packages`");
        } else {
            return if let Some(error) = externally_managed.into_error() {
                Err(anyhow::anyhow!(
                    "The interpreter at `{}` is externally managed, and indicates the following:\n\n{}\n\nConsider creating a virtual environment with `uv venv`.",
                    environment.root().user_display().cyan(),
                    textwrap::indent(&error, "  ").green(),
                ))
            } else {
                Err(anyhow::anyhow!(
                    "The interpreter at `{}` is externally managed. Instead, create a virtual environment with `uv venv`.",
                    environment.root().user_display().cyan()
                ))
            };
        }
    }

    let _lock = environment
        .lock()
        .await
        .inspect_err(|err| {
            warn!("Failed to acquire environment lock: {err}");
        })
        .ok();

    // Cache index credentials before fetching lockfiles or metadata build constraints.
    let _client = if spec.pylock.is_some() || !spec.source_trees.is_empty() {
        Some(
            RegistryClientBuilder::new(client_builder.clone(), cache.clone())
                .index_locations(settings.index_locations.clone())
                .build()?,
        )
    } else {
        None
    };

    let mut locked_names = Vec::new();
    if let Some(source) = &spec.pylock {
        let (_, lock) = read_pylock_toml(source, &client_builder).await?;
        let selection =
            select_pylock_packages(&lock, &ExtrasSpecification::default(), &spec.pylock_groups);
        let markers = environment.interpreter().to_resolver_marker_environment();
        locked_names.extend(lock.packages.into_iter().filter_map(|package| {
            package
                .marker()
                .evaluate_pep751(&markers, &selection.extras, &selection.groups)
                .then_some(package.name)
        }));
    }

    let metadata_requirements = if spec.source_trees.is_empty() {
        Vec::new()
    } else {
        let build_constraints = Constraints::from_specifications(
            uv_resolve_operations::read_constraints(build_constraints, &client_builder, &cache)
                .await?
                .into_iter()
                .chain(build_constraints_from_workspace),
        );
        uv_resolve_operations::resolve_requirements(
            Vec::new(),
            &spec.source_trees,
            environment.interpreter(),
            &settings,
            &build_constraints,
            &client_builder,
            &SharedState::default(),
            &concurrency,
            &cache,
            &workspace_cache,
            printer,
            preview,
            GitLfsSetting::default(),
        )
        .await?
    };

    // Index the current `site-packages` directory.
    let site_packages = uv_installer::SitePackages::from_environment(&environment)?;

    // Partition the requirements into named and unnamed requirements.
    let (mut named, unnamed): (Vec<Requirement>, Vec<UnnamedRequirement<VerbatimParsedUrl>>) = spec
        .requirements
        .into_iter()
        .partition_map(|entry| match entry.requirement {
            UnresolvedRequirement::Named(requirement) => Either::Left(requirement),
            UnresolvedRequirement::Unnamed(requirement) => Either::Right(requirement),
        });

    named.extend(metadata_requirements);

    // Sort and deduplicate the packages, which are keyed by name. Like `pip`, we ignore the
    // dependency specifier (even if it's a URL).
    let names = {
        let mut packages = named
            .into_iter()
            .map(|requirement| requirement.name)
            .chain(locked_names)
            .collect::<Vec<_>>();
        packages.sort_unstable();
        packages.dedup();
        packages
    };

    // Sort and deduplicate the unnamed requirements, which are keyed by URL rather than package name.
    let urls = {
        let mut urls = unnamed
            .into_iter()
            .map(|requirement| requirement.url.verbatim.into_url())
            .collect::<Vec<_>>();
        urls.sort_unstable();
        urls.dedup();
        urls
    };

    // Map to the local distributions.
    let distributions = {
        let mut distributions = Vec::with_capacity(names.len() + urls.len());

        // Identify all packages that are installed.
        for package in &names {
            let installed = site_packages.get_packages(package);
            if installed.is_empty() {
                if !dry_run.enabled() {
                    writeln!(
                        printer.stderr(),
                        "{}{} Skipping {} as it is not installed",
                        "warning".yellow().bold(),
                        ":".bold(),
                        package.as_ref().bold()
                    )?;
                }
            } else {
                distributions.extend(installed);
            }
        }

        // Identify all unnamed distributions that are installed.
        for url in &urls {
            let installed = site_packages.get_urls(url);
            if installed.is_empty() {
                if !dry_run.enabled() {
                    writeln!(
                        printer.stderr(),
                        "{}{} Skipping `{}` as it is not installed",
                        "warning".yellow().bold(),
                        ":".bold(),
                        url.as_ref().bold()
                    )?;
                }
            } else {
                distributions.extend(installed);
            }
        }

        // Deduplicate, since a package could be listed both by name and editable URL.
        distributions.sort_unstable_by_key(|dist| dist.install_path());
        distributions.dedup_by_key(|dist| dist.install_path());
        distributions
    };

    if distributions.is_empty() {
        if dry_run.enabled() {
            writeln!(printer.stderr(), "Would make no changes")?;
        } else {
            writeln!(
                printer.stderr(),
                "{}{} No packages to uninstall",
                "warning".yellow().bold(),
                ":".bold(),
            )?;
        }
        return Ok(ExitStatus::Success);
    }

    // Uninstall each package.
    if !dry_run.enabled() {
        let layout = environment.interpreter().layout();
        for distribution in &distributions {
            let summary = uv_installer::uninstall(distribution, &layout).await?;
            debug!(
                "Uninstalled {} ({} file{}, {} director{})",
                distribution.name(),
                summary.file_count,
                if summary.file_count == 1 { "" } else { "s" },
                summary.dir_count,
                if summary.dir_count == 1 { "y" } else { "ies" },
            );
        }
    }

    let uninstalls = distributions.len();
    let s = if uninstalls == 1 { "" } else { "s" };
    if dry_run.enabled() {
        writeln!(
            printer.stderr(),
            "{}",
            format!(
                "Would uninstall {}",
                format!("{uninstalls} package{s}").bold(),
            )
            .dimmed()
        )?;
    } else {
        writeln!(
            printer.stderr(),
            "{}",
            format!(
                "Uninstalled {} {}",
                format!("{uninstalls} package{s}").bold(),
                format!("in {}", elapsed(start.elapsed())).dimmed(),
            )
            .dimmed()
        )?;
    }

    for distribution in distributions {
        writeln!(
            printer.stderr(),
            " {} {}{}",
            "-".red(),
            distribution.name().as_ref().bold(),
            distribution.installed_version().to_string().dimmed()
        )?;
    }

    Ok(ExitStatus::Success)
}
