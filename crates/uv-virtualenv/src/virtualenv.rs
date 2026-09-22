//! Create a virtual environment.

use std::borrow::Cow;
use std::env::consts::EXE_SUFFIX;
use std::ffi::{OsStr, OsString};
use std::io;
use std::io::{BufWriter, Write};
use std::path::Path;

use console::Term;
use fs_err::File;
use itertools::Itertools;
use owo_colors::OwoColorize;

use tracing::{debug, trace};

use crate::{Error, Prompt};
use uv_fs::{CWD, PythonExt, Simplified, cachedir};
use uv_platform_tags::Os;
use uv_preview::PreviewFeature;
use uv_pypi_types::Scheme;
use uv_python::managed::{
    ManagedPythonInstallation, PythonExecutable, PythonMinorVersionLink, replace_link_to_executable,
};
use uv_python::{Interpreter, VirtualEnvironment};
use uv_shell::escape_posix_for_single_quotes;
use uv_version::version;

/// Activation scripts for the environment, with dependent paths templated out.
const ACTIVATE_TEMPLATES: &[(&str, &str)] = &[
    ("activate", include_str!("activator/activate")),
    ("activate.csh", include_str!("activator/activate.csh")),
    ("activate.fish", include_str!("activator/activate.fish")),
    ("activate.nu", include_str!("activator/activate.nu")),
    ("activate.xsh", include_str!("activator/activate.xsh")),
    ("activate.ps1", include_str!("activator/activate.ps1")),
    ("activate.bat", include_str!("activator/activate.bat")),
    ("deactivate.bat", include_str!("activator/deactivate.bat")),
    ("pydoc.bat", include_str!("activator/pydoc.bat")),
    (
        "activate_this.py",
        include_str!("activator/activate_this.py"),
    ),
];
const VIRTUALENV_PATCH: &str = include_str!("_virtualenv.py");

/// Python 3.10 and later already ignore the distutils install config keys this hook guards
/// against, while the last pip release supporting Python 3.9 still needs the workaround.
///
/// See <https://github.com/pypa/virtualenv/issues/3181>
fn install_distutils_patch(interpreter: &Interpreter) -> bool {
    interpreter.python_tuple() < (3, 10)
        || !uv_preview::is_enabled(PreviewFeature::NoDistutilsPatch)
}

/// Very basic `.cfg` file format writer.
fn write_cfg(f: &mut impl Write, data: &[(String, String)]) -> io::Result<()> {
    for (key, value) in data {
        writeln!(f, "{key} = {value}")?;
    }
    Ok(())
}

/// Create a [`VirtualEnvironment`] at the given location.
pub(crate) fn create(
    location: &Path,
    interpreter: &Interpreter,
    prompt: Prompt,
    system_site_packages: bool,
    on_existing: OnExisting,
    relocatable: bool,
    seed: Seed,
    upgradeable: bool,
) -> Result<VirtualEnvironment, Error> {
    let debug = interpreter.implementation_name() == "cpython" && interpreter.debug_enabled();

    // Determine the base Python executable; that is, the Python executable that should be
    // considered the "base" for the virtual environment.
    //
    // For consistency with the standard library, rely on `sys._base_executable`, _unless_ we're
    // using a uv-managed Python (in which case, we can do better for symlinked executables).
    let base_python = if cfg!(unix) && interpreter.is_standalone() {
        interpreter.find_base_python()?
    } else {
        interpreter.to_base_python()?
    };

    debug!(
        "Using base executable for virtual environment: {}",
        base_python.display()
    );

    // Extract the prompt and compute the absolute path prior to validating the location; otherwise,
    // we risk deleting (and recreating) the current working directory, which would cause the `CWD`
    // queries to fail.
    let prompt = match prompt {
        Prompt::CurrentDirectoryName => CWD
            .file_name()
            .map(|name| name.to_string_lossy().to_string()),
        Prompt::Static(value) => Some(value),
        Prompt::None => None,
    };
    let absolute = std::path::absolute(location)?;

    // Validate the path before creating the virtual environment, since some filesystems, e.g.,
    // APFS, reject non-UTF-8 paths before the activation scripts are generated.
    if absolute.simplified().to_str().is_none() {
        return Err(Error::NonUtf8Path { path: absolute });
    }

    // Validate the existing location.
    match location.metadata() {
        Ok(metadata) if metadata.is_file() => {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("File exists at `{}`", location.user_display()),
            )));
        }
        Ok(metadata)
            if metadata.is_dir()
                && location
                    .read_dir()
                    .is_ok_and(|mut dir| dir.next().is_none()) =>
        {
            // If it's an empty directory, we can proceed
            trace!(
                "Using empty directory at `{}` for virtual environment",
                location.user_display()
            );
        }
        Ok(metadata) if metadata.is_dir() => {
            let is_virtualenv = uv_fs::is_virtualenv_base(location);
            let name = if is_virtualenv {
                "virtual environment"
            } else {
                "directory"
            };
            // TODO(zanieb): We may want to consider omitting the hint in some of these cases, e.g.,
            // when `--no-clear` is used do we want to suggest `--clear`?
            let err = Err(Error::Exists {
                name,
                path: location.to_path_buf(),
            });
            match on_existing {
                OnExisting::Allow => {
                    debug!("Allowing existing {name} due to `--allow-existing`");
                }
                OnExisting::Remove(reason) => {
                    if !is_virtualenv
                        && let RemovalReason::UserRequest(clear_non_virtualenv) = reason
                    {
                        match clear_non_virtualenv {
                            ClearNonVirtualenv::Allow => {}
                            ClearNonVirtualenv::Error => {
                                return Err(Error::ClearNonVirtualenv {
                                    path: location.to_path_buf(),
                                });
                            }
                        }
                    }
                    debug!("Removing existing {name} ({reason})");
                    uv_fs::clear_virtualenv(location)?;
                }
                OnExisting::Fail => return err,
                // If not a virtual environment, fail without prompting.
                OnExisting::Prompt if !is_virtualenv => return err,
                OnExisting::Prompt => {
                    match confirm_clear(location, name)? {
                        Some(true) => {
                            debug!("Removing existing {name} due to confirmation");
                            uv_fs::clear_virtualenv(location)?;
                        }
                        Some(false) => return err,
                        // When we don't have a TTY, require `--clear` explicitly.
                        None => {
                            return Err(Error::Exists {
                                name,
                                path: location.to_path_buf(),
                            });
                        }
                    }
                }
            }
        }
        Ok(_) => {
            // It's not a file or a directory
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("Object already exists at `{}`", location.user_display()),
            )));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            fs_err::create_dir_all(location)?;
        }
        Err(err) => return Err(Error::Io(err)),
    }

    // Use the absolute path for all further operations.
    let location = absolute;

    let bin_name = if cfg!(unix) {
        "bin"
    } else if cfg!(windows) {
        "Scripts"
    } else {
        unimplemented!("Only Windows and Unix are supported")
    };
    let scripts = location.join(&interpreter.virtualenv().scripts);

    // Add the CACHEDIR.TAG.
    cachedir::ensure_tag(&location)?;

    // Create a `.gitignore` file to ignore all files in the venv.
    fs_err::write(location.join(".gitignore"), "*")?;

    let mut using_minor_version_link = false;
    let executable_target = if upgradeable {
        if let Some(minor_version_link) =
            ManagedPythonInstallation::try_from_interpreter(interpreter)
                .and_then(|installation| PythonMinorVersionLink::from_installation(&installation))
        {
            if !minor_version_link.exists() {
                base_python.clone()
            } else {
                let debug_symlink_term = if cfg!(windows) {
                    "junction"
                } else {
                    "symlink directory"
                };
                debug!(
                    "Using {} {} instead of base Python path: {}",
                    debug_symlink_term,
                    &minor_version_link.symlink_directory.display(),
                    &base_python.display()
                );
                using_minor_version_link = true;
                minor_version_link.symlink_executable.clone()
            }
        } else {
            base_python.clone()
        }
    } else {
        base_python.clone()
    };

    // Per PEP 405, the Python `home` is the parent directory of the interpreter.
    // For standalone interpreters, this `home` value will include a
    // symlink directory on Unix or junction on Windows to enable transparent Python patch
    // upgrades.
    let python_home = executable_target
        .parent()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "The Python interpreter needs to have a parent directory",
            )
        })?
        .to_path_buf();
    let python_home = python_home.as_path();

    // Different names for the python interpreter
    fs_err::create_dir_all(&scripts)?;
    let executable = scripts.join(format!("python{EXE_SUFFIX}"));

    #[cfg(unix)]
    {
        uv_fs::replace_symlink(&executable_target, &executable)?;
        uv_fs::replace_symlink(
            "python",
            scripts.join(format!("python{}", interpreter.python_major())),
        )?;
        uv_fs::replace_symlink(
            "python",
            scripts.join(format!(
                "python{}.{}",
                interpreter.python_major(),
                interpreter.python_minor(),
            )),
        )?;
        if interpreter.gil_disabled() {
            uv_fs::replace_symlink(
                "python",
                scripts.join(format!(
                    "python{}.{}t",
                    interpreter.python_major(),
                    interpreter.python_minor(),
                )),
            )?;
        }

        if interpreter.markers().implementation_name() == "pypy" {
            uv_fs::replace_symlink(
                "python",
                scripts.join(format!("pypy{}", interpreter.python_major())),
            )?;
            uv_fs::replace_symlink("python", scripts.join("pypy"))?;
        }

        if interpreter.markers().implementation_name() == "graalpy" {
            uv_fs::replace_symlink("python", scripts.join("graalpy"))?;
        }
    }

    // On Windows, we use trampolines that point to an executable target. For standalone
    // interpreters, this target path includes a minor version junction to enable
    // transparent upgrades.
    if cfg!(windows) {
        if using_minor_version_link {
            let target = scripts.join(WindowsExecutable::Python.exe(interpreter.python_tuple()));
            replace_link_to_executable(
                target.as_path(),
                PythonExecutable::console(&executable_target),
            )
            .map_err(Error::Python)?;
            let windowed_executable_name =
                WindowsExecutable::Pythonw.exe(interpreter.python_tuple());
            let targetw = scripts.join(&windowed_executable_name);
            let mut windowed_executable_target = executable_target.with_file_name(
                WindowsExecutable::Pythonw.source_exe(interpreter.python_tuple(), debug),
            );
            if interpreter.gil_disabled() && !windowed_executable_target.exists() {
                windowed_executable_target = executable_target.with_file_name(
                    WindowsExecutable::PythonwMajorMinort
                        .source_exe(interpreter.python_tuple(), debug),
                );
            }
            replace_link_to_executable(
                targetw.as_path(),
                PythonExecutable::windowed(&windowed_executable_target),
            )
            .map_err(Error::Python)?;
            if interpreter.gil_disabled() {
                let targett = scripts
                    .join(WindowsExecutable::PythonMajorMinort.exe(interpreter.python_tuple()));
                replace_link_to_executable(
                    targett.as_path(),
                    PythonExecutable::console(&executable_target),
                )
                .map_err(Error::Python)?;
                let targetwt = scripts
                    .join(WindowsExecutable::PythonwMajorMinort.exe(interpreter.python_tuple()));
                replace_link_to_executable(
                    targetwt.as_path(),
                    PythonExecutable::windowed(&windowed_executable_target),
                )
                .map_err(Error::Python)?;
            }
        } else if matches!(
            interpreter.platform().os(),
            Os::Pyodide { .. } | Os::PyEmscripten { .. }
        ) {
            // For PyEmscripten, link only `python.exe`.
            // This should not be copied as `python.exe` is a wrapper that launches Pyodide.
            let target = scripts.join(WindowsExecutable::Python.exe(interpreter.python_tuple()));
            replace_link_to_executable(
                target.as_path(),
                PythonExecutable::console(&executable_target),
            )
            .map_err(Error::Python)?;
        } else {
            // Always copy `python.exe`.
            copy_launcher_windows(
                WindowsExecutable::Python,
                interpreter,
                &base_python,
                &scripts,
                python_home,
            )?;

            match interpreter.implementation_name() {
                "graalpy" => {
                    // For GraalPy, copy `graalpy.exe` and `python3.exe`.
                    copy_launcher_windows(
                        WindowsExecutable::GraalPy,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                    copy_launcher_windows(
                        WindowsExecutable::PythonMajor,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                }
                "pypy" => {
                    // For PyPy, copy all versioned executables and all PyPy-specific executables.
                    copy_launcher_windows(
                        WindowsExecutable::PythonMajor,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                    copy_launcher_windows(
                        WindowsExecutable::PythonMajorMinor,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                    copy_launcher_windows(
                        WindowsExecutable::Pythonw,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                    copy_launcher_windows(
                        WindowsExecutable::PyPy,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                    copy_launcher_windows(
                        WindowsExecutable::PyPyMajor,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                    copy_launcher_windows(
                        WindowsExecutable::PyPyMajorMinor,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                    copy_launcher_windows(
                        WindowsExecutable::PyPyw,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                    copy_launcher_windows(
                        WindowsExecutable::PyPyMajorMinorw,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;
                }
                _ => {
                    // For all other interpreters, copy `pythonw.exe`.
                    copy_launcher_windows(
                        WindowsExecutable::Pythonw,
                        interpreter,
                        &base_python,
                        &scripts,
                        python_home,
                    )?;

                    // If the GIL is disabled, copy `venvlaunchert.exe` and `venvwlaunchert.exe`.
                    if interpreter.gil_disabled() {
                        copy_launcher_windows(
                            WindowsExecutable::PythonMajorMinort,
                            interpreter,
                            &base_python,
                            &scripts,
                            python_home,
                        )?;
                        copy_launcher_windows(
                            WindowsExecutable::PythonwMajorMinort,
                            interpreter,
                            &base_python,
                            &scripts,
                            python_home,
                        )?;
                    }
                }
            }
        }

        if debug {
            // Match the stdlib's debug aliases while retaining `python.exe` for activation.
            for executable in [WindowsExecutable::Python, WindowsExecutable::Pythonw]
                .into_iter()
                .chain(
                    interpreter
                        .gil_disabled()
                        .then_some([
                            WindowsExecutable::PythonMajorMinort,
                            WindowsExecutable::PythonwMajorMinort,
                        ])
                        .into_iter()
                        .flatten(),
                )
            {
                fs_err::copy(
                    scripts.join(executable.exe(interpreter.python_tuple())),
                    scripts.join(executable.source_exe(interpreter.python_tuple(), debug)),
                )?;
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        compile_error!("Only Windows and Unix are supported")
    }

    // Add all the activate scripts for different shells
    for (name, template) in ACTIVATE_TEMPLATES {
        // csh has no way to determine its own script location, so a relocatable
        // activate.csh is not possible. Skip it entirely instead of generating a
        // non-functional script.
        if relocatable && *name == "activate.csh" {
            continue;
        }

        let path_sep = if cfg!(windows) { ";" } else { ":" };

        let relative_site_packages = [
            interpreter.virtualenv().purelib.as_path(),
            interpreter.virtualenv().platlib.as_path(),
        ]
        .iter()
        .dedup()
        .map(|path| {
            pathdiff::diff_paths(path, &interpreter.virtualenv().scripts)
                .expect("Failed to calculate relative path to site-packages")
        })
        .map(|path| path.simplified().to_str().unwrap().replace('\\', "\\\\"))
        .join(path_sep);

        let location_string = location
            .simplified()
            .to_str()
            .ok_or_else(|| Error::NonUtf8Path {
                path: location.clone(),
            })?;
        let virtual_env_dir = match (relocatable, name.to_owned()) {
            (true, "activate") => Cow::Borrowed(
                r#"'"$(dirname -- "$(dirname -- "$(realpath -- "$SCRIPT_PATH")")")"'"#,
            ),
            (true, "activate.bat") => Cow::Borrowed(r"%~dp0.."),
            (true, "activate.fish") => {
                Cow::Borrowed(r"'(dirname -- (dirname -- (realpath -- (status -f))))'")
            }
            (true, "activate.nu") => Cow::Borrowed(r"(path self | path dirname | path dirname)"),
            (false, "activate.nu") => Cow::Owned(format!(
                "'{}'",
                escape_posix_for_single_quotes(location_string)
            )),
            // Note: `activate.ps1` is already relocatable by default.
            _ => escape_posix_for_single_quotes(location_string),
        };

        let virtual_prompt = prompt.as_deref().unwrap_or_default();
        let virtual_prompt = match *name {
            "activate.xsh" => Cow::Owned(format!(
                r#"b"{}".decode("utf-8")"#,
                virtual_prompt.as_bytes().escape_ascii(),
            )),
            _ => Cow::Borrowed(virtual_prompt),
        };

        let bin_name = match *name {
            "activate.xsh" => Cow::Owned(bin_name.escape_for_python()),
            _ => Cow::Borrowed(bin_name),
        };

        let activator = template
            .replace("{{ VIRTUAL_ENV_DIR }}", &virtual_env_dir)
            .replace("{{ BIN_NAME }}", &bin_name)
            .replace("{{ VIRTUAL_PROMPT }}", &virtual_prompt)
            .replace("{{ PATH_SEP }}", path_sep)
            .replace("{{ RELATIVE_SITE_PACKAGES }}", &relative_site_packages);
        fs_err::write(scripts.join(name), activator)?;
    }

    let mut pyvenv_cfg_data: Vec<(String, String)> = vec![
        (
            "home".to_string(),
            python_home.simplified_display().to_string(),
        ),
        (
            "implementation".to_string(),
            interpreter
                .markers()
                .platform_python_implementation()
                .to_string(),
        ),
        ("uv".to_string(), version().to_string()),
        (
            "version_info".to_string(),
            if using_minor_version_link {
                interpreter.python_minor_version().to_string()
            } else {
                interpreter.markers().python_full_version().string.clone()
            },
        ),
        (
            "include-system-site-packages".to_string(),
            if system_site_packages {
                "true".to_string()
            } else {
                "false".to_string()
            },
        ),
    ];

    if relocatable {
        pyvenv_cfg_data.push(("relocatable".to_string(), "true".to_string()));
    }

    match seed {
        Seed::Enabled => pyvenv_cfg_data.push(("seed".to_string(), "true".to_string())),
        Seed::Disabled => {}
    }

    if let Some(prompt) = prompt {
        pyvenv_cfg_data.push(("prompt".to_string(), prompt));
    }

    if cfg!(windows) && interpreter.markers().implementation_name() == "graalpy" {
        pyvenv_cfg_data.push((
            "venvlauncher_command".to_string(),
            python_home
                .join("graalpy.exe")
                .simplified_display()
                .to_string(),
        ));
    }

    let mut pyvenv_cfg = BufWriter::new(File::create(location.join("pyvenv.cfg"))?);
    write_cfg(&mut pyvenv_cfg, &pyvenv_cfg_data)?;
    drop(pyvenv_cfg);

    // Construct the path to the `site-packages` directory.
    let site_packages = location.join(&interpreter.virtualenv().purelib);
    fs_err::create_dir_all(&site_packages)?;

    // If necessary, create a symlink from `lib64` to `lib`.
    // See: https://github.com/python/cpython/blob/b228655c227b2ca298a8ffac44d14ce3d22f6faa/Lib/venv/__init__.py#L135C11-L135C16
    #[cfg(unix)]
    if interpreter.pointer_size().is_64()
        && interpreter.markers().os_name() == "posix"
        && interpreter.markers().sys_platform() != "darwin"
    {
        match fs_err::os::unix::fs::symlink("lib", location.join("lib64")) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => {
                return Err(err.into());
            }
        }
    }

    if install_distutils_patch(interpreter) {
        fs_err::write(site_packages.join("_virtualenv.py"), VIRTUALENV_PATCH)?;
        fs_err::write(site_packages.join("_virtualenv.pth"), "import _virtualenv")?;
    }

    Ok(VirtualEnvironment {
        scheme: Scheme {
            purelib: location.join(&interpreter.virtualenv().purelib),
            platlib: location.join(&interpreter.virtualenv().platlib),
            scripts: location.join(&interpreter.virtualenv().scripts),
            data: location.join(&interpreter.virtualenv().data),
            include: location.join(&interpreter.virtualenv().include),
        },
        root: location,
        executable,
        base_executable: base_python,
    })
}

/// Prompt a confirmation that the virtual environment should be cleared.
///
/// If not a TTY, returns `None`.
fn confirm_clear(location: &Path, name: &'static str) -> Result<Option<bool>, io::Error> {
    let term = Term::stderr();
    if term.is_term() {
        let prompt = format!(
            "A {name} already exists at `{}`. Do you want to replace it?",
            location.user_display(),
        );
        let hint = format!(
            "Use the `{}` flag or set `{}` to skip this prompt",
            "--clear".green(),
            "UV_VENV_CLEAR=1".green()
        );
        Ok(Some(uv_console::confirm_with_hint(
            &prompt, &hint, &term, true,
        )?))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ClearNonVirtualenv {
    /// Allow clearing a non-virtual environment directory.
    Allow,
    /// Refuse to clear a non-virtual environment directory.
    Error,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RemovalReason {
    /// The removal was explicitly requested, i.e., with `--clear`.
    UserRequest(ClearNonVirtualenv),
    /// The environment can be removed because it is considered temporary, e.g., a build
    /// environment.
    TemporaryEnvironment,
    /// The environment can be removed because it is managed by uv, e.g., a project or tool
    /// environment.
    ManagedEnvironment,
}

impl std::fmt::Display for RemovalReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserRequest(_) => f.write_str("requested with `--clear`"),
            Self::ManagedEnvironment => f.write_str("environment is managed by uv"),
            Self::TemporaryEnvironment => f.write_str("environment is temporary"),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum OnExisting {
    /// Prompt before removing an existing directory.
    ///
    /// If a TTY is not available, fail.
    #[default]
    Prompt,
    /// Fail if the directory already exists and is non-empty.
    Fail,
    /// Allow an existing directory, overwriting virtual environment files while retaining other
    /// files in the directory.
    Allow,
    /// Remove an existing directory.
    Remove(RemovalReason),
}

impl OnExisting {
    pub fn from_args(
        allow_existing: bool,
        clear: bool,
        no_clear: bool,
        clear_non_virtualenv: ClearNonVirtualenv,
    ) -> Self {
        if allow_existing {
            Self::Allow
        } else if clear {
            Self::Remove(RemovalReason::UserRequest(clear_non_virtualenv))
        } else if no_clear {
            Self::Fail
        } else {
            Self::Prompt
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum Seed {
    /// Seed the virtual environment with one or more of `pip`, `setuptools`, and `wheel`.
    Enabled,
    /// Do not seed the virtual environment.
    #[default]
    Disabled,
}

impl Seed {
    /// Determine the [`Seed`] setting based on the command-line arguments.
    pub fn from_args(seed: bool) -> Self {
        if seed { Self::Enabled } else { Self::Disabled }
    }
}

#[derive(Debug, Copy, Clone)]
enum WindowsExecutable {
    /// The `python.exe` executable (or `venvlauncher.exe` launcher shim).
    Python,
    /// The `python3.exe` executable (or `venvlauncher.exe` launcher shim).
    PythonMajor,
    /// The `python3.<minor>.exe` executable (or `venvlauncher.exe` launcher shim).
    PythonMajorMinor,
    /// The `python3.<minor>t.exe` executable (or `venvlaunchert.exe` launcher shim).
    PythonMajorMinort,
    /// The `pythonw.exe` executable (or `venvwlauncher.exe` launcher shim).
    Pythonw,
    /// The `pythonw3.<minor>t.exe` executable (or `venvwlaunchert.exe` launcher shim).
    PythonwMajorMinort,
    /// The `pypy.exe` executable.
    PyPy,
    /// The `pypy3.exe` executable.
    PyPyMajor,
    /// The `pypy3.<minor>.exe` executable.
    PyPyMajorMinor,
    /// The `pypyw.exe` executable.
    PyPyw,
    /// The `pypy3.<minor>w.exe` executable.
    PyPyMajorMinorw,
    /// The `graalpy.exe` executable.
    GraalPy,
}

impl WindowsExecutable {
    /// The name of the Python executable.
    fn exe(self, python_version: (u8, u8)) -> Cow<'static, OsStr> {
        match self {
            Self::Python => Cow::Borrowed(OsStr::new("python.exe")),
            Self::PythonMajor => {
                Cow::Owned(OsString::from(format!("python{}.exe", python_version.0)))
            }
            Self::PythonMajorMinor => Cow::Owned(OsString::from(format!(
                "python{}.{}.exe",
                python_version.0, python_version.1
            ))),
            Self::PythonMajorMinort => Cow::Owned(OsString::from(format!(
                "python{}.{}t.exe",
                python_version.0, python_version.1
            ))),
            Self::Pythonw => Cow::Borrowed(OsStr::new("pythonw.exe")),
            Self::PythonwMajorMinort => Cow::Owned(OsString::from(format!(
                "pythonw{}.{}t.exe",
                python_version.0, python_version.1
            ))),
            Self::PyPy => Cow::Borrowed(OsStr::new("pypy.exe")),
            Self::PyPyMajor => Cow::Owned(OsString::from(format!("pypy{}.exe", python_version.0))),
            Self::PyPyMajorMinor => Cow::Owned(OsString::from(format!(
                "pypy{}.{}.exe",
                python_version.0, python_version.1
            ))),
            Self::PyPyw => Cow::Borrowed(OsStr::new("pypyw.exe")),
            Self::PyPyMajorMinorw => Cow::Owned(OsString::from(format!(
                "pypy{}.{}w.exe",
                python_version.0, python_version.1
            ))),
            Self::GraalPy => Cow::Borrowed(OsStr::new("graalpy.exe")),
        }
    }

    /// The name supplied by the base interpreter, including its debug suffix.
    fn source_exe(self, python_version: (u8, u8), debug: bool) -> Cow<'static, OsStr> {
        let name = self.exe(python_version);
        if debug {
            let mut name = Path::new(&*name).with_extension("").into_os_string();
            name.push("_d.exe");
            Cow::Owned(name)
        } else {
            name
        }
    }

    /// The name of the launcher shim.
    fn launcher(self, gil_disabled: bool, debug: bool) -> Cow<'static, str> {
        let name = match self {
            Self::Python | Self::PythonMajor | Self::PythonMajorMinor if gil_disabled => {
                "venvlaunchert.exe"
            }
            Self::Python | Self::PythonMajor | Self::PythonMajorMinor => "venvlauncher.exe",
            Self::Pythonw if gil_disabled => "venvwlaunchert.exe",
            Self::Pythonw => "venvwlauncher.exe",
            Self::PythonMajorMinort => "venvlaunchert.exe",
            Self::PythonwMajorMinort => "venvwlaunchert.exe",
            // From 3.13 on these should replace the `python.exe` and `pythonw.exe` shims.
            // These are not relevant as of now for PyPy as it doesn't yet support Python 3.13.
            Self::PyPy | Self::PyPyMajor | Self::PyPyMajorMinor => "venvlauncher.exe",
            Self::PyPyw | Self::PyPyMajorMinorw => "venvwlauncher.exe",
            Self::GraalPy => "venvlauncher.exe",
        };
        if debug {
            Cow::Owned(format!("{}_d.exe", name.trim_end_matches(".exe")))
        } else {
            Cow::Borrowed(name)
        }
    }
}

/// <https://github.com/python/cpython/blob/d457345bbc6414db0443819290b04a9a4333313d/Lib/venv/__init__.py#L261-L267>
/// <https://github.com/pypa/virtualenv/blob/d9fdf48d69f0d0ca56140cf0381edbb5d6fe09f5/src/virtualenv/create/via_global_ref/builtin/cpython/cpython3.py#L78-L83>
///
/// There's two kinds of applications on windows: Those that allocate a console (python.exe)
/// and those that don't because they use window(s) (pythonw.exe).
fn copy_launcher_windows(
    executable: WindowsExecutable,
    interpreter: &Interpreter,
    base_python: &Path,
    scripts: &Path,
    python_home: &Path,
) -> Result<(), Error> {
    let debug = interpreter.implementation_name() == "cpython" && interpreter.debug_enabled();
    if copy_launcher_windows_shim(
        executable,
        interpreter.python_tuple(),
        interpreter.gil_disabled(),
        debug,
        interpreter.stdlib(),
        base_python,
        scripts,
    )? {
        return Ok(());
    }

    // Fourth priority: if the launcher shim doesn't exist, assume this is
    // an embedded Python. Copy the Python executable itself, along with
    // the DLLs, `.pyd` files, and `.zip` files in the same directory.
    match fs_err::copy(
        base_python.with_file_name(executable.source_exe(interpreter.python_tuple(), debug)),
        scripts.join(executable.exe(interpreter.python_tuple())),
    ) {
        Ok(_) => {
            // Copy `.dll` and `.pyd` files from the top-level, and from the
            // `DLLs` subdirectory (if it exists).
            for directory in [
                python_home,
                interpreter.sys_base_prefix().join("DLLs").as_path(),
            ] {
                let entries = match fs_err::read_dir(directory) {
                    Ok(read_dir) => read_dir,
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {
                        continue;
                    }
                    Err(err) => {
                        return Err(err.into());
                    }
                };
                for entry in entries {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("dll") || ext.eq_ignore_ascii_case("pyd")
                    }) {
                        if let Some(file_name) = path.file_name() {
                            fs_err::copy(&path, scripts.join(file_name))?;
                        }
                    }
                }
            }

            // Copy `.zip` files from the top-level.
            match fs_err::read_dir(python_home) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = entry?;
                        let path = entry.path();
                        if path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
                        {
                            if let Some(file_name) = path.file_name() {
                                fs_err::copy(&path, scripts.join(file_name))?;
                            }
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err.into());
                }
            }

            return Ok(());
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err.into());
        }
    }

    Err(Error::NotFound(base_python.user_display().to_string()))
}

/// Copy the stdlib launcher, including layouts used before Python 3.13 and by Conda.
fn copy_launcher_windows_shim(
    executable: WindowsExecutable,
    python_version: (u8, u8),
    gil_disabled: bool,
    debug: bool,
    stdlib: &Path,
    base_python: &Path,
    scripts: &Path,
) -> io::Result<bool> {
    let stdlib_scripts = stdlib.join("venv").join("scripts").join("nt");
    let launcher = executable.launcher(gil_disabled, debug);
    for source in [
        stdlib_scripts.join(executable.source_exe(python_version, debug)),
        stdlib_scripts.join(launcher.as_ref()),
        base_python.with_file_name(launcher.as_ref()),
    ] {
        match fs_err::copy(source, scripts.join(executable.exe(python_version))) {
            Ok(_) => return Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::{WindowsExecutable, copy_launcher_windows_shim};
    use std::io;

    #[test]
    fn windows_debug_launchers() -> io::Result<()> {
        for (executable, source, destination, gil_disabled) in [
            (
                WindowsExecutable::Python,
                "python_d.exe",
                "python.exe",
                false,
            ),
            (
                WindowsExecutable::Pythonw,
                "pythonw_d.exe",
                "pythonw.exe",
                false,
            ),
            (
                WindowsExecutable::Python,
                "venvlauncher_d.exe",
                "python.exe",
                false,
            ),
            (
                WindowsExecutable::Pythonw,
                "venvwlauncher_d.exe",
                "pythonw.exe",
                false,
            ),
            (
                WindowsExecutable::Python,
                "venvlaunchert_d.exe",
                "python.exe",
                true,
            ),
            (
                WindowsExecutable::Pythonw,
                "venvwlaunchert_d.exe",
                "pythonw.exe",
                true,
            ),
            (
                WindowsExecutable::PythonMajorMinort,
                "venvlaunchert_d.exe",
                "python3.13t.exe",
                true,
            ),
            (
                WindowsExecutable::PythonwMajorMinort,
                "venvwlaunchert_d.exe",
                "pythonw3.13t.exe",
                true,
            ),
        ] {
            let temp = tempfile::tempdir()?;
            let stdlib = temp.path().join("Lib");
            let stdlib_scripts = stdlib.join("venv/scripts/nt");
            let scripts = temp.path().join("Scripts");
            fs_err::create_dir_all(&stdlib_scripts)?;
            fs_err::create_dir_all(&scripts)?;
            for name in [
                "python.exe",
                "pythonw.exe",
                "venvlauncher.exe",
                "venvwlauncher.exe",
                "venvlaunchert.exe",
                "venvwlaunchert.exe",
            ] {
                fs_err::write(stdlib_scripts.join(name), b"release")?;
            }
            let base_python = temp.path().join("python_d.exe");
            assert!(!copy_launcher_windows_shim(
                executable,
                (3, 13),
                gil_disabled,
                true,
                &stdlib,
                &base_python,
                &scripts
            )?);
            fs_err::write(stdlib_scripts.join(source), b"debug")?;
            assert!(copy_launcher_windows_shim(
                executable,
                (3, 13),
                gil_disabled,
                true,
                &stdlib,
                &base_python,
                &scripts
            )?);
            assert_eq!(fs_err::read(scripts.join(destination))?, b"debug");
        }
        Ok(())
    }
}
