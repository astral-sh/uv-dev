//! Parse and prepare run targets before environment synchronization and execution.

use std::borrow::Cow;
use std::ffi::OsString;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use futures::StreamExt;
use tokio::process::Command;
use url::Url;

use uv_client::BaseClientBuilder;
use uv_python_interpreter::Interpreter;
use uv_redacted::DisplaySafeUrl;
use uv_scripts::{Pep723Error, Pep723Item, Pep723Metadata, Pep723Script};
use uv_shell::WindowsRunnable;
use uv_static::EnvVars;

/// GitHub Gist API response structure
#[derive(serde::Deserialize)]
struct GistResponse {
    files: std::collections::HashMap<String, GistFile>,
}

#[derive(serde::Deserialize)]
struct GistFile {
    raw_url: String,
}

#[derive(Debug)]
pub enum RunCommand {
    /// Execute `python`.
    Python(Vec<OsString>),
    /// Execute a `python` script.
    PythonScript(PathBuf, Vec<OsString>),
    /// Search `sys.path` for the named module and execute its contents as the `__main__` module.
    /// Equivalent to `python -m module`.
    PythonModule(OsString, Vec<OsString>),
    /// Execute a `pythonw` GUI script.
    PythonGuiScript(PathBuf, Vec<OsString>),
    /// Execute a Python package containing a `__main__.py` file.
    /// If an entrypoint with the target name is installed in the environment, it is preferred.
    PythonPackage(OsString, PathBuf, Vec<OsString>),
    /// Execute a Python [zipapp](https://docs.python.org/3/library/zipapp.html).
    PythonZipapp(PathBuf, Vec<OsString>),
    /// Execute a `python` script provided via `stdin`.
    PythonStdin(Vec<u8>, Vec<OsString>),
    /// Execute a `pythonw` script provided via `stdin`.
    PythonGuiStdin(Vec<u8>, Vec<OsString>),
    /// Execute a Python script downloaded from a remote URL.
    PythonRemote(tempfile::NamedTempFile, Vec<OsString>),
    /// Execute an external command.
    External(OsString, Vec<OsString>),
    /// Execute an empty command (in practice, `python` with no arguments).
    Empty,
}

/// A parsed `uv run` target before any remote script has been downloaded.
#[derive(Debug)]
pub enum ParsedRunCommand {
    /// A target that is already fully resolved and ready to execute.
    Ready(RunCommand),
    /// A remote target that must be downloaded before it can be inspected or executed.
    PendingRemote(PendingRemoteRunCommand),
}

/// The information needed to fetch and execute a remote `uv run` target.
#[derive(Debug)]
pub struct PendingRemoteRunCommand {
    /// The remote URL to download.
    url: DisplaySafeUrl,
    /// The arguments to forward after the downloaded script path.
    args: Vec<OsString>,
}

impl PendingRemoteRunCommand {
    /// Download the remote script and return the URL, temporary file, and forwarded arguments.
    async fn download(
        self,
        client_builder: &BaseClientBuilder<'_>,
    ) -> anyhow::Result<(DisplaySafeUrl, tempfile::NamedTempFile, Vec<OsString>)> {
        let url = self.url.clone();
        let downloaded_script =
            ParsedRunCommand::download_remote_script(&self.url, client_builder).await?;
        Ok((url, downloaded_script, self.args))
    }
}

impl ParsedRunCommand {
    /// Return the local script directory used for target workspace discovery, if any.
    pub fn script_dir(&self) -> Option<&Path> {
        match self {
            Self::Ready(run_command) => run_command.script_dir(),
            Self::PendingRemote(..) => None,
        }
    }

    /// Resolve the parsed target into a [`RunCommand`] and any associated PEP 723 metadata.
    ///
    /// The client factory is called only for remote scripts, so local target discovery does not
    /// resolve global or network settings before reading the target's configuration.
    pub async fn resolve(
        self,
        client_builder: &(dyn Fn() -> anyhow::Result<BaseClientBuilder<'static>> + Sync),
    ) -> anyhow::Result<(Option<Pep723Item>, RunCommand)> {
        match self {
            Self::Ready(run_command) => {
                let script = run_command.read_pep723_item().await?;
                Ok((script, run_command))
            }
            Self::PendingRemote(remote_command) => {
                let client_builder = client_builder()?;

                let (url, downloaded_script, args) =
                    remote_command.download(&client_builder).await?;
                let script = match Pep723Metadata::read(&downloaded_script).await {
                    Ok(Some(metadata)) => Some(Pep723Item::Remote(metadata, url)),
                    Ok(None) => None,
                    Err(Pep723Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => None,
                    Err(err) => return Err(err.into()),
                };

                Ok((script, RunCommand::PythonRemote(downloaded_script, args)))
            }
        }
    }

    /// Determine the [`ParsedRunCommand`] for a given set of arguments.
    pub fn from_args(
        command: &[OsString],
        module: bool,
        script: bool,
        gui_script: bool,
    ) -> anyhow::Result<Self> {
        let Some((target, args)) = command.split_first() else {
            return Ok(Self::Ready(RunCommand::Empty));
        };

        if target.eq_ignore_ascii_case("-") {
            let mut buf = Vec::with_capacity(1024);
            std::io::stdin().read_to_end(&mut buf)?;

            return if module {
                Err(anyhow!("Cannot run a Python module from stdin"))
            } else if gui_script {
                Ok(Self::Ready(RunCommand::PythonGuiStdin(buf, args.to_vec())))
            } else {
                Ok(Self::Ready(RunCommand::PythonStdin(buf, args.to_vec())))
            };
        }

        let target_path = PathBuf::from(target);

        // Determine whether the user provided a remote script.
        if target_path.starts_with("http://") || target_path.starts_with("https://") {
            // Only continue if we are absolutely certain no local file exists.
            //
            // We don't do this check on Windows since the file path would
            // be invalid anyway, and thus couldn't refer to a local file.
            if !cfg!(unix) || matches!(target_path.try_exists(), Ok(false)) {
                let url = DisplaySafeUrl::parse(&target.to_string_lossy())?;
                return Ok(Self::PendingRemote(PendingRemoteRunCommand {
                    url,
                    args: args.to_vec(),
                }));
            }
        }

        if module {
            return Ok(Self::Ready(RunCommand::PythonModule(
                target.clone(),
                args.to_vec(),
            )));
        } else if gui_script {
            return Ok(Self::Ready(RunCommand::PythonGuiScript(
                target.clone().into(),
                args.to_vec(),
            )));
        } else if script {
            return Ok(Self::Ready(RunCommand::PythonScript(
                target.clone().into(),
                args.to_vec(),
            )));
        }

        let metadata = target_path.metadata();
        let is_file = metadata.as_ref().is_ok_and(std::fs::Metadata::is_file);
        let is_dir = metadata.as_ref().is_ok_and(std::fs::Metadata::is_dir);

        if target.eq_ignore_ascii_case("python") {
            Ok(Self::Ready(RunCommand::Python(args.to_vec())))
        } else if target_path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("py") || ext.eq_ignore_ascii_case("pyc"))
            && is_file
        {
            Ok(Self::Ready(RunCommand::PythonScript(
                target_path,
                args.to_vec(),
            )))
        } else if target_path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pyw"))
            && is_file
        {
            Ok(Self::Ready(RunCommand::PythonGuiScript(
                target_path,
                args.to_vec(),
            )))
        } else if is_dir && target_path.join("__main__.py").is_file() {
            Ok(Self::Ready(RunCommand::PythonPackage(
                target.clone(),
                target_path,
                args.to_vec(),
            )))
        } else if is_file && is_python_zipapp(&target_path) {
            Ok(Self::Ready(RunCommand::PythonZipapp(
                target_path,
                args.to_vec(),
            )))
        } else {
            Ok(Self::Ready(RunCommand::External(
                target.clone(),
                args.iter().map(std::clone::Clone::clone).collect(),
            )))
        }
    }

    /// Download a remote script target into a temporary file ready for execution.
    async fn download_remote_script(
        mut url: &DisplaySafeUrl,
        client_builder: &BaseClientBuilder<'_>,
    ) -> anyhow::Result<tempfile::NamedTempFile> {
        let client = client_builder.build()?;
        let mut response = client
            .for_host(url)
            .get(Url::from(url.clone()))
            .send()
            .await?;

        let gist_url;
        // If it's a Gist URL, use the GitHub API to get the raw URL.
        if response.url().host_str() == Some("gist.github.com") {
            gist_url =
                resolve_gist_url(DisplaySafeUrl::ref_cast(response.url()), client_builder).await?;
            url = &gist_url;

            response = client
                .for_host(url)
                .get(Url::from(url.clone()))
                .send()
                .await?;
        }

        let file_stem = url
            .path_segments()
            .and_then(Iterator::last)
            .and_then(|segment| segment.strip_suffix(".py"))
            .unwrap_or("script");
        let file = tempfile::Builder::new()
            .prefix(file_stem)
            .suffix(".py")
            .tempfile()?;

        // Stream the response to the file.
        let mut writer = file.as_file();
        let mut reader = response.bytes_stream();
        while let Some(chunk) = reader.next().await {
            writer.write_all(&chunk?)?;
        }

        Ok(file)
    }
}

impl RunCommand {
    /// Read any inline PEP 723 metadata associated with this command target.
    async fn read_pep723_item(&self) -> Result<Option<Pep723Item>, Pep723Error> {
        match self {
            Self::PythonScript(script, _) | Self::PythonGuiScript(script, _) => {
                match Pep723Script::read(script).await {
                    Ok(Some(script)) => Ok(Some(Pep723Item::Script(script))),
                    Ok(None) => Ok(None),
                    Err(Pep723Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {
                        Ok(None)
                    }
                    Err(err) => Err(err),
                }
            }
            Self::PythonStdin(contents, _) | Self::PythonGuiStdin(contents, _) => {
                Pep723Metadata::parse(contents).map(|metadata| metadata.map(Pep723Item::Stdin))
            }
            Self::Python(_)
            | Self::PythonPackage(..)
            | Self::PythonZipapp(..)
            | Self::PythonModule(..)
            | Self::PythonRemote(..)
            | Self::External(..)
            | Self::Empty => Ok(None),
        }
    }

    /// Return the name of the target executable, for display purposes.
    pub(super) fn display_executable(&self) -> Cow<'_, str> {
        match self {
            Self::Python(_)
            | Self::PythonScript(..)
            | Self::PythonZipapp(..)
            | Self::PythonRemote(..)
            | Self::Empty => Cow::Borrowed("python"),
            // N.B. We can't know if we'll invoke `<target>` or `python <target>` without checking
            // the available scripts in the interpreter — we could improve this message
            Self::PythonPackage(target, ..) => target.to_string_lossy(),
            Self::PythonModule(..) => Cow::Borrowed("python -m"),
            Self::PythonGuiScript(..) => {
                if cfg!(windows) {
                    Cow::Borrowed("pythonw")
                } else {
                    Cow::Borrowed("python")
                }
            }
            Self::PythonStdin(..) => Cow::Borrowed("python -c"),
            Self::PythonGuiStdin(..) => {
                if cfg!(windows) {
                    Cow::Borrowed("pythonw -c")
                } else {
                    Cow::Borrowed("python -c")
                }
            }
            Self::External(executable, _) => executable.to_string_lossy(),
        }
    }

    /// Convert a [`RunCommand`] into a [`Command`].
    pub(super) fn as_command(&self, interpreter: &Interpreter) -> Command {
        match self {
            Self::Python(args) => {
                let mut process = Command::new(interpreter.sys_executable());
                process.args(args);
                process
            }
            Self::PythonPackage(target, path, args) => {
                let name = PathBuf::from(target).with_extension(std::env::consts::EXE_EXTENSION);
                let entrypoint = interpreter.scripts().join(name);

                // If the target is an installed, executable script — prefer that
                if uv_fs::which::is_executable(&entrypoint) {
                    let mut process = Command::new(entrypoint);
                    process.args(args);
                    process
                // Otherwise, invoke `python <module>`
                } else {
                    let mut process = Command::new(interpreter.sys_executable());
                    process.arg(path);
                    process.args(args);
                    process
                }
            }
            Self::PythonScript(target, args) | Self::PythonZipapp(target, args) => {
                let mut process = Command::new(interpreter.sys_executable());
                process.arg(target);
                process.args(args);
                process
            }
            Self::PythonRemote(downloaded_script, args) => {
                let mut process = Command::new(interpreter.sys_executable());
                process.arg(downloaded_script.path());
                process.args(args);
                process
            }
            Self::PythonModule(module, args) => {
                let mut process = Command::new(interpreter.sys_executable());
                process.arg("-m");
                process.arg(module);
                process.args(args);
                process
            }
            Self::PythonGuiScript(target, args) => {
                let python_executable = interpreter.sys_executable();

                // Use `pythonw.exe` if it exists, otherwise fall back to `python.exe`.
                // See `install-wheel-rs::get_script_executable`.gd
                let pythonw_executable = python_executable
                    .file_name()
                    .map(|name| {
                        let new_name = name.to_string_lossy().replace("python", "pythonw");
                        python_executable.with_file_name(new_name)
                    })
                    .filter(|path| path.is_file())
                    .unwrap_or_else(|| python_executable.to_path_buf());

                let mut process = Command::new(&pythonw_executable);
                process.arg(target);
                process.args(args);
                process
            }
            Self::PythonStdin(script, args) => {
                let mut process = Command::new(interpreter.sys_executable());
                process.arg("-c");

                cfg_select! {
                    unix => {
                        process.arg(OsString::from_vec(script.clone()));
                    },
                    _ => {
                        let script =
                            String::from_utf8(script.clone()).expect("script is valid UTF-8");
                        process.arg(script);
                    },
                }
                process.args(args);

                process
            }
            Self::PythonGuiStdin(script, args) => {
                let python_executable = interpreter.sys_executable();

                // Use `pythonw.exe` if it exists, otherwise fall back to `python.exe`.
                // See `install-wheel-rs::get_script_executable`.gd
                let pythonw_executable = python_executable
                    .file_name()
                    .map(|name| {
                        let new_name = name.to_string_lossy().replace("python", "pythonw");
                        python_executable.with_file_name(new_name)
                    })
                    .filter(|path| path.is_file())
                    .unwrap_or_else(|| python_executable.to_path_buf());

                let mut process = Command::new(&pythonw_executable);
                process.arg("-c");

                cfg_select! {
                    unix => {
                        process.arg(OsString::from_vec(script.clone()));
                    },
                    _ => {
                        let script =
                            String::from_utf8(script.clone()).expect("script is valid UTF-8");
                        process.arg(script);
                    },
                }
                process.args(args);

                process
            }
            Self::External(executable, args) => {
                let mut process = if cfg!(windows) {
                    WindowsRunnable::from_script_path(interpreter.scripts(), executable).into()
                } else {
                    Command::new(executable)
                };
                process.args(args);
                process
            }
            Self::Empty => Command::new(interpreter.sys_executable()),
        }
    }

    /// Return the directory containing the script, if any.
    fn script_dir(&self) -> Option<&Path> {
        let parent = match self {
            Self::PythonScript(target, _)
            | Self::PythonGuiScript(target, _)
            | Self::PythonZipapp(target, _) => target.parent(),
            Self::PythonPackage(_, path, _) => path.parent(),
            Self::Python(_)
            | Self::PythonModule(..)
            | Self::PythonStdin(..)
            | Self::PythonGuiStdin(..)
            | Self::PythonRemote(..)
            | Self::External(..)
            | Self::Empty => None,
        };
        // The parent is `Some("")` for bare filenames.
        parent.filter(|parent| !parent.as_os_str().is_empty())
    }
}

impl std::fmt::Display for RunCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Python(args) => {
                write!(f, "python")?;
                for arg in args {
                    write!(f, " {}", arg.to_string_lossy())?;
                }
                Ok(())
            }
            Self::PythonPackage(target, _path, args) => {
                write!(f, "{}", target.to_string_lossy())?;
                for arg in args {
                    write!(f, " {}", arg.to_string_lossy())?;
                }
                Ok(())
            }
            Self::PythonScript(target, args) | Self::PythonZipapp(target, args) => {
                write!(f, "python {}", target.display())?;
                for arg in args {
                    write!(f, " {}", arg.to_string_lossy())?;
                }
                Ok(())
            }
            Self::PythonModule(module, args) => {
                write!(f, "python -m")?;
                write!(f, " {}", module.to_string_lossy())?;
                for arg in args {
                    write!(f, " {}", arg.to_string_lossy())?;
                }
                Ok(())
            }
            Self::PythonGuiScript(target, args) => {
                write!(f, "pythonw {}", target.display())?;
                for arg in args {
                    write!(f, " {}", arg.to_string_lossy())?;
                }
                Ok(())
            }
            Self::PythonStdin(..) | Self::PythonRemote(..) => {
                write!(f, "python -c")?;
                Ok(())
            }
            Self::PythonGuiStdin(..) => {
                write!(f, "pythonw -c")?;
                Ok(())
            }
            Self::External(executable, args) => {
                write!(f, "{}", executable.to_string_lossy())?;
                for arg in args {
                    write!(f, " {}", arg.to_string_lossy())?;
                }
                Ok(())
            }
            Self::Empty => {
                write!(f, "python")?;
                Ok(())
            }
        }
    }
}

/// Resolve a GitHub Gist URL to its raw file URL using the GitHub API.
async fn resolve_gist_url(
    url: &DisplaySafeUrl,
    client_builder: &BaseClientBuilder<'_>,
) -> anyhow::Result<DisplaySafeUrl> {
    // Extract the Gist ID from the URL.
    let gist_id = url
        .path_segments()
        .and_then(|mut segments| segments.nth(1))
        .ok_or_else(|| anyhow!("Invalid Gist URL format"))?;

    // Build the API URL.
    let api_url = format!("https://api.github.com/gists/{gist_id}");

    let client = client_builder.build()?;

    // Build the request with appropriate headers.
    let api_url_parsed = DisplaySafeUrl::parse(&api_url)?;
    let mut request = client
        .for_host(&api_url_parsed)
        .get(Url::from(api_url_parsed));
    request = request.header("Accept", "application/vnd.github.v3+json");

    // Add GitHub token, if available.
    if let Ok(token) = std::env::var(EnvVars::UV_GITHUB_TOKEN) {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    // Make the API request.
    let response = request.send().await?;
    response.error_for_status_ref()?;

    // Parse the response
    let gist_data: GistResponse = response.json().await?;

    // Get the raw URL of the first `.py` file (or just the first file).
    let raw_url = gist_data
        .files
        .iter()
        .filter(|(name, _)| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
        })
        .map(|(_, file)| &file.raw_url)
        .next()
        // If no `.py` file is found, use the first file.
        .or_else(|| gist_data.files.values().next().map(|file| &file.raw_url))
        .ok_or_else(|| anyhow!("No files found in the Gist"))?;

    let url = DisplaySafeUrl::parse(raw_url)?;

    Ok(url)
}

/// Returns `true` if the target is a ZIP archive containing a `__main__.py` file.
fn is_python_zipapp(target: &Path) -> bool {
    if let Ok(file) = fs_err::File::open(target) {
        let reader = std::io::BufReader::new(file);
        return futures::executor::block_on(async {
            let archive = async_zip::base::read::seek::ZipFileReader::new(
                futures::io::AllowStdIo::new(reader),
            )
            .await
            .ok()?;
            archive
                .file()
                .entries()
                .iter()
                .find(|entry| {
                    entry
                        .filename()
                        .as_str()
                        .is_ok_and(|name| name == "__main__.py")
                })
                .map(|entry| entry.dir().is_ok_and(|is_dir| !is_dir))
        })
        .unwrap_or(false);
    }
    false
}
