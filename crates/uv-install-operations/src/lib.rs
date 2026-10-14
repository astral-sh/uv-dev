//! Installation workflows used by uv commands.

use std::collections::HashSet;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use itertools::Either;
use owo_colors::OwoColorize;
use tracing::debug;
use uv_cache::Cache;
use uv_client::RegistryClient;
use uv_command_support::Printer;
use uv_configuration::{BuildOptions, Concurrency, DryRun, Modifications, Reinstall};
use uv_dispatch::BuildDispatch;
use uv_distribution::DistributionDatabase;
use uv_distribution_types::{
    CachedDist, ConfigSettings, DependencyMetadata, Diagnostic, Dist, DistributionMetadata,
    ExtraBuildRequires, ExtraBuildVariables, IndexLocations, InstalledDist, InstalledMetadata,
    InstalledVersion, LocalDist, Name, PackageConfigSettings, Resolution, VersionOrUrlRef,
};
use uv_fs::{CWD, Simplified, normalize_path_under};
use uv_install_wheel::{Layout, LinkMode, installed_dist_info_path, read_record_into_iter};
use uv_installer::{InstallationStrategy, Plan, Planner, Preparer, SitePackages};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pep508::VerbatimUrl;
use uv_platform_tags::Tags;
use uv_preview::Preview;
use uv_pypi_types::ResolverMarkerEnvironment;
use uv_python_interpreter::PythonEnvironment;
use uv_types::{BuildContext, HashStrategy, InFlight};
use uv_warnings::warn_user;

use crate::bytecode::{compile_bytecode, compile_bytecode_files};
use crate::loggers::InstallLogger;
use crate::reporters::{InstallReporter, PrepareReporter};

mod bytecode;
pub mod editable;
mod error;
pub mod loggers;
pub mod report;
mod reporters;

pub use error::Error;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
enum ChangeEventKind {
    /// The package was removed from the environment.
    Removed,
    /// The package was added to the environment.
    Added,
    /// The package was reinstalled without changing versions.
    Reinstalled,
}

#[derive(Debug)]
struct ChangeEvent<'a> {
    dist: &'a ChangedDist,
    kind: ChangeEventKind,
}

/// A distribution which was or would be modified
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ChangedDist {
    Local(LocalDist),
    Remote(Arc<Dist>),
}

impl Name for ChangedDist {
    fn name(&self) -> &PackageName {
        match self {
            Self::Local(dist) => dist.name(),
            Self::Remote(dist) => dist.name(),
        }
    }
}

/// The [`Version`] or [`VerbatimUrl`] for a changed dist.
#[derive(Debug, PartialOrd, Ord, PartialEq, Eq, Hash)]
enum ShortSpecifier<'a> {
    Version(&'a Version),
    Url(&'a VerbatimUrl),
}

impl std::fmt::Display for ShortSpecifier<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version(version) => version.fmt(f),
            Self::Url(url) => write!(f, " @ {url}"),
        }
    }
}

/// The [`InstalledVersion`] or [`VerbatimUrl`] for a changed dist.
#[derive(Debug, PartialOrd, Ord, PartialEq, Eq, Hash)]
enum LongSpecifier<'a> {
    InstalledVersion(InstalledVersion<'a>),
    Url(&'a VerbatimUrl),
}

impl std::fmt::Display for LongSpecifier<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InstalledVersion(version) => version.fmt(f),
            Self::Url(url) => write!(f, " @ {url}"),
        }
    }
}

impl ChangedDist {
    fn short_specifier(&self) -> ShortSpecifier<'_> {
        match self {
            Self::Local(dist) => ShortSpecifier::Version(dist.installed_version().version()),
            Self::Remote(dist) => match dist.version_or_url() {
                VersionOrUrlRef::Version(version) => ShortSpecifier::Version(version),
                VersionOrUrlRef::Url(url) => ShortSpecifier::Url(url),
            },
        }
    }

    fn long_specifier(&self) -> LongSpecifier<'_> {
        match self {
            Self::Local(dist) => LongSpecifier::InstalledVersion(dist.installed_version()),
            Self::Remote(dist) => match dist.version_or_url() {
                VersionOrUrlRef::Version(version) => {
                    LongSpecifier::InstalledVersion(InstalledVersion::Version(version))
                }
                VersionOrUrlRef::Url(url) => LongSpecifier::Url(url),
            },
        }
    }

    fn version(&self) -> Option<&Version> {
        match self {
            Self::Local(dist) => Some(dist.installed_version().version()),
            Self::Remote(dist) => dist.version(),
        }
    }
}

/// A summary of the changes made to the environment during an installation.
#[derive(Debug, Clone, Default)]
pub struct Changelog {
    /// The distributions that were installed.
    installed: HashSet<ChangedDist>,
    /// The distributions that were uninstalled.
    uninstalled: HashSet<ChangedDist>,
    /// The distributions that were reinstalled.
    reinstalled: HashSet<ChangedDist>,
}

impl Changelog {
    /// Create a [`Changelog`] from two iterators of [`ChangedDist`]s.
    fn new<I, U>(installed: I, uninstalled: U) -> Self
    where
        I: IntoIterator<Item = ChangedDist>,
        U: IntoIterator<Item = ChangedDist>,
    {
        // SAFETY: This is allowed because `LocalDist` implements `Hash` and `Eq` based solely on
        // the inner `kind`, and omits the types that rely on internal mutability.
        #[expect(clippy::mutable_key_type)]
        let mut uninstalled: HashSet<_> = uninstalled.into_iter().collect();
        let (reinstalled, installed): (HashSet<_>, HashSet<_>) = installed
            .into_iter()
            .partition(|dist| uninstalled.contains(dist));
        uninstalled.retain(|dist| !reinstalled.contains(dist));

        Self {
            installed,
            uninstalled,
            reinstalled,
        }
    }

    /// Create a [`Changelog`] from a list of local distributions.
    fn from_local(installed: Vec<CachedDist>, uninstalled: Vec<InstalledDist>) -> Self {
        Self::new(
            installed
                .into_iter()
                .map(|dist| ChangedDist::Local(dist.into())),
            uninstalled
                .into_iter()
                .map(|dist| ChangedDist::Local(dist.into())),
        )
    }

    /// Create a [`Changelog`] from a list of installed distributions.
    pub fn from_installed(installed: Vec<CachedDist>) -> Self {
        Self::from_local(installed, Vec::new())
    }

    /// Return package names for added or removed distributions, excluding unchanged reinstalls.
    /// A package name can appear more than once.
    pub fn added_or_removed_packages(&self) -> impl Iterator<Item = &PackageName> {
        self.installed
            .iter()
            .chain(&self.uninstalled)
            .map(ChangedDist::name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BytecodeCompilation {
    /// Compile all Python source files in the environment.
    All,
    /// Compile Python source files installed by this operation.
    Installed,
}

/// An installation plan and the time required to create it.
pub struct InstallationPlan {
    plan: Plan,
    elapsed: Duration,
}

impl InstallationPlan {
    /// Determine the changes required to make an environment satisfy a resolution.
    pub fn build(
        resolution: &Resolution,
        site_packages: SitePackages,
        installation: InstallationStrategy,
        reinstall: &Reinstall,
        build_options: &BuildOptions,
        hasher: &HashStrategy,
        index_locations: &IndexLocations,
        config_settings: &ConfigSettings,
        config_settings_package: &PackageConfigSettings,
        extra_build_requires: &ExtraBuildRequires,
        extra_build_variables: &ExtraBuildVariables,
        cache: &Cache,
        venv: &PythonEnvironment,
        tags: &Tags,
    ) -> Result<Self, Error> {
        let start = Instant::now();
        let plan = Planner::new(resolution)
            .build(
                site_packages,
                installation,
                reinstall,
                build_options,
                hasher,
                index_locations,
                config_settings,
                config_settings_package,
                extra_build_requires,
                extra_build_variables,
                cache,
                venv,
                tags,
            )
            .map_err(Error::Plan)?;

        Ok(Self {
            plan,
            elapsed: start.elapsed(),
        })
    }

    /// Returns `true` if executing the plan would not modify the environment.
    pub fn is_noop(
        &self,
        modifications: Modifications,
        compile: Option<BytecodeCompilation>,
        dry_run: DryRun,
    ) -> bool {
        self.plan.cached.is_empty()
            && self.plan.remote.is_empty()
            && self.plan.reinstalls.is_empty()
            && (self.plan.extraneous.is_empty()
                || matches!(modifications, Modifications::Sufficient))
            && (compile.is_none() || dry_run.enabled())
    }

    /// Complete an installation that was determined to be a no-op.
    pub fn finish_noop(
        self,
        resolution: &Resolution,
        modifications: Modifications,
        compile: Option<BytecodeCompilation>,
        logger: &dyn InstallLogger,
        dry_run: DryRun,
        printer: Printer,
    ) -> Result<Changelog, Error> {
        debug_assert!(self.is_noop(modifications, compile, dry_run));

        let (plan, start) = self.into_execution_parts(modifications);
        if dry_run.enabled() {
            report_dry_run(dry_run, resolution, plan, start, logger, printer)
        } else {
            logger.on_check(resolution.len(), start, printer, dry_run)?;
            Ok(Changelog::default())
        }
    }

    fn into_execution_parts(self, modifications: Modifications) -> (Plan, Instant) {
        let now = Instant::now();
        let start = now.checked_sub(self.elapsed).unwrap_or(now);
        let plan = match modifications {
            Modifications::Sufficient => Plan {
                extraneous: Vec::new(),
                ..self.plan
            },
            Modifications::Exact => self.plan,
        };
        (plan, start)
    }
}

/// Install a set of requirements into the current environment.
///
/// Returns a [`Changelog`] summarizing the changes made to the environment.
pub async fn install(
    resolution: &Resolution,
    site_packages: SitePackages,
    installation: InstallationStrategy,
    modifications: Modifications,
    reinstall: &Reinstall,
    build_options: &BuildOptions,
    link_mode: LinkMode,
    compile: Option<BytecodeCompilation>,
    hasher: &HashStrategy,
    tags: &Tags,
    client: &RegistryClient,
    in_flight: &InFlight,
    concurrency: &Concurrency,
    build_dispatch: &BuildDispatch<'_>,
    cache: &Cache,
    venv: &PythonEnvironment,
    logger: Box<dyn InstallLogger>,
    installer_metadata: bool,
    dry_run: DryRun,
    printer: Printer,
    preview: Preview,
) -> Result<Changelog, Error> {
    let plan = InstallationPlan::build(
        resolution,
        site_packages,
        installation,
        reinstall,
        build_options,
        hasher,
        build_dispatch.locations(),
        build_dispatch.config_settings(),
        build_dispatch.config_settings_package(),
        build_dispatch.extra_build_requires(),
        build_dispatch.extra_build_variables(),
        cache,
        venv,
        tags,
    )?;

    plan.execute(
        resolution,
        modifications,
        build_options,
        link_mode,
        compile,
        hasher,
        tags,
        client,
        in_flight,
        concurrency,
        build_dispatch,
        cache,
        venv,
        logger,
        installer_metadata,
        dry_run,
        printer,
        preview,
    )
    .await
}

impl InstallationPlan {
    /// Execute a previously computed installation plan.
    pub async fn execute(
        self,
        resolution: &Resolution,
        modifications: Modifications,
        build_options: &BuildOptions,
        link_mode: LinkMode,
        compile: Option<BytecodeCompilation>,
        hasher: &HashStrategy,
        tags: &Tags,
        client: &RegistryClient,
        in_flight: &InFlight,
        concurrency: &Concurrency,
        build_dispatch: &BuildDispatch<'_>,
        cache: &Cache,
        venv: &PythonEnvironment,
        logger: Box<dyn InstallLogger>,
        installer_metadata: bool,
        dry_run: DryRun,
        printer: Printer,
        preview: Preview,
    ) -> Result<Changelog, Error> {
        let (plan, start) = self.into_execution_parts(modifications);

        if dry_run.enabled() {
            return report_dry_run(dry_run, resolution, plan, start, logger.as_ref(), printer);
        }

        // Nothing to do.
        if plan.is_empty() && compile.is_none() {
            logger.on_check(resolution.len(), start, printer, dry_run)?;
            return Ok(Changelog::default());
        }

        // Partition into two sets: those that require build isolation, and those that disable it. This
        // is effectively a heuristic to make `--no-build-isolation` work "more often" by way of giving
        // `--no-build-isolation` packages "access" to the rest of the environment.
        let (isolated_phase, shared_phase) =
            plan.partition(|name| build_dispatch.build_isolation().is_isolated(Some(name)));

        let shared_phase_label = (!isolated_phase.is_empty()).then_some(InstallPhase::Shared);

        let mut installs = vec![];
        let mut uninstalls = vec![];

        for (plan, phase) in [(isolated_phase, None), (shared_phase, shared_phase_label)] {
            if plan.is_empty() {
                continue;
            }
            let (phase_installs, phase_uninstalls) = execute_plan(
                plan,
                phase,
                resolution,
                build_options,
                link_mode,
                hasher,
                tags,
                client,
                in_flight,
                concurrency,
                build_dispatch,
                cache,
                venv,
                logger.as_ref(),
                installer_metadata,
                printer,
                preview,
            )
            .await?;
            installs.extend(phase_installs);
            uninstalls.extend(phase_uninstalls);
        }

        if let Some(compile) = compile {
            match compile {
                BytecodeCompilation::All => {
                    compile_bytecode(venv, concurrency, cache, printer).await?;
                }
                BytecodeCompilation::Installed => {
                    compile_bytecode_for_installs(venv, &installs, concurrency, cache, printer)
                        .await?;
                }
            }
        }

        // Construct a summary of the changes made to the environment.
        let changelog = Changelog::from_local(installs, uninstalls);

        // Notify the user of any environment modifications.
        logger.on_complete(&changelog, printer, dry_run)?;

        Ok(changelog)
    }
}

/// Compile the Python source files owned by the distributions installed by this operation.
async fn compile_bytecode_for_installs(
    venv: &PythonEnvironment,
    installs: &[CachedDist],
    concurrency: &Concurrency,
    cache: &Cache,
    printer: Printer,
) -> Result<(), Error> {
    let layout = venv.interpreter().layout();
    let site_packages = [
        CWD.join(&layout.scheme.purelib),
        CWD.join(&layout.scheme.platlib),
    ];
    let files = installs
        .iter()
        .filter_map(|install| InstalledRecord::open(&layout, install).transpose())
        .flat_map(|record| match record {
            Ok(record) => Either::Left(record.python_sources(&site_packages)),
            Err(err) => Either::Right(std::iter::once(Err(err))),
        });
    compile_bytecode_files(files, venv, concurrency, cache, printer).await
}

/// An installed distribution's `RECORD` file and the paths needed to interpret it.
struct InstalledRecord {
    root: PathBuf,
    path: PathBuf,
    file: fs_err::File,
}

impl InstalledRecord {
    fn open(layout: &Layout, install: &CachedDist) -> anyhow::Result<Option<Self>> {
        let dist_info = installed_dist_info_path(layout, install.path()).with_context(|| {
            format!("Failed to locate installed distribution for bytecode compilation: {install}")
        })?;
        let root = dist_info
            .parent()
            .map(|path| CWD.join(path))
            .ok_or_else(|| {
                anyhow!(
                    "Invalid installed distribution path: {}",
                    dist_info.user_display()
                )
            })?;
        let path = dist_info.join("RECORD");
        let file = match fs_err::File::open(&path) {
            // Another process may have removed the installed distribution.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            result => {
                result.with_context(|| format!("Failed to read `{}`", path.user_display()))?
            }
        };
        Ok(Some(Self { root, path, file }))
    }

    fn python_sources(
        self,
        site_packages: &[PathBuf],
    ) -> impl Iterator<Item = anyhow::Result<PathBuf>> {
        let Self { root, path, file } = self;
        read_record_into_iter(file).filter_map(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    return Some(
                        Err(err)
                            .with_context(|| format!("Failed to read `{}`", path.user_display())),
                    );
                }
            };
            let source = python_source_path_from_record(&root, &entry.path, site_packages)?;
            source.is_file().then_some(Ok(source))
        })
    }
}

/// Resolve a Python source path from an installed `RECORD` entry.
fn python_source_path_from_record(
    record_root: &Path,
    entry: &str,
    site_packages: &[PathBuf],
) -> Option<PathBuf> {
    let path = Path::new(entry);
    if path.extension().is_none_or(|extension| extension != "py") {
        return None;
    }

    let path = record_root.join(path);
    site_packages
        .iter()
        .find_map(|site_packages| normalize_path_under(&path, site_packages))
}

#[cfg(test)]
mod tests {
    use super::{Error, python_source_path_from_record};
    use insta::assert_snapshot;
    use std::path::{Path, PathBuf};
    use uv_normalize::PackageName;

    #[test]
    fn record_python_sources_stay_in_site_packages() {
        let record_root = Path::new("venv/purelib");
        let site_packages = [PathBuf::from("venv/purelib"), PathBuf::from("venv/platlib")];

        assert_eq!(
            python_source_path_from_record(record_root, "package/__init__.py", &site_packages,),
            Some(PathBuf::from("venv/purelib/package/__init__.py"))
        );
        assert_eq!(
            python_source_path_from_record(
                record_root,
                "../platlib/package/module.py",
                &site_packages,
            ),
            Some(PathBuf::from("venv/platlib/package/module.py"))
        );
        assert_eq!(
            python_source_path_from_record(record_root, "../scripts/tool.py", &site_packages),
            None
        );
        assert_eq!(
            python_source_path_from_record(record_root, "/outside.py", &site_packages),
            None
        );
        assert_eq!(
            python_source_path_from_record(record_root, "package/data.txt", &site_packages),
            None
        );
    }

    #[test]
    fn preparation_errors_are_transparent() -> Result<(), uv_normalize::InvalidNameError> {
        let error = Error::Prepare(uv_installer::PrepareError::NoBuild(
            PackageName::from_owned("demo".to_string())?,
        ));
        assert_snapshot!(error, @"Building source distributions is disabled, but attempted to build `demo`");
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallPhase {
    /// A dedicated phase for building and installing packages with build-isolation disabled.
    Shared,
}

impl InstallPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Shared => "without build isolation",
        }
    }
}

/// Execute a [`Plan`] to install distributions into a Python environment.
async fn execute_plan(
    plan: Plan,
    phase: Option<InstallPhase>,
    resolution: &Resolution,
    build_options: &BuildOptions,
    link_mode: LinkMode,
    hasher: &HashStrategy,
    tags: &Tags,
    client: &RegistryClient,
    in_flight: &InFlight,
    concurrency: &Concurrency,
    build_dispatch: &BuildDispatch<'_>,
    cache: &Cache,
    venv: &PythonEnvironment,
    logger: &dyn InstallLogger,
    installer_metadata: bool,
    printer: Printer,
    preview: Preview,
) -> Result<(Vec<CachedDist>, Vec<InstalledDist>), Error> {
    let Plan {
        cached,
        remote,
        reinstalls,
        extraneous,
    } = plan;

    // Download, build, and unzip any missing distributions.
    let wheels = if remote.is_empty() {
        vec![]
    } else {
        let start = std::time::Instant::now();

        let preparer = Preparer::new(
            tags,
            hasher,
            build_options,
            DistributionDatabase::new(
                client,
                build_dispatch,
                concurrency.downloads_semaphore.clone(),
            ),
        )
        .with_reporter(Arc::new(
            PrepareReporter::from(printer).with_length(remote.len() as u64),
        ));

        let wheels = preparer.prepare(remote, in_flight, resolution).await?;

        logger.on_prepare(
            wheels.len(),
            phase.map(InstallPhase::label),
            start,
            printer,
            DryRun::Disabled,
        )?;

        wheels
    };

    // Remove any upgraded or extraneous installations.
    let mut uninstalls = extraneous;
    uninstalls.extend(reinstalls);
    if !uninstalls.is_empty() {
        let start = std::time::Instant::now();

        let layout = venv.interpreter().layout();
        for dist_info in &uninstalls {
            match uv_installer::uninstall(dist_info, &layout).await {
                Ok(summary) => {
                    debug!(
                        "Uninstalled {} ({} file{}, {} director{})",
                        dist_info.name(),
                        summary.file_count,
                        if summary.file_count == 1 { "" } else { "s" },
                        summary.dir_count,
                        if summary.dir_count == 1 { "y" } else { "ies" },
                    );
                }
                Err(error) => {
                    let missing_file = match error {
                        uv_installer::UninstallError::Uninstall(
                            uv_install_wheel::Error::MissingRecord(_),
                        ) => "RECORD",
                        uv_installer::UninstallError::Uninstall(
                            uv_install_wheel::Error::MissingTopLevel(_),
                        ) => "top_level.txt",
                        error => return Err(error.into()),
                    };
                    warn_user!(
                        "Failed to uninstall package at `{}` due to missing `{missing_file}` file. Installation may result in an incomplete environment.",
                        dist_info.install_path().user_display().cyan(),
                    );
                }
            }
        }

        logger.on_uninstall(uninstalls.len(), start, printer, DryRun::Disabled)?;
    }

    // Install the resolved distributions.
    let mut installs = wheels;
    installs.extend(cached);
    if !installs.is_empty() {
        let start = std::time::Instant::now();
        installs = uv_installer::Installer::new(venv, preview)
            .with_link_mode(link_mode)
            .with_cache(cache)
            .with_installer_metadata(installer_metadata)
            .with_reporter(Arc::new(
                InstallReporter::from(printer).with_length(installs.len() as u64),
            ))
            // This technically can block the runtime, but we are on the main thread and
            // have no other running tasks at this point, so this lets us avoid spawning a blocking
            // task.
            .install_blocking(installs)?;

        logger.on_install(installs.len(), start, printer, DryRun::Disabled)?;
    }

    Ok((installs, uninstalls))
}

/// Report on the results of a dry-run installation.
fn report_dry_run(
    dry_run: DryRun,
    resolution: &Resolution,
    plan: Plan,
    start: std::time::Instant,
    logger: &dyn InstallLogger,
    printer: Printer,
) -> Result<Changelog, Error> {
    // Nothing to do.
    if plan.is_empty() {
        logger.on_check(resolution.len(), start, printer, dry_run)?;
        return Ok(Changelog::default());
    }

    let Plan {
        cached,
        remote,
        reinstalls,
        extraneous,
    } = plan;

    // Report any distributions that would need to be prepared.
    if !remote.is_empty() {
        logger.on_prepare(remote.len(), None, start, printer, dry_run)?;
    }

    // Remove any upgraded or extraneous installations.
    let uninstalls = extraneous.len() + reinstalls.len();

    if uninstalls > 0 {
        logger.on_uninstall(uninstalls, start, printer, dry_run)?;
    }

    // Install the resolved distributions.
    let installs = remote.len() + cached.len();

    if installs > 0 {
        logger.on_install(installs, start, printer, dry_run)?;
    }

    let uninstalled = reinstalls
        .into_iter()
        .chain(extraneous)
        .map(|dist| ChangedDist::Local(dist.into()));
    let installed = remote.into_iter().map(ChangedDist::Remote).chain(
        cached
            .into_iter()
            .map(|dist| ChangedDist::Local(dist.into())),
    );

    let changelog = Changelog::new(installed, uninstalled);

    logger.on_complete(&changelog, printer, dry_run)?;

    if matches!(dry_run, DryRun::Check) {
        return Err(Error::OutdatedEnvironment(Box::new(changelog)));
    }

    Ok(changelog)
}

/// Report any diagnostics on installed distributions in the Python environment.
pub fn diagnose_environment<'a>(
    relevant_packages: impl Iterator<Item = &'a PackageName>,
    venv: &PythonEnvironment,
    markers: &ResolverMarkerEnvironment,
    tags: &Tags,
    dependency_metadata: &DependencyMetadata,
    printer: Printer,
) -> Result<(), Error> {
    let site_packages = SitePackages::from_environment(venv)?;
    let relevant_packages = relevant_packages.collect::<HashSet<_>>();
    for diagnostic in site_packages.diagnostics(markers, tags, dependency_metadata) {
        // Only surface diagnostics that are "relevant" to the current resolution.
        if diagnostic.includes_any(|name| relevant_packages.contains(name)) {
            writeln!(
                printer.stderr(),
                "{}{} {}",
                "warning".yellow().bold(),
                ":".bold(),
                diagnostic.message().bold()
            )?;
        }
    }
    Ok(())
}
