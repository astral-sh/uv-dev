use std::cell::OnceCell;
use std::fmt::Write;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};
use owo_colors::OwoColorize;
use tracing::debug;
use uv_python_managed::downloads::ManagedPythonDownloadList;

use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_configuration::DependencyGroupsWithDefaults;
use uv_distribution_types::RequiresPython;
use uv_fs::Simplified;
use uv_python_discovery::PYTHON_VERSION_FILENAME;
use uv_python_discovery::PythonVersionFile;
use uv_python_discovery::VersionFileDiscoveryOptions;
use uv_python_discovery::{PythonInstallation, PythonSelectionError};
use uv_python_types::{
    EnvironmentPreference, PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest,
};
use uv_settings::PythonInstallMirrors;
use uv_warnings::{warn_user_once, warn_user_once_with_chain};
use uv_workspace::{DiscoveryOptions, VirtualProject, WorkspaceCache};

use uv_command_support::ExitStatus;
use uv_command_support::Printer;
use uv_python_discovery::PythonDownloadReporter;
use uv_python_discovery::find_requires_python;

/// Pin to a specific Python version.
#[expect(clippy::fn_params_excessive_bools)]
pub async fn pin(
    project_dir: &Path,
    request: Option<String>,
    resolved: bool,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    no_project: bool,
    global: bool,
    rm: bool,
    install_mirrors: PythonInstallMirrors,
    client_builder: BaseClientBuilder<'_>,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
) -> Result<ExitStatus> {
    let virtual_project = if no_project {
        None
    } else {
        match VirtualProject::discover(
            project_dir,
            &DiscoveryOptions::default(),
            cache,
            workspace_cache,
        )
        .await
        {
            Ok(virtual_project) => Some(virtual_project),
            Err(err) => {
                debug!("Failed to discover virtual project: {err}");
                None
            }
        }
    };

    // Search for an existing file, we won't necessarily write to this, we'll construct a target
    // path if there's a request later on.
    let version_file = PythonVersionFile::discover(
        project_dir,
        &VersionFileDiscoveryOptions::default().with_no_local(global),
    )
    .await;

    if rm {
        let Some(file) = version_file? else {
            if global {
                bail!("No global Python pin found");
            }
            bail!("No Python version file found");
        };

        if !global && file.is_global() {
            bail!("No Python version file found; use `--rm --global` to remove the global pin");
        }

        fs_err::tokio::remove_file(file.path()).await?;
        writeln!(
            printer.stdout(),
            "Removed {} at `{}`",
            if global {
                "global Python pin"
            } else {
                "Python version file"
            },
            file.path().user_display()
        )?;
        return Ok(ExitStatus::Success);
    }

    let compatibility = virtual_project.as_ref().map(PinCompatibility::new);

    let Some(request) = request else {
        // Display the current pinned Python version
        if let Some(file) = version_file? {
            let mut pins = file.versions().peekable();
            let download_list = if compatibility.is_some() && pins.peek().is_some() {
                Some(
                    ManagedPythonDownloadList::new(
                        &client_builder,
                        cache,
                        install_mirrors.python_downloads_json_url.as_deref(),
                    )
                    .await?,
                )
            } else {
                None
            };

            for pin in pins {
                writeln!(printer.stdout(), "{}", pin.to_canonical_string())?;
                if let Some(compatibility) = &compatibility
                    && let Some(download_list) = &download_list
                {
                    warn_if_existing_pin_incompatible_with_project(
                        pin,
                        compatibility,
                        python_preference,
                        python_arch,
                        download_list,
                        cache,
                    );
                }
            }
            return Ok(ExitStatus::Success);
        }
        bail!("No Python version file found; specify a version to create one")
    };
    let request = PythonRequest::parse(&request);

    if let PythonRequest::ExecutableName(name) = request {
        bail!("Requests for arbitrary names (e.g., `{name}`) are not supported in version files");
    }

    let request_version = request.as_pep440_version();
    if let Some(compatibility) = &compatibility
        && let Some(request_version) = &request_version
    {
        assert_pin_compatible_with_project(
            &Pin {
                request: &request,
                version: request_version,
                resolved: false,
                existing: false,
            },
            compatibility,
        )?;
    }

    let reporter = PythonDownloadReporter::single(printer);

    let python = match PythonInstallation::find_or_download(
        Some(&request),
        EnvironmentPreference::OnlySystem,
        python_preference,
        python_arch,
        python_downloads,
        &client_builder,
        cache,
        Some(&reporter),
        install_mirrors.mirrors(),
        install_mirrors.python_downloads_json_url.as_deref(),
    )
    .await
    {
        Ok(python) => Some(python),
        // If no matching Python version is found, don't fail unless `resolved` was requested
        Err(uv_python_discovery::Error::MissingPython(err, ..)) if !resolved => {
            // N.B. We omit the hint and just show the inner error message
            warn_user_once!("{err}");
            None
        }
        // If there was some other error, log it
        Err(err) if !resolved => {
            debug!("{err}");
            None
        }
        // If `resolved` was requested, we must find an interpreter — fail otherwise
        Err(err) => return Err(err.into()),
    };

    if let Some(compatibility) = &compatibility
        && request_version.is_none()
        && let Some(python) = &python
    {
        // Ranges and paths need an interpreter before compatibility can be checked.
        if let Err(err) = assert_pin_compatible_with_project(
            &Pin {
                request: &request,
                version: python.python_version(),
                resolved: true,
                existing: false,
            },
            compatibility,
        ) {
            if resolved {
                return Err(err);
            }
            warn_user_once!("{err}");
        }
    }

    let request = if resolved {
        // SAFETY: We exit early if Python is not found and resolved is `true`
        // TODO(zanieb): Maybe avoid reparsing here?
        PythonRequest::parse(
            &python
                .unwrap()
                .interpreter()
                .sys_executable()
                .user_display()
                .to_string(),
        )
    } else {
        request
    };

    let existing = version_file.ok().flatten();
    // TODO(zanieb): Allow updating the discovered version file with an `--update` flag.
    let new = if global {
        let Some(new) = PythonVersionFile::global() else {
            // TODO(zanieb): We should find a nice way to surface that as an error
            bail!("Failed to determine directory for global Python pin");
        };
        new.with_versions(vec![request])
    } else {
        PythonVersionFile::new(project_dir.join(PYTHON_VERSION_FILENAME))
            .with_versions(vec![request])
    };

    new.write().await?;

    // If we updated an existing version file to a new version
    if let Some(existing) = existing
        .as_ref()
        .filter(|existing| existing.path() == new.path())
        .and_then(PythonVersionFile::version)
        .filter(|version| *version != new.version().unwrap())
    {
        writeln!(
            printer.stdout(),
            "Updated `{}` from `{}` -> `{}`",
            new.path().user_display().cyan(),
            existing.to_canonical_string().green(),
            new.version().unwrap().to_canonical_string().green()
        )?;
    } else {
        writeln!(
            printer.stdout(),
            "Pinned `{}` to `{}`",
            new.path().user_display().cyan(),
            new.version().unwrap().to_canonical_string().green()
        )?;
    }

    Ok(ExitStatus::Success)
}

/// Check if pinned request is compatible with the workspace/project's `Requires-Python`.
fn warn_if_existing_pin_incompatible_with_project(
    pin: &PythonRequest,
    compatibility: &PinCompatibility<'_>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    downloads_list: &ManagedPythonDownloadList,
    cache: &Cache,
) {
    // Check if the pinned version is compatible with the project.
    if let Some(pin_version) = pin.as_pep440_version() {
        if let Err(err) = assert_pin_compatible_with_project(
            &Pin {
                request: pin,
                version: &pin_version,
                resolved: false,
                existing: true,
            },
            compatibility,
        ) {
            warn_user_once!("{err}");
            return;
        }
    }

    // If the request itself didn't prove an incompatibility, resolve the pin into an
    // interpreter to check the concrete version on the current system.
    match PythonInstallation::find(
        pin,
        EnvironmentPreference::OnlySystem,
        python_preference,
        python_arch,
        downloads_list,
        cache,
    ) {
        Ok(python) => {
            let python_version = python.python_version();
            debug!(
                "The pinned Python version `{}` resolves to `{}`",
                pin, python_version
            );
            // Warn on incompatibilities when viewing existing pins
            if let Err(err) = assert_pin_compatible_with_project(
                &Pin {
                    request: pin,
                    version: python_version,
                    resolved: true,
                    existing: true,
                },
                compatibility,
            ) {
                warn_user_once!("{err}");
            }
        }
        Err(err) => {
            warn_user_once_with_chain!(
                anyhow::Error::from(err)
                    .context(format!(
                        "Failed to resolve pinned Python version `{}`",
                        pin.to_canonical_string(),
                    ))
                    .as_ref()
            );
        }
    }
}

/// Utility struct for representing pins in error messages.
struct Pin<'a> {
    request: &'a PythonRequest,
    version: &'a uv_pep440::Version,
    resolved: bool,
    existing: bool,
}

/// Reuse the immutable workspace requirement across requested and resolved pin checks.
struct PinCompatibility<'a> {
    virtual_project: &'a VirtualProject,
    requires_python: OnceCell<Result<Option<RequiresPython>, Arc<PythonSelectionError>>>,
}

impl<'a> PinCompatibility<'a> {
    fn new(virtual_project: &'a VirtualProject) -> Self {
        Self {
            virtual_project,
            requires_python: OnceCell::new(),
        }
    }

    fn requires_python(&self) -> Result<Option<&RequiresPython>> {
        self.requires_python
            .get_or_init(|| {
                // Don't factor in requires-python settings on dependency-groups
                let groups = DependencyGroupsWithDefaults::none();

                let requires_python = match self.virtual_project {
                    VirtualProject::Project(project_workspace) => {
                        debug!(
                            "Discovered project `{}` at: {}",
                            project_workspace.project_name(),
                            project_workspace.workspace().install_path().display()
                        );

                        find_requires_python(project_workspace.workspace(), &groups)
                    }
                    VirtualProject::NonProject(workspace) => {
                        debug!(
                            "Discovered virtual workspace at: {}",
                            workspace.install_path().display()
                        );
                        find_requires_python(workspace, &groups)
                    }
                };

                requires_python.map_err(Arc::new)
            })
            .as_ref()
            .map(Option::as_ref)
            .map_err(|error| anyhow::Error::new(Arc::clone(error)))
    }
}

/// Checks if the pinned Python version is compatible with the workspace/project's `Requires-Python`.
fn assert_pin_compatible_with_project(
    pin: &Pin,
    compatibility: &PinCompatibility<'_>,
) -> Result<()> {
    let requires_python = compatibility.requires_python()?;
    let project_type = match compatibility.virtual_project {
        VirtualProject::Project(_) => "project",
        VirtualProject::NonProject(_) => "workspace",
    };

    let Some(requires_python) = requires_python else {
        return Ok(());
    };

    if requires_python.contains(pin.version) {
        return Ok(());
    }

    let given = if pin.existing { "pinned" } else { "requested" };
    let resolved = if pin.resolved {
        format!(" resolves to `{}` which ", pin.version)
    } else {
        String::new()
    };

    Err(anyhow::anyhow!(
        "The {given} Python version `{}`{resolved} is incompatible with the {} `requires-python` value of `{}`.",
        pin.request.to_canonical_string(),
        project_type,
        requires_python
    ))
}
