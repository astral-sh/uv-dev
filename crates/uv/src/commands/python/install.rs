use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::future::{Future, ready};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Error, Result};
use futures::{Stream, StreamExt, join};
use indexmap::IndexSet;
use itertools::Itertools;
use owo_colors::{AnsiColors, OwoColorize};
use rustc_hash::{FxHashMap, FxHashSet};
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use uv_cache::Cache;
use uv_cli::PythonUpgradeFormat;
use uv_client::BaseClientBuilder;
use uv_configuration::Concurrency;
use uv_errors::{ErrorOptions, Hints, write_error_chain_with_options};
use uv_fs::Simplified;
use uv_platform::{Arch, Libc};
use uv_preview::{Preview, PreviewFeature};
use uv_python::downloads::{
    self, ArchRequest, DownloadResult, ManagedPythonDownload, ManagedPythonDownloadList,
    PythonDownloadRequest,
};
use uv_python::managed::{
    ManagedPythonInstallation, ManagedPythonInstallations, PythonExecutable,
    PythonMinorVersionLink, compare_build_versions, create_link_to_executable,
    python_executable_dir, replace_link_to_executable,
};
use uv_python::{
    ConfigDiscovery, ImplementationName, Interpreter, PythonDownloads, PythonInstallationKey,
    PythonInstallationMinorVersionKey, PythonRequest, PythonVersionFile,
    VersionFileDiscoveryOptions, VersionFilePreference, VersionRequest,
};
use uv_shell::Shell;
use uv_trampoline_builder::{Launcher, LauncherKind};
use uv_warnings::warn_user;

use crate::commands::python::upgrade_report::{
    ExecutableChange, InstallationReport, UpgradeEntry, UpgradeError, UpgradeErrorKind,
    UpgradeReport, installation_outcome,
};
use crate::commands::python::{ChangeEvent, ChangeEventKind};
use crate::commands::reporters::PythonDownloadReporter;
use crate::commands::{ExitStatus, UvError, conjunction, elapsed};
use crate::printer::Printer;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InstallRequest<'a> {
    /// The original request from the user
    request: PythonRequest,
    /// A download request corresponding to the `request` with platform information filled
    download_request: PythonDownloadRequest,
    /// A download that satisfies the request
    download: &'a ManagedPythonDownload,
}

impl<'a> InstallRequest<'a> {
    fn new(request: PythonRequest, download_list: &'a ManagedPythonDownloadList) -> Result<Self> {
        // Make sure the request is a valid download request and fill platform information
        let download_request = PythonDownloadRequest::from_request(&request)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "`{}` is not a valid Python download request; see `uv help python` for supported formats and `uv python list --only-downloads` for available versions",
                    request.to_canonical_string()
                )
            })?
            .fill()?;

        // Find a matching download
        let download = match download_list.find(&download_request) {
            Ok(download) => download,
            Err(downloads::Error::NoDownloadFound(request))
                if request.libc().is_some_and(Libc::is_musl)
                    && request.arch().is_some_and(|arch| {
                        arch.inner() == Arch::from(&uv_platform_tags::Arch::Armv7L)
                    }) =>
            {
                return Err(anyhow::anyhow!(
                    "uv does not yet provide musl Python distributions on armv7."
                ));
            }
            Err(err) => return Err(err.into()),
        };

        Ok(Self {
            request,
            download_request,
            download,
        })
    }

    fn matches_installation(&self, installation: &ManagedPythonInstallation) -> bool {
        self.download_request.satisfied_by_key(installation.key())
    }

    fn python_request(&self) -> &PythonRequest {
        &self.request
    }
}

impl std::fmt::Display for InstallRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let request = self.request.to_canonical_string();
        let download = self.download_request.to_string();
        if request != download {
            write!(f, "{request} ({download})")
        } else {
            write!(f, "{request}")
        }
    }
}

#[derive(Debug, Default)]
struct Changelog {
    existing: FxHashSet<PythonInstallationKey>,
    installed: FxHashSet<PythonInstallationKey>,
    uninstalled: FxHashSet<PythonInstallationKey>,
    installed_executables: FxHashMap<PythonInstallationKey, FxHashSet<PathBuf>>,
    report: Option<UpgradeChanges>,
}

#[derive(Debug, Default)]
struct UpgradeChanges {
    before: FxHashMap<PythonInstallationKey, InstallationReport>,
    executables: BTreeMap<PathBuf, ExecutableChange>,
}

impl Changelog {
    fn record_executable_change(
        &mut self,
        path: &Path,
        from: Option<&ManagedPythonInstallation>,
        to: &ManagedPythonInstallation,
    ) {
        let Some(report) = self.report.as_mut() else {
            return;
        };
        // Downloads can replace a build without changing its installation key or executable
        // path. Use the identity captured under the install lock before downloading.
        let from = from.map(|installation| {
            report
                .before
                .get(installation.key())
                .cloned()
                .unwrap_or_else(|| installation.into())
        });
        let to = InstallationReport::from(to);
        report
            .executables
            .entry(path.to_path_buf())
            .and_modify(|change| change.to = to.clone())
            .or_insert_with(|| ExecutableChange {
                path: path.into(),
                from,
                to,
            });
    }

    fn events(&self) -> impl Iterator<Item = ChangeEvent> {
        let reinstalled = self
            .uninstalled
            .intersection(&self.installed)
            .cloned()
            .collect::<FxHashSet<_>>();
        let uninstalled = self.uninstalled.difference(&reinstalled).cloned();
        let installed = self.installed.difference(&reinstalled).cloned();

        uninstalled
            .map(|key| ChangeEvent {
                key: key.clone(),
                kind: ChangeEventKind::Removed,
            })
            .chain(installed.map(|key| ChangeEvent {
                key: key.clone(),
                kind: ChangeEventKind::Added,
            }))
            .chain(reinstalled.iter().map(|key| ChangeEvent {
                key: key.clone(),
                kind: ChangeEventKind::Reinstalled,
            }))
            .sorted_unstable_by(|a, b| a.key.cmp(&b.key).then_with(|| a.kind.cmp(&b.kind)))
    }
}

#[derive(Debug, Clone, Copy)]
enum InstallErrorKind {
    DownloadUnpack,
    Bin,
    #[cfg_attr(not(windows), allow(dead_code))]
    Registry,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PythonUpgradeSource {
    /// The user invoked `uv python install --upgrade`
    Install,
    /// The user invoked `uv python upgrade`
    Upgrade,
}

impl std::fmt::Display for PythonUpgradeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Install => write!(f, "uv python install --upgrade"),
            Self::Upgrade => write!(f, "uv python upgrade"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("`{command}` only accepts minor versions, got: {request}")]
pub(crate) struct InvalidUpgradeRequestError {
    command: PythonUpgradeSource,
    request: String,
    from_version_file: bool,
}

impl uv_errors::Hinted for InvalidUpgradeRequestError {
    fn hints(&self) -> Hints<'_> {
        if self.from_version_file {
            Hints::from(
                "The version request came from a `.python-version` file; change the patch version in the file to upgrade instead",
            )
        } else {
            Hints::none()
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PythonUpgrade {
    /// Python upgrades are enabled.
    Enabled(PythonUpgradeSource),
    /// Python upgrades are disabled.
    Disabled,
}

/// Download and install Python versions.
#[expect(clippy::fn_params_excessive_bools)]
pub(crate) async fn install(
    project_dir: &Path,
    install_dir: Option<PathBuf>,
    targets: Vec<String>,
    reinstall: bool,
    upgrade: PythonUpgrade,
    output_format: PythonUpgradeFormat,
    bin: Option<bool>,
    registry: Option<bool>,
    force: bool,
    python_install_mirror: Option<String>,
    pypy_install_mirror: Option<String>,
    python_downloads_json_url: Option<String>,
    client_builder: BaseClientBuilder<'_>,
    default: bool,
    python_downloads: PythonDownloads,
    config_discovery: ConfigDiscovery,
    compile_bytecode: bool,
    concurrency: &Concurrency,
    cache: &Cache,
    preview: Preview,
    printer: Printer,
) -> Result<ExitStatus> {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let json_output = matches!(output_format, PythonUpgradeFormat::Json);
    let mut report = json_output.then(UpgradeReport::default);
    let mut compiler_failed_key = None;
    let compiler = async {
        let mut total_files = 0;
        let mut total_elapsed = std::time::Duration::default();
        let mut total_skipped = 0;
        while let Some(installation) = receiver.recv().await {
            let result = compile_stdlib_bytecode(&installation, concurrency, cache)
                .await
                .with_context(|| {
                    format!(
                        "Failed to bytecode-compile Python standard library for: {}",
                        installation.key()
                    )
                });
            let result = match result {
                Ok(result) => result,
                Err(err) => {
                    compiler_failed_key = Some(installation.key().clone());
                    if json_output {
                        receiver.close();
                    }
                    return Err(err);
                }
            };
            if let Some((files, elapsed)) = result {
                total_files += files;
                total_elapsed += elapsed;
            } else {
                total_skipped += 1;
            }
        }
        Ok::<_, anyhow::Error>((total_files, total_elapsed, total_skipped))
    };

    let installer = perform_install(
        project_dir,
        install_dir,
        targets,
        reinstall,
        upgrade,
        output_format,
        &mut report,
        bin,
        registry,
        force,
        python_install_mirror,
        pypy_install_mirror,
        python_downloads_json_url,
        client_builder,
        cache,
        default,
        python_downloads,
        config_discovery,
        compile_bytecode.then_some(sender),
        concurrency,
        preview,
        printer,
    );

    let (installer_result, compiler_result) = join!(installer, compiler);

    if let Some(report) = report.as_mut() {
        if let Err(err) = &installer_result {
            report.errors.push(UpgradeError {
                kind: UpgradeErrorKind::Operation,
                message: format!("{err:#}"),
            });
        }
        if let Err(err) = &compiler_result {
            if let Some(key) = compiler_failed_key.as_ref() {
                report.fail_installation(key, UpgradeErrorKind::Bytecode, format!("{err:#}"));
            }
            report.errors.push(UpgradeError {
                kind: UpgradeErrorKind::Bytecode,
                message: format!("{err:#}"),
            });
        }
        write_upgrade_report(output_format, report, printer)?;
    }

    let (total_files, total_elapsed, total_skipped) = compiler_result?;
    if total_files > 0 {
        let s = if total_files == 1 { "" } else { "s" };
        writeln!(
            printer.stderr(),
            "{}",
            format!(
                "Bytecode compiled {} {}{}",
                format!("{total_files} file{s}").bold(),
                format!("in {}", elapsed(total_elapsed)).dimmed(),
                if total_skipped > 0 {
                    format!(
                        " (skipped {total_skipped} incompatible version{})",
                        if total_skipped == 1 { "" } else { "s" }
                    )
                } else {
                    String::new()
                }
                .dimmed()
            )
            .dimmed()
        )?;
    } else if total_skipped > 0 {
        writeln!(
            printer.stderr(),
            "{}",
            format!("No compatible versions to bytecode compile (skipped {total_skipped})")
                .dimmed()
        )?;
    }

    installer_result
}

async fn perform_install(
    project_dir: &Path,
    install_dir: Option<PathBuf>,
    targets: Vec<String>,
    reinstall: bool,
    upgrade: PythonUpgrade,
    output_format: PythonUpgradeFormat,
    report: &mut Option<UpgradeReport>,
    bin: Option<bool>,
    registry: Option<bool>,
    force: bool,
    python_install_mirror: Option<String>,
    pypy_install_mirror: Option<String>,
    python_downloads_json_url: Option<String>,
    client_builder: BaseClientBuilder<'_>,
    cache: &Cache,
    default: bool,
    python_downloads: PythonDownloads,
    config_discovery: ConfigDiscovery,
    bytecode_compilation_sender: Option<mpsc::UnboundedSender<ManagedPythonInstallation>>,
    concurrency: &Concurrency,
    preview: Preview,
    printer: Printer,
) -> Result<ExitStatus> {
    let start = std::time::Instant::now();

    if matches!(output_format, PythonUpgradeFormat::Json)
        && !preview.is_enabled(PreviewFeature::JsonOutput)
    {
        warn_user!(
            "The `--output-format json` option is experimental and the schema may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::JsonOutput
        );
    }

    // TODO(zanieb): We should consider marking the Python installation as the default when
    // `--default` is used. It's not clear how this overlaps with a global Python pin, but I'd be
    // surprised if `uv python find` returned the "newest" Python version rather than the one I just
    // installed with the `--default` flag.
    if default && !preview.is_enabled(PreviewFeature::PythonInstallDefault) {
        warn_user!(
            "The `--default` option is experimental and may change without warning. Pass `--preview-features {}` to disable this warning",
            PreviewFeature::PythonInstallDefault
        );
    }

    if default && targets.len() > 1 {
        anyhow::bail!("The `--default` flag cannot be used with multiple targets");
    }

    // Read the existing installations, lock the directory for the duration
    let installations = ManagedPythonInstallations::from_settings(install_dir.clone())?.init()?;
    let installations_dir = installations.root();
    let scratch_dir = installations.scratch();
    let _lock = installations.lock().await?;
    let existing_installations: Vec<_> = installations
        .find_all()?
        .inspect(|installation| trace!("Found existing installation {}", installation.key()))
        .collect();

    // Resolve the requests
    let mut is_default_install = false;
    let mut is_unspecified_upgrade = false;
    let retry_policy = client_builder.retry_policy();
    let download_list = ManagedPythonDownloadList::new(
        &client_builder,
        cache,
        python_downloads_json_url.as_deref(),
    )
    .await?;
    // Python downloads are performing their own retries to catch stream errors, disable the
    // default retries to avoid the middleware from performing uncontrolled retries.
    let client = client_builder.retries(0).build()?;
    // TODO(zanieb): We use this variable to special-case .python-version files, but it'd be nice to
    // have generalized request source tracking instead
    let mut is_from_python_version_file = false;
    let requests: Vec<_> = if targets.is_empty() {
        if matches!(
            upgrade,
            PythonUpgrade::Enabled(PythonUpgradeSource::Upgrade)
        ) {
            is_unspecified_upgrade = true;
            // On upgrade, derive requests for all of the existing installations
            let mut minor_version_requests = IndexSet::<InstallRequest>::default();
            for installation in &existing_installations {
                let mut request = PythonDownloadRequest::from(installation);
                // We should always have a version in the request from an existing installation
                let version = request.take_version().unwrap();
                // Drop the patch and prerelease parts from the request
                request = request.with_version(version.only_minor());
                let install_request =
                    InstallRequest::new(PythonRequest::Key(request), &download_list)?;
                minor_version_requests.insert(install_request);
            }
            minor_version_requests.into_iter().collect::<Vec<_>>()
        } else {
            PythonVersionFile::discover(
                project_dir,
                &VersionFileDiscoveryOptions::default()
                    .with_config_discovery(config_discovery)
                    .with_preference(VersionFilePreference::Versions),
            )
            .await?
            .inspect(|file| {
                debug!(
                    "Found Python version file at: {}",
                    file.path().user_display()
                );
            })
            .map(PythonVersionFile::into_versions)
            .inspect(|_| is_from_python_version_file = true)
            .unwrap_or_else(|| {
                // If no version file is found and no requests were made
                // TODO(zanieb): We should consider differentiating between a global Python version
                // file here, allowing a request from there to enable `is_default_install`.
                is_default_install = true;
                vec![if reinstall {
                    // On bare `--reinstall`, reinstall all Python versions
                    PythonRequest::Any
                } else {
                    PythonRequest::Default
                }]
            })
            .into_iter()
            .map(|request| InstallRequest::new(request, &download_list))
            .collect::<Result<Vec<_>>>()?
        }
    } else {
        targets
            .iter()
            .map(|target| PythonRequest::parse(target.as_str()))
            .map(|request| InstallRequest::new(request, &download_list))
            .collect::<Result<Vec<_>>>()?
    };

    if requests.is_empty() {
        match upgrade {
            PythonUpgrade::Enabled(PythonUpgradeSource::Upgrade) => {
                writeln!(
                    printer.stderr(),
                    "There are no installed versions to upgrade"
                )?;
            }
            PythonUpgrade::Enabled(PythonUpgradeSource::Install) => {
                writeln!(
                    printer.stderr(),
                    "No Python versions specified for upgrade; did you mean `uv python upgrade`?"
                )?;
            }
            PythonUpgrade::Disabled => {}
        }
        return Ok(ExitStatus::Success);
    }

    let requested_minor_versions = requests
        .iter()
        .filter_map(|request| {
            if let PythonRequest::Version(VersionRequest::MajorMinor(major, minor, ..)) =
                request.python_request()
            {
                uv_pep440::Version::from_str(&format!("{major}.{minor}")).ok()
            } else {
                None
            }
        })
        .collect::<IndexSet<_>>();

    if let PythonUpgrade::Enabled(source) = upgrade {
        if let Some(request) = requests.iter().find(|request| {
            request.request.includes_patch() || request.request.includes_prerelease()
        }) {
            return Err(UvError::user(InvalidUpgradeRequestError {
                command: source,
                request: request.request.to_canonical_string().into_owned(),
                from_version_file: is_from_python_version_file,
            })
            .into());
        }
    }

    // Find requests that are already satisfied
    let mut changelog = Changelog {
        report: matches!(output_format, PythonUpgradeFormat::Json).then(|| UpgradeChanges {
            before: existing_installations
                .iter()
                .map(|installation| (installation.key().clone(), installation.into()))
                .collect(),
            executables: BTreeMap::new(),
        }),
        ..Changelog::default()
    };
    let (satisfied, unsatisfied): (Vec<_>, Vec<_>) = if reinstall {
        // In the reinstall case, we want to iterate over all matching installations instead of
        // stopping at the first match.

        // Keep each resolved operation paired with its originating request for reporting.
        let mut unsatisfied: Vec<(usize, Cow<InstallRequest>)> =
            Vec::with_capacity(existing_installations.len() + requests.len());

        for (request_index, request) in requests.iter().enumerate() {
            let mut matching_installations = existing_installations
                .iter()
                .filter(|installation| request.matches_installation(installation))
                .peekable();

            if matching_installations.peek().is_none() {
                debug!("No installation found for request `{}`", request);
                unsatisfied.push((request_index, Cow::Borrowed(request)));
            }

            for installation in matching_installations {
                changelog.existing.insert(installation.key().clone());

                if matches!(upgrade, PythonUpgrade::Enabled(_))
                    && !matches!(&request.request, &PythonRequest::Any)
                {
                    // An upgrade must reinstall the latest patch, not every matching patch.
                    debug!("Will reinstall the latest patch for `{}`", request);
                    unsatisfied.push((request_index, Cow::Borrowed(request)));
                    break;
                }

                // Construct an install request matching the existing installation.
                match InstallRequest::new(PythonRequest::Key(installation.into()), &download_list) {
                    Ok(request) => {
                        debug!("Will reinstall `{}`", installation.key());
                        unsatisfied.push((request_index, Cow::Owned(request)));
                    }
                    Err(err) => {
                        // This shouldn't really happen, but maybe a new version of uv dropped
                        // support for a key we previously supported.
                        warn_user!(
                            "Failed to create reinstall request for existing installation `{}`: {err}",
                            installation.key().green()
                        );
                    }
                }
            }
        }
        (vec![], unsatisfied)
    } else {
        // If we can find one existing installation that matches the request, it is satisfied
        let mut satisfied = Vec::new();
        let mut unsatisfied = Vec::new();

        for (request_index, request) in requests.iter().enumerate() {
            if matches!(upgrade, PythonUpgrade::Enabled(_)) {
                // If this is an upgrade, the requested version is a minor version but the
                // requested download is the highest patch for that minor version. We need to
                // install it unless an exact match is found (including build version).
                if let Some(installation) = existing_installations
                    .iter()
                    .find(|inst| request.download.key() == inst.key())
                {
                    if matches_build(request.download.build(), installation.build()) {
                        debug!("Found `{}` for request `{}`", installation.key(), request);
                        satisfied.push(installation);
                    } else {
                        // Key matches but build version differs - track as existing for reinstall
                        debug!(
                            "Build version mismatch for `{}`, will upgrade",
                            installation.key()
                        );
                        changelog.existing.insert(installation.key().clone());
                        unsatisfied.push((request_index, Cow::Borrowed(request)));
                    }
                } else {
                    debug!("No installation found for request `{}`", request);
                    unsatisfied.push((request_index, Cow::Borrowed(request)));
                }
            } else if let Some(installation) = existing_installations
                .iter()
                .find(|inst| request.matches_installation(inst))
            {
                debug!("Found `{}` for request `{}`", installation.key(), request);
                satisfied.push(installation);
            } else {
                debug!("No installation found for request `{}`", request);
                unsatisfied.push((request_index, Cow::Borrowed(request)));
            }
        }

        (satisfied, unsatisfied)
    };

    let report_requests = if report.is_some() {
        resolved_report_requests(&requests, &unsatisfied, reinstall)
    } else {
        Vec::new()
    };
    if let Some(report) = report.as_mut() {
        *report = upgrade_report(
            &report_requests,
            &existing_installations,
            &satisfied,
            &[],
            &changelog,
            &[],
            false,
        );
    }

    // For all satisfied installs, bytecode compile them now before any future
    // early return.
    if let Some(ref sender) = bytecode_compilation_sender {
        satisfied
            .iter()
            .copied()
            .cloned()
            .try_for_each(|installation| {
                sender
                    .send(installation)
                    .map_err(|err| anyhow::anyhow!(err))
            })?;
    }

    // Check if Python downloads are banned
    if matches!(python_downloads, PythonDownloads::Never) && !unsatisfied.is_empty() {
        writeln!(
            printer.stderr(),
            "Python downloads are not allowed (`python-downloads = \"never\"`). Change to `python-downloads = \"manual\"` to allow explicit installs.",
        )?;
        if let Some(report) = report.as_mut() {
            *report = upgrade_report(
                &report_requests,
                &existing_installations,
                &satisfied,
                &[],
                &changelog,
                &[],
                true,
            );
        }
        return Ok(ExitStatus::Failure);
    }

    // Find downloads for the requests
    let downloads = unsatisfied
        .iter()
        .map(|(_, request)| request)
        .inspect(|request| {
            debug!(
                "Found download `{}` for request `{}`",
                request.download, request,
            );
        })
        .map(|request| request.download)
        // Ensure we only download each version once
        .unique_by(|download| download.key())
        .collect::<Vec<_>>();

    // Download and unpack the Python versions concurrently
    let reporter = PythonDownloadReporter::new(printer, Some(downloads.len() as u64));
    let replacements = changelog.existing.clone();

    let tasks = buffered_downloads(
        &downloads,
        async |download| {
            (
                *download,
                download
                    .fetch_with_retry(
                        &client,
                        &retry_policy,
                        installations_dir,
                        &scratch_dir,
                        reinstall || replacements.contains(download.key()),
                        python_install_mirror.as_deref(),
                        pypy_install_mirror.as_deref(),
                        Some(&reporter),
                    )
                    .await,
            )
        },
        concurrency.downloads,
        if report.is_some() {
            bytecode_compilation_sender.as_ref()
        } else {
            None
        },
    );

    let mut errors = vec![];
    let mut downloaded = Vec::with_capacity(downloads.len());
    let mut not_finalized = Vec::new();
    let mut requests_by_new_installation = BTreeMap::new();
    let settle_on_error = report.is_some();
    settle_downloads(tasks, settle_on_error, |(download, result)| {
        match result {
            Ok(download_result) => {
                // Downloads finalize installation contents before publishing the directory.
                // An existing installation still needs any missing metadata repaired.
                let (path, finalized_in_download) = match download_result {
                    DownloadResult::AlreadyAvailable(path) => (path, false),
                    DownloadResult::Fetched(path) => (path, true),
                };

                let installation = ManagedPythonInstallation::new(path, download);
                record_downloaded_installation(
                    report,
                    bytecode_compilation_sender.as_ref(),
                    &installation,
                )?;
                changelog.installed.insert(installation.key().clone());
                for request in &requests {
                    // Take note of which installations satisfied which requests
                    if request.matches_installation(&installation) {
                        requests_by_new_installation
                            .entry(installation.key().clone())
                            .or_insert(Vec::new())
                            .push(request);
                    }
                }
                if changelog.existing.contains(installation.key()) {
                    changelog.uninstalled.insert(installation.key().clone());
                }
                if !finalized_in_download {
                    not_finalized.push(installation.clone());
                }
                downloaded.push(installation.clone());
            }
            Err(err) => {
                if let Some(report) = report.as_mut() {
                    report.fail_installation(
                        download.key(),
                        UpgradeErrorKind::Download,
                        format!("{err:#}"),
                    );
                }
                errors.push((
                    InstallErrorKind::DownloadUnpack,
                    download.key().clone(),
                    anyhow::Error::new(err),
                ));
            }
        }
        Ok(())
    })
    .await?;
    if report.is_some()
        && bytecode_compilation_sender
            .as_ref()
            .is_some_and(mpsc::UnboundedSender::is_closed)
    {
        // The compiler can fail before any download is admitted or returns successfully. Its
        // original error is returned by the outer join, without starting the remaining lifecycle.
        return Ok(ExitStatus::Failure);
    }

    let installations: Vec<_> = downloaded.iter().chain(satisfied.iter().copied()).collect();
    if let Some(report) = report.as_mut() {
        *report = upgrade_report(
            &report_requests,
            &existing_installations,
            &installations,
            &downloaded,
            &changelog,
            &errors,
            false,
        );
    }

    let bin_dir = if matches!(bin, Some(false)) {
        None
    } else {
        Some(python_executable_dir()?)
    };

    // Repair installations that did not pass through the staged download path.
    for installation in not_finalized.iter().chain(satisfied.iter().copied()) {
        if let Err(err) = installation.finalize() {
            if let Some(report) = report.as_mut() {
                report.fail_installation(
                    installation.key(),
                    UpgradeErrorKind::Finalize,
                    format!("{err:#}"),
                );
            }
            return Err(err.into());
        }
    }

    let minor_versions =
        PythonInstallationMinorVersionKey::highest_installations_by_minor_version_key(
            installations
                .iter()
                .copied()
                .chain(existing_installations.iter()),
        );

    // Retargeting a minor-version directory can change the apparent owner of an existing
    // executable. Keep its previous target and ownership for replacement policy and reporting.
    let mut bin_links = BinLinkStates::default();
    if let Some(bin_dir) = bin_dir.as_ref()
        && minor_versions.values().any(|installation| {
            PythonMinorVersionLink::from_installation(installation)
                .is_some_and(|link| !link.exists())
        })
    {
        bin_links = BinLinkStates::capture(
            bin_dir,
            &installations,
            &existing_installations,
            default,
            is_default_install,
            preview,
        );
    }

    // Executable links may point through these directories. Prepare those targets before writing
    // entry points so a failed minor-version link cannot publish a new unresolved executable.
    for installation in minor_versions.values() {
        if let Err(err) = installation.ensure_minor_version_link() {
            if let Some(report) = report.as_mut() {
                report.fail_installation(
                    installation.key(),
                    UpgradeErrorKind::MinorVersionLink,
                    format!("{err:#}"),
                );
            }
            return Err(err.into());
        }
    }

    for installation in &installations {
        let upgradeable = (default || is_default_install)
            || requested_minor_versions.contains(&installation.key().version().python_version());

        if let Some(bin_dir) = bin_dir.as_ref() {
            create_bin_links(
                installation,
                bin_dir,
                reinstall,
                force,
                default,
                upgradeable,
                matches!(
                    upgrade,
                    PythonUpgrade::Enabled(PythonUpgradeSource::Upgrade)
                ),
                is_default_install,
                &existing_installations,
                &installations,
                &mut bin_links,
                &mut changelog,
                &mut errors,
                preview,
            );
        }

        if !matches!(registry, Some(false)) {
            #[cfg(windows)]
            {
                match uv_python::windows_registry::create_registry_entry(installation) {
                    Ok(()) => {}
                    Err(err) => {
                        errors.push((
                            InstallErrorKind::Registry,
                            installation.key().clone(),
                            err.into(),
                        ));
                    }
                }
            }
        }
    }

    if let Some(report) = report.as_mut() {
        *report = upgrade_report(
            &report_requests,
            &existing_installations,
            &installations,
            &downloaded,
            &changelog,
            &errors,
            false,
        );
    }

    if changelog.installed.is_empty() && errors.is_empty() {
        if is_default_install {
            if matches!(
                upgrade,
                PythonUpgrade::Enabled(PythonUpgradeSource::Install)
            ) {
                writeln!(
                    printer.stderr(),
                    "The default Python installation is already on the latest supported patch release. Use `uv python install <request>` to install another version.",
                )?;
            } else {
                writeln!(
                    printer.stderr(),
                    "Python is already installed. Use `uv python install <request>` to install another version.",
                )?;
            }
        } else if matches!(
            upgrade,
            PythonUpgrade::Enabled(PythonUpgradeSource::Upgrade)
        ) && requests.is_empty()
        {
            writeln!(
                printer.stderr(),
                "There are no installed versions to upgrade"
            )?;
        } else if let [request] = requests.as_slice() {
            // Convert to the inner request
            let request = &request.request;
            if is_unspecified_upgrade {
                writeln!(
                    printer.stderr(),
                    "All versions already on latest supported patch release"
                )?;
            } else if matches!(upgrade, PythonUpgrade::Enabled(_)) {
                writeln!(
                    printer.stderr(),
                    "{request} is already on the latest supported patch release"
                )?;
            } else {
                writeln!(printer.stderr(), "{request} is already installed")?;
            }
        } else {
            if matches!(upgrade, PythonUpgrade::Enabled(_)) {
                if is_unspecified_upgrade {
                    writeln!(
                        printer.stderr(),
                        "All versions already on latest supported patch release"
                    )?;
                } else {
                    writeln!(
                        printer.stderr(),
                        "All requested versions already on latest supported patch release"
                    )?;
                }
            } else {
                writeln!(printer.stderr(), "All requested versions already installed")?;
            }
        }
        return Ok(ExitStatus::Success);
    }

    if !changelog.installed.is_empty() {
        for install_key in &changelog.installed {
            // Make a note if the selected python is non-native for the architecture, if none of the
            // matching user requests were explicit.
            //
            // Emscripten is exempted as it is always "emulated".
            let native_arch = Arch::from_env();
            if install_key.arch().family() != native_arch.family()
                && !install_key.os().is_emscripten()
            {
                let not_explicit =
                    requests_by_new_installation
                        .get(install_key)
                        .and_then(|requests| {
                            let all_non_explicit = requests.iter().all(|request| {
                                if let PythonRequest::Key(key) = &request.request {
                                    !matches!(key.arch(), Some(ArchRequest::Explicit(_)))
                                } else {
                                    true
                                }
                            });
                            if all_non_explicit {
                                requests.iter().next()
                            } else {
                                None
                            }
                        });
                if let Some(not_explicit) = not_explicit {
                    let native_request =
                        not_explicit.download_request.clone().with_arch(native_arch);
                    writeln!(
                        printer.stderr(),
                        "{} uv selected a Python distribution with an emulated architecture ({}) for your platform because support for the native architecture ({}) is not yet mature; to override this behaviour, request the native architecture explicitly with: {}",
                        "note:".bold(),
                        install_key.arch(),
                        native_arch,
                        native_request
                    )?;
                }
            }
        }
        if changelog.installed.len() == 1 {
            let installed = changelog.installed.iter().next().unwrap();
            // Ex) "Installed Python 3.9.7 in 1.68s"
            writeln!(
                printer.stderr(),
                "{}",
                format!(
                    "Installed {} {}",
                    format!("Python {}", installed.version()).bold(),
                    format!("in {}", elapsed(start.elapsed())).dimmed()
                )
                .dimmed()
            )?;
        } else {
            // Ex) "Installed 2 versions in 1.68s"
            writeln!(
                printer.stderr(),
                "{}",
                format!(
                    "Installed {} {}",
                    format!("{} versions", changelog.installed.len()).bold(),
                    format!("in {}", elapsed(start.elapsed())).dimmed()
                )
                .dimmed()
            )?;
        }

        for event in changelog.events() {
            let executables = format_executables(&event, &changelog.installed_executables);
            match event.kind {
                ChangeEventKind::Added => {
                    writeln!(
                        printer.stderr(),
                        " {} {}{executables}",
                        "+".green(),
                        event.key.bold()
                    )?;
                }
                ChangeEventKind::Removed => {
                    writeln!(
                        printer.stderr(),
                        " {} {}{executables}",
                        "-".red(),
                        event.key.bold()
                    )?;
                }
                ChangeEventKind::Reinstalled => {
                    writeln!(
                        printer.stderr(),
                        " {} {}{executables}",
                        "~".yellow(),
                        event.key.bold(),
                    )?;
                }
            }
        }

        if let Some(bin_dir) = bin_dir.as_ref() {
            warn_if_not_on_path(bin_dir);
        }
    }

    if !errors.is_empty() {
        // If there are only side-effect install errors and the user didn't opt-in, we're only going
        // to warn
        let fatal = !errors.iter().all(|(kind, _, _)| match kind {
            InstallErrorKind::Bin => bin.is_none(),
            InstallErrorKind::Registry => registry.is_none(),
            InstallErrorKind::DownloadUnpack => false,
        });

        for (kind, key, err) in errors
            .into_iter()
            .sorted_unstable_by(|(_, key_a, _), (_, key_b, _)| key_a.cmp(key_b))
        {
            match kind {
                InstallErrorKind::DownloadUnpack => {
                    write_error_chain_with_options(
                        err.context(format!("Failed to install {key}")).as_ref(),
                        &Hints::none(),
                        ErrorOptions::default().with_stream(printer.stderr()),
                    )?;
                }
                InstallErrorKind::Bin => {
                    let (level, color) = match bin {
                        None => ("warning", AnsiColors::Yellow),
                        Some(false) => continue,
                        Some(true) => ("error", AnsiColors::Red),
                    };

                    write_error_chain_with_options(
                        err.context(format!("Failed to install executable for {key}"))
                            .as_ref(),
                        &Hints::none(),
                        ErrorOptions::default()
                            .with_level(level)
                            .with_color(color)
                            .with_stream(printer.stderr()),
                    )?;
                }
                InstallErrorKind::Registry => {
                    let (level, color) = match registry {
                        None => ("warning", AnsiColors::Yellow),
                        Some(false) => continue,
                        Some(true) => ("error", AnsiColors::Red),
                    };

                    trace!("Error trace: {err:?}");
                    write_error_chain_with_options(
                        err.context(format!("Failed to create registry entry for {key}"))
                            .as_ref(),
                        &Hints::none(),
                        ErrorOptions::default()
                            .with_level(level)
                            .with_color(color)
                            .with_stream(printer.stderr()),
                    )?;
                }
            }
        }

        if fatal {
            return Ok(ExitStatus::Failure);
        }
    }

    Ok(ExitStatus::Success)
}

/// Stop admitting downloads when the bytecode receiver closes, then settle the admitted work.
///
/// Without a receiver, this has the same lazy admission as [`StreamExt::buffer_unordered`].
fn buffered_downloads<I, F, Fut, T>(
    downloads: I,
    fetch: F,
    concurrency: usize,
    bytecode_sender: Option<&mpsc::UnboundedSender<T>>,
) -> impl Stream<Item = Fut::Output> + Unpin
where
    I: IntoIterator,
    F: FnMut(I::Item) -> Fut,
    Fut: Future,
{
    futures::stream::iter(downloads)
        .take_while(move |_| ready(!bytecode_sender.is_some_and(mpsc::UnboundedSender::is_closed)))
        .map(fetch)
        .buffer_unordered(concurrency)
}

/// Drain admitted downloads before returning the first observation error in JSON mode.
async fn settle_downloads<S, F>(mut tasks: S, settle_on_error: bool, mut observe: F) -> Result<()>
where
    S: Stream + Unpin,
    F: FnMut(S::Item) -> Result<()>,
{
    let mut first_error = None;
    while let Some(result) = tasks.next().await {
        if let Err(err) = observe(result) {
            if !settle_on_error {
                return Err(err);
            }
            if first_error.is_none() {
                first_error = Some(err);
            }
        }
    }
    if let Some(err) = first_error {
        return Err(err);
    }
    Ok(())
}

/// Record the published directory before a failed bytecode send can stop its observation.
fn record_downloaded_installation(
    report: &mut Option<UpgradeReport>,
    bytecode_sender: Option<&mpsc::UnboundedSender<ManagedPythonInstallation>>,
    installation: &ManagedPythonInstallation,
) -> Result<()> {
    if let Some(report) = report.as_mut() {
        report.record_installation(installation);
    }
    if let Some(sender) = bytecode_sender {
        sender
            .send(installation.clone())
            .map_err(|err| anyhow::anyhow!(err))?;
    }
    Ok(())
}

/// Expand `--reinstall any` into the exact-key operations selected by the installer.
fn resolved_report_requests<'a>(
    requests: &[InstallRequest<'a>],
    unsatisfied: &[(usize, Cow<'_, InstallRequest<'a>>)],
    reinstall: bool,
) -> Vec<InstallRequest<'a>> {
    requests
        .iter()
        .enumerate()
        .flat_map(|(request_index, request)| {
            if reinstall && matches!(request.request, PythonRequest::Any) {
                unsatisfied
                    .iter()
                    .filter(|(origin, _)| *origin == request_index)
                    .map(|(_, resolved)| resolved)
                    .unique_by(|resolved| resolved.download.key())
                    .map(|resolved| InstallRequest {
                        request: request.request.clone(),
                        download_request: resolved.download_request.clone(),
                        download: resolved.download,
                    })
                    .collect::<Vec<_>>()
            } else {
                vec![request.clone()]
            }
        })
        .collect()
}

/// Describe completed installation and executable changes while the install lock is held.
fn upgrade_report(
    requests: &[InstallRequest<'_>],
    before: &[ManagedPythonInstallation],
    completed: &[&ManagedPythonInstallation],
    downloaded: &[ManagedPythonInstallation],
    changelog: &Changelog,
    errors: &[(InstallErrorKind, PythonInstallationKey, Error)],
    downloads_disabled: bool,
) -> UpgradeReport {
    let mut upgrades = Vec::with_capacity(requests.len());
    for request in requests {
        let key = request.download.key();
        let mut from = before
            .iter()
            .filter(|installation| request.matches_installation(installation))
            .map(InstallationReport::from)
            .collect::<Vec<_>>();
        from.sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.build.cmp(&b.build)));
        let to = completed
            .iter()
            .find(|installation| installation.key() == key)
            .map(|installation| InstallationReport::from(*installation));
        let executables = changelog
            .report
            .iter()
            .flat_map(|report| report.executables.values())
            .filter(|change| change.to.key == key.to_string())
            .cloned()
            .collect::<Vec<_>>();
        let mut errors = errors
            .iter()
            .filter(|(_, failed_key, _)| failed_key == key)
            .map(|(kind, _, error)| UpgradeError {
                kind: match kind {
                    InstallErrorKind::DownloadUnpack => UpgradeErrorKind::Download,
                    InstallErrorKind::Bin => UpgradeErrorKind::Executable,
                    InstallErrorKind::Registry => UpgradeErrorKind::Registry,
                },
                message: format!("{error:#}"),
            })
            .collect::<Vec<_>>();
        if downloads_disabled && to.is_none() {
            errors.push(UpgradeError {
                kind: UpgradeErrorKind::DownloadsDisabled,
                message: "Python downloads are not allowed".to_owned(),
            });
        }
        errors.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.message.cmp(&b.message)));
        let outcome = installation_outcome(
            &from,
            to.as_ref(),
            downloaded
                .iter()
                .any(|installation| installation.key() == key),
            !executables.is_empty(),
            !errors.is_empty(),
        );
        upgrades.push(UpgradeEntry {
            selected_key: key.to_string(),
            request: request.python_request().to_canonical_string().into_owned(),
            outcome,
            from,
            to,
            executables,
            errors,
        });
    }
    upgrades.sort_by(|a, b| {
        a.request
            .cmp(&b.request)
            .then_with(|| a.selected_key.cmp(&b.selected_key))
    });
    UpgradeReport {
        upgrades,
        ..UpgradeReport::default()
    }
}

fn write_upgrade_report(
    format: PythonUpgradeFormat,
    report: &UpgradeReport,
    printer: Printer,
) -> Result<()> {
    if matches!(format, PythonUpgradeFormat::Json) {
        writeln!(
            printer.stdout_important_raw(),
            "{}",
            serde_json::to_string_pretty(report)?
        )?;
    }
    Ok(())
}

#[derive(Debug)]
struct BinLinkState {
    encoded_target: PathBuf,
    owner: Option<ManagedPythonInstallation>,
    // `uv python upgrade` only updates entry points that already exist.
    path_exists: bool,
    // Broken Unix links can be replaced without `--force`; unknown Windows launchers cannot.
    valid_link: bool,
}

/// Executable-link state from before minor-version directories were retargeted.
#[derive(Debug, Default)]
struct BinLinkStates {
    states: FxHashMap<PathBuf, BinLinkState>,
}

impl BinLinkStates {
    fn capture(
        bin: &Path,
        installations: &[&ManagedPythonInstallation],
        existing_installations: &[ManagedPythonInstallation],
        default: bool,
        is_default_install: bool,
        preview: Preview,
    ) -> Self {
        let mut states = FxHashMap::default();
        for name in installations
            .iter()
            .flat_map(|installation| {
                bin_link_names(installation.key(), default, is_default_install, preview)
            })
            .unique()
        {
            let path = bin.join(name);
            let Some(encoded_target) = read_bin_link_target(&path) else {
                continue;
            };
            let owner = resolve_bin_link_target(&path, &encoded_target)
                .and_then(|target| {
                    installations
                        .iter()
                        .copied()
                        .chain(existing_installations.iter())
                        .find(|installation| installation.executable(false) == target)
                })
                .cloned();
            let state = BinLinkState {
                encoded_target,
                owner,
                path_exists: path.try_exists().unwrap_or_default(),
                valid_link: is_valid_unmanaged_bin_link(&path),
            };
            states.insert(path, state);
        }
        Self { states }
    }

    /// Only use captured ownership while the executable still encodes the same target.
    fn get(&self, path: &Path) -> Option<&BinLinkState> {
        let state = self.states.get(path)?;
        let encoded_target = read_bin_link_target(path)?;
        (encoded_target == state.encoded_target).then_some(state)
    }

    fn record(&mut self, path: &Path, executable: &Path, installation: &ManagedPythonInstallation) {
        self.states.insert(
            path.to_path_buf(),
            BinLinkState {
                // Windows launchers store simplified paths in their metadata.
                encoded_target: dunce::simplified(executable).to_path_buf(),
                owner: Some(installation.clone()),
                path_exists: path.try_exists().unwrap_or_default(),
                valid_link: true,
            },
        );
    }
}

fn bin_link_names(
    key: &PythonInstallationKey,
    default: bool,
    is_default_install: bool,
    preview: Preview,
) -> Vec<String> {
    // TODO(zanieb): We want more feedback on the `is_default_install` behavior before stabilizing
    // it. In particular, it may be confusing because it does not apply when versions are loaded
    // from a `.python-version` file.
    let should_create_default_links =
        default || (is_default_install && preview.is_enabled(PreviewFeature::PythonInstallDefault));

    if should_create_default_links {
        vec![
            key.executable_name_minor(),
            key.executable_name_major(),
            key.executable_name(),
        ]
    } else {
        vec![key.executable_name_minor()]
    }
}

/// Link the binaries of a managed Python installation to the bin directory.
///
/// This function is fallible, but errors are pushed to `errors` instead of being thrown.
#[expect(clippy::fn_params_excessive_bools)]
fn create_bin_links(
    installation: &ManagedPythonInstallation,
    bin: &Path,
    reinstall: bool,
    force: bool,
    default: bool,
    upgradeable: bool,
    upgrade: bool,
    is_default_install: bool,
    existing_installations: &[ManagedPythonInstallation],
    installations: &[&ManagedPythonInstallation],
    bin_links: &mut BinLinkStates,
    changelog: &mut Changelog,
    errors: &mut Vec<(InstallErrorKind, PythonInstallationKey, Error)>,
    preview: Preview,
) {
    let mut existing_unmanaged = Vec::new();

    for target in bin_link_names(installation.key(), default, is_default_install, preview) {
        let target = bin.join(target);
        if upgrade
            && !bin_links.get(&target).map_or_else(
                || target.try_exists().unwrap_or_default(),
                |state| state.path_exists,
            )
        {
            continue;
        }
        let executable = if upgradeable {
            if let Some(minor_version_link) =
                PythonMinorVersionLink::from_installation(installation)
            {
                minor_version_link.symlink_executable.clone()
            } else {
                installation.executable(false)
            }
        } else {
            installation.executable(false)
        };

        match create_link_to_executable(&target, PythonExecutable::console(&executable)) {
            Ok(()) => {
                changelog.record_executable_change(&target, None, installation);
                bin_links.record(&target, &executable, installation);
                debug!(
                    "Installed executable at `{}` for {}",
                    target.simplified_display(),
                    installation.key(),
                );
                changelog.installed.insert(installation.key().clone());
                changelog
                    .installed_executables
                    .entry(installation.key().clone())
                    .or_default()
                    .insert(target.clone());
            }
            Err(uv_python::managed::Error::LinkExecutable(err))
                if err.kind() == ErrorKind::AlreadyExists =>
            {
                debug!(
                    "Inspecting existing executable at `{}`",
                    target.simplified_display()
                );

                // A minor-version link may already point to the new patch. Recheck the encoded
                // executable target before relying on its owner from before that retargeting.
                let captured = bin_links.get(&target);
                let valid_link = captured.map(|state| state.valid_link);
                let existing = captured
                    .map(|state| state.owner.as_ref())
                    .unwrap_or_else(|| {
                        find_matching_bin_link(
                            installations
                                .iter()
                                .copied()
                                .chain(existing_installations.iter()),
                            &target,
                        )
                    })
                    .cloned();

                match existing.as_ref() {
                    None => {
                        // Determine if the link is valid, i.e., if it points to an existing
                        // Python we don't manage. On Windows, we just assume it is valid because
                        // symlinks are not common for Python interpreters.
                        let valid_link =
                            valid_link.unwrap_or_else(|| is_valid_unmanaged_bin_link(&target));

                        // There's an existing executable we don't manage, require `--force`
                        if valid_link {
                            if !force {
                                if upgrade {
                                    warn_user!(
                                        "Executable already exists at `{}` but is not managed by uv; use `uv python install {}.{}{} --force` to replace it",
                                        target.simplified_display(),
                                        installation.key().major(),
                                        installation.key().minor(),
                                        installation.key().variant().display_suffix()
                                    );
                                } else {
                                    // Defer reporting to allow grouping.
                                    existing_unmanaged.push(target.clone());
                                }
                                continue;
                            }
                            debug!(
                                "Replacing existing executable at `{}` due to `--force`",
                                target.simplified_display()
                            );
                        } else {
                            debug!(
                                "Replacing broken symlink at `{}`",
                                target.simplified_display()
                            );
                        }
                    }
                    Some(existing) if existing == installation => {
                        // The existing link points to the same installation, so we're done unless
                        // they requested we reinstall
                        if !(reinstall || force) {
                            debug!(
                                "Executable at `{}` is already for `{}`",
                                target.simplified_display(),
                                installation.key(),
                            );
                            continue;
                        }
                        debug!(
                            "Replacing existing executable for `{}` at `{}`",
                            installation.key(),
                            target.simplified_display(),
                        );
                    }
                    Some(existing) => {
                        // The existing link points to a different installation, check if it
                        // is reasonable to replace
                        if force {
                            debug!(
                                "Replacing existing executable for `{}` at `{}` with executable for `{}` due to `--force` flag",
                                existing.key(),
                                target.simplified_display(),
                                installation.key(),
                            );
                        } else {
                            if installation.is_upgrade_of(existing) {
                                debug!(
                                    "Replacing existing executable for `{}` at `{}` with executable for `{}` since it is an upgrade",
                                    existing.key(),
                                    target.simplified_display(),
                                    installation.key(),
                                );
                            } else if default {
                                debug!(
                                    "Replacing existing executable for `{}` at `{}` with executable for `{}` since `--default` was requested`",
                                    existing.key(),
                                    target.simplified_display(),
                                    installation.key(),
                                );
                            } else {
                                debug!(
                                    "Executable already exists for `{}` at `{}`. Use `--force` to replace it",
                                    existing.key(),
                                    target.simplified_display()
                                );
                                continue;
                            }
                        }
                    }
                }

                // Replace the existing link
                if let Err(err) =
                    replace_link_to_executable(&target, PythonExecutable::console(&executable))
                {
                    errors.push((
                        InstallErrorKind::Bin,
                        installation.key().clone(),
                        anyhow::anyhow!(
                            "Failed to replace link at `{}`: {err}",
                            target.simplified_display()
                        ),
                    ));
                    continue;
                }

                if let Some(existing) = existing.as_ref() {
                    // Ensure we do not report installation of this executable for an existing
                    // key if we undo it
                    changelog
                        .installed_executables
                        .entry(existing.key().clone())
                        .or_default()
                        .remove(&target);
                }

                changelog.record_executable_change(&target, existing.as_ref(), installation);
                bin_links.record(&target, &executable, installation);
                debug!(
                    "Updated executable at `{}` to {}",
                    target.simplified_display(),
                    installation.key(),
                );
                changelog.installed.insert(installation.key().clone());
                changelog
                    .installed_executables
                    .entry(installation.key().clone())
                    .or_default()
                    .insert(target.clone());
            }
            Err(err) => {
                errors.push((
                    InstallErrorKind::Bin,
                    installation.key().clone(),
                    Error::new(err),
                ));
            }
        }
    }

    match existing_unmanaged.as_slice() {
        [] => {}
        [executable] => errors.push((
            InstallErrorKind::Bin,
            installation.key().clone(),
            anyhow::anyhow!(
                "Executable already exists at `{}` but is not managed by uv; use `--force` to replace it",
                executable.simplified_display()
            ),
        )),
        executables => errors.push((
            InstallErrorKind::Bin,
            installation.key().clone(),
            anyhow::anyhow!(
                "Executables {} already exist in `{}` but are not managed by uv; use `--force` to replace them",
                conjunction(
                    executables
                        .iter()
                        .filter_map(|path| path.file_name())
                        .map(|name| format!("`{}`", name.to_string_lossy()))
                        .collect(),
                ),
                bin.simplified_display()
            ),
        )),
    }
}

/// Attempt to compile the bytecode for a [`ManagedPythonInstallation`]'s stdlib
async fn compile_stdlib_bytecode(
    installation: &ManagedPythonInstallation,
    concurrency: &Concurrency,
    cache: &Cache,
) -> Result<Option<(usize, std::time::Duration)>> {
    let start = std::time::Instant::now();

    // Explicit matching so this heuristic is updated for future additions
    match installation.implementation() {
        ImplementationName::Pyodide => return Ok(None),
        ImplementationName::GraalPy | ImplementationName::PyPy | ImplementationName::CPython => (),
    }

    let interpreter = Interpreter::query(installation.executable(false), cache)
        .context("Couldn't locate the interpreter")?;

    // Ensure the bytecode compilation occurs in the correct place, in case the installed
    // interpreter reports a weird stdlib path.
    let interpreter_path = installation.path().canonicalize()?;
    let stdlib_path = match interpreter.stdlib().canonicalize() {
        Ok(path) if path.starts_with(&interpreter_path) => path,
        _ => {
            warn!(
                "The stdlib path for {} ({}) is not a subdirectory of its installation path ({}).",
                installation.key(),
                interpreter.stdlib().display(),
                interpreter_path.display()
            );
            return Ok(None);
        }
    };

    let files = uv_installer::compile_tree(
        &stdlib_path,
        &installation.executable(false),
        concurrency,
        cache.root(),
    )
    .await
    .with_context(|| format!("Error compiling bytecode in: {}", stdlib_path.display()))?;
    if files == 0 {
        return Ok(None);
    }
    Ok(Some((files, start.elapsed())))
}

pub(crate) fn format_executables(
    event: &ChangeEvent,
    executables: &FxHashMap<PythonInstallationKey, FxHashSet<PathBuf>>,
) -> String {
    let Some(installed) = executables.get(&event.key) else {
        return String::new();
    };

    if installed.is_empty() {
        return String::new();
    }

    let names = installed
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy())
        // Do not include the `.exe` during comparisons, it can change the ordering
        .sorted_unstable_by(|a, b| a.trim_end_matches(".exe").cmp(b.trim_end_matches(".exe")))
        .join(", ");

    format!(" ({names})")
}

fn warn_if_not_on_path(bin: &Path) {
    if !Shell::contains_path(bin) {
        if let Some(shell) = Shell::from_env() {
            if let Some(command) = shell.prepend_path(bin) {
                if shell.supports_update() {
                    warn_user!(
                        "`{}` is not on your PATH. To use installed Python executables, run `{}` or `{}`.",
                        bin.simplified_display().cyan(),
                        command.green(),
                        "uv python update-shell".green()
                    );
                } else {
                    warn_user!(
                        "`{}` is not on your PATH. To use installed Python executables, run `{}`.",
                        bin.simplified_display().cyan(),
                        command.green()
                    );
                }
            } else {
                warn_user!(
                    "`{}` is not on your PATH. To use installed Python executables, add the directory to your PATH.",
                    bin.simplified_display().cyan(),
                );
            }
        } else {
            warn_user!(
                "`{}` is not on your PATH. To use installed Python executables, add the directory to your PATH.",
                bin.simplified_display().cyan(),
            );
        }
    }
}

/// Find the [`ManagedPythonInstallation`] corresponding to an executable link installed at the
/// given path, if any.
///
/// Will resolve symlinks on Unix. On Windows, will resolve the target link for a trampoline.
fn find_matching_bin_link<'a>(
    mut installations: impl Iterator<Item = &'a ManagedPythonInstallation>,
    path: &Path,
) -> Option<&'a ManagedPythonInstallation> {
    let encoded_target = read_bin_link_target(path)?;
    let target = resolve_bin_link_target(path, &encoded_target)?;
    installations.find(|installation| installation.executable(false) == target)
}

/// Read the target stored in a Unix symlink or a Windows Python launcher.
fn read_bin_link_target(path: &Path) -> Option<PathBuf> {
    if cfg!(unix) {
        fs_err::read_link(path).ok()
    } else if cfg!(windows) {
        let launcher = Launcher::try_from_path(path).ok()??;
        if !matches!(launcher.kind, LauncherKind::Python) {
            return None;
        }
        Some(launcher.python_path)
    } else {
        unreachable!("Only Unix and Windows are supported")
    }
}

fn resolve_bin_link_target(path: &Path, encoded_target: &Path) -> Option<PathBuf> {
    if cfg!(unix) {
        fs_err::canonicalize(path.parent()?.join(encoded_target)).ok()
    } else if cfg!(windows) {
        dunce::canonicalize(encoded_target).ok()
    } else {
        unreachable!("Only Unix and Windows are supported")
    }
}

fn is_valid_unmanaged_bin_link(path: &Path) -> bool {
    cfg!(windows)
        || path
            .read_link()
            // Resolve relative targets from the executable's directory.
            .and_then(|_| path.try_exists())
            .inspect_err(|err| {
                debug!("Failed to inspect executable with error: {err}");
            })
            // If we can't verify the link, assume it is valid.
            .unwrap_or(true)
}

/// Check if a download's build version matches an installation's build version.
///
/// Returns `true` if the build versions match (no upgrade needed), `false` if an upgrade is needed.
fn matches_build(download_build: Option<&str>, installation_build: Option<&str>) -> bool {
    match (download_build, installation_build) {
        // Both have build, check if they match
        (Some(d), Some(i)) => compare_build_versions(d, i) == std::cmp::Ordering::Equal,
        // Legacy installation without BUILD file needs upgrade
        (Some(_), None) => false,
        // Download doesn't have build info, assume matches
        (None, _) => true,
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;
    use std::io::ErrorKind;
    use std::path::Path;

    use crate::commands::python::upgrade_report::{
        UpgradeEntry, UpgradeErrorKind, UpgradeOutcome, UpgradeReport,
    };
    use anyhow::{Context, Result};
    use futures::{StreamExt, join};
    use tokio::sync::{mpsc, oneshot};
    use uv_preview::Preview;
    use uv_python::managed::{
        ManagedPythonInstallation, ManagedPythonInstallations, PythonExecutable,
        PythonMinorVersionLink, create_link_to_executable, platform_key_from_env,
        replace_link_to_executable,
    };

    use super::{
        BinLinkStates, Changelog, InstallErrorKind, buffered_downloads, create_bin_links,
        find_matching_bin_link, read_bin_link_target, record_downloaded_installation,
        settle_downloads,
    };

    #[tokio::test]
    async fn buffered_downloads_settle_publications_after_bytecode_failure() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let root = dunce::canonicalize(temp_dir.path())?;
        let published = root.join("published");
        fs_err::create_dir(&published)?;
        let staged = root.join("staged");
        let first = create_installation(&staged, "3.12.8")?;
        let second = create_installation(&staged, "3.12.9")?;
        let missing = create_installation(&staged, "3.12.10")?;
        let unstarted = create_installation(&staged, "3.12.11")?;
        let keys = [first.key(), second.key(), missing.key(), unstarted.key()];
        let names = keys.iter().map(ToString::to_string).collect::<Vec<_>>();
        let mut report = Some(UpgradeReport {
            upgrades: names
                .iter()
                .map(|key| UpgradeEntry {
                    selected_key: key.clone(),
                    request: key.clone(),
                    outcome: UpgradeOutcome::NotCompleted,
                    from: Vec::new(),
                    to: None,
                    executables: Vec::new(),
                    errors: Vec::new(),
                })
                .collect(),
            ..UpgradeReport::default()
        });
        let published_installations =
            ManagedPythonInstallations::from_settings(Some(published.clone()))?;

        let (release_first, first_ready) = oneshot::channel();
        let (release_second, second_ready) = oneshot::channel();
        let (release_missing, missing_ready) = oneshot::channel();
        let (release_unstarted, unstarted_ready) = oneshot::channel();
        let downloads = [
            (
                0,
                first.path().to_path_buf(),
                published.join(&names[0]),
                first_ready,
            ),
            (
                1,
                second.path().to_path_buf(),
                published.join(&names[1]),
                second_ready,
            ),
            (
                2,
                missing.path().join("absent"),
                published.join(&names[2]),
                missing_ready,
            ),
            (
                3,
                unstarted.path().to_path_buf(),
                published.join(&names[3]),
                unstarted_ready,
            ),
        ];
        let (started_sender, mut started_receiver) = mpsc::unbounded_channel();
        let (bytecode_sender, mut bytecode_receiver) = mpsc::unbounded_channel();
        let tasks = buffered_downloads(
            downloads,
            |(index, staged, published, release)| {
                let started_sender = started_sender.clone();
                let published_installations = published_installations.clone();
                async move {
                    started_sender.send(index)?;
                    release.await.context("publication was not released")?;
                    let result = match uv_fs::rename_with_retry(staged, &published).await {
                        Ok(()) => Ok(published_installations
                            .find_all()?
                            .find(|installation| installation.path() == published)
                            .context("missing published installation")?),
                        Err(err) => Err(err),
                    };
                    Ok::<_, anyhow::Error>((index, result))
                }
            },
            3,
            Some(&bytecode_sender),
        );
        let (failed_send_sender, failed_send_receiver) = oneshot::channel();
        let mut failed_send_sender = Some(failed_send_sender);
        let mut observed_errors = Vec::new();
        let installer = settle_downloads(tasks, true, |result| {
            let (index, result) = result?;
            match result {
                Ok(installation) => {
                    let result = record_downloaded_installation(
                        &mut report,
                        Some(&bytecode_sender),
                        &installation,
                    );
                    if result.is_err()
                        && let Some(sender) = failed_send_sender.take()
                    {
                        sender
                            .send(())
                            .map_err(|()| anyhow::anyhow!("failed-send observer was dropped"))?;
                    }
                    result
                }
                Err(err) => {
                    report
                        .as_mut()
                        .context("missing upgrade report")?
                        .fail_installation(
                            keys[index],
                            UpgradeErrorKind::Download,
                            err.to_string(),
                        );
                    observed_errors.push((index, err.kind()));
                    Ok(())
                }
            }
        });
        let controller = async {
            let mut started = Vec::new();
            for _ in 0..3 {
                started.push(
                    started_receiver
                        .recv()
                        .await
                        .context("download was not admitted")?,
                );
            }
            started.sort_unstable();
            assert_eq!(started, [0, 1, 2]);

            bytecode_receiver.close();
            drop(release_unstarted);
            release_first
                .send(())
                .map_err(|()| anyhow::anyhow!("first publication was dropped"))?;
            failed_send_receiver
                .await
                .context("closed bytecode channel was not observed")?;
            assert!(published.join(&names[0]).is_dir());
            assert!(!published.join(&names[1]).exists());

            release_second
                .send(())
                .map_err(|()| anyhow::anyhow!("second publication was dropped"))?;
            release_missing
                .send(())
                .map_err(|()| anyhow::anyhow!("missing publication was dropped"))?;
            Ok::<_, anyhow::Error>(())
        };
        let (installed, controlled) = join!(installer, controller);
        controlled?;
        let Err(err) = installed else {
            anyhow::bail!("closed bytecode channel unexpectedly accepted a publication");
        };
        assert_eq!(err.to_string(), "channel closed");
        assert_eq!(observed_errors, [(2, ErrorKind::NotFound)]);
        assert_eq!(
            started_receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        );
        assert_eq!(published_installations.find_all()?.count(), 2);
        assert!(!first.path().exists());
        assert!(!second.path().exists());
        assert!(!published.join(&names[2]).exists());
        assert!(unstarted.executable(false).is_file());
        assert!(!published.join(&names[3]).exists());
        let report = serde_json::to_value(report.context("missing upgrade report")?)?;
        assert_eq!(report["upgrades"][0]["outcome"], "installed");
        assert_eq!(report["upgrades"][0]["to"]["key"], names[0]);
        assert_eq!(report["upgrades"][1]["outcome"], "installed");
        assert_eq!(report["upgrades"][1]["to"]["key"], names[1]);
        assert_eq!(report["upgrades"][2]["outcome"], "failed");
        assert_eq!(report["upgrades"][2]["errors"][0]["kind"], "download");
        assert_eq!(report["upgrades"][3]["outcome"], "not_completed");
        assert!(report["upgrades"][3]["to"].is_null());
        Ok(())
    }

    #[tokio::test]
    async fn buffered_downloads_without_receiver_admits_every_download() {
        let mut observed = buffered_downloads(0..4, ready, 3, None::<&mpsc::UnboundedSender<()>>)
            .collect::<Vec<_>>()
            .await;
        observed.sort_unstable();
        assert_eq!(observed, [0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn settle_downloads_without_reporting_returns_first_error() -> Result<()> {
        let tasks = buffered_downloads(0..4, ready, 3, None::<&mpsc::UnboundedSender<()>>);
        let mut observed = Vec::new();
        let result = settle_downloads(tasks, false, |index| {
            observed.push(index);
            anyhow::bail!("first observation failed");
        })
        .await;
        let Err(err) = result else {
            anyhow::bail!("observation failure was not returned");
        };
        assert_eq!(err.to_string(), "first observation failed");
        assert_eq!(observed.len(), 1);
        Ok(())
    }

    fn create_installation(root: &Path, version: &str) -> Result<ManagedPythonInstallation> {
        let managed = root.join("managed");
        let path = managed.join(format!("cpython-{version}-{}", platform_key_from_env()?));
        fs_err::create_dir_all(&path)?;
        let installation = ManagedPythonInstallations::from_settings(Some(managed))?
            .find_all()?
            .find(|installation| installation.path() == path)
            .context("missing test installation")?;
        let executable = installation.executable(false);
        fs_err::create_dir_all(executable.parent().context("missing executable parent")?)?;
        fs_err::write(executable, b"inert Python fixture")?;
        Ok(installation)
    }

    #[test]
    #[cfg(unix)]
    fn resolve_bin_link_target_uses_captured_target() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let root = dunce::canonicalize(temp_dir.path())?;
        let first = root.join("first-python");
        let second = root.join("second-python");
        fs_err::write(&first, b"first Python")?;
        fs_err::write(&second, b"second Python")?;
        let bin = root.join("bin");
        fs_err::create_dir_all(&bin)?;
        let link = bin.join("python3");
        fs_err::os::unix::fs::symlink("../first-python", &link)?;
        let captured_target = read_bin_link_target(&link).context("missing captured target")?;
        assert_eq!(captured_target, Path::new("../first-python"));

        uv_fs::replace_symlink("../second-python", &link)?;
        let current_target = read_bin_link_target(&link).context("missing current target")?;
        assert_eq!(current_target, Path::new("../second-python"));

        assert_eq!(
            super::resolve_bin_link_target(&link, &captured_target),
            Some(first)
        );
        assert_eq!(
            super::resolve_bin_link_target(&link, &current_target),
            Some(second)
        );
        Ok(())
    }

    #[test]
    fn create_bin_links_requires_force_for_unmanaged_executable() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let root = dunce::canonicalize(temp_dir.path())?;
        let installation = create_installation(&root, "3.12.8")?;
        let bin = root.join("bin");
        fs_err::create_dir_all(&bin)?;
        let target = bin.join(installation.key().executable_name_minor());
        fs_err::write(&target, b"unmanaged executable")?;
        let mut bin_links = BinLinkStates::default();
        let mut changelog = Changelog::default();
        let mut errors = Vec::new();

        create_bin_links(
            &installation,
            &bin,
            false,
            false,
            false,
            false,
            false,
            false,
            &[],
            &[&installation],
            &mut bin_links,
            &mut changelog,
            &mut errors,
            Preview::default(),
        );
        assert_eq!(fs_err::read(&target)?, b"unmanaged executable");
        assert!(changelog.installed.is_empty());
        assert!(changelog.installed_executables.is_empty());
        assert!(bin_links.states.is_empty());
        let [(InstallErrorKind::Bin, key, _)] = errors.as_slice() else {
            anyhow::bail!("unexpected errors: {errors:?}");
        };
        assert_eq!(key, installation.key());

        errors.clear();
        create_bin_links(
            &installation,
            &bin,
            false,
            true,
            false,
            false,
            false,
            false,
            &[],
            &[&installation],
            &mut bin_links,
            &mut changelog,
            &mut errors,
            Preview::default(),
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            find_matching_bin_link([&installation].into_iter(), &target)
                .map(ManagedPythonInstallation::key),
            Some(installation.key())
        );
        assert!(changelog.installed.contains(installation.key()));
        assert_eq!(
            bin_links
                .get(&target)
                .and_then(|state| state.owner.as_ref())
                .map(ManagedPythonInstallation::key),
            Some(installation.key())
        );
        assert_eq!(
            changelog.installed_executables.get(installation.key()),
            Some(&[target].into_iter().collect())
        );
        Ok(())
    }

    #[test]
    fn create_bin_links_updates_managed_executable_owner() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let root = dunce::canonicalize(temp_dir.path())?;
        let older = create_installation(&root, "3.12.6")?;
        let newer = create_installation(&root, "3.12.8")?;
        let bin = root.join("bin");
        let target = bin.join(older.key().executable_name_minor());
        create_link_to_executable(&target, PythonExecutable::console(&older.executable(false)))?;
        let mut bin_links = BinLinkStates::default();
        let mut changelog = Changelog::default();
        changelog.installed.insert(older.key().clone());
        changelog
            .installed_executables
            .insert(older.key().clone(), [target.clone()].into_iter().collect());
        let mut errors = Vec::new();

        create_bin_links(
            &newer,
            &bin,
            false,
            false,
            false,
            false,
            true,
            false,
            std::slice::from_ref(&older),
            &[&newer],
            &mut bin_links,
            &mut changelog,
            &mut errors,
            Preview::default(),
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            find_matching_bin_link([&older, &newer].into_iter(), &target)
                .map(ManagedPythonInstallation::key),
            Some(newer.key())
        );
        assert!(changelog.installed.contains(newer.key()));
        assert_eq!(
            bin_links
                .get(&target)
                .and_then(|state| state.owner.as_ref())
                .map(ManagedPythonInstallation::key),
            Some(newer.key())
        );
        assert!(
            changelog
                .installed_executables
                .get(older.key())
                .context("missing previous executable owner")?
                .is_empty()
        );
        assert_eq!(
            changelog.installed_executables.get(newer.key()),
            Some(&[target].into_iter().collect())
        );
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn create_bin_links_preserves_directory_on_replacement_failure() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let root = dunce::canonicalize(temp_dir.path())?;
        let installation = create_installation(&root, "3.12.8")?;
        let bin = root.join("bin");
        let target = bin.join(installation.key().executable_name_minor());
        fs_err::create_dir_all(&target)?;
        let existing = target.join("existing");
        fs_err::write(&existing, b"existing directory contents")?;
        let mut bin_links = BinLinkStates::default();
        let mut changelog = Changelog::default();
        let mut errors = Vec::new();

        create_bin_links(
            &installation,
            &bin,
            false,
            true,
            false,
            false,
            false,
            false,
            &[],
            &[&installation],
            &mut bin_links,
            &mut changelog,
            &mut errors,
            Preview::default(),
        );
        assert!(target.is_dir());
        assert_eq!(fs_err::read(&existing)?, b"existing directory contents");
        assert!(changelog.installed.is_empty());
        assert!(changelog.installed_executables.is_empty());
        assert!(bin_links.states.is_empty());
        let [(InstallErrorKind::Bin, key, _)] = errors.as_slice() else {
            anyhow::bail!("unexpected errors: {errors:?}");
        };
        assert_eq!(key, installation.key());
        Ok(())
    }

    #[test]
    fn create_bin_links_keeps_owner_across_minor_retarget() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let root = dunce::canonicalize(temp_dir.path())?;
        let older = create_installation(&root, "3.12.6")?;
        let newer = create_installation(&root, "3.12.8")?;
        let bin = root.join("bin");
        let target = bin.join(older.key().executable_name_minor());
        older.ensure_minor_version_link()?;
        let minor_link = PythonMinorVersionLink::from_installation(&older)
            .context("CPython must have a minor-version link")?;
        create_link_to_executable(
            &target,
            PythonExecutable::console(&minor_link.symlink_executable),
        )?;
        let mut bin_links = BinLinkStates::capture(
            &bin,
            &[&newer],
            std::slice::from_ref(&older),
            false,
            false,
            Preview::default(),
        );

        newer.ensure_minor_version_link()?;
        assert_eq!(
            find_matching_bin_link([&older, &newer].into_iter(), &target)
                .map(ManagedPythonInstallation::key),
            Some(newer.key())
        );
        assert_eq!(
            bin_links
                .get(&target)
                .and_then(|state| state.owner.as_ref())
                .map(ManagedPythonInstallation::key),
            Some(older.key())
        );

        let mut changelog = Changelog::default();
        changelog.installed.insert(older.key().clone());
        changelog
            .installed_executables
            .insert(older.key().clone(), [target.clone()].into_iter().collect());
        let mut errors = Vec::new();
        create_bin_links(
            &newer,
            &bin,
            false,
            false,
            false,
            false,
            true,
            false,
            std::slice::from_ref(&older),
            &[&newer],
            &mut bin_links,
            &mut changelog,
            &mut errors,
            Preview::default(),
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            read_bin_link_target(&target).as_deref(),
            Some(dunce::simplified(&newer.executable(false)))
        );
        assert_eq!(
            bin_links
                .get(&target)
                .and_then(|state| state.owner.as_ref())
                .map(ManagedPythonInstallation::key),
            Some(newer.key())
        );
        assert!(
            changelog
                .installed_executables
                .get(older.key())
                .context("missing previous executable owner")?
                .is_empty()
        );
        assert_eq!(
            changelog.installed_executables.get(newer.key()),
            Some(&[target].into_iter().collect())
        );
        Ok(())
    }

    #[test]
    fn create_bin_links_rechecks_a_captured_target() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let root = dunce::canonicalize(temp_dir.path())?;
        let older = create_installation(&root, "3.12.6")?;
        let newer = create_installation(&root, "3.12.8")?;
        let bin = root.join("bin");
        let target = bin.join(older.key().executable_name_minor());
        create_link_to_executable(&target, PythonExecutable::console(&older.executable(false)))?;
        let mut bin_links = BinLinkStates::capture(
            &bin,
            &[&newer],
            std::slice::from_ref(&older),
            false,
            false,
            Preview::default(),
        );

        let unmanaged = root.join("unmanaged-python");
        fs_err::write(&unmanaged, b"unmanaged executable")?;
        replace_link_to_executable(&target, PythonExecutable::console(&unmanaged))?;
        assert!(bin_links.get(&target).is_none());

        let mut changelog = Changelog::default();
        let mut errors = Vec::new();
        create_bin_links(
            &newer,
            &bin,
            false,
            false,
            false,
            false,
            false,
            false,
            std::slice::from_ref(&older),
            &[&newer],
            &mut bin_links,
            &mut changelog,
            &mut errors,
            Preview::default(),
        );
        assert_eq!(
            read_bin_link_target(&target).as_deref(),
            Some(dunce::simplified(&unmanaged))
        );
        assert!(changelog.installed.is_empty());
        assert!(changelog.installed_executables.is_empty());
        let [(InstallErrorKind::Bin, key, _)] = errors.as_slice() else {
            anyhow::bail!("unexpected errors: {errors:?}");
        };
        assert_eq!(key, newer.key());
        assert_eq!(
            bin_links
                .states
                .get(&target)
                .and_then(|state| state.owner.as_ref())
                .map(ManagedPythonInstallation::key),
            Some(older.key())
        );

        errors.clear();
        create_bin_links(
            &newer,
            &bin,
            false,
            true,
            false,
            false,
            false,
            false,
            std::slice::from_ref(&older),
            &[&newer],
            &mut bin_links,
            &mut changelog,
            &mut errors,
            Preview::default(),
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            bin_links
                .get(&target)
                .and_then(|state| state.owner.as_ref())
                .map(ManagedPythonInstallation::key),
            Some(newer.key())
        );
        assert_eq!(
            changelog.installed_executables.get(newer.key()),
            Some(&[target].into_iter().collect())
        );
        Ok(())
    }
}
