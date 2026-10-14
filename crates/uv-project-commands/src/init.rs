use std::fmt::Write;
use std::iter;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use owo_colors::OwoColorize;
use toml_edit::{InlineTable, Value};
use tracing::{debug, trace, warn};

use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_command_support::{ExitStatus, Printer};
use uv_configuration::{
    AuthorFrom, DependencyGroupsWithDefaults, InitKind, InitProjectKind, ProjectBuildBackend,
    VersionControlSystem,
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
use uv_python_interpreter::{Interpreter, PythonEnvironment};
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
                Some(_) => {
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
            Ok(_) if no_workspace => {
                debug!("Ignoring discovered workspace due to `--no-workspace`");
                None
            }
            Ok(workspace) => Some(workspace),
            Err(err)
                if matches!(
                    err.as_ref(),
                    WorkspaceErrorKind::MissingPyprojectToml | WorkspaceErrorKind::NonWorkspace(_)
                ) =>
            {
                if no_workspace {
                    warn!("`--no-workspace` was provided, but no workspace was found");
                }
                None
            }
            Err(err) if no_workspace => {
                warn!("Ignoring workspace discovery error due to `--no-workspace`: {err}");
                None
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "Failed to discover parent workspace; use `{}` to ignore",
                        "uv init --no-workspace".green()
                    )
                });
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

    if let Some(workspace) = workspace.as_deref() {
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
    }

    if let Some(python_request) = python_pin {
        // Standalone projects retain any local pin. Projects in a workspace reuse a matching pin
        // from either the project or workspace root.
        let reuse_version_file =
            PythonVersionFile::discover(path, &VersionFileDiscoveryOptions::default())
                .await?
                .is_some_and(|file| {
                    let Some(version) = file.version() else {
                        return false;
                    };
                    match workspace.as_deref() {
                        Some(workspace) => {
                            *version == python_request
                                && file.path().parent().is_some_and(|parent| {
                                    parent == workspace.install_path() || parent == path
                                })
                        }
                        None => file.path().parent() == Some(path),
                    }
                });
        if !reuse_version_file {
            PythonVersionFile::new(path.join(".python-version"))
                .with_versions(vec![python_request])
                .write()
                .await?;
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
    let find_interpreter = async |request: Option<&PythonRequest>| {
        PythonInstallation::find_or_download(
            request,
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
        .await
        .map(PythonInstallation::into_interpreter)
    };

    // Add a `requires-python` field to the `pyproject.toml` and return the corresponding interpreter.
    if let Some(python_request) = python_request {
        // (1) A request from the user or `.python-version` file
        // This can be arbitrary, i.e., not a version — in which case we may need to resolve the
        // interpreter
        let (requires_python, python_pin) = match &python_request {
            PythonRequest::Version(
                version_request @ (VersionRequest::MajorMinor(major, minor, _)
                | VersionRequest::MajorMinorPatch(major, minor, _, _)),
            ) => {
                let release = [*major, *minor].into_iter().chain(version_request.patch());
                let requires_python = RequiresPython::greater_than_equal_version(&Version::new(
                    release.map(u64::from),
                ));
                (requires_python, pin_python.then(|| python_request.clone()))
            }
            python_request @ PythonRequest::Version(VersionRequest::Range(specifiers, variant)) => {
                let requires_python = RequiresPython::from_specifiers(specifiers.clone());

                let python_pin = if pin_python {
                    let interpreter = find_interpreter(Some(python_request)).await?;

                    Some(python_minor_pin(&interpreter, *variant))
                } else {
                    None
                };

                (requires_python, python_pin)
            }
            python_request => {
                let interpreter = find_interpreter(Some(python_request)).await?;

                let requires_python =
                    RequiresPython::greater_than_equal_version(&interpreter.python_minor_version());

                let python_pin =
                    pin_python.then(|| python_minor_pin(&interpreter, PythonVariant::Default));

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

        let python_pin = pin_python.then(|| python_minor_pin(&interpreter, PythonVariant::Default));

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

        let python_pin = if pin_python {
            let interpreter = find_interpreter(Some(&python_request)).await?;

            Some(python_minor_pin(&interpreter, PythonVariant::Default))
        } else {
            None
        };

        debug!("Using Python version `{requires_python}` from project workspace");

        Ok((requires_python, python_pin))
    } else {
        // (4) Default to the system Python
        let interpreter = find_interpreter(None).await?;

        let requires_python =
            RequiresPython::greater_than_equal_version(&interpreter.python_minor_version());

        let python_pin = pin_python.then(|| python_minor_pin(&interpreter, PythonVariant::Default));

        debug!("Using Python version `{requires_python}` from default interpreter");

        Ok((requires_python, python_pin))
    }
}

/// Pin to the interpreter's minor version with the specified Python variant.
fn python_minor_pin(interpreter: &Interpreter, variant: PythonVariant) -> PythonRequest {
    PythonRequest::Version(VersionRequest::MajorMinor(
        interpreter.python_major(),
        interpreter.python_minor(),
        variant,
    ))
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
        InitProjectKind::BareWithBuildSystem
        | InitProjectKind::ApplicationWithLibrary
        | InitProjectKind::Library => {
            if matches!(project_kind, InitProjectKind::ApplicationWithLibrary) {
                pyproject.push('\n');
                pyproject.push_str(&pyproject_project_scripts(name, name.as_str(), "main"));
            }

            let build_backend = build_backend.unwrap_or(ProjectBuildBackend::Uv);
            pyproject.push('\n');
            pyproject.push_str(&pyproject_build_system(name, build_backend));
            if !matches!(project_kind, InitProjectKind::BareWithBuildSystem) {
                pyproject_build_backend_prerequisites(name, path, build_backend)?;
                generate_package_scripts(
                    name,
                    path,
                    build_backend,
                    matches!(project_kind, InitProjectKind::Library),
                )?;
            }
        }
        InitProjectKind::Application => {
            let main_contents = indoc::formatdoc! {r#"
                    def main():
                        print("Hello from {name}!")


                    if __name__ == "__main__":
                        main()
                "#};

            // Create `main.py` if it doesn't exist
            // (This isn't intended to be a particularly special or magical filename, just nice)
            // TODO(zanieb): Only create `main.py` if there are no other Python files?
            let main_py = path.join("main.py");
            if !main_py.try_exists()? && !bare {
                fs_err::write(path.join("main.py"), main_contents)?;
            }
        }
    }
    fs_err::write(path.join("pyproject.toml"), pyproject)?;
    Ok(())
}

#[derive(Debug)]
enum Author {
    Name(String),
    Email(String),
    NameEmail { name: String, email: String },
}

impl Author {
    fn to_toml_string(&self) -> String {
        let mut inline = InlineTable::new();

        match self {
            Self::NameEmail { name, email } => {
                inline.insert("name", Value::from(name));
                inline.insert("email", Value::from(email));
            }
            Self::Name(name) => {
                inline.insert("name", Value::from(name));
            }
            Self::Email(email) => {
                inline.insert("email", Value::from(email));
            }
        }

        inline.to_string()
    }
}

/// Generate the `[project]` section of a `pyproject.toml`.
fn pyproject_project(
    name: &PackageName,
    requires_python: &RequiresPython,
    author: Option<&Author>,
    description: Option<&str>,
    no_description: bool,
    no_readme: bool,
) -> String {
    indoc::formatdoc! {r#"
        [project]
        name = "{name}"
        version = "0.1.0"{description}{readme}{authors}
        requires-python = "{requires_python}"
        dependencies = []
    "#,
        readme = if no_readme { "" } else { "\nreadme = \"README.md\"" },
        description = if no_description {
            String::new()
        } else {
            format!("\ndescription = {}", Value::from(description.unwrap_or("Add your description here")))
        },
        authors = author.map_or_else(String::new, |author| format!("\nauthors = [\n    {}\n]", author.to_toml_string())),
        requires_python = requires_python.specifiers(),
    }
}

/// Generate the `[build-system]` section of a `pyproject.toml`.
/// Generate the `[tool.]` section of a `pyproject.toml` where applicable.
fn pyproject_build_system(package: &PackageName, build_backend: ProjectBuildBackend) -> String {
    let module_name = package.as_dist_info_name();
    match build_backend {
        ProjectBuildBackend::Uv => {
            // Limit to the stable version range.
            let min_version = Version::from_str(uv_version::version()).unwrap();
            debug_assert!(
                min_version.release()[0] == 0,
                "migrate to major version bumps"
            );
            let max_version = Version::new(
                [0, min_version.release()[1] + 1]
                    .into_iter()
                    // Add trailing zeroes to match the version length, to use the same style
                    // as `--bounds`.
                    .chain(iter::repeat_n(0, min_version.release().len() - 2)),
            );
            indoc::formatdoc! {r#"
                [build-system]
                requires = ["uv_build>={min_version},<{max_version}"]
                build-backend = "uv_build"
            "#}
        },
        // Pure-python backends
        ProjectBuildBackend::Hatch => indoc::indoc! {r#"
                [build-system]
                requires = ["hatchling"]
                build-backend = "hatchling.build"
            "#}
        .to_string(),
        ProjectBuildBackend::Flit => indoc::indoc! {r#"
                [build-system]
                requires = ["flit_core>=3.2,<4"]
                build-backend = "flit_core.buildapi"
            "#}
        .to_string(),
        ProjectBuildBackend::PDM => indoc::indoc! {r#"
                [build-system]
                requires = ["pdm-backend"]
                build-backend = "pdm.backend"
            "#}
        .to_string(),
        ProjectBuildBackend::Setuptools => indoc::indoc! {r#"
                [build-system]
                requires = ["setuptools>=61"]
                build-backend = "setuptools.build_meta"
            "#}
        .to_string(),
        ProjectBuildBackend::Poetry => indoc::indoc! {r#"
                [build-system]
                requires = ["poetry-core>=2,<3"]
                build-backend = "poetry.core.masonry.api"
            "#}
        .to_string(),
        // Binary build backends
        ProjectBuildBackend::Maturin => indoc::formatdoc! {r#"
                [tool.maturin]
                module-name = "{module_name}._core"
                python-packages = ["{module_name}"]
                python-source = "src"

                [tool.uv]
                cache-keys = [{{ file = "pyproject.toml" }}, {{ file = "src/**/*.rs" }}, {{ file = "Cargo.toml" }}, {{ file = "Cargo.lock" }}]

                [build-system]
                requires = ["maturin>=1.0,<2.0"]
                build-backend = "maturin"
            "#},
        ProjectBuildBackend::Scikit => indoc::indoc! {r#"
                [tool.scikit-build]
                minimum-version = "build-system.requires"
                build-dir = "build/{wheel_tag}"

                [tool.uv]
                cache-keys = [{ file = "pyproject.toml" }, { file = "src/**/*.{h,c,hpp,cpp}" }, { file = "CMakeLists.txt" }]

                [build-system]
                requires = ["scikit-build-core>=0.12", "pybind11>=3"]
                build-backend = "scikit_build_core.build"
            "#}
        .to_string(),
    }
}

/// Generate the `[project.scripts]` section of a `pyproject.toml`.
fn pyproject_project_scripts(package: &PackageName, executable_name: &str, target: &str) -> String {
    let module_name = package.as_dist_info_name();
    indoc::formatdoc! {r#"
        [project.scripts]
        {executable_name} = "{module_name}:{target}"
    "#}
}

/// Generate additional files as needed for specific build backends.
fn pyproject_build_backend_prerequisites(
    package: &PackageName,
    path: &Path,
    build_backend: ProjectBuildBackend,
) -> Result<()> {
    let module_name = package.as_dist_info_name();
    match build_backend {
        ProjectBuildBackend::Maturin => {
            // Generate Cargo.toml
            let build_file = path.join("Cargo.toml");
            if !build_file.try_exists()? {
                fs_err::write(
                    build_file,
                    indoc::formatdoc! {r#"
                    [package]
                    name = "{module_name}"
                    version = "0.1.0"
                    edition = "2024"

                    [lib]
                    name = "_core"
                    # "cdylib" is necessary to produce a shared library for Python to import from.
                    crate-type = ["cdylib"]

                    [dependencies]
                    # "extension-module" tells pyo3 we want to build an extension module (skips linking against libpython.so)
                    # "abi3-py39" tells pyo3 (and maturin) to build using the stable ABI with minimum Python version 3.9
                    pyo3 = {{ version = "0.28.2", features = ["extension-module", "abi3-py39"] }}
                "#},
                )?;
            }
        }
        ProjectBuildBackend::Scikit => {
            // Generate CMakeLists.txt
            let build_file = path.join("CMakeLists.txt");
            if !build_file.try_exists()? {
                fs_err::write(
                    build_file,
                    indoc::formatdoc! {r"
                    cmake_minimum_required(VERSION 3.15...4.0)
                    project(${{SKBUILD_PROJECT_NAME}} LANGUAGES CXX)

                    find_package(pybind11 CONFIG REQUIRED)

                    pybind11_add_module(_core MODULE src/main.cpp)
                    install(TARGETS _core DESTINATION ${{SKBUILD_PROJECT_NAME}})
                "},
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Generate startup scripts for a package-based application or library.
fn generate_package_scripts(
    package: &PackageName,
    path: &Path,
    build_backend: ProjectBuildBackend,
    is_lib: bool,
) -> Result<()> {
    let module_name = package.as_dist_info_name();

    let src_dir = path.join("src");
    let pkg_dir = src_dir.join(&*module_name);
    fs_err::create_dir_all(&pkg_dir)?;

    // Both native backends expose the same Python interface.
    let generate_native_package_script = || -> Result<String> {
        let pyi_file = pkg_dir.join("_core.pyi");
        if !pyi_file.try_exists()? {
            fs_err::write(
                pyi_file,
                indoc::indoc! {r"
                    def hello_from_bin() -> str: ...
                "},
            )?;
        }

        Ok(if is_lib {
            indoc::formatdoc! {r"
            from {module_name}._core import hello_from_bin


            def hello() -> str:
                return hello_from_bin()
            "}
        } else {
            indoc::formatdoc! {r"
            from {module_name}._core import hello_from_bin


            def main() -> None:
                print(hello_from_bin())
            "}
        })
    };

    let package_script = match build_backend {
        ProjectBuildBackend::Maturin => {
            // Generate lib.rs
            let native_src = src_dir.join("lib.rs");
            if !native_src.try_exists()? {
                fs_err::write(
                    native_src,
                    indoc::formatdoc! {r#"
                    use pyo3::prelude::*;

                    /// A Python module implemented in Rust. The name of this module must match
                    /// the `lib.name` setting in the `Cargo.toml`, else Python will not be able to
                    /// import the module.
                    #[pymodule]
                    mod _core {{
                        use pyo3::prelude::*;

                        #[pyfunction]
                        fn hello_from_bin() -> String {{
                            "Hello from {package}!".to_string()
                        }}
                    }}
                "#},
                )?;
            }
            generate_native_package_script()?
        }
        ProjectBuildBackend::Scikit => {
            // Generate main.cpp
            let native_src = src_dir.join("main.cpp");
            if !native_src.try_exists()? {
                fs_err::write(
                    native_src,
                    indoc::formatdoc! {r#"
                    #include <pybind11/pybind11.h>

                    std::string hello_from_bin() {{ return "Hello from {package}!"; }}

                    namespace py = pybind11;

                    PYBIND11_MODULE(_core, m) {{
                      m.doc() = "pybind11 hello module";

                      m.def("hello_from_bin", &hello_from_bin, R"pbdoc(
                          A function that returns a Hello string.
                      )pbdoc");
                    }}
                "#},
                )?;
            }
            generate_native_package_script()?
        }
        _ if is_lib => indoc::formatdoc! {r#"
            def hello() -> str:
                return "Hello from {package}!"
            "#},
        _ => indoc::formatdoc! {r#"
            def main() -> None:
                print("Hello from {package}!")
            "#},
    };

    // Create `src/{name}/__init__.py`, if it doesn't exist already.
    let init_py = pkg_dir.join("__init__.py");
    if !init_py.try_exists()? {
        fs_err::write(init_py, package_script)?;
    }

    // Create `src/{name}/py.typed`, if it doesn't exist already.
    if is_lib {
        let py_typed = pkg_dir.join("py.typed");
        if !py_typed.try_exists()? {
            fs_err::write(py_typed, "")?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
enum GitDiscoveryResult {
    /// Git is initialized at the path.
    Repository,
    /// Git is not initialized at the path.
    NoRepository,
    /// Git repository discovery did not produce a definitive result.
    Unknown,
}

/// Checks if there is a Git work tree at the given path.
fn detect_git_repository(path: &Path) -> GitDiscoveryResult {
    // Determine whether the path is inside a Git work tree.
    let Ok(git) = GIT.as_ref() else {
        return GitDiscoveryResult::Unknown;
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
        return GitDiscoveryResult::Unknown;
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
            GitDiscoveryResult::Unknown
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
            GitDiscoveryResult::Unknown
        }
    }
}

/// Initialize the version control system at the given path, if applicable.
fn init_vcs(path: &Path, vcs: Option<VersionControlSystem>) -> Result<()> {
    let initialize_git = match vcs {
        None => matches!(
            detect_git_repository(path),
            GitDiscoveryResult::NoRepository
        ),
        Some(VersionControlSystem::None) => false,
        // An explicit request should report initialization errors even if discovery failed.
        Some(VersionControlSystem::Git) => {
            !matches!(detect_git_repository(path), GitDiscoveryResult::Repository)
        }
    };
    if initialize_git {
        VersionControlSystem::Git.init(path)?;
    }
    Ok(())
}

/// Try to get the author information.
///
/// Currently, this only tries to get the author information from git.
fn get_author_info(path: &Path, author_from: AuthorFrom) -> Option<Author> {
    match author_from {
        AuthorFrom::None => None,
        AuthorFrom::Auto | AuthorFrom::Git => get_author_from_git(path)
            .inspect_err(|err| warn!("Failed to get author from git: {err}"))
            .ok(),
    }
}

/// Fetch the default author from git configuration.
fn get_author_from_git(path: &Path) -> Result<Author> {
    let Ok(git) = GIT.as_ref() else {
        anyhow::bail!("`git` not found in PATH")
    };

    let read_config = |key: &str| -> std::io::Result<Option<String>> {
        let output = git
            .build_command()
            .arg("config")
            .arg("--get")
            .arg(key)
            .current_dir(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()))
    };

    let author = match (read_config("user.name")?, read_config("user.email")?) {
        (Some(name), Some(email)) => Author::NameEmail { name, email },
        (Some(name), None) => Author::Name(name),
        (None, Some(email)) => Author::Email(email),
        (None, None) => anyhow::bail!("No author information found"),
    };

    Ok(author)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_to_toml_string_handles_inline_quotes() {
        let author = Author::NameEmail {
            name: "Tony \"Iron Man\" Stark".to_string(),
            email: "ironman@example.com".to_string(),
        };

        assert_eq!(
            author.to_toml_string(),
            "{ name = 'Tony \"Iron Man\" Stark', email = \"ironman@example.com\" }"
        );
    }
}
