use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use owo_colors::OwoColorize;
use tracing::{debug, trace, warn};

use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_command_support::{ExitStatus, Printer};
use uv_configuration::{
    AuthorFrom, DependencyGroupsWithDefaults, InitKind, InitProjectKind, ProjectBuildBackend,
    VersionControlError, VersionControlSystem,
};
use uv_distribution_types::RequiresPython;
use uv_fs::{CWD, Simplified};
use uv_git::GIT;
use uv_install_wheel::reserved_script_name;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_project_edit::{DependencyTarget, PyProjectTomlMut};
use uv_python_discovery::ConfigDiscovery;
use uv_python_discovery::PythonDownloadReporter;
use uv_python_discovery::PythonInstallation;
use uv_python_discovery::PythonVersionFile;
use uv_python_discovery::VersionFileDiscoveryOptions;
use uv_python_discovery::find_requires_python;
use uv_python_discovery::init_script_python_requirement;
use uv_python_interpreter::PythonEnvironment;
use uv_python_types::{
    EnvironmentPreference, PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest,
    PythonVariant, VersionRequest,
};
use uv_scripts::{Pep723Script, ScriptTag};
use uv_settings::PythonInstallMirrors;
use uv_static::EnvVars;
use uv_warnings::warn_user_once;
use uv_workspace::{
    DiscoveryOptions, MemberDiscovery, Workspace, WorkspaceCache, WorkspaceErrorKind,
};

mod template;

use template::{Author, pyproject_build_system, pyproject_project, pyproject_project_scripts};

/// Add one or more packages to the project requirements.
#[expect(clippy::single_match_else, clippy::fn_params_excessive_bools)]
pub async fn init(
    project_dir: &Path,
    explicit_path: Option<PathBuf>,
    name: Option<PackageName>,
    init_kind: InitKind,
    bare: bool,
    description: Option<String>,
    no_description: bool,
    vcs: Option<VersionControlSystem>,
    build_backend: Option<ProjectBuildBackend>,
    no_readme: bool,
    author_from: Option<AuthorFrom>,
    pin_python: bool,
    python: Option<String>,
    install_mirrors: PythonInstallMirrors,
    no_workspace: bool,
    client_builder: &BaseClientBuilder<'_>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    config_discovery: ConfigDiscovery,
    cache: &Cache,
    printer: Printer,
) -> Result<ExitStatus> {
    match init_kind {
        InitKind::Script => {
            let Some(path) = explicit_path.as_deref() else {
                anyhow::bail!("Script initialization requires a file path")
            };

            init_script(
                path,
                bare,
                python,
                install_mirrors,
                client_builder,
                python_preference,
                python_arch,
                python_downloads,
                cache,
                printer,
                no_workspace,
                no_readme,
                author_from,
                pin_python,
                config_discovery,
            )
            .await?;

            writeln!(
                printer.stderr(),
                "Initialized script at `{}`",
                path.user_display().cyan()
            )?;
        }
        InitKind::Project(project_kind) => {
            // Default to the current directory if a path was not provided.
            let path = match explicit_path {
                None => project_dir.to_path_buf(),
                Some(ref path) => std::path::absolute(path)?,
            };

            // Make sure a project does not already exist in the given directory.
            if path.join("pyproject.toml").exists() {
                let path =
                    std::path::absolute(&path).unwrap_or_else(|_| path.simplified().to_path_buf());
                anyhow::bail!(
                    "Project is already initialized in `{}` (`pyproject.toml` file exists)",
                    path.display().cyan()
                );
            }

            // Default to the directory name if a name was not provided.
            let name = match name {
                Some(name) => name,
                None => {
                    let directory_name = path
                        .file_name()
                        .and_then(|path| path.to_str())
                        .context("Missing directory name")?;

                    // Pre-normalize the package name by removing any leading or trailing
                    // whitespace, and replacing any internal whitespace with hyphens.
                    let candidate = directory_name.trim().replace(' ', "-");
                    match PackageName::from_owned(candidate) {
                        Ok(name) if reserved_script_name(name.as_str()).is_some() => {
                            anyhow::bail!(
                                "The directory name (`{directory_name}`) cannot be used as project \
                                name, please provide a package name with `--name`."
                            );
                        }
                        Ok(name) => name,
                        Err(_) => {
                            let directory_description = if explicit_path.is_some() {
                                "target directory"
                            } else {
                                "current directory"
                            };
                            anyhow::bail!(
                                "The {directory_description} (`{directory_name}`) is not a valid package name. Please provide a package name with `--name`."
                            );
                        }
                    }
                }
            };

            Box::pin(init_project(
                &path,
                &name,
                project_kind,
                bare,
                description,
                no_description,
                vcs,
                build_backend,
                no_readme,
                author_from,
                pin_python,
                python,
                install_mirrors,
                no_workspace,
                client_builder,
                python_preference,
                python_arch,
                python_downloads,
                config_discovery,
                cache,
                printer,
            ))
            .await?;

            // Create the `README.md` if it does not already exist.
            if !no_readme && !bare {
                let readme = path.join("README.md");
                if !readme.exists() {
                    fs_err::write(readme, String::new())?;
                }
            }

            match explicit_path {
                // Initialized a project in the current directory.
                None => {
                    writeln!(printer.stderr(), "Initialized project `{}`", name.cyan())?;
                }
                // Initialized a project in the given directory.
                Some(path) => {
                    let path = std::path::absolute(&path)
                        .unwrap_or_else(|_| path.simplified().to_path_buf());
                    writeln!(
                        printer.stderr(),
                        "Initialized project `{}` at `{}`",
                        name.cyan(),
                        path.display().cyan()
                    )?;
                }
            }
        }
    }

    Ok(ExitStatus::Success)
}

#[expect(clippy::fn_params_excessive_bools)]
async fn init_script(
    script_path: &Path,
    bare: bool,
    python: Option<String>,
    install_mirrors: PythonInstallMirrors,
    client_builder: &BaseClientBuilder<'_>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    cache: &Cache,
    printer: Printer,
    no_workspace: bool,
    no_readme: bool,
    author_from: Option<AuthorFrom>,
    pin_python: bool,
    config_discovery: ConfigDiscovery,
) -> Result<()> {
    if no_workspace {
        warn_user_once!("`--no-workspace` is a no-op for Python scripts, which are standalone");
    }
    if no_readme {
        warn_user_once!("`--no-readme` is a no-op for Python scripts, which are standalone");
    }
    if author_from.is_some() {
        warn_user_once!("`--author-from` is a no-op for Python scripts, which are standalone");
    }
    let reporter = PythonDownloadReporter::single(printer);

    // If the file already exists, read its content.
    let content = match fs_err::tokio::read(script_path).await {
        Ok(metadata) => {
            // If the file is already a script, raise an error.
            if ScriptTag::parse(&metadata)?.is_some() {
                anyhow::bail!(
                    "`{}` is already a PEP 723 script; use `{}` to execute it",
                    script_path.simplified_display().cyan(),
                    "uv run".green()
                );
            }

            Some(metadata)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "Failed to read script at `{}`",
                    script_path.simplified_display().cyan()
                )
            });
        }
    };

    let requires_python = init_script_python_requirement(
        python.as_deref(),
        &install_mirrors,
        script_path.parent().unwrap_or(&CWD),
        !pin_python,
        python_preference,
        python_arch,
        python_downloads,
        config_discovery,
        client_builder,
        cache,
        &reporter,
    )
    .await?;

    if let Some(parent) = script_path.parent() {
        fs_err::tokio::create_dir_all(parent).await?;
    }

    Pep723Script::create(script_path, requires_python.specifiers(), content, bare).await?;

    Ok(())
}

/// Initialize a project (and, implicitly, a workspace root) at the given path.
#[expect(clippy::fn_params_excessive_bools)]
async fn init_project(
    path: &Path,
    name: &PackageName,
    project_kind: InitProjectKind,
    bare: bool,
    description: Option<String>,
    no_description: bool,
    vcs: Option<VersionControlSystem>,
    build_backend: Option<ProjectBuildBackend>,
    no_readme: bool,
    author_from: Option<AuthorFrom>,
    pin_python: bool,
    python: Option<String>,
    install_mirrors: PythonInstallMirrors,
    no_workspace: bool,
    client_builder: &BaseClientBuilder<'_>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    config_discovery: ConfigDiscovery,
    cache: &Cache,
    printer: Printer,
) -> Result<()> {
    // Discover the current workspace, if it exists.
    let workspace_cache = WorkspaceCache::default();
    let workspace = {
        let parent = match path.parent() {
            Some(parent) => parent,
            None => {
                if path.is_dir() {
                    // Support creating a project in the filesystem root (`/` on Unix).
                    path
                } else {
                    // Not sure how we'd end up here, but we need to handle the case.
                    bail!("Project directory has no parent directory");
                }
            }
        };
        match Workspace::discover(
            parent,
            &DiscoveryOptions {
                members: MemberDiscovery::Ignore(std::iter::once(path.to_path_buf()).collect()),
                ..DiscoveryOptions::default()
            },
            cache,
            &workspace_cache,
        )
        .await
        {
            Ok(workspace) => {
                // Ignore the current workspace if `--no-workspace` was provided.
                if no_workspace {
                    debug!("Ignoring discovered workspace due to `--no-workspace`");
                    None
                } else {
                    Some(workspace)
                }
            }
            Err(err) => {
                if matches!(
                    err.as_ref(),
                    WorkspaceErrorKind::MissingPyprojectToml | WorkspaceErrorKind::NonWorkspace(_)
                ) {
                    if no_workspace {
                        warn!("`--no-workspace` was provided, but no workspace was found");
                    }
                    None
                } else {
                    // If the user runs with `--no-workspace`, ignore the error.
                    if no_workspace {
                        warn!("Ignoring workspace discovery error due to `--no-workspace`: {err}");
                        None
                    } else {
                        return Err(err).with_context(|| {
                            format!(
                                "Failed to discover parent workspace; use `{}` to ignore",
                                "uv init --no-workspace".green()
                            )
                        });
                    }
                }
            }
        }
    };

    let reporter = PythonDownloadReporter::single(printer);

    // First, determine if there is an request for Python
    let python_request = if let Some(request) = python {
        // (1) Explicit request from user
        Some(PythonRequest::parse(&request))
    } else if let Some(file) = PythonVersionFile::discover(
        path,
        &VersionFileDiscoveryOptions::default()
            .with_stop_discovery_at(
                workspace
                    .as_deref()
                    .map(Workspace::install_path)
                    .map(PathBuf::as_ref),
            )
            .with_config_discovery(config_discovery),
    )
    .await?
    {
        // (2) Request from `.python-version`
        file.into_version()
    } else {
        None
    };

    let (requires_python, python_pin) = determine_requires_python(
        path,
        pin_python,
        install_mirrors,
        client_builder,
        python_preference,
        python_arch,
        python_downloads,
        cache,
        workspace.as_deref(),
        &reporter,
        python_request,
    )
    .await?;

    init_project_kind(
        project_kind,
        name,
        path,
        &requires_python,
        description.as_deref(),
        no_description,
        bare,
        vcs,
        build_backend,
        author_from,
        no_readme,
    )?;

    if let Some(workspace) = workspace {
        if workspace.excludes(path)? {
            // If the member is excluded by the workspace, ignore it.
            writeln!(
                printer.stderr(),
                "Project `{}` is excluded by workspace `{}`",
                name.cyan(),
                workspace.install_path().simplified_display().cyan()
            )?;
        } else if workspace.includes(path)? {
            // If the member is already included in the workspace, skip the `members` addition.
            writeln!(
                printer.stderr(),
                "Project `{}` is already a member of workspace `{}`",
                name.cyan(),
                workspace.install_path().simplified_display().cyan()
            )?;
        } else {
            // Add the package to the workspace.
            let mut pyproject = PyProjectTomlMut::from_toml(
                &workspace.pyproject_toml().raw,
                DependencyTarget::PyProjectToml,
            )?;
            pyproject.add_workspace(path.strip_prefix(workspace.install_path())?)?;

            // Save the modified `pyproject.toml`.
            fs_err::write(
                workspace.install_path().join("pyproject.toml"),
                pyproject.to_string(),
            )?;

            writeln!(
                printer.stderr(),
                "Adding `{}` as member of workspace `{}`",
                name.cyan(),
                workspace.install_path().simplified_display().cyan()
            )?;
        }
        // Write .python-version if it doesn't exist in the workspace or if the version differs
        if let Some(python_request) = python_pin {
            if PythonVersionFile::discover(path, &VersionFileDiscoveryOptions::default())
                .await?
                .as_ref()
                .is_none_or(|file| !{
                    file.version()
                        .is_some_and(|version| *version == python_request)
                        && file.path().parent().is_some_and(|parent| {
                            parent == workspace.install_path() || parent == path
                        })
                })
            {
                PythonVersionFile::new(path.join(".python-version"))
                    .with_versions(vec![python_request.clone()])
                    .write()
                    .await?;
            }
        }
    } else {
        // Write .python-version if it doesn't exist in the project directory.
        if let Some(python_request) = python_pin {
            if PythonVersionFile::discover(path, &VersionFileDiscoveryOptions::default())
                .await?
                .filter(|file| file.version().is_some())
                .as_ref()
                .is_none_or(|file| file.path().parent().is_none_or(|parent| parent != path))
            {
                PythonVersionFile::new(path.join(".python-version"))
                    .with_versions(vec![python_request.clone()])
                    .write()
                    .await?;
            }
        }
    }

    Ok(())
}

async fn determine_requires_python(
    path: &Path,
    pin_python: bool,
    install_mirrors: PythonInstallMirrors,
    client_builder: &BaseClientBuilder<'_>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    cache: &Cache,
    workspace: Option<&Workspace>,
    reporter: &PythonDownloadReporter,
    python_request: Option<PythonRequest>,
) -> Result<(RequiresPython, Option<PythonRequest>)> {
    // Add a `requires-python` field to the `pyproject.toml` and return the corresponding interpreter.
    if let Some(python_request) = python_request {
        // (1) A request from the user or `.python-version` file
        // This can be arbitrary, i.e., not a version — in which case we may need to resolve the
        // interpreter
        let (requires_python, python_pin) = match &python_request {
            PythonRequest::Version(VersionRequest::MajorMinor(major, minor, variant)) => {
                let requires_python = RequiresPython::greater_than_equal_version(&Version::new([
                    u64::from(*major),
                    u64::from(*minor),
                ]));

                let python_pin = if pin_python {
                    Some(PythonRequest::Version(VersionRequest::MajorMinor(
                        *major, *minor, *variant,
                    )))
                } else {
                    None
                };

                (requires_python, python_pin)
            }
            PythonRequest::Version(VersionRequest::MajorMinorPatch(
                major,
                minor,
                patch,
                variant,
            )) => {
                let requires_python = RequiresPython::greater_than_equal_version(&Version::new([
                    u64::from(*major),
                    u64::from(*minor),
                    u64::from(*patch),
                ]));

                let python_pin = if pin_python {
                    Some(PythonRequest::Version(VersionRequest::MajorMinorPatch(
                        *major, *minor, *patch, *variant,
                    )))
                } else {
                    None
                };

                (requires_python, python_pin)
            }
            python_request @ PythonRequest::Version(VersionRequest::Range(specifiers, variant)) => {
                let requires_python = RequiresPython::from_specifiers(specifiers.clone());

                let python_pin = if pin_python {
                    let interpreter = PythonInstallation::find_or_download(
                        Some(python_request),
                        EnvironmentPreference::OnlySystem,
                        python_preference,
                        python_arch,
                        python_downloads,
                        client_builder,
                        cache,
                        Some(reporter),
                        install_mirrors.mirrors(),
                        install_mirrors.python_downloads_json_url.as_deref(),
                    )
                    .await?
                    .into_interpreter();

                    Some(PythonRequest::Version(VersionRequest::MajorMinor(
                        interpreter.python_major(),
                        interpreter.python_minor(),
                        *variant,
                    )))
                } else {
                    None
                };

                (requires_python, python_pin)
            }
            python_request => {
                let interpreter = PythonInstallation::find_or_download(
                    Some(python_request),
                    EnvironmentPreference::OnlySystem,
                    python_preference,
                    python_arch,
                    python_downloads,
                    client_builder,
                    cache,
                    Some(reporter),
                    install_mirrors.mirrors(),
                    install_mirrors.python_downloads_json_url.as_deref(),
                )
                .await?
                .into_interpreter();

                let requires_python =
                    RequiresPython::greater_than_equal_version(&interpreter.python_minor_version());

                let python_pin = if pin_python {
                    Some(PythonRequest::Version(VersionRequest::MajorMinor(
                        interpreter.python_major(),
                        interpreter.python_minor(),
                        PythonVariant::Default,
                    )))
                } else {
                    None
                };

                (requires_python, python_pin)
            }
        };

        debug!("Using Python version `{requires_python}` from request `{python_request}`");

        Ok((requires_python, python_pin))
    } else if let Ok(virtualenv) = PythonEnvironment::from_root(path.join(".venv"), cache) {
        // (2) An existing Python environment in the target directory
        let interpreter = virtualenv.into_interpreter();

        let requires_python =
            RequiresPython::greater_than_equal_version(&interpreter.python_minor_version());

        // Pin to the minor version.
        let python_pin = if pin_python {
            Some(PythonRequest::Version(VersionRequest::MajorMinor(
                interpreter.python_major(),
                interpreter.python_minor(),
                PythonVariant::Default,
            )))
        } else {
            None
        };

        debug!(
            "Using Python version `{requires_python}` from existing virtual environment in project"
        );

        Ok((requires_python, python_pin))
    } else if let Some(requires_python) = workspace
        .as_ref()
        .map(|workspace| find_requires_python(workspace, &DependencyGroupsWithDefaults::none()))
        .transpose()?
        .flatten()
    {
        // (3) `requires-python` from the workspace
        let python_request = PythonRequest::from_specifiers(requires_python.specifiers())
            .unwrap_or(PythonRequest::Default);

        // Pin to the minor version.
        let python_pin = if pin_python {
            let interpreter = PythonInstallation::find_or_download(
                Some(&python_request),
                EnvironmentPreference::OnlySystem,
                python_preference,
                python_arch,
                python_downloads,
                client_builder,
                cache,
                Some(reporter),
                install_mirrors.mirrors(),
                install_mirrors.python_downloads_json_url.as_deref(),
            )
            .await?
            .into_interpreter();

            Some(PythonRequest::Version(VersionRequest::MajorMinor(
                interpreter.python_major(),
                interpreter.python_minor(),
                PythonVariant::Default,
            )))
        } else {
            None
        };

        debug!("Using Python version `{requires_python}` from project workspace");

        Ok((requires_python, python_pin))
    } else {
        // (4) Default to the system Python
        let interpreter = PythonInstallation::find_or_download(
            None,
            EnvironmentPreference::OnlySystem,
            python_preference,
            python_arch,
            python_downloads,
            client_builder,
            cache,
            Some(reporter),
            install_mirrors.mirrors(),
            install_mirrors.python_downloads_json_url.as_deref(),
        )
        .await?
        .into_interpreter();

        let requires_python =
            RequiresPython::greater_than_equal_version(&interpreter.python_minor_version());

        // Pin to the minor version.
        let python_pin = if pin_python {
            Some(PythonRequest::Version(VersionRequest::MajorMinor(
                interpreter.python_major(),
                interpreter.python_minor(),
                PythonVariant::Default,
            )))
        } else {
            None
        };

        debug!("Using Python version `{requires_python}` from default interpreter");

        Ok((requires_python, python_pin))
    }
}

/// Initialize this project kind at the target path.
fn init_project_kind(
    project_kind: InitProjectKind,
    name: &PackageName,
    path: &Path,
    requires_python: &RequiresPython,
    description: Option<&str>,
    no_description: bool,
    bare: bool,
    vcs: Option<VersionControlSystem>,
    build_backend: Option<ProjectBuildBackend>,
    author_from: Option<AuthorFrom>,
    no_readme: bool,
) -> Result<()> {
    fs_err::create_dir_all(path)?;

    // Initialize the version control system first so that Git configuration can properly
    // read conditional includes that depend on the repository path.
    init_vcs(path, vcs)?;

    // Do not fill in `authors` for non-packaged applications unless explicitly requested.
    let author_from = author_from.unwrap_or_else(|| match project_kind {
        InitProjectKind::ApplicationWithLibrary
        | InitProjectKind::Library
        | InitProjectKind::BareWithBuildSystem => AuthorFrom::default(),
        InitProjectKind::Application | InitProjectKind::Bare => AuthorFrom::None,
    });
    let author = get_author_info(path, author_from);

    // Create the `pyproject.toml`
    let mut pyproject = pyproject_project(
        name,
        requires_python,
        author.as_ref(),
        description,
        no_description,
        no_readme || bare,
    );

    match project_kind {
        // Create only the most barebones `pyproject.toml`, no build system
        InitProjectKind::Bare => {}
        // Create only a barebones `pyproject.toml`, but with a build system table
        InitProjectKind::BareWithBuildSystem => {
            // Add a build system
            let build_backend = build_backend.unwrap_or(ProjectBuildBackend::Uv);
            pyproject.push('\n');
            pyproject.push_str(&pyproject_build_system(name, build_backend));
        }
        InitProjectKind::ApplicationWithLibrary => {
            // Since it'll be packaged, we can add a `[project.scripts]` entry
            pyproject.push('\n');
            pyproject.push_str(&pyproject_project_scripts(name, name.as_str(), "main"));

            // Add a build system
            let build_backend = build_backend.unwrap_or(ProjectBuildBackend::Uv);
            pyproject.push('\n');
            pyproject.push_str(&pyproject_build_system(name, build_backend));
            write_template_files(
                path,
                template::build_backend_prerequisites(name, build_backend),
            )?;

            // Generate `src` files with app-style `main()` in `__init__.py`
            write_package_scripts(name, path, build_backend, false)?;
        }
        InitProjectKind::Application => {
            let main_contents = template::application_script(name);

            // Create `main.py` if it doesn't exist
            // (This isn't intended to be a particularly special or magical filename, just nice)
            // TODO(zanieb): Only create `main.py` if there are no other Python files?
            let main_py = path.join("main.py");
            if !main_py.try_exists()? && !bare {
                fs_err::write(path.join("main.py"), main_contents)?;
            }
        }
        InitProjectKind::Library => {
            let build_backend = build_backend.unwrap_or(ProjectBuildBackend::Uv);
            pyproject.push('\n');
            pyproject.push_str(&pyproject_build_system(name, build_backend));
            write_template_files(
                path,
                template::build_backend_prerequisites(name, build_backend),
            )?;

            // Generate `src` files
            write_package_scripts(name, path, build_backend, true)?;
        }
    }
    fs_err::write(path.join("pyproject.toml"), pyproject)?;
    Ok(())
}

/// Write generated files without replacing existing project files.
fn write_template_files(path: &Path, files: Vec<template::TemplateFile>) -> Result<()> {
    for file in files {
        let target = path.join(file.path);
        if !target.try_exists()? {
            fs_err::write(target, file.contents)?;
        }
    }
    Ok(())
}

/// Create the package directory before writing its generated sources.
fn write_package_scripts(
    package: &PackageName,
    path: &Path,
    build_backend: ProjectBuildBackend,
    is_lib: bool,
) -> Result<()> {
    let template = template::package_scripts(package, build_backend, is_lib);
    fs_err::create_dir_all(path.join(template.package_dir))?;
    write_template_files(path, template.files)
}

#[derive(Debug, Clone)]
enum GitDiscoveryResult {
    /// Git is initialized at the path.
    Repository,
    /// Git is not initialized at the path.
    NoRepository,
    /// There is no `git[.exe]` binary in PATH.
    NoGit,
    /// There is a `git[.exe]` binary in PATH, but it returned an unexpected output.
    BrokenGit,
}

/// Checks if there is a Git work tree at the given path.
fn detect_git_repository(path: &Path) -> GitDiscoveryResult {
    // Determine whether the path is inside a Git work tree.
    let Ok(git) = GIT.as_ref() else {
        return GitDiscoveryResult::NoGit;
    };
    let Ok(output) = git
        .build_command()
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .env(EnvVars::LC_ALL, "C")
        .current_dir(path)
        .output()
    else {
        debug!(
            "`git rev-parse --is-inside-work-tree` failed to launch for `{}`",
            path.display()
        );
        return GitDiscoveryResult::BrokenGit;
    };
    if output.status.success() {
        if std::str::from_utf8(&output.stdout).map(str::trim) == Ok("true") {
            debug!("Found a Git repository for `{}`", path.display());
            GitDiscoveryResult::Repository
        } else {
            debug!(
                "`git rev-parse --is-inside-work-tree` succeeded but didn't return `true` for `{}`",
                path.display()
            );
            trace!(
                "`git rev-parse --is-inside-work-tree` stdout: {:?}",
                String::from_utf8_lossy(&output.stdout)
            );
            GitDiscoveryResult::BrokenGit
        }
    } else {
        if std::str::from_utf8(&output.stderr).is_ok_and(|err| err.contains("not a git repository"))
        {
            debug!("Not a Git repository `{}`", path.display());
            GitDiscoveryResult::NoRepository
        } else {
            debug!(
                "`git rev-parse --is-inside-work-tree` failed but didn't contain `not a git repository` in stderr for `{}`",
                path.display()
            );
            GitDiscoveryResult::BrokenGit
        }
    }
}

/// Initialize the version control system at the given path, if applicable.
fn init_vcs(path: &Path, vcs: Option<VersionControlSystem>) -> Result<()> {
    // vcs is None for an existing repository because we don't want to initialize again.
    let (vcs, implicit) = match vcs {
        None => match detect_git_repository(path) {
            GitDiscoveryResult::NoRepository => (VersionControlSystem::Git, true),
            GitDiscoveryResult::Repository
            | GitDiscoveryResult::NoGit
            | GitDiscoveryResult::BrokenGit => (VersionControlSystem::None, false),
        },
        Some(VersionControlSystem::None) => (VersionControlSystem::None, false),
        // The user requested Git explicitly, so the only reason not to invoke it is that Git is
        // already initialized. In case of an error (broken git), we will raise the real error
        // when trying to initialize, which should give us a better error message.
        Some(VersionControlSystem::Git) => match detect_git_repository(path) {
            GitDiscoveryResult::NoRepository
            | GitDiscoveryResult::BrokenGit
            | GitDiscoveryResult::NoGit => (VersionControlSystem::Git, false),
            GitDiscoveryResult::Repository => (VersionControlSystem::None, false),
        },
    };

    // Attempt to initialize the VCS.
    match vcs.init(path) {
        Ok(()) => Ok(()),
        // If the VCS isn't installed, only raise an error if a VCS was explicitly specified.
        Err(err @ VersionControlError::GitNotInstalled) if implicit => {
            debug!("Failed to initialize version control: {err}");
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

/// Try to get the author information.
///
/// Currently, this only tries to get the author information from git.
fn get_author_info(path: &Path, author_from: AuthorFrom) -> Option<Author> {
    if matches!(author_from, AuthorFrom::None) {
        return None;
    }
    if matches!(author_from, AuthorFrom::Auto | AuthorFrom::Git) {
        match get_author_from_git(path) {
            Ok(author) => return Some(author),
            Err(err) => warn!("Failed to get author from git: {err}"),
        }
    }

    None
}

/// Fetch the default author from git configuration.
fn get_author_from_git(path: &Path) -> Result<Author> {
    let Ok(git) = GIT.as_ref() else {
        anyhow::bail!("`git` not found in PATH")
    };

    let mut name = None;
    let mut email = None;

    let output = git
        .build_command()
        .arg("config")
        .arg("--get")
        .arg("user.name")
        .current_dir(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;
    if output.status.success() {
        name = Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    let output = git
        .build_command()
        .arg("config")
        .arg("--get")
        .arg("user.email")
        .current_dir(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;
    if output.status.success() {
        email = Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    let author = match (name, email) {
        (Some(name), Some(email)) => Author::NameEmail { name, email },
        (Some(name), None) => Author::Name(name),
        (None, Some(email)) => Author::Email(email),
        (None, None) => anyhow::bail!("No author information found"),
    };

    Ok(author)
}
