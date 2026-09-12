use std::ffi::OsString;
use std::fmt::{self, Display, Formatter};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Result, anyhow};
use clap::builder::styling::{AnsiColor, Effects, Style};
use clap::builder::{PossibleValue, Styles, TypedValueParser, ValueParserFactory};
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};
use clap::{ValueEnum, ValueHint};

use uv_audit::VulnerabilityServiceFormat;
use uv_auth::Service;
use uv_cache::CacheArgs;
use uv_configuration::{
    ExportFormat, IndexStrategy, KeyringProviderType, PackageNameSpecifier, PipCompileFormat,
    ProjectBuildBackend, TargetTriple, TrustedHost, TrustedPublishing, VersionControlSystem,
};
use uv_distribution_types::{
    ConfigSettingEntry, ConfigSettingPackageEntry, Index, IndexName, IndexSourceError, IndexUrl,
    Origin, PipExtraIndex, PipFindLinks, PipIndex,
};
use uv_normalize::{ExtraName, GroupName, PackageName, PipGroupName};
use uv_pep508::{MarkerTree, Requirement, VerbatimUrl};
use uv_preview::{MaybePreviewFeature, PreviewFeature};
use uv_pypi_types::VerbatimParsedUrl;
use uv_python::{PythonDownloads, PythonPreference, PythonVersion};
use uv_redacted::DisplaySafeUrl;
use uv_resolver::{
    AnnotationStyle, ExcludeNewerOverride, ExcludeNewerPackageEntry, ForkStrategy, PrereleaseMode,
    PrereleasePackageEntry, ResolutionMode,
};
use uv_settings::PythonInstallMirrors;
use uv_static::EnvVars;
use uv_torch::TorchMode;
use uv_warnings::warn_user_once;
use uv_workspace::pyproject_mut::AddBoundsKind;

pub mod comma;
pub mod compat;
pub mod options;
pub mod version;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum VersionFormat {
    /// Display the version as plain text.
    Text,
    /// Display the version as JSON.
    Json,
}

#[derive(Debug, Default, Clone, Copy, clap::ValueEnum)]
pub enum PythonListFormat {
    /// Plain text (for humans).
    #[default]
    Text,
    /// JSON (for computers).
    Json,
}

#[derive(Debug, Default, Clone, Copy, clap::ValueEnum)]
pub enum SyncFormat {
    /// Display the result in a human-readable format.
    #[default]
    Text,
    /// Display the result in JSON format.
    Json,
}

#[derive(Debug, Default, Clone, Copy, clap::ValueEnum)]
pub enum AuditOutputFormat {
    /// Display the result in a human-readable format.
    #[default]
    Text,
    /// Display the result in JSON format.
    Json,
    /// Display the result in SARIF format.
    Sarif,
}

#[derive(Debug, Default, Clone, Copy, clap::ValueEnum)]
pub enum CacheSizeOutputFormat {
    /// Display a human-readable size in terminals and raw bytes otherwise.
    #[default]
    Auto,
    /// Display the cache size in a human-readable format.
    Human,
    /// Display the cache size in raw bytes.
    Machine,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TreeFormat {
    /// Display the dependency graph as a human-readable tree.
    #[default]
    Text,
    /// Display the dependency graph as JSON.
    Json,
}

#[derive(Debug, Default, Clone, clap::ValueEnum)]
pub enum ListFormat {
    /// Display the list of packages in a human-readable table.
    #[default]
    Columns,
    /// Display the list of packages in a `pip freeze`-like format, with one package per line
    /// alongside its version.
    Freeze,
    /// Display the list of packages in a machine-readable JSON format.
    Json,
}

fn extra_name_with_clap_error(arg: &str) -> Result<ExtraName> {
    ExtraName::from_str(arg).map_err(|_err| {
        anyhow!(
            "Extra names must start and end with a letter or digit and may only \
            contain -, _, ., and alphanumeric characters"
        )
    })
}

// Configures Clap v3-style help menu colors
const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Cyan.on_default());

#[derive(Parser)]
#[command(name = "uv", author, long_version = crate::version::uv_self_version())]
#[command(about = "An extremely fast Python package manager.")]
#[command(
    after_help = "Use `uv help` for more details.",
    after_long_help = "",
    disable_help_flag = true,
    disable_help_subcommand = true,
    disable_version_flag = true
)]
#[command(styles=STYLES)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Box<Commands>,

    #[command(flatten)]
    pub top_level: TopLevelArgs,
}

#[derive(Parser)]
#[command(disable_help_flag = true, disable_version_flag = true)]
pub struct TopLevelArgs {
    #[command(flatten)]
    pub cache_args: Box<CacheArgs>,

    #[command(flatten)]
    pub global_args: Box<GlobalArgs>,

    /// The path to a `uv.toml` file to use for configuration.
    ///
    /// A `pyproject.toml` file can contain uv configuration, but this option does not accept one.
    #[arg(
        global = true,
        long,
        env = EnvVars::UV_CONFIG_FILE,
        help_heading = "Global options",
        value_hint = ValueHint::FilePath,
    )]
    pub config_file: Option<PathBuf>,

    /// Avoid discovering configuration files (`pyproject.toml`, `uv.toml`).
    ///
    /// By default, uv searches the current directory, parent directories, and user configuration
    /// directories for configuration files.
    #[arg(global = true, long, env = EnvVars::UV_NO_CONFIG, value_parser = clap::builder::BoolishValueParser::new(), help_heading = "Global options")]
    pub no_config: bool,

    /// Display the concise help for this command.
    #[arg(global = true, short, long, action = clap::ArgAction::HelpShort, help_heading = "Global options")]
    help: Option<bool>,

    /// Display the uv version.
    #[arg(short = 'V', long, action = clap::ArgAction::Version)]
    version: Option<bool>,
}

#[derive(Parser, Debug, Clone)]
#[command(next_help_heading = "Global options", next_display_order = 1000)]
pub struct GlobalArgs {
    #[arg(
        global = true,
        long,
        help_heading = "Python options",
        display_order = 700,
        env = EnvVars::UV_PYTHON_PREFERENCE,
        hide = true
    )]
    pub python_preference: Option<PythonPreference>,

    /// Require use of uv-managed Python versions [env: UV_MANAGED_PYTHON=]
    ///
    /// By default, uv prefers Python versions that it manages. If no managed version is installed,
    /// uv uses a system Python version. This option prevents uv from using system Python versions.
    #[arg(
        global = true,
        long,
        help_heading = "Python options",
        overrides_with = "no_managed_python"
    )]
    pub managed_python: bool,

    /// Disable use of uv-managed Python versions [env: UV_NO_MANAGED_PYTHON=]
    ///
    /// Instead, uv searches the system for a suitable Python version.
    #[arg(
        global = true,
        long,
        help_heading = "Python options",
        overrides_with = "managed_python"
    )]
    pub no_managed_python: bool,

    #[expect(clippy::doc_markdown)]
    /// Allow automatically downloading Python when required. [env: "UV_PYTHON_DOWNLOADS=auto"]
    #[arg(global = true, long, help_heading = "Python options", hide = true)]
    pub allow_python_downloads: bool,

    #[expect(clippy::doc_markdown)]
    /// Disable automatic downloads of Python. [env: "UV_PYTHON_DOWNLOADS=never"]
    #[arg(global = true, long, help_heading = "Python options")]
    pub no_python_downloads: bool,

    /// Deprecated version of [`Self::python_downloads`].
    #[arg(global = true, long, hide = true)]
    pub python_fetch: Option<PythonDownloads>,

    /// Use quiet output.
    ///
    /// Repeat this option, such as `-qq`, to prevent uv from writing output to stdout.
    #[arg(global = true, action = clap::ArgAction::Count, long, short, conflicts_with = "verbose")]
    pub quiet: u8,

    /// Use verbose output.
    ///
    /// Use the `RUST_LOG` environment variable to configure detailed logging.
    /// (<https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives>)
    #[arg(global = true, action = clap::ArgAction::Count, long, short, conflicts_with = "quiet")]
    pub verbose: u8,

    /// Disable colors.
    ///
    /// This option exists for compatibility with `pip`. Use `--color` instead.
    #[arg(global = true, long, hide = true, conflicts_with = "color")]
    pub no_color: bool,

    /// Control the use of color in output.
    ///
    /// By default, uv detects whether the terminal supports color.
    #[arg(
        global = true,
        long,
        value_enum,
        conflicts_with = "no_color",
        value_name = "COLOR_CHOICE"
    )]
    pub color: Option<ColorChoice>,

    /// (Deprecated: use `--system-certs` instead.) Whether to load TLS certificates from the
    /// platform's native certificate store [env: UV_NATIVE_TLS=]
    ///
    /// By default, uv uses bundled Mozilla root certificates. When enabled, this flag loads
    /// certificates from the platform's native certificate store instead.
    ///
    /// This is equivalent to `--system-certs`.
    #[arg(global = true, long, value_parser = clap::builder::BoolishValueParser::new(), overrides_with_all = ["no_native_tls", "system_certs", "no_system_certs"], hide = true)]
    pub native_tls: bool,

    #[arg(global = true, long, overrides_with_all = ["native_tls", "system_certs", "no_system_certs"], hide = true)]
    pub no_native_tls: bool,

    /// Whether to load TLS certificates from the platform's native certificate store [env: UV_SYSTEM_CERTS=]
    ///
    /// By default, uv uses bundled Mozilla root certificates. This improves portability and
    /// performance, especially on macOS.
    ///
    /// Use the platform's native certificate store if you need a certificate that is in the system
    /// store. For example, a corporate proxy may require a corporate trust root.
    #[arg(global = true, long, value_parser = clap::builder::BoolishValueParser::new(), overrides_with_all = ["no_system_certs", "native_tls", "no_native_tls"])]
    pub system_certs: bool,

    #[arg(global = true, long, overrides_with_all = ["system_certs", "native_tls", "no_native_tls"], hide = true)]
    pub no_system_certs: bool,

    /// Disable network access [env: UV_OFFLINE=]
    ///
    /// When network access is disabled, uv uses only cached data and local files.
    #[arg(global = true, long, overrides_with("no_offline"))]
    pub offline: bool,

    #[arg(global = true, long, overrides_with("offline"), hide = true)]
    pub no_offline: bool,

    /// Allow insecure connections to a host.
    ///
    /// Use this option multiple times to add multiple hosts.
    ///
    /// Accepts a hostname, such as `localhost`; a host-port pair, such as `localhost:8080`; or a
    /// URL, such as `https://localhost`.
    ///
    /// WARNING: uv does not verify these hosts against the system's certificate store. This option
    /// bypasses SSL verification and can expose you to MITM attacks. Use
    /// `--allow-insecure-host` only on a secure network with verified sources.
    #[arg(
        global = true,
        long,
        alias = "trusted-host",
        env = EnvVars::UV_INSECURE_HOST,
        value_delimiter = ' ',
        value_parser = parse_insecure_host,
        value_hint = ValueHint::Url,
    )]
    pub allow_insecure_host: Option<Vec<Maybe<TrustedHost>>>,

    /// Whether to enable all experimental preview features [env: UV_PREVIEW=]
    ///
    /// Preview features may change without warning.
    #[arg(global = true, long, hide = true, value_parser = clap::builder::BoolishValueParser::new(), overrides_with("no_preview"))]
    pub preview: bool,

    #[arg(global = true, long, overrides_with("preview"), hide = true)]
    pub no_preview: bool,

    /// Enable experimental preview features.
    ///
    /// Preview features may change without warning.
    ///
    /// Use comma-separated values or pass multiple times to enable multiple features.
    #[arg(
        global = true,
        long = "preview-features",
        env = EnvVars::UV_PREVIEW_FEATURES,
        value_delimiter = ',',
        hide = true,
        alias = "preview-feature",
    )]
    pub preview_features: Vec<MaybePreviewFeature>,

    /// Avoid discovering a `pyproject.toml` or `uv.toml` file [env: UV_ISOLATED=]
    ///
    /// By default, uv searches the current directory, parent directories, and user configuration
    /// directories for configuration files.
    ///
    /// This option is deprecated in favor of `--no-config`.
    #[arg(global = true, long, hide = true, value_parser = clap::builder::BoolishValueParser::new())]
    pub isolated: bool,

    /// Show the resolved settings for the current command.
    ///
    /// Use this option for debugging and development.
    #[arg(global = true, long, hide = true)]
    pub show_settings: bool,

    /// Hide all progress outputs [env: UV_NO_PROGRESS=]
    ///
    /// For example, spinners or progress bars.
    #[arg(global = true, long, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_progress: bool,

    /// Skip writing `uv` installer metadata files (e.g., `INSTALLER`, `REQUESTED`, and
    /// `direct_url.json`) to site-packages `.dist-info` directories [env: UV_NO_INSTALLER_METADATA=]
    #[arg(global = true, long, hide = true, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_installer_metadata: bool,

    /// Change to the given directory prior to running the command.
    ///
    /// uv resolves relative paths from the specified directory.
    ///
    /// See `--project` to only change the project root directory.
    #[arg(global = true, long, env = EnvVars::UV_WORKING_DIR, value_hint = ValueHint::DirPath)]
    pub directory: Option<PathBuf>,

    /// Discover a project in the given directory.
    ///
    /// uv searches the project root and its parent directories for `pyproject.toml`, `uv.toml`, and
    /// `.python-version` files. It also searches for the project's virtual environment (`.venv`).
    ///
    /// uv resolves other command-line arguments, such as relative paths, from the current working
    /// directory.
    ///
    /// See `--directory` to change the working directory entirely.
    ///
    /// This setting has no effect when used in the `uv pip` interface.
    #[arg(global = true, long, env = EnvVars::UV_PROJECT, value_hint = ValueHint::DirPath)]
    pub project: Option<PathBuf>,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum ColorChoice {
    /// Enables colored output only when the output is going to a terminal or TTY with support.
    Auto,

    /// Enables colored output regardless of the detected environment.
    Always,

    /// Disables colored output.
    Never,
}

impl ColorChoice {
    /// Return the command-line representation of this color choice.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    /// Combine self (higher priority) with an [`anstream::ColorChoice`] (lower priority).
    ///
    /// Prefer the user's choice. If the user does not choose, use the inferred stream setting.
    #[must_use]
    pub fn and_colorchoice(self, next: anstream::ColorChoice) -> Self {
        match self {
            Self::Auto => match next {
                anstream::ColorChoice::Auto => Self::Auto,
                anstream::ColorChoice::Always | anstream::ColorChoice::AlwaysAnsi => Self::Always,
                anstream::ColorChoice::Never => Self::Never,
            },
            Self::Always | Self::Never => self,
        }
    }
}

impl From<ColorChoice> for anstream::ColorChoice {
    fn from(value: ColorChoice) -> Self {
        match value {
            ColorChoice::Auto => Self::Auto,
            ColorChoice::Always => Self::Always,
            ColorChoice::Never => Self::Never,
        }
    }
}

#[derive(Subcommand)]
pub enum Commands {
    /// Manage authentication.
    #[command(
        after_help = "Use `uv help auth` for more details.",
        after_long_help = ""
    )]
    Auth(AuthNamespace),

    /// Manage Python projects.
    #[command(flatten)]
    Project(Box<ProjectCommand>),

    /// Run and install commands provided by Python packages.
    #[command(
        after_help = "Use `uv help tool` for more details.",
        after_long_help = ""
    )]
    Tool(ToolNamespace),

    /// Manage Python versions and installations
    ///
    /// uv first searches for Python in an active virtual environment or a `.venv` directory. The
    /// `.venv` directory can be in the current working directory or a parent directory. If a virtual
    /// environment is not required, uv then searches `PATH` for a Python executable.
    ///
    /// On Windows, uv also searches the registry for Python executables.
    ///
    /// By default, uv downloads Python if it cannot find the requested version. Use
    /// `--no-python-downloads` or the `python-downloads` setting to disable downloads.
    ///
    /// Use `--python` to request a different interpreter.
    ///
    /// The following Python version request formats are supported:
    ///
    /// - `<version>` e.g. `3`, `3.12`, `3.12.3`
    /// - `<version-specifier>` e.g. `>=3.12,<3.13`
    /// - `<version><short-variant>` (e.g., `3.13t`, `3.12.0d`)
    /// - `<version>+<variant>` (e.g., `3.13+freethreaded`, `3.12.0+debug`)
    /// - `<implementation>` e.g. `cpython` or `cp`
    /// - `<implementation>@<version>` e.g. `cpython@3.12`
    /// - `<implementation><version>` e.g. `cpython3.12` or `cp312`
    /// - `<implementation><version-specifier>` e.g. `cpython>=3.12,<3.13`
    /// - `<implementation>-<version>-<os>-<arch>-<libc>` e.g. `cpython-3.12.3-macos-aarch64-none`
    ///
    /// You can also request a specific system Python interpreter with:
    ///
    /// - `<executable-path>` e.g. `/opt/homebrew/bin/python3`
    /// - `<executable-name>` e.g. `mypython3`
    /// - `<install-dir>` e.g. `/some/environment/`
    ///
    /// When you use `--python`, uv follows the normal discovery rules and checks each interpreter
    /// against the request. For example, if you request `pypy`, uv first checks the virtual
    /// environment for a PyPy interpreter. It then checks each executable in `PATH`.
    ///
    /// uv finds CPython, PyPy, and GraalPy interpreters and skips unsupported interpreters. If you
    /// request an unsupported interpreter implementation, uv exits with an error.
    #[clap(verbatim_doc_comment)]
    #[command(
        after_help = "Use `uv help python` for more details.",
        after_long_help = ""
    )]
    Python(PythonNamespace),
    /// Manage Python packages with a pip-compatible interface.
    #[command(
        after_help = "Use `uv help pip` for more details.",
        after_long_help = ""
    )]
    Pip(PipNamespace),
    /// Create a virtual environment.
    ///
    /// By default, uv creates a virtual environment named `.venv` in the working directory. You
    /// can specify a different path as a positional argument.
    ///
    /// In a project, use `UV_PROJECT_ENVIRONMENT` to change the default environment name. This
    /// setting applies only when you run the command from the project root directory.
    ///
    /// If a virtual environment exists at the target path, uv replaces it with a new, empty
    /// virtual environment.
    ///
    /// You do not need to activate the virtual environment. uv finds a `.venv` directory in the
    /// working directory or a parent directory.
    #[command(
        alias = "virtualenv",
        alias = "v",
        after_help = "Use `uv help venv` for more details.",
        after_long_help = ""
    )]
    Venv(VenvArgs),
    /// Build Python packages into source distributions and wheels.
    ///
    /// `uv build` accepts a path to a directory or source distribution. The default path is the
    /// current working directory.
    ///
    /// For a directory, `uv build` first builds a source distribution ("sdist"). It then builds a
    /// binary distribution ("wheel") from that source distribution.
    ///
    /// Use `uv build --sdist` to build only the source distribution. Use `uv build --wheel` to
    /// build only the binary distribution. Use `uv build --sdist --wheel` to build both
    /// distributions from source.
    ///
    /// For a source distribution, `uv build --wheel` builds a wheel from that distribution.
    #[command(
        after_help = "Use `uv help build` for more details.",
        after_long_help = ""
    )]
    Build(BuildArgs),
    /// Upload distributions to an index.
    Publish(PublishArgs),
    /// Inspect uv workspaces.
    #[command(
        after_help = "Use `uv help workspace` for more details.",
        after_long_help = ""
    )]
    Workspace(WorkspaceNamespace),
    /// The implementation of the build backend.
    ///
    /// These commands are not exposed directly. A PEP 517 build frontend calls Python shims, which
    /// then call uv with this method.
    #[command(hide = true)]
    BuildBackend {
        #[command(subcommand)]
        command: BuildBackendCommand,
    },
    /// Manage uv's cache.
    #[command(
        after_help = "Use `uv help cache` for more details.",
        after_long_help = ""
    )]
    Cache(CacheNamespace),
    /// Manage the uv executable.
    #[command(name = "self")]
    Self_(SelfNamespace),
    /// Clear the cache, removing all entries or those linked to specific packages.
    #[command(hide = true)]
    Clean(CleanArgs),
    /// Generate shell completion
    #[command(alias = "--generate-shell-completion", hide = true)]
    GenerateShellCompletion(GenerateShellCompletionArgs),
    /// Display documentation for a command.
    // Maintain these options with `after_help` so help for the help command omits global options.
    #[command(help_template = "\
{about-with-newline}
{usage-heading} {usage}{after-help}
",
        after_help = format!("\
{heading}Options:{heading:#}
  {option}--no-pager{option:#} Disable pager when printing help
",
            heading = Style::new().bold().underline(),
            option = Style::new().bold(),
        ),
    )]
    Help(HelpArgs),
}

#[derive(Args, Debug)]
pub struct HelpArgs {
    /// Disable pager when printing help
    #[arg(long)]
    pub no_pager: bool,

    #[arg(value_hint = ValueHint::Other)]
    pub command: Option<Vec<String>>,
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("operation"))]
pub struct VersionArgs {
    /// Set the project version to this value
    ///
    /// To update the project using semantic versioning components instead, use `--bump`.
    #[arg(group = "operation", value_hint = ValueHint::Other)]
    pub value: Option<String>,

    /// Update the project version using the given semantics
    ///
    /// This flag can be passed multiple times.
    #[arg(group = "operation", long, value_name = "BUMP[=VALUE]")]
    pub bump: Vec<VersionBumpSpec>,

    /// Don't write a new version to the `pyproject.toml`
    ///
    /// Instead, uv displays the version.
    #[arg(long)]
    pub dry_run: bool,

    /// Only show the version
    ///
    /// By default, uv will show the project name before the version.
    #[arg(long)]
    pub short: bool,

    /// The format of the output
    #[arg(long, value_enum, default_value = "text")]
    pub output_format: VersionFormat,

    /// Avoid syncing the virtual environment after re-locking the project [env: UV_NO_SYNC=]
    #[arg(long)]
    pub no_sync: bool,

    /// Prefer the active virtual environment over the project's virtual environment.
    ///
    /// If the project virtual environment is active or no virtual environment is active, this has
    /// no effect.
    #[arg(long, overrides_with = "no_active")]
    pub active: bool,

    /// Prefer project's virtual environment over an active environment.
    ///
    /// This is the default behavior.
    #[arg(long, overrides_with = "active", hide = true)]
    pub no_active: bool,

    /// Assert that the `uv.lock` will remain unchanged [env: UV_LOCKED=]
    ///
    /// Requires that the lockfile is up-to-date. If the lockfile is missing or needs to be updated,
    /// uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Update the version without re-locking the project [env: UV_FROZEN=]
    ///
    /// The project environment will not be synced.
    #[arg(long, conflicts_with_all = ["locked", "upgrade", "no_sources"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    #[command(flatten)]
    pub installer: ResolverInstallerArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Update the version of a specific package in the workspace.
    #[arg(long, conflicts_with = "isolated", value_hint = ValueHint::Other)]
    pub package: Option<PackageName>,

    /// The Python interpreter to use for resolving and syncing.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,
}

// Note that the ordering of the variants is significant, as when given a list of operations
// to perform, we sort them and apply them in order, so users don't have to think too hard about it.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum VersionBump {
    /// Increase the major version (e.g., 1.2.3 => 2.0.0)
    Major,
    /// Increase the minor version (e.g., 1.2.3 => 1.3.0)
    Minor,
    /// Increase the patch version (e.g., 1.2.3 => 1.2.4)
    Patch,
    /// Move from a pre-release to stable version (e.g., 1.2.3b4.post5.dev6 => 1.2.3)
    ///
    /// Removes all pre-release components, but will not remove "local" components.
    Stable,
    /// Increase the alpha version (e.g., 1.2.3a4 => 1.2.3a5)
    ///
    /// To move from a stable to a pre-release version, combine this with a stable component, e.g.,
    /// for 1.2.3 => 2.0.0a1, you'd also include [`VersionBump::Major`].
    Alpha,
    /// Increase the beta version (e.g., 1.2.3b4 => 1.2.3b5)
    ///
    /// To move from a stable to a pre-release version, combine this with a stable component, e.g.,
    /// for 1.2.3 => 2.0.0b1, you'd also include [`VersionBump::Major`].
    Beta,
    /// Increase the rc version (e.g., 1.2.3rc4 => 1.2.3rc5)
    ///
    /// To move from a stable to a pre-release version, combine this with a stable component, e.g.,
    /// for 1.2.3 => 2.0.0rc1, you'd also include [`VersionBump::Major`].]
    Rc,
    /// Increase the post version (e.g., 1.2.3.post5 => 1.2.3.post6)
    Post,
    /// Increase the dev version (e.g., 1.2.3a4.dev6 => 1.2.3.dev7)
    Dev,
}

impl Display for VersionBump {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let string = match self {
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
            Self::Stable => "stable",
            Self::Alpha => "alpha",
            Self::Beta => "beta",
            Self::Rc => "rc",
            Self::Post => "post",
            Self::Dev => "dev",
        };
        string.fmt(f)
    }
}

impl FromStr for VersionBump {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "major" => Ok(Self::Major),
            "minor" => Ok(Self::Minor),
            "patch" => Ok(Self::Patch),
            "stable" => Ok(Self::Stable),
            "alpha" => Ok(Self::Alpha),
            "beta" => Ok(Self::Beta),
            "rc" => Ok(Self::Rc),
            "post" => Ok(Self::Post),
            "dev" => Ok(Self::Dev),
            _ => Err(format!("invalid bump component `{value}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VersionBumpSpec {
    pub bump: VersionBump,
    pub value: Option<u64>,
}

impl Display for VersionBumpSpec {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.value {
            Some(value) => write!(f, "{}={value}", self.bump),
            None => self.bump.fmt(f),
        }
    }
}

impl FromStr for VersionBumpSpec {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (name, value) = match input.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (input, None),
        };

        let bump = name.parse::<VersionBump>()?;

        if bump == VersionBump::Stable && value.is_some() {
            return Err("`--bump stable` does not accept a value".to_string());
        }

        let value = match value {
            Some("") => {
                return Err("`--bump` values cannot be empty".to_string());
            }
            Some(raw) => Some(
                raw.parse::<u64>()
                    .map_err(|_| format!("invalid numeric value `{raw}` for `--bump {name}`"))?,
            ),
            None => None,
        };

        Ok(Self { bump, value })
    }
}

impl ValueParserFactory for VersionBumpSpec {
    type Parser = VersionBumpSpecValueParser;

    fn value_parser() -> Self::Parser {
        VersionBumpSpecValueParser
    }
}

#[derive(Clone, Debug)]
pub struct VersionBumpSpecValueParser;

impl TypedValueParser for VersionBumpSpecValueParser {
    type Value = VersionBumpSpec;

    fn parse_ref(
        &self,
        command: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let raw = value.to_str().ok_or_else(|| {
            command.clone().error(
                ErrorKind::InvalidUtf8,
                "`--bump` values must be valid UTF-8",
            )
        })?;

        VersionBumpSpec::from_str(raw)
            .map_err(|message| command.clone().error(ErrorKind::InvalidValue, message))
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(
            VersionBump::value_variants()
                .iter()
                .filter_map(ValueEnum::to_possible_value),
        ))
    }
}

#[derive(Args)]
pub struct SelfNamespace {
    #[command(subcommand)]
    pub command: SelfCommand,
}

#[derive(Subcommand)]
pub enum SelfCommand {
    /// Update uv.
    Update(SelfUpdateArgs),
    /// Display uv's version
    Version {
        /// Only print the version
        #[arg(long)]
        short: bool,
        #[arg(long, value_enum, default_value = "text")]
        output_format: VersionFormat,
    },
}

#[derive(Args, Debug)]
pub struct SelfUpdateArgs {
    /// Update to the specified version. If not provided, uv will update to the latest version.
    #[arg(value_hint = ValueHint::Other)]
    pub target_version: Option<String>,

    /// A GitHub token for authentication.
    /// A token is not required but can be used to reduce the chance of encountering rate limits.
    #[arg(long, env = EnvVars::UV_GITHUB_TOKEN, value_hint = ValueHint::Other)]
    pub token: Option<String>,

    /// Run without performing the update.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct CacheNamespace {
    #[command(subcommand)]
    pub command: CacheCommand,
}

#[derive(Subcommand)]
pub enum CacheCommand {
    /// Clear the cache, removing all entries or those linked to specific packages.
    #[command(alias = "clear")]
    Clean(CleanArgs),
    /// Prune dangling cache entries and cached environments.
    Prune(PruneArgs),
    /// Show the cache directory.
    ///
    /// By default, the cache is stored in `$XDG_CACHE_HOME/uv` or `$HOME/.cache/uv` on Unix and
    /// `%LOCALAPPDATA%\uv\cache` on Windows.
    ///
    /// When `--no-cache` is used, the cache is stored in a temporary directory and discarded when
    /// the process exits.
    ///
    /// An alternative cache directory may be specified via the `cache-dir` setting, the
    /// `--cache-dir` option, or the `$UV_CACHE_DIR` environment variable.
    ///
    /// Note that it is important for performance for the cache directory to be located on the same
    /// file system as the Python environment uv is operating on.
    Dir,
    /// Show the cache size.
    ///
    /// Displays the total size of the cache directory. This includes all downloaded and built
    /// wheels, source distributions, and other cached data. By default, displays a human-readable
    /// size when the output is a terminal and raw bytes otherwise.
    Size(SizeArgs),
}

#[derive(Args, Debug)]
pub struct CleanArgs {
    /// The packages to remove from the cache.
    #[arg(value_hint = ValueHint::Other)]
    pub package: Vec<PackageName>,

    /// Force removal of the cache, ignoring in-use checks.
    ///
    /// By default, `uv cache clean` will block until no process is reading the cache. When
    /// `--force` is used, `uv cache clean` will proceed without taking a lock.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct PruneArgs {
    /// Optimize the cache for persistence in a continuous integration environment, like GitHub
    /// Actions.
    ///
    /// By default, uv caches both the wheels that it builds from source and the pre-built wheels
    /// that it downloads directly, to enable high-performance package installation. In some
    /// scenarios, though, persisting pre-built wheels may be undesirable. For example, in GitHub
    /// Actions, it's faster to omit pre-built wheels from the cache and instead have re-download
    /// them on each run. However, it typically _is_ faster to cache wheels that are built from
    /// source, since the wheel building process can be expensive, especially for extension
    /// modules.
    ///
    /// In `--ci` mode, uv will prune any pre-built wheels from the cache, but retain any wheels
    /// that were built from source.
    #[arg(long)]
    pub ci: bool,

    /// Force removal of the cache, ignoring in-use checks.
    ///
    /// By default, `uv cache prune` will block until no process is reading the cache. When
    /// `--force` is used, `uv cache prune` will proceed without taking a lock.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct SizeArgs {
    /// Select the output format.
    #[arg(long, value_enum, default_value_t = CacheSizeOutputFormat::default())]
    pub output_format: CacheSizeOutputFormat,

    /// Display the cache size in human-readable format (e.g., `1.2GiB` instead of raw bytes).
    #[arg(
        long = "human",
        short = 'H',
        alias = "human-readable",
        conflicts_with = "output_format"
    )]
    pub human: bool,
}

#[derive(Args)]
pub struct PipNamespace {
    #[command(subcommand)]
    pub command: PipCommand,

    /// Path to a PEM-encoded CA certificate bundle.
    ///
    /// This option overrides the default certificate source.
    #[arg(global = true, long, value_name = "FILE", value_hint = ValueHint::FilePath)]
    pub cert: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum PipCommand {
    /// Compile a `requirements.in` file to a `requirements.txt` or `pylock.toml` file.
    #[command(
        after_help = "Use `uv help pip compile` for more details.",
        after_long_help = ""
    )]
    Compile(PipCompileArgs),
    /// Sync an environment with a `requirements.txt` or `pylock.toml` file.
    ///
    /// When syncing an environment, uv removes packages that are not in the `requirements.txt` or
    /// `pylock.toml` file. To keep those packages, use `uv pip install` instead.
    ///
    /// The input file is expected to come from `pip compile` or `uv export` and include all
    /// transitive dependencies. uv does not install transitive dependencies that are absent from
    /// the file. Use `--strict` to warn about missing transitive dependencies.
    #[command(
        after_help = "Use `uv help pip sync` for more details.",
        after_long_help = ""
    )]
    Sync(Box<PipSyncArgs>),
    /// Install packages into an environment.
    #[command(
        after_help = "Use `uv help pip install` for more details.",
        after_long_help = ""
    )]
    Install(PipInstallArgs),
    /// Uninstall packages from an environment.
    #[command(
        after_help = "Use `uv help pip uninstall` for more details.",
        after_long_help = ""
    )]
    Uninstall(PipUninstallArgs),
    /// List, in requirements format, packages installed in an environment.
    #[command(
        after_help = "Use `uv help pip freeze` for more details.",
        after_long_help = ""
    )]
    Freeze(PipFreezeArgs),
    /// List, in tabular format, packages installed in an environment.
    #[command(
        after_help = "Use `uv help pip list` for more details.",
        after_long_help = "",
        alias = "ls"
    )]
    List(PipListArgs),
    /// Show information about one or more installed packages.
    #[command(
        after_help = "Use `uv help pip show` for more details.",
        after_long_help = ""
    )]
    Show(PipShowArgs),
    /// Display the dependency tree for an environment.
    #[command(
        after_help = "Use `uv help pip tree` for more details.",
        after_long_help = ""
    )]
    Tree(PipTreeArgs),
    /// Verify installed packages have compatible dependencies.
    #[command(
        after_help = "Use `uv help pip check` for more details.",
        after_long_help = ""
    )]
    Check(PipCheckArgs),
    /// Display debug information (unsupported)
    #[command(hide = true)]
    Debug(PipDebugArgs),
}

#[derive(Subcommand)]
pub enum ProjectCommand {
    /// Run a command or script.
    ///
    /// Run the command in a Python environment.
    ///
    /// uv runs a `.py` file or HTTP(S) URL as a Python script. For example, `uv run file.py` is
    /// equivalent to `uv run python file.py`. For a URL, uv temporarily downloads the script
    /// before it runs. If the script contains inline dependency metadata, uv installs it into an
    /// isolated, temporary environment. Use `-` to read a Python script from stdin.
    ///
    /// In a project, uv creates and updates the project environment before it runs the command.
    ///
    /// Outside a project, uv uses a virtual environment in the current directory or a parent
    /// directory, if one exists. Otherwise, it uses the discovered interpreter's environment.
    ///
    /// When running a script, uv searches for a project or workspace from the script's directory.
    /// Otherwise, it searches from the current working directory.
    ///
    /// Arguments after the command or script are passed to that command or script, not to uv.
    /// Specify uv options before the command, as in `uv run --verbose foo`. Use `--` to separate uv
    /// options from the command, as in `uv run --python 3.12 -- python`.
    #[command(
        after_help = "Use `uv help run` for more details.",
        after_long_help = ""
    )]
    Run(RunArgs),
    /// Create a new project.
    ///
    /// Follows the `pyproject.toml` specification.
    ///
    /// If a `pyproject.toml` already exists at the target, uv exits with an error.
    ///
    /// If a parent directory of the target contains a `pyproject.toml`, uv adds the project to the
    /// parent workspace.
    ///
    /// uv creates some project state only when needed. For example, it creates the virtual
    /// environment (`.venv`) and lockfile (`uv.lock`) during the first sync.
    Init(InitArgs),
    /// Add dependencies to the project.
    ///
    /// uv adds dependencies to the project's `pyproject.toml` file.
    ///
    /// If a dependency already exists, uv updates its version specifier. If the new dependency has
    /// different markers, uv adds a separate entry instead.
    ///
    /// uv updates the lockfile and project environment to include the new dependencies. Use
    /// `--frozen` to skip the lockfile update. Use `--no-sync` to skip the environment update.
    ///
    /// If uv cannot find a requested dependency, it exits with an error. With `--frozen`, uv adds
    /// dependencies exactly as specified. It does not check whether they exist or are compatible
    /// with the project.
    ///
    /// uv searches the current directory and parent directories for a project. If it cannot find a
    /// project, it exits with an error.
    #[command(
        after_help = "Use `uv help add` for more details.",
        after_long_help = ""
    )]
    Add(AddArgs),
    /// Remove dependencies from the project.
    ///
    /// uv removes dependencies from the project's `pyproject.toml` file.
    ///
    /// If a dependency has multiple entries with different markers, uv removes every entry.
    ///
    /// uv updates the lockfile and project environment to remove the dependencies. Use `--frozen`
    /// to skip the lockfile update. Use `--no-sync` to skip the environment update.
    ///
    /// If a requested dependency is not in the project, uv exits with an error.
    ///
    /// `uv remove` does not remove packages that you installed manually with `uv pip install`.
    ///
    /// uv searches the current directory and parent directories for a project. If it cannot find a
    /// project, it exits with an error.
    #[command(
        after_help = "Use `uv help remove` for more details.",
        after_long_help = ""
    )]
    Remove(RemoveArgs),
    /// Read or update the project's version.
    Version(VersionArgs),
    /// Update the project's environment.
    ///
    /// Syncing installs all project dependencies and updates them to match the lockfile.
    ///
    /// By default, uv performs an exact sync and removes packages that the project does not
    /// declare as dependencies. Use `--inexact` to keep those packages. uv still removes packages
    /// that conflict with a project dependency. With `--no-build-isolation`, uv keeps extra
    /// packages because they may be build dependencies.
    ///
    /// If the project virtual environment (`.venv`) does not exist, uv creates it.
    ///
    /// uv re-locks the project before syncing unless you use `--locked` or `--frozen`.
    ///
    /// uv searches the current directory and parent directories for a project. If it cannot find a
    /// project, it exits with an error.
    ///
    /// When installing from a lockfile, uv does not warn about yanked package versions.
    #[command(
        after_help = "Use `uv help sync` for more details.",
        after_long_help = ""
    )]
    Sync(SyncArgs),
    /// Update the project's lockfile.
    ///
    /// If the project lockfile (`uv.lock`) does not exist, uv creates it. If the lockfile exists,
    /// uv uses its contents as resolution preferences.
    ///
    /// If the project dependencies have not changed, locking has no effect unless you use
    /// `--upgrade`.
    #[command(
        after_help = "Use `uv help lock` for more details.",
        after_long_help = ""
    )]
    Lock(LockArgs),
    /// Upgrade a dependency in the project.
    #[command(hide = true)]
    Upgrade(UpgradeArgs),
    /// Export the project's lockfile to an alternate format.
    ///
    /// uv supports `requirements.txt`, `pylock.toml` (PEP 751), and CycloneDX v1.5 JSON output.
    ///
    /// uv re-locks the project before exporting unless you use `--locked` or `--frozen`.
    ///
    /// uv searches the current directory and parent directories for a project. If it cannot find a
    /// project, it exits with an error.
    ///
    /// In a workspace, uv exports the root by default. Use `--package` to select a specific member.
    #[command(
        after_help = "Use `uv help export` for more details.",
        after_long_help = ""
    )]
    Export(ExportArgs),
    /// Display the project's dependency tree.
    Tree(TreeArgs),
    /// Format Python code in the project.
    ///
    /// Format Python code with the Ruff formatter. By default, uv formats all Python files in the
    /// project. This command behaves like `ruff format` in the project root.
    ///
    /// Use `--check` to check formatting without changing files. Use `--diff` to show formatting
    /// changes.
    ///
    /// Pass additional arguments to Ruff after `--`.
    #[command(
        after_help = "Use `uv help format` for more details.",
        after_long_help = ""
    )]
    Format(FormatArgs),
    /// Run checks on the project.
    ///
    /// This command checks Python types with ty. By default, it checks all Python files in the
    /// project.
    ///
    /// To apply safe fixes to type-checking errors, use `--fix`.
    #[command(
        after_help = "Use `uv help check` for more details.",
        after_long_help = ""
    )]
    Check(CheckArgs),
    /// Audit the project's dependencies.
    ///
    /// Check dependencies for known vulnerabilities and adverse statuses, such as deprecation and
    /// quarantine.
    ///
    /// By default, uv audits all project extras and groups. Use `--no-extra`, `--no-group`, and
    /// related options to exclude extras or groups.
    #[command(
        after_help = "Use `uv help audit` for more details.",
        after_long_help = ""
    )]
    Audit(AuditArgs),
}

/// A re-implementation of `Option`, used to avoid Clap's automatic `Option` flattening in
/// [`parse_index_url`].
#[derive(Debug, Clone)]
pub enum Maybe<T> {
    Some(T),
    None,
}

impl<T> Maybe<T> {
    pub fn into_option(self) -> Option<T> {
        match self {
            Self::Some(value) => Some(value),
            Self::None => None,
        }
    }

    pub fn is_some(&self) -> bool {
        matches!(self, Self::Some(_))
    }
}

/// Parse an `--index-url` argument into an [`PipIndex`], mapping the empty string to `None`.
fn parse_index_url(input: &str) -> Result<Maybe<PipIndex>, String> {
    if input.is_empty() {
        Ok(Maybe::None)
    } else {
        IndexUrl::from_str(input)
            .map(Index::from_index_url)
            .map(|index| Index {
                origin: Some(Origin::Cli),
                ..index
            })
            .map(PipIndex::from)
            .map(Maybe::Some)
            .map_err(|err| err.to_string())
    }
}

/// Parse an `--extra-index-url` argument into an [`PipExtraIndex`], mapping the empty string to `None`.
fn parse_extra_index_url(input: &str) -> Result<Maybe<PipExtraIndex>, String> {
    if input.is_empty() {
        Ok(Maybe::None)
    } else {
        IndexUrl::from_str(input)
            .map(Index::from_extra_index_url)
            .map(|index| Index {
                origin: Some(Origin::Cli),
                ..index
            })
            .map(PipExtraIndex::from)
            .map(Maybe::Some)
            .map_err(|err| err.to_string())
    }
}

/// Parse a `--find-links` argument into an [`PipFindLinks`], mapping the empty string to `None`.
fn parse_find_links(input: &str) -> Result<Maybe<PipFindLinks>, String> {
    if input.is_empty() {
        Ok(Maybe::None)
    } else {
        IndexUrl::from_str(input)
            .map(Index::from_find_links)
            .map(|index| Index {
                origin: Some(Origin::Cli),
                ..index
            })
            .map(PipFindLinks::from)
            .map(Maybe::Some)
            .map_err(|err| err.to_string())
    }
}

/// An unresolved index passed by the user by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedIndex {
    name: IndexName,
    default: bool,
}

impl UnresolvedIndex {
    /// Resolve an index name against the effective filesystem configuration.
    fn resolve(self, indexes: &[Index], preview_enabled: bool) -> Result<Index> {
        let Self { name, default } = self;
        let path_exists = Path::new(name.as_ref()).exists();

        // Outside preview, an existing path retains its current interpretation.
        if preview_enabled || !path_exists {
            if let Some(index) = indexes
                .iter()
                .find(|index| index.name.as_ref() == Some(&name))
            {
                if !preview_enabled {
                    warn_user_once!(
                        "Referencing an index by name is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
                        PreviewFeature::IndexByName
                    );
                }

                let mut index = index.clone();
                // Keep relative paths anchored to their configuration file without marking them
                // as absolute when CLI settings are rebased or written back to a project.
                if let IndexUrl::Path(url) = index.url()
                    && !url.was_given_absolute()
                {
                    index.url = IndexUrl::from(VerbatimUrl::from_url(index.raw_url().clone()));
                }

                return Ok(Index {
                    default,
                    explicit: false,
                    origin: Some(Origin::Cli),
                    ..index
                });
            }

            if preview_enabled && !path_exists {
                return Err(anyhow!("Could not find an index named `{name}`"));
            }
        }

        Ok(Index {
            default,
            origin: Some(Origin::Cli),
            ..Index::from_str(name.as_ref())?
        })
    }
}

/// A potentially unresolved index.
#[expect(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexArg {
    /// A usable index with a URL.
    Resolved(Index),
    /// An unresolved index specification.
    Unresolved(UnresolvedIndex),
}

impl IndexArg {
    fn new(value: &str, default: bool) -> Result<Self, IndexSourceError> {
        if let Ok(name) = IndexName::from_str(value) {
            return Ok(Self::Unresolved(UnresolvedIndex { name, default }));
        }

        let index = Index::from_str(value)?;
        Ok(Self::Resolved(Index {
            default,
            origin: Some(Origin::Cli),
            ..index
        }))
    }

    /// Parse an index passed via `--index`.
    fn from_index(value: &str) -> Result<Self, IndexSourceError> {
        Self::new(value, false)
    }

    /// Parse an index passed via `--default-index`.
    fn from_default_index(value: &str) -> Result<Self, IndexSourceError> {
        Self::new(value, true)
    }

    /// Resolve the argument against indexes from the effective configuration.
    fn resolve(self, indexes: &[Index]) -> Result<Index> {
        let index = match self {
            Self::Resolved(index) => index,
            Self::Unresolved(index) => {
                index.resolve(indexes, uv_preview::is_enabled(PreviewFeature::IndexByName))?
            }
        };

        index.url().warn_on_disambiguated_relative_path();

        Ok(index)
    }
}

/// Parse an `--index` argument into a [`Vec<IndexArg>`], mapping the empty string to an empty Vec.
///
/// This function splits the input on all whitespace characters rather than a single delimiter,
/// which is necessary to parse environment variables like `PIP_EXTRA_INDEX_URL`.
/// The standard `clap::Args` `value_delimiter` only supports single-character delimiters.
fn parse_indices(input: &str) -> Result<Vec<Maybe<IndexArg>>, String> {
    if input.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut indices = Vec::new();
    for token in input.split_whitespace() {
        match IndexArg::from_index(token) {
            Ok(index) => indices.push(Maybe::Some(index)),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(indices)
}

/// Parse a `--default-index` argument into an [`IndexArg`], mapping the empty string to `None`.
fn parse_default_index(input: &str) -> Result<Maybe<IndexArg>, String> {
    if input.is_empty() {
        Ok(Maybe::None)
    } else {
        match IndexArg::from_default_index(input) {
            Ok(index) => Ok(Maybe::Some(index)),
            Err(err) => Err(err.to_string()),
        }
    }
}

/// Parse a string into an [`Url`], mapping the empty string to `None`.
fn parse_insecure_host(input: &str) -> Result<Maybe<TrustedHost>, String> {
    if input.is_empty() {
        Ok(Maybe::None)
    } else {
        match TrustedHost::from_str(input) {
            Ok(host) => Ok(Maybe::Some(host)),
            Err(err) => Err(err.to_string()),
        }
    }
}

/// Parse a string into a [`PathBuf`]. The string can represent a file, either as a path or a
/// `file://` URL.
fn parse_file_path(input: &str) -> Result<PathBuf, String> {
    if input.starts_with("file://") {
        let url = match url::Url::from_str(input) {
            Ok(url) => url,
            Err(err) => return Err(err.to_string()),
        };
        url.to_file_path()
            .map_err(|()| "invalid file URL".to_string())
    } else {
        Ok(PathBuf::from(input))
    }
}

/// Parse a string into a [`PathBuf`], mapping the empty string to `None`.
fn parse_maybe_file_path(input: &str) -> Result<Maybe<PathBuf>, String> {
    if input.is_empty() {
        Ok(Maybe::None)
    } else {
        parse_file_path(input).map(Maybe::Some)
    }
}

// Parse a string, mapping the empty string to `None`.
#[expect(clippy::unnecessary_wraps)]
fn parse_maybe_string(input: &str) -> Result<Maybe<String>, String> {
    if input.is_empty() {
        Ok(Maybe::None)
    } else {
        Ok(Maybe::Some(input.to_string()))
    }
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("sources").required(true).multiple(true))]
pub struct PipCompileArgs {
    /// Include the packages listed in the given files.
    ///
    /// Supported formats include `requirements.txt`, `.py` files with inline metadata,
    /// `pylock.toml`, `pyproject.toml`, `setup.py`, and `setup.cfg`.
    ///
    /// For a `pyproject.toml`, `setup.py`, or `setup.cfg` file, uv reads the project's
    /// requirements.
    ///
    /// Use `-` to read requirements from stdin.
    ///
    /// The order of requirements files and their contents determines resolution priority.
    #[arg(group = "sources", value_parser = parse_file_path, value_hint = ValueHint::FilePath)]
    pub src_file: Vec<PathBuf>,

    /// Constrain versions using the given requirements files.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    ///
    /// This is equivalent to pip's `--constraint` option.
    #[arg(
        long,
        short,
        alias = "constraint",
        env = EnvVars::UV_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub constraints: Vec<Maybe<PathBuf>>,

    /// Override versions using the given requirements files.
    ///
    /// Overrides files use the `requirements.txt` format and force a specific package version.
    /// The selected version replaces package requirements, even if the result is invalid.
    ///
    /// Constraints are _additive_: uv combines them with package requirements. Overrides are
    /// _absolute_: they replace package requirements.
    #[arg(
        long,
        alias = "override",
        env = EnvVars::UV_OVERRIDE,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub overrides: Vec<Maybe<PathBuf>>,

    /// Exclude packages from resolution using the given requirements files.
    ///
    /// Excludes files use the `requirements.txt` format and identify packages to exclude from
    /// resolution. uv omits each excluded package and ignores its dependencies. Exclusions are
    /// unconditional: uv ignores requirement specifiers and markers, and omits each listed package
    /// from every resolved environment.
    #[arg(
        long,
        alias = "exclude",
        env = EnvVars::UV_EXCLUDE,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub excludes: Vec<Maybe<PathBuf>>,

    /// Constrain build dependencies using the given requirements files when building source
    /// distributions.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    #[arg(
        long,
        short,
        alias = "build-constraint",
        env = EnvVars::UV_BUILD_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub build_constraints: Vec<Maybe<PathBuf>>,

    /// Include optional dependencies from the specified extra name; may be provided more than once.
    ///
    /// Only applies to `pyproject.toml`, `setup.py`, and `setup.cfg` sources.
    #[arg(long, value_delimiter = ',', conflicts_with = "all_extras", value_parser = extra_name_with_clap_error)]
    pub extra: Option<Vec<ExtraName>>,

    /// Include all optional dependencies.
    ///
    /// Only applies to `pyproject.toml`, `setup.py`, and `setup.cfg` sources.
    #[arg(long, conflicts_with = "extra")]
    pub all_extras: bool,

    #[arg(long, overrides_with("all_extras"), hide = true)]
    pub no_all_extras: bool,

    /// Install the specified dependency group from a `pyproject.toml`.
    ///
    /// If you do not specify a path, uv uses the `pyproject.toml` in the working directory.
    ///
    /// May be provided multiple times.
    #[arg(long, group = "sources")]
    pub group: Vec<PipGroupName>,

    #[command(flatten)]
    pub resolver: ResolverArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Ignore package dependencies, instead only add those packages explicitly listed
    /// on the command line to the resulting requirements file.
    #[arg(long)]
    pub no_deps: bool,

    #[arg(long, overrides_with("no_deps"), hide = true)]
    pub deps: bool,

    /// Write the compiled requirements to the given `requirements.txt` or `pylock.toml` file.
    ///
    /// If the file exists, uv prefers its current versions during resolution unless you also use
    /// `--upgrade`.
    #[arg(long, short, value_hint = ValueHint::FilePath)]
    pub output_file: Option<PathBuf>,

    /// The format in which the resolution should be output.
    ///
    /// Supports `requirements.txt` and `pylock.toml` (PEP 751) output formats.
    ///
    /// If you specify an output file, uv infers the format from its extension. Otherwise, it uses
    /// `requirements.txt`.
    #[arg(long, value_enum)]
    pub format: Option<PipCompileFormat>,

    /// Include extras in the output file.
    ///
    /// By default, uv removes extras because their packages already appear in the output file.
    /// Files created with `--no-strip-extras` cannot serve as constraints files for `install` or
    /// `sync`.
    #[arg(long, overrides_with("strip_extras"))]
    pub no_strip_extras: bool,

    #[arg(long, overrides_with("no_strip_extras"), hide = true)]
    pub strip_extras: bool,

    /// Include environment markers in the output file.
    ///
    /// By default, uv removes environment markers because `compile` only guarantees a valid
    /// resolution for the target environment.
    #[arg(long, overrides_with("strip_markers"))]
    pub no_strip_markers: bool,

    #[arg(long, overrides_with("no_strip_markers"), hide = true)]
    pub strip_markers: bool,

    /// Exclude comment annotations indicating the source of each package.
    #[arg(long, overrides_with("annotate"))]
    pub no_annotate: bool,

    #[arg(long, overrides_with("no_annotate"), hide = true)]
    pub annotate: bool,

    /// Exclude the comment header at the top of the generated output file.
    #[arg(long, overrides_with("header"))]
    pub no_header: bool,

    #[arg(long, overrides_with("no_header"), hide = true)]
    pub header: bool,

    /// The style of the annotation comments included in the output file, used to indicate the
    /// source of each package.
    ///
    /// Defaults to `split`.
    #[arg(long, value_enum)]
    pub annotation_style: Option<AnnotationStyle>,

    /// The header comment to include at the top of the output file generated by `uv pip compile`.
    ///
    /// Used to reflect custom build scripts and commands that wrap `uv pip compile`.
    #[arg(long, env = EnvVars::UV_CUSTOM_COMPILE_COMMAND, value_hint = ValueHint::Other)]
    pub custom_compile_command: Option<String>,

    /// The Python interpreter to use during resolution.
    ///
    /// A Python interpreter is required to build source distributions and read package metadata
    /// when wheels are not available.
    ///
    /// uv also uses the interpreter to determine the minimum Python version unless you specify
    /// `--python-version`.
    ///
    /// This option respects `UV_PYTHON`. However, `--python-version` overrides a value from the
    /// environment variable.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Install packages into the system Python environment.
    ///
    /// By default, uv uses a virtual environment in the current directory or a parent directory.
    /// If it cannot find one, it searches `PATH` for Python. Use `--system` to skip virtual
    /// environments and search only the system path.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// Include distribution hashes in the output file.
    #[arg(long, overrides_with("no_generate_hashes"))]
    pub generate_hashes: bool,

    #[arg(long, overrides_with("generate_hashes"), hide = true)]
    pub no_generate_hashes: bool,

    /// Don't build source distributions.
    ///
    /// uv reuses cached wheels from previously built source distributions. If an operation
    /// requires a new source build, uv exits with an error. uv may still build editable
    /// requirements, and their build backends may run arbitrary Python code.
    ///
    /// Alias for `--only-binary :all:`.
    #[arg(
        long,
        conflicts_with = "no_binary",
        conflicts_with = "only_binary",
        overrides_with("build")
    )]
    pub no_build: bool,

    #[arg(
        long,
        conflicts_with = "no_binary",
        conflicts_with = "only_binary",
        overrides_with("no_build"),
        hide = true
    )]
    pub build: bool,

    /// Don't install pre-built wheels.
    ///
    /// uv builds and installs the specified packages from source. If a pre-built wheel is
    /// available, the resolver still uses it to read package metadata.
    ///
    /// Specify multiple packages if needed. Use `:all:` to disable binaries for every package.
    /// Use `:none:` to clear previously specified packages.
    #[arg(long, value_delimiter = ',', conflicts_with = "no_build")]
    pub no_binary: Option<Vec<PackageNameSpecifier>>,

    /// Only use pre-built wheels; don't build source distributions.
    ///
    /// uv reuses cached wheels from previously built source distributions. If an operation must
    /// build a specified package from source, uv exits with an error. uv may still build editable
    /// requirements, and their build backends may run arbitrary Python code.
    ///
    /// Specify multiple packages if needed. Use `:all:` to disable binaries for every package.
    /// Use `:none:` to clear previously specified packages.
    #[arg(long, value_delimiter = ',', conflicts_with = "no_build")]
    pub only_binary: Option<Vec<PackageNameSpecifier>>,

    /// The Python version to use for resolution.
    ///
    /// For example, `3.8` or `3.8.17`.
    ///
    /// Defaults to the version of the Python interpreter used for resolution.
    ///
    /// Defines the minimum Python version that must be supported by the
    /// resolved requirements.
    ///
    /// If you omit the patch version, uv uses the minimum patch version. For example, `3.8` means
    /// `3.8.0`.
    #[arg(long, help_heading = "Python options")]
    pub python_version: Option<PythonVersion>,

    /// The platform for which requirements should be resolved.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    ///
    /// For macOS (Darwin), the minimum version defaults to `13.0`. Use
    /// `MACOSX_DEPLOYMENT_TARGET` to set a different minimum, such as `14.0`.
    ///
    /// For iOS, the minimum version defaults to `13.0`. Use `IPHONEOS_DEPLOYMENT_TARGET` to set
    /// a different minimum, such as `14.0`.
    ///
    /// For Android, the minimum API level defaults to `24`. Use `ANDROID_API_LEVEL` to set a
    /// different minimum, such as `26`.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,

    /// Perform a universal resolution, attempting to generate a single `requirements.txt` output
    /// file that is compatible with all operating systems, architectures, and Python
    /// implementations.
    ///
    /// In universal mode, the current Python version or `--python-version` sets the lower bound.
    /// For example, `--universal --python-version 3.7` resolves for Python 3.7 and later.
    ///
    /// Implies `--no-strip-markers`.
    #[arg(
        long,
        overrides_with("no_universal"),
        conflicts_with("python_platform"),
        conflicts_with("strip_markers")
    )]
    pub universal: bool,

    #[arg(long, overrides_with("universal"), hide = true)]
    pub no_universal: bool,

    /// Specify a package to omit from the output resolution. Its dependencies will still be
    /// included in the resolution. Equivalent to pip-compile's `--unsafe-package` option.
    #[arg(long, alias = "unsafe-package", value_delimiter = ',', value_hint = ValueHint::Other)]
    pub no_emit_package: Option<Vec<PackageName>>,

    /// Include `--index-url` and `--extra-index-url` entries in the generated output file.
    #[arg(long, overrides_with("no_emit_index_url"))]
    pub emit_index_url: bool,

    #[arg(long, overrides_with("emit_index_url"), hide = true)]
    pub no_emit_index_url: bool,

    /// Include `--find-links` entries in the generated output file.
    #[arg(long, overrides_with("no_emit_find_links"))]
    pub emit_find_links: bool,

    #[arg(long, overrides_with("emit_find_links"), hide = true)]
    pub no_emit_find_links: bool,

    /// Include `--no-binary` and `--only-binary` entries in the generated output file.
    #[arg(long, overrides_with("no_emit_build_options"))]
    pub emit_build_options: bool,

    #[arg(long, overrides_with("emit_build_options"), hide = true)]
    pub no_emit_build_options: bool,

    /// Whether to emit a marker string indicating when it is known that the
    /// resulting set of pinned dependencies is valid.
    ///
    /// If the marker expression is true, the requirements are valid. If it is false, the pinned
    /// dependencies may still be valid.
    #[arg(long, overrides_with("no_emit_marker_expression"), hide = true)]
    pub emit_marker_expression: bool,

    #[arg(long, overrides_with("emit_marker_expression"), hide = true)]
    pub no_emit_marker_expression: bool,

    /// Include comment annotations indicating the index used to resolve each package (e.g.,
    /// `# from https://pypi.org/simple`).
    #[arg(long, overrides_with("no_emit_index_annotation"))]
    pub emit_index_annotation: bool,

    #[arg(long, overrides_with("emit_index_annotation"), hide = true)]
    pub no_emit_index_annotation: bool,

    /// The backend to use when fetching packages in the PyTorch ecosystem (e.g., `cpu`, `cu126`, or `auto`).
    ///
    /// When set, uv will ignore the configured index URLs for packages in the PyTorch ecosystem,
    /// and will instead use the defined backend.
    ///
    /// For example, when set to `cpu`, uv will use the CPU-only PyTorch index; when set to `cu126`,
    /// uv will use the PyTorch index for CUDA 12.6.
    ///
    /// The `auto` mode will attempt to detect the appropriate PyTorch index based on the currently
    /// installed CUDA drivers.
    ///
    /// This option is in preview and may change in any future release.
    #[arg(long, value_enum, env = EnvVars::UV_TORCH_BACKEND)]
    pub torch_backend: Option<TorchMode>,

    #[command(flatten)]
    pub compat_args: compat::PipCompileCompatArgs,
}

#[derive(Args)]
pub struct PipSyncArgs {
    /// Include the packages listed in the given files.
    ///
    /// The following formats are supported: `requirements.txt`, `.py` files with inline metadata,
    /// `pylock.toml`, `pyproject.toml`, `setup.py`, and `setup.cfg`.
    ///
    /// For a `pyproject.toml`, `setup.py`, or `setup.cfg` file, uv reads the project's
    /// requirements.
    ///
    /// Use `-` to read requirements from stdin.
    #[arg(required(true), value_parser = parse_file_path, value_hint = ValueHint::FilePath)]
    pub src_file: Vec<PathBuf>,

    /// Constrain versions using the given requirements files.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    ///
    /// This is equivalent to pip's `--constraint` option.
    #[arg(
        long,
        short,
        alias = "constraint",
        env = EnvVars::UV_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub constraints: Vec<Maybe<PathBuf>>,

    /// Constrain build dependencies using the given requirements files when building source
    /// distributions.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    #[arg(
        long,
        short,
        alias = "build-constraint",
        env = EnvVars::UV_BUILD_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub build_constraints: Vec<Maybe<PathBuf>>,

    /// Include optional dependencies from the specified extra name; may be provided more than once.
    ///
    /// Only applies to `pylock.toml`, `pyproject.toml`, `setup.py`, and `setup.cfg` sources.
    #[arg(long, value_delimiter = ',', conflicts_with = "all_extras", value_parser = extra_name_with_clap_error)]
    pub extra: Option<Vec<ExtraName>>,

    /// Include all optional dependencies.
    ///
    /// Only applies to `pylock.toml`, `pyproject.toml`, `setup.py`, and `setup.cfg` sources.
    #[arg(long, conflicts_with = "extra", overrides_with = "no_all_extras")]
    pub all_extras: bool,

    #[arg(long, overrides_with("all_extras"), hide = true)]
    pub no_all_extras: bool,

    /// Install the specified dependency group from a `pylock.toml` or `pyproject.toml`.
    ///
    /// If no path is provided, the `pylock.toml` or `pyproject.toml` in the working directory is
    /// used.
    ///
    /// May be provided multiple times.
    #[arg(long, group = "sources")]
    pub group: Vec<PipGroupName>,

    #[command(flatten)]
    pub installer: InstallerArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    #[command(flatten)]
    pub hash_checking: HashCheckingArgs,

    /// The Python interpreter into which packages should be installed.
    ///
    /// By default, syncing requires a virtual environment. A path to an alternative Python can be
    /// provided, but it is only recommended in continuous integration (CI) environments and should
    /// be used with caution, as it can modify the system Python installation.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Install packages into the system Python environment.
    ///
    /// By default, uv installs into a virtual environment in the current directory or a parent
    /// directory. With `--system`, uv uses the first Python interpreter in the system `PATH`.
    ///
    /// WARNING: `--system` can modify the system Python installation. Use it with caution, and
    /// primarily in continuous integration (CI) environments.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// Allow uv to modify an `EXTERNALLY-MANAGED` Python installation.
    ///
    /// WARNING: `--break-system-packages` can modify Python installations that an external
    /// package manager, such as `apt`, manages. These installations explicitly warn against
    /// changes from other package managers, such as uv or `pip`. Use this option with caution,
    /// primarily in continuous integration (CI) environments.
    #[arg(
        long,
        env = EnvVars::UV_BREAK_SYSTEM_PACKAGES,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_break_system_packages")
    )]
    pub break_system_packages: bool,

    #[arg(long, overrides_with("break_system_packages"))]
    pub no_break_system_packages: bool,

    /// Install packages into the specified directory, rather than into the virtual or system Python
    /// environment. The packages will be installed at the top-level of the directory.
    ///
    /// Unlike other install operations, this command does not require discovery of an existing Python
    /// environment and only searches for a Python interpreter to use for package resolution.
    /// If a suitable Python interpreter cannot be found, uv will install one.
    /// To disable this, add `--no-python-downloads`.
    #[arg(short = 't', long, conflicts_with = "prefix", value_hint = ValueHint::DirPath)]
    pub target: Option<PathBuf>,

    /// Install packages into `lib`, `bin`, and other top-level folders under the specified
    /// directory, as if a virtual environment were present at that location.
    ///
    /// In general, prefer the use of `--python` to install into an alternate environment, as
    /// scripts and other artifacts installed via `--prefix` will reference the installing
    /// interpreter, rather than any interpreter added to the `--prefix` directory, rendering them
    /// non-portable.
    ///
    /// Unlike other install operations, this command does not require discovery of an existing Python
    /// environment and only searches for a Python interpreter to use for package resolution.
    /// If a suitable Python interpreter cannot be found, uv will install one.
    /// To disable this, add `--no-python-downloads`.
    #[arg(long, conflicts_with = "target", value_hint = ValueHint::DirPath)]
    pub prefix: Option<PathBuf>,

    /// Don't build source distributions.
    ///
    /// uv reuses cached wheels from previously built source distributions. If an operation
    /// requires a new source build, uv exits with an error. uv may still build editable
    /// requirements, and their build backends may run arbitrary Python code.
    ///
    /// Alias for `--only-binary :all:`.
    #[arg(
        long,
        conflicts_with = "no_binary",
        conflicts_with = "only_binary",
        overrides_with("build")
    )]
    pub no_build: bool,

    #[arg(
        long,
        conflicts_with = "no_binary",
        conflicts_with = "only_binary",
        overrides_with("no_build"),
        hide = true
    )]
    pub build: bool,

    /// Don't install pre-built wheels.
    ///
    /// uv builds and installs the specified packages from source. If a pre-built wheel is
    /// available, the resolver still uses it to read package metadata.
    ///
    /// Specify multiple packages if needed. Use `:all:` to disable binaries for every package.
    /// Use `:none:` to clear previously specified packages.
    #[arg(long, value_delimiter = ',', conflicts_with = "no_build")]
    pub no_binary: Option<Vec<PackageNameSpecifier>>,

    /// Only use pre-built wheels; don't build source distributions.
    ///
    /// uv reuses cached wheels from previously built source distributions. If an operation must
    /// build a specified package from source, uv exits with an error. uv may still build editable
    /// requirements, and their build backends may run arbitrary Python code.
    ///
    /// Specify multiple packages if needed. Use `:all:` to disable binaries for every package.
    /// Use `:none:` to clear previously specified packages.
    #[arg(long, value_delimiter = ',', conflicts_with = "no_build")]
    pub only_binary: Option<Vec<PackageNameSpecifier>>,

    /// Allow sync of empty requirements, which will clear the environment of all packages.
    #[arg(long, overrides_with("no_allow_empty_requirements"))]
    pub allow_empty_requirements: bool,

    #[arg(long, overrides_with("allow_empty_requirements"))]
    pub no_allow_empty_requirements: bool,

    /// The minimum Python version that should be supported by the requirements (e.g., `3.7` or
    /// `3.7.9`).
    ///
    /// If a patch version is omitted, the minimum patch version is assumed. For example, `3.7` is
    /// mapped to `3.7.0`.
    #[arg(long)]
    pub python_version: Option<PythonVersion>,

    /// The platform for which requirements should be installed.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    ///
    /// For macOS (Darwin), the minimum version defaults to `13.0`. Use
    /// `MACOSX_DEPLOYMENT_TARGET` to set a different minimum, such as `14.0`.
    ///
    /// For iOS, the minimum version defaults to `13.0`. Use `IPHONEOS_DEPLOYMENT_TARGET` to set
    /// a different minimum, such as `14.0`.
    ///
    /// For Android, the minimum API level defaults to `24`. Use `ANDROID_API_LEVEL` to set a
    /// different minimum, such as `26`.
    ///
    /// WARNING: uv selects wheels for the _target_ platform, so installed distributions may not
    /// work on the _current_ platform. uv builds source distributions for the _current_ platform,
    /// so they may not work on the _target_ platform. Use `--python-platform` only for advanced
    /// use cases.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,

    /// Validate the Python environment after completing the installation, to detect packages with
    /// missing dependencies or other issues.
    #[arg(long, overrides_with("no_strict"))]
    pub strict: bool,

    #[arg(long, overrides_with("strict"), hide = true)]
    pub no_strict: bool,

    /// Perform a dry run, i.e., don't actually install anything but resolve the dependencies and
    /// print the resulting plan.
    #[arg(long)]
    pub dry_run: bool,

    /// The backend to use when fetching packages in the PyTorch ecosystem (e.g., `cpu`, `cu126`, or `auto`).
    ///
    /// When set, uv will ignore the configured index URLs for packages in the PyTorch ecosystem,
    /// and will instead use the defined backend.
    ///
    /// For example, when set to `cpu`, uv will use the CPU-only PyTorch index; when set to `cu126`,
    /// uv will use the PyTorch index for CUDA 12.6.
    ///
    /// The `auto` mode will attempt to detect the appropriate PyTorch index based on the currently
    /// installed CUDA drivers.
    ///
    /// This option is in preview and may change in any future release.
    #[arg(long, value_enum, env = EnvVars::UV_TORCH_BACKEND)]
    pub torch_backend: Option<TorchMode>,

    #[command(flatten)]
    pub compat_args: compat::PipSyncCompatArgs,
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("sources").required(true).multiple(true))]
pub struct PipInstallArgs {
    /// Install all listed packages.
    ///
    /// The order of the packages is used to determine priority during resolution.
    #[arg(group = "sources", value_hint = ValueHint::Other)]
    pub package: Vec<String>,

    /// Install the packages listed in the given files.
    ///
    /// The following formats are supported: `requirements.txt`, `.py` files with inline metadata,
    /// `pylock.toml`, `pyproject.toml`, `setup.py`, and `setup.cfg`.
    ///
    /// For a `pyproject.toml`, `setup.py`, or `setup.cfg` file, uv reads the project's
    /// requirements.
    ///
    /// Use `-` to read requirements from stdin.
    #[arg(
        long,
        short,
        alias = "requirement",
        group = "sources",
        value_parser = parse_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub requirements: Vec<PathBuf>,

    /// Install the editable package based on the provided local file path.
    #[arg(long, short, group = "sources")]
    pub editable: Vec<String>,

    /// Install any editable dependencies as non-editable [env: UV_NO_EDITABLE=]
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_editable: bool,

    /// Install the specified editable packages as non-editable.
    #[arg(long, value_delimiter = ' ', value_hint = ValueHint::Other)]
    pub no_editable_package: Vec<PackageName>,

    /// Constrain versions using the given requirements files.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    ///
    /// This is equivalent to pip's `--constraint` option.
    #[arg(
        long,
        short,
        alias = "constraint",
        env = EnvVars::UV_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub constraints: Vec<Maybe<PathBuf>>,

    /// Override versions using the given requirements files.
    ///
    /// Overrides files use the `requirements.txt` format and force a specific package version.
    /// The selected version replaces package requirements, even if the result is invalid.
    ///
    /// Constraints are _additive_: uv combines them with package requirements. Overrides are
    /// _absolute_: they replace package requirements.
    #[arg(
        long,
        alias = "override",
        env = EnvVars::UV_OVERRIDE,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub overrides: Vec<Maybe<PathBuf>>,

    /// Exclude packages from resolution using the given requirements files.
    ///
    /// Excludes files use the `requirements.txt` format and identify packages to exclude from
    /// resolution. uv omits each excluded package and ignores its dependencies. Exclusions are
    /// unconditional: uv ignores requirement specifiers and markers, and omits each listed package
    /// from every resolved environment.
    #[arg(
        long,
        alias = "exclude",
        env = EnvVars::UV_EXCLUDE,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub excludes: Vec<Maybe<PathBuf>>,

    /// Constrain build dependencies using the given requirements files when building source
    /// distributions.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    #[arg(
        long,
        short,
        alias = "build-constraint",
        env = EnvVars::UV_BUILD_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub build_constraints: Vec<Maybe<PathBuf>>,

    /// Include optional dependencies from the specified extra name; may be provided more than once.
    ///
    /// Only applies to `pylock.toml`, `pyproject.toml`, `setup.py`, and `setup.cfg` sources.
    #[arg(long, value_delimiter = ',', conflicts_with = "all_extras", value_parser = extra_name_with_clap_error)]
    pub extra: Option<Vec<ExtraName>>,

    /// Include all optional dependencies.
    ///
    /// Only applies to `pylock.toml`, `pyproject.toml`, `setup.py`, and `setup.cfg` sources.
    #[arg(long, conflicts_with = "extra", overrides_with = "no_all_extras")]
    pub all_extras: bool,

    #[arg(long, overrides_with("all_extras"), hide = true)]
    pub no_all_extras: bool,

    /// Install the specified dependency group from a `pylock.toml` or `pyproject.toml`.
    ///
    /// If no path is provided, the `pylock.toml` or `pyproject.toml` in the working directory is
    /// used.
    ///
    /// May be provided multiple times.
    #[arg(long, group = "sources")]
    pub group: Vec<PipGroupName>,

    #[command(flatten)]
    pub installer: ResolverInstallerArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Ignore package dependencies, instead only installing those packages explicitly listed
    /// on the command line or in the requirements files.
    #[arg(long, overrides_with("deps"))]
    pub no_deps: bool,

    #[arg(long, overrides_with("no_deps"), hide = true)]
    pub deps: bool,

    #[command(flatten)]
    pub hash_checking: HashCheckingArgs,

    /// The Python interpreter into which packages should be installed.
    ///
    /// By default, installation requires a virtual environment. A path to an alternative Python can
    /// be provided, but it is only recommended in continuous integration (CI) environments and
    /// should be used with caution, as it can modify the system Python installation.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Install packages into the system Python environment.
    ///
    /// By default, uv installs into a virtual environment in the current directory or a parent
    /// directory. With `--system`, uv uses the first Python interpreter in the system `PATH`.
    ///
    /// WARNING: `--system` can modify the system Python installation. Use it with caution, and
    /// primarily in continuous integration (CI) environments.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// Allow uv to modify an `EXTERNALLY-MANAGED` Python installation.
    ///
    /// WARNING: `--break-system-packages` can modify Python installations that an external
    /// package manager, such as `apt`, manages. These installations explicitly warn against
    /// changes from other package managers, such as uv or `pip`. Use this option with caution,
    /// primarily in continuous integration (CI) environments.
    #[arg(
        long,
        env = EnvVars::UV_BREAK_SYSTEM_PACKAGES,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_break_system_packages")
    )]
    pub break_system_packages: bool,

    #[arg(long, overrides_with("break_system_packages"))]
    pub no_break_system_packages: bool,

    /// Install packages into the specified directory, rather than into the virtual or system Python
    /// environment. The packages will be installed at the top-level of the directory.
    ///
    /// Unlike other install operations, this command does not require discovery of an existing Python
    /// environment and only searches for a Python interpreter to use for package resolution.
    /// If a suitable Python interpreter cannot be found, uv will install one.
    /// To disable this, add `--no-python-downloads`.
    #[arg(short = 't', long, conflicts_with = "prefix", value_hint = ValueHint::DirPath)]
    pub target: Option<PathBuf>,

    /// Install packages into `lib`, `bin`, and other top-level folders under the specified
    /// directory, as if a virtual environment were present at that location.
    ///
    /// In general, prefer the use of `--python` to install into an alternate environment, as
    /// scripts and other artifacts installed via `--prefix` will reference the installing
    /// interpreter, rather than any interpreter added to the `--prefix` directory, rendering them
    /// non-portable.
    ///
    /// Unlike other install operations, this command does not require discovery of an existing Python
    /// environment and only searches for a Python interpreter to use for package resolution.
    /// If a suitable Python interpreter cannot be found, uv will install one.
    /// To disable this, add `--no-python-downloads`.
    #[arg(long, conflicts_with = "target", value_hint = ValueHint::DirPath)]
    pub prefix: Option<PathBuf>,

    /// Don't build source distributions.
    ///
    /// uv reuses cached wheels from previously built source distributions. If an operation
    /// requires a new source build, uv exits with an error. uv may still build editable
    /// requirements, and their build backends may run arbitrary Python code.
    ///
    /// Alias for `--only-binary :all:`.
    #[arg(
        long,
        conflicts_with = "no_binary",
        conflicts_with = "only_binary",
        overrides_with("build")
    )]
    pub no_build: bool,

    #[arg(
        long,
        conflicts_with = "no_binary",
        conflicts_with = "only_binary",
        overrides_with("no_build"),
        hide = true
    )]
    pub build: bool,

    /// Don't install pre-built wheels.
    ///
    /// uv builds and installs the specified packages from source. If a pre-built wheel is
    /// available, the resolver still uses it to read package metadata.
    ///
    /// Specify multiple packages if needed. Use `:all:` to disable binaries for every package.
    /// Use `:none:` to clear previously specified packages.
    #[arg(long, value_delimiter = ',', conflicts_with = "no_build")]
    pub no_binary: Option<Vec<PackageNameSpecifier>>,

    /// Only use pre-built wheels; don't build source distributions.
    ///
    /// uv reuses cached wheels from previously built source distributions. If an operation must
    /// build a specified package from source, uv exits with an error. uv may still build editable
    /// requirements, and their build backends may run arbitrary Python code.
    ///
    /// Specify multiple packages if needed. Use `:all:` to disable binaries for every package.
    /// Use `:none:` to clear previously specified packages.
    #[arg(long, value_delimiter = ',', conflicts_with = "no_build")]
    pub only_binary: Option<Vec<PackageNameSpecifier>>,

    /// The minimum Python version that should be supported by the requirements (e.g., `3.7` or
    /// `3.7.9`).
    ///
    /// If a patch version is omitted, the minimum patch version is assumed. For example, `3.7` is
    /// mapped to `3.7.0`.
    #[arg(long)]
    pub python_version: Option<PythonVersion>,

    /// The platform for which requirements should be installed.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    ///
    /// For macOS (Darwin), the minimum version defaults to `13.0`. Use
    /// `MACOSX_DEPLOYMENT_TARGET` to set a different minimum, such as `14.0`.
    ///
    /// For iOS, the minimum version defaults to `13.0`. Use `IPHONEOS_DEPLOYMENT_TARGET` to set
    /// a different minimum, such as `14.0`.
    ///
    /// For Android, the minimum API level defaults to `24`. Use `ANDROID_API_LEVEL` to set a
    /// different minimum, such as `26`.
    ///
    /// WARNING: uv selects wheels for the _target_ platform, so installed distributions may not
    /// work on the _current_ platform. uv builds source distributions for the _current_ platform,
    /// so they may not work on the _target_ platform. Use `--python-platform` only for advanced
    /// use cases.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,

    /// Do not remove extraneous packages present in the environment.
    #[arg(long, overrides_with("exact"), alias = "no-exact", hide = true)]
    pub inexact: bool,

    /// Perform an exact sync, removing extraneous packages.
    ///
    /// By default, installing will make the minimum necessary changes to satisfy the requirements.
    /// When enabled, uv will update the environment to exactly match the requirements, removing
    /// packages that are not included in the requirements.
    #[arg(long, overrides_with("inexact"))]
    pub exact: bool,

    /// Validate the Python environment after completing the installation, to detect packages with
    /// missing dependencies or other issues.
    #[arg(long, overrides_with("no_strict"))]
    pub strict: bool,

    #[arg(long, overrides_with("strict"), hide = true)]
    pub no_strict: bool,

    /// Perform a dry run, i.e., don't actually install anything but resolve the dependencies and
    /// print the resulting plan.
    #[arg(long)]
    pub dry_run: bool,

    /// The backend to use when fetching packages in the PyTorch ecosystem (e.g., `cpu`, `cu126`, or `auto`)
    ///
    /// When set, uv will ignore the configured index URLs for packages in the PyTorch ecosystem,
    /// and will instead use the defined backend.
    ///
    /// For example, when set to `cpu`, uv will use the CPU-only PyTorch index; when set to `cu126`,
    /// uv will use the PyTorch index for CUDA 12.6.
    ///
    /// The `auto` mode will attempt to detect the appropriate PyTorch index based on the currently
    /// installed CUDA drivers.
    ///
    /// This option is in preview and may change in any future release.
    #[arg(long, value_enum, env = EnvVars::UV_TORCH_BACKEND)]
    pub torch_backend: Option<TorchMode>,

    #[command(flatten)]
    pub compat_args: compat::PipInstallCompatArgs,
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("sources").required(true).multiple(true))]
pub struct PipUninstallArgs {
    /// Uninstall all listed packages.
    #[arg(group = "sources", value_hint = ValueHint::Other)]
    pub package: Vec<String>,

    /// Uninstall the packages listed in the given files.
    ///
    /// The following formats are supported: `requirements.txt`, `.py` files with inline metadata,
    /// `pylock.toml`, `pyproject.toml`, `setup.py`, and `setup.cfg`.
    #[arg(long, short, alias = "requirement", group = "sources", value_parser = parse_file_path, value_hint = ValueHint::FilePath)]
    pub requirements: Vec<PathBuf>,

    /// The Python interpreter from which packages should be uninstalled.
    ///
    /// By default, uninstallation requires a virtual environment. A path to an alternative Python
    /// can be provided, but it is only recommended in continuous integration (CI) environments and
    /// should be used with caution, as it can modify the system Python installation.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Attempt to use `keyring` for authentication for remote requirements files.
    ///
    /// At present, only `--keyring-provider subprocess` is supported, which configures uv to use
    /// the `keyring` CLI to handle authentication.
    ///
    /// Defaults to `disabled`.
    #[arg(long, value_enum, env = EnvVars::UV_KEYRING_PROVIDER)]
    pub keyring_provider: Option<KeyringProviderType>,

    /// Use the system Python to uninstall packages.
    ///
    /// By default, uv uninstalls from the virtual environment in the current working directory or
    /// any parent directory. The `--system` option instructs uv to instead use the first Python
    /// found in the system `PATH`.
    ///
    /// WARNING: `--system` can modify the system Python installation. Use it with caution, and
    /// primarily in continuous integration (CI) environments.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// Allow uv to modify an `EXTERNALLY-MANAGED` Python installation.
    ///
    /// WARNING: `--break-system-packages` can modify Python installations that an external
    /// package manager, such as `apt`, manages. These installations explicitly warn against
    /// changes from other package managers, such as uv or `pip`. Use this option with caution,
    /// primarily in continuous integration (CI) environments.
    #[arg(
        long,
        env = EnvVars::UV_BREAK_SYSTEM_PACKAGES,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_break_system_packages")
    )]
    pub break_system_packages: bool,

    #[arg(long, overrides_with("break_system_packages"))]
    pub no_break_system_packages: bool,

    /// Uninstall packages from the specified `--target` directory.
    #[arg(short = 't', long, conflicts_with = "prefix", value_hint = ValueHint::DirPath)]
    pub target: Option<PathBuf>,

    /// Uninstall packages from the specified `--prefix` directory.
    #[arg(long, conflicts_with = "target", value_hint = ValueHint::DirPath)]
    pub prefix: Option<PathBuf>,

    /// Perform a dry run, i.e., don't actually uninstall anything but print the resulting plan.
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub compat_args: compat::PipUninstallCompatArgs,
}

#[derive(Args)]
pub struct PipFreezeArgs {
    /// Exclude any editable packages from output.
    #[arg(long)]
    pub exclude_editable: bool,

    /// Exclude the specified package(s) from the output.
    #[arg(long)]
    pub r#exclude: Vec<PackageName>,

    /// Validate the Python environment, to detect packages with missing dependencies and other
    /// issues.
    #[arg(long, overrides_with("no_strict"))]
    pub strict: bool,

    #[arg(long, overrides_with("strict"), hide = true)]
    pub no_strict: bool,

    /// The Python interpreter for which packages should be listed.
    ///
    /// By default, uv lists packages in a virtual environment. If it cannot find one, it lists
    /// packages in a system Python environment.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Restrict to the specified installation path for listing packages (can be used multiple times).
    #[arg(long("path"), value_parser = parse_file_path, value_hint = ValueHint::DirPath)]
    pub paths: Option<Vec<PathBuf>>,

    /// List packages in the system Python environment.
    ///
    /// Disables discovery of virtual environments.
    ///
    /// See `uv help python` for details on Python discovery.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// List packages from the specified `--target` directory.
    #[arg(short = 't', long, conflicts_with_all = ["prefix", "paths"], value_hint = ValueHint::DirPath)]
    pub target: Option<PathBuf>,

    /// List packages from the specified `--prefix` directory.
    #[arg(long, conflicts_with_all = ["target", "paths"], value_hint = ValueHint::DirPath)]
    pub prefix: Option<PathBuf>,

    #[command(flatten)]
    pub compat_args: compat::PipGlobalCompatArgs,
}

#[derive(Args)]
pub struct PipListArgs {
    /// Only include editable projects.
    #[arg(short, long)]
    pub editable: bool,

    /// Exclude any editable packages from output.
    #[arg(long, conflicts_with = "editable")]
    pub exclude_editable: bool,

    /// Exclude the specified package(s) from the output.
    #[arg(long, value_hint = ValueHint::Other)]
    pub r#exclude: Vec<PackageName>,

    /// Select the output format.
    #[arg(long, value_enum, default_value_t = ListFormat::default())]
    pub format: ListFormat,

    /// List outdated packages.
    ///
    /// uv shows the latest version of each package next to the installed version. It omits
    /// up-to-date packages.
    #[arg(long, overrides_with("no_outdated"))]
    pub outdated: bool,

    #[arg(long, overrides_with("outdated"), hide = true)]
    pub no_outdated: bool,

    /// Validate the Python environment, to detect packages with missing dependencies and other
    /// issues.
    #[arg(long, overrides_with("no_strict"))]
    pub strict: bool,

    #[arg(long, overrides_with("strict"), hide = true)]
    pub no_strict: bool,

    #[command(flatten)]
    pub fetch: FetchArgs,

    /// The Python interpreter for which packages should be listed.
    ///
    /// By default, uv lists packages in a virtual environment. If it cannot find one, it lists
    /// packages in a system Python environment.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// List packages in the system Python environment.
    ///
    /// Disables discovery of virtual environments.
    ///
    /// See `uv help python` for details on Python discovery.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// List packages from the specified `--target` directory.
    #[arg(short = 't', long, conflicts_with = "prefix", value_hint = ValueHint::DirPath)]
    pub target: Option<PathBuf>,

    /// List packages from the specified `--prefix` directory.
    #[arg(long, conflicts_with = "target", value_hint = ValueHint::DirPath)]
    pub prefix: Option<PathBuf>,

    #[command(flatten)]
    pub compat_args: compat::PipListCompatArgs,
}

#[derive(Args)]
pub struct PipCheckArgs {
    /// The Python interpreter for which packages should be checked.
    ///
    /// By default, uv checks packages in a virtual environment but will check packages in a system
    /// Python environment if no virtual environment is found.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Check packages in the system Python environment.
    ///
    /// Disables discovery of virtual environments.
    ///
    /// See `uv help python` for details on Python discovery.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// The Python version against which packages should be checked.
    ///
    /// By default, the installed packages are checked against the version of the current
    /// interpreter.
    #[arg(long)]
    pub python_version: Option<PythonVersion>,

    /// The platform for which packages should be checked.
    ///
    /// By default, the installed packages are checked against the platform of the current
    /// interpreter.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    ///
    /// For macOS (Darwin), the minimum version defaults to `13.0`. Use
    /// `MACOSX_DEPLOYMENT_TARGET` to set a different minimum, such as `14.0`.
    ///
    /// For iOS, the minimum version defaults to `13.0`. Use `IPHONEOS_DEPLOYMENT_TARGET` to set
    /// a different minimum, such as `14.0`.
    ///
    /// For Android, the minimum API level defaults to `24`. Use `ANDROID_API_LEVEL` to set a
    /// different minimum, such as `26`.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,
}

#[derive(Args)]
pub struct PipShowArgs {
    /// The package(s) to display.
    #[arg(value_hint = ValueHint::Other)]
    pub package: Vec<PackageName>,

    /// Validate the Python environment, to detect packages with missing dependencies and other
    /// issues.
    #[arg(long, overrides_with("no_strict"))]
    pub strict: bool,

    #[arg(long, overrides_with("strict"), hide = true)]
    pub no_strict: bool,

    /// Show the full list of installed files for each package.
    #[arg(short, long)]
    pub files: bool,

    /// The Python interpreter to find the package in.
    ///
    /// By default, uv looks for packages in a virtual environment but will look for packages in a
    /// system Python environment if no virtual environment is found.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Show a package in the system Python environment.
    ///
    /// Disables discovery of virtual environments.
    ///
    /// See `uv help python` for details on Python discovery.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// Show a package from the specified `--target` directory.
    #[arg(short = 't', long, conflicts_with = "prefix", value_hint = ValueHint::DirPath)]
    pub target: Option<PathBuf>,

    /// Show a package from the specified `--prefix` directory.
    #[arg(long, conflicts_with = "target", value_hint = ValueHint::DirPath)]
    pub prefix: Option<PathBuf>,

    #[command(flatten)]
    pub compat_args: compat::PipGlobalCompatArgs,
}

#[derive(Args)]
pub struct PipTreeArgs {
    /// Show the version constraint(s) imposed on each package.
    #[arg(long)]
    pub show_version_specifiers: bool,

    #[command(flatten)]
    pub tree: DisplayTreeArgs,

    /// Validate the Python environment, to detect packages with missing dependencies and other
    /// issues.
    #[arg(long, overrides_with("no_strict"))]
    pub strict: bool,

    #[arg(long, overrides_with("strict"), hide = true)]
    pub no_strict: bool,

    #[command(flatten)]
    pub fetch: FetchArgs,

    /// The Python interpreter for which packages should be listed.
    ///
    /// By default, uv lists packages in a virtual environment. If it cannot find one, it lists
    /// packages in a system Python environment.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// List packages in the system Python environment.
    ///
    /// Disables discovery of virtual environments.
    ///
    /// See `uv help python` for details on Python discovery.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    #[command(flatten)]
    pub compat_args: compat::PipGlobalCompatArgs,
}

#[derive(Args)]
pub struct PipDebugArgs {
    #[arg(long, hide = true)]
    platform: Option<String>,

    #[arg(long, hide = true)]
    python_version: Option<String>,

    #[arg(long, hide = true)]
    implementation: Option<String>,

    #[arg(long, hide = true)]
    abi: Option<String>,
}

#[derive(Args)]
pub struct BuildArgs {
    /// The directory from which distributions should be built, or a source
    /// distribution archive to build into a wheel.
    ///
    /// Defaults to the current working directory.
    #[arg(value_parser = parse_file_path, value_hint = ValueHint::DirPath)]
    pub src: Option<PathBuf>,

    /// Build a specific package in the workspace.
    ///
    /// uv searches for the workspace from the source directory. If no source directory is
    /// specified, it searches from the current directory.
    ///
    /// If the workspace member does not exist, uv will exit with an error.
    #[arg(long, conflicts_with("all_packages"), value_hint = ValueHint::Other)]
    pub package: Option<PackageName>,

    /// Builds all packages in the workspace.
    ///
    /// uv searches for the workspace from the source directory. If no source directory is
    /// specified, it searches from the current directory.
    ///
    /// If the workspace member does not exist, uv will exit with an error.
    #[arg(long, alias = "all", conflicts_with("package"))]
    pub all_packages: bool,

    /// The output directory to which distributions should be written.
    ///
    /// Defaults to the `dist` subdirectory within the source directory, or the
    /// directory containing the source distribution archive.
    #[arg(long, short, value_parser = parse_file_path, value_hint = ValueHint::DirPath)]
    pub out_dir: Option<PathBuf>,

    /// Build a source distribution ("sdist") from the given directory.
    #[arg(long)]
    pub sdist: bool,

    /// Build a binary distribution ("wheel") from the given directory.
    #[arg(long)]
    pub wheel: bool,

    /// When using the uv build backend, list the files that would be included when building.
    ///
    /// uv does not build a distribution unless it needs a source distribution to build a wheel. It
    /// collects the file list directly without a PEP 517 environment. This option works only with
    /// the uv build backend because PEP 517 has no file-list build hook.
    ///
    /// Combine this option with `--sdist` or `--wheel` to inspect different build paths.
    // Hidden while in preview.
    #[arg(long, hide = true)]
    pub list: bool,

    #[arg(long, overrides_with("no_build_logs"), hide = true)]
    pub build_logs: bool,

    /// Hide logs from the build backend.
    #[arg(long, overrides_with("build_logs"))]
    pub no_build_logs: bool,

    /// Always build through PEP 517, don't use the fast path for the uv build backend.
    ///
    /// By default, uv calls its build backend directly and does not create a PEP 517 build
    /// environment. This option forces uv to use PEP 517 instead.
    #[arg(long, conflicts_with = "list")]
    pub force_pep517: bool,

    /// Clear the output directory before the build, removing stale artifacts.
    #[arg(long)]
    pub clear: bool,

    #[arg(long, overrides_with("no_create_gitignore"), hide = true)]
    pub create_gitignore: bool,

    /// Do not create a `.gitignore` file in the output directory.
    ///
    /// By default, uv creates a `.gitignore` file to exclude build artifacts from version control.
    /// This option prevents uv from creating that file.
    #[arg(long, overrides_with("create_gitignore"))]
    pub no_create_gitignore: bool,

    /// Constrain build dependencies using the given requirements files when building distributions.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a build dependency. Listing a package does _not_ install it.
    #[arg(
        long,
        short,
        alias = "build-constraint",
        env = EnvVars::UV_BUILD_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub build_constraints: Vec<Maybe<PathBuf>>,

    #[command(flatten)]
    pub hash_checking: HashCheckingArgs,

    /// The Python interpreter to use for the build environment.
    ///
    /// By default, uv runs builds in isolated virtual environments. It creates those environments
    /// with the discovered interpreter and copies or links the interpreter based on the platform.
    ///
    /// See `uv help python` to view supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    #[command(flatten)]
    pub resolver: ResolverArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,
}

#[derive(Args)]
pub struct VenvArgs {
    /// The Python interpreter to use for the virtual environment.
    ///
    /// When creating a virtual environment, uv does not search other virtual environments for
    /// Python interpreters.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Ignore virtual environments when searching for the Python interpreter.
    ///
    /// This is the default behavior and has no effect.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system"),
        hide = true,
    )]
    pub system: bool,

    /// This flag is included for compatibility only, it has no effect.
    ///
    /// uv never searches virtual environments for interpreters when creating a virtual
    /// environment.
    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// Avoid discovering a project or workspace.
    ///
    /// By default, uv searches the current directory and parent directories for a project. It uses
    /// the project to determine the virtual environment path and check Python version constraints.
    #[arg(
        long,
        alias = "no-workspace",
        env = EnvVars::UV_NO_PROJECT,
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub no_project: bool,

    /// Install seed packages (one or more of: `pip`, `setuptools`, and `wheel`) into the virtual
    /// environment [env: UV_VENV_SEED=]
    ///
    /// Python 3.12 and later environments do not include `setuptools` or `wheel`.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
    pub seed: bool,

    /// Remove any existing files or directories at the target path [env: UV_VENV_CLEAR=]
    ///
    /// By default, `uv venv` exits with an error if the target path is not empty. Use `--clear` to
    /// remove its contents before creating the virtual environment.
    #[clap(long, short, overrides_with = "allow_existing", value_parser = clap::builder::BoolishValueParser::new())]
    pub clear: bool,

    /// Allow `--clear` to remove a non-virtual environment directory.
    ///
    /// This removes all files and directories at the target path.
    #[arg(long)]
    pub force: bool,

    /// Fail without prompting if any existing files or directories are present at the target path.
    ///
    /// By default, `uv venv` prompts before clearing a non-empty directory if a TTY is available.
    /// With `--no-clear`, the command exits with an error instead.
    #[clap(
        long,
        overrides_with = "clear",
        conflicts_with = "allow_existing",
        hide = true
    )]
    pub no_clear: bool,

    /// Preserve any existing files or directories at the target path.
    ///
    /// By default, `uv venv` exits with an error if the target path is not empty. With
    /// `--allow-existing`, uv writes to the existing path without clearing it.
    ///
    /// WARNING: This option can cause unexpected behavior if the existing and new virtual
    /// environments use different Python interpreters.
    #[clap(long, overrides_with = "clear")]
    pub allow_existing: bool,

    /// The path to the virtual environment to create.
    ///
    /// Default to `.venv` in the working directory.
    ///
    /// uv resolves relative paths from the working directory.
    #[arg(value_hint = ValueHint::DirPath)]
    pub path: Option<PathBuf>,

    /// Provide an alternative prompt prefix for the virtual environment.
    ///
    /// By default, the prompt depends on whether you specify a path. For `uv venv project`, uv
    /// uses the target directory name. For `uv venv`, it uses the current directory name.
    ///
    /// Use `.` to select the current directory name, even if you specify a path.
    #[arg(long, verbatim_doc_comment, value_hint = ValueHint::Other)]
    pub prompt: Option<String>,

    /// Give the virtual environment access to the system site packages directory.
    ///
    /// Unlike `pip`, uv does _not_ include system site packages in commands such as `uv pip list`
    /// or `uv pip install`. The virtual environment can access system site packages at runtime, but
    /// this does not change how uv commands behave.
    #[arg(long)]
    pub system_site_packages: bool,

    /// Make the virtual environment relocatable [env: UV_VENV_RELOCATABLE=]
    ///
    /// You can move or redistribute a relocatable virtual environment without breaking its entry
    /// points or activation scripts.
    ///
    /// uv guarantees this behavior only for standard `console_scripts` and `gui_scripts`. It may
    /// update other scripts that have a generic `#!python[w]` shebang. It does not change binaries.
    ///
    /// uv makes the environment relocatable by writing relative paths. The entry points and
    /// scripts themselves are _not_ relocatable: they do not work if you copy them outside the
    /// environment.
    #[expect(clippy::doc_markdown)]
    #[arg(long, overrides_with("no_relocatable"))]
    pub relocatable: bool,

    /// Don't make the virtual environment relocatable.
    ///
    /// Disables the default relocatable behavior when the `relocatable-envs-default` preview
    /// feature is enabled.
    #[arg(long, overrides_with("relocatable"), hide = true)]
    pub no_relocatable: bool,

    #[command(flatten)]
    pub index_args: IndexArgs,

    #[command(flatten)]
    pub registry_client: RegistryClientArgs,

    #[command(flatten)]
    pub exclude_newer: PackageExcludeNewerArgs,

    /// The method to use when installing packages from the global cache.
    ///
    /// This option is only used for installing seed packages.
    ///
    /// Defaults to `clone` (also known as Copy-on-Write) on macOS and Linux, and `hardlink` on
    /// Windows.
    ///
    /// WARNING: Symlink mode links the target environment to the cache. Clearing the cache with
    /// `uv cache clean` removes the source files and breaks all installed packages. Avoid symlink
    /// mode unless you understand this risk.
    #[arg(long, value_enum, env = EnvVars::UV_LINK_MODE)]
    pub link_mode: Option<uv_install_wheel::LinkMode>,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    #[command(flatten)]
    pub compat_args: compat::VenvCompatArgs,
}

#[derive(Parser, Debug, Clone)]
pub enum ExternalCommand {
    #[command(external_subcommand)]
    Cmd(Vec<OsString>),
}

impl Deref for ExternalCommand {
    type Target = Vec<OsString>;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Cmd(cmd) => cmd,
        }
    }
}

impl DerefMut for ExternalCommand {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Cmd(cmd) => cmd,
        }
    }
}

impl ExternalCommand {
    pub fn split(&self) -> (Option<&OsString>, &[OsString]) {
        match self.as_slice() {
            [] => (None, &[]),
            [cmd, args @ ..] => (Some(cmd), args),
        }
    }
}

#[derive(Debug, Default, Copy, Clone, clap::ValueEnum)]
pub enum AuthorFrom {
    /// Fetch the author information from some sources (e.g., Git) automatically.
    #[default]
    Auto,
    /// Fetch the author information from Git configuration only.
    Git,
    /// Do not infer the author information.
    None,
}

#[derive(Args)]
pub struct InitArgs {
    /// The path to use for the project/script.
    ///
    /// For an app or library, the default is the current working directory. A script requires a
    /// path. Both relative and absolute paths are accepted.
    ///
    /// If a parent directory contains a `pyproject.toml`, uv adds the project to the parent
    /// workspace unless you use `--no-workspace`.
    #[arg(required_if_eq("script", "true"), value_hint = ValueHint::DirPath)]
    pub path: Option<PathBuf>,

    /// The name of the project.
    ///
    /// Defaults to the name of the directory.
    #[arg(long, conflicts_with = "script", value_hint = ValueHint::Other)]
    pub name: Option<PackageName>,

    /// Only create a `pyproject.toml`.
    ///
    /// Do not create extra files, such as `README.md`, a `src/` tree, or `.python-version`.
    ///
    /// A `[build-system]` table is only created with `--package` or `--build-backend`.
    ///
    /// With `--script`, create a script that contains only the inline metadata header.
    #[arg(long)]
    pub bare: bool,

    /// Create a virtual project, rather than a package.
    ///
    /// This option is deprecated and will be removed in a future release.
    #[arg(long, hide = true, conflicts_with = "package")]
    pub r#virtual: bool,

    /// Set up the project to be built as a Python package.
    ///
    /// Defines a `[build-system]` for the project.
    ///
    /// This is the default behavior.
    #[arg(long, overrides_with = "no_package")]
    pub r#package: bool,

    /// Do not set up the project to be built as a Python package.
    ///
    /// Create a flat project directory that cannot be imported as a module and has no
    /// `[build-system]` entry. Use this option for applications that you do not plan to distribute
    /// as packages.
    #[arg(long, overrides_with = "package", conflicts_with_all = ["lib", "build_backend"])]
    pub r#no_package: bool,

    /// Create a project for an application.
    ///
    /// This project kind is for web servers, scripts, and command-line interfaces.
    ///
    /// Applications are packaged by default. Use `--no-package` to create an unpackaged application.
    #[arg(long, alias = "application", conflicts_with_all = ["lib", "script"])]
    pub r#app: bool,

    /// Create a project for a library.
    ///
    /// A library is a project that is intended to be built and distributed as a Python package.
    #[arg(long, alias = "library", conflicts_with_all=["app", "script"])]
    pub r#lib: bool,

    /// Create a script.
    ///
    /// A script is a standalone file with inline metadata for its dependencies and Python version
    /// requirements, as defined by PEP 723.
    ///
    /// PEP 723 scripts can be executed directly with `uv run`.
    ///
    /// By default, uv adds a requirement for the system Python version. Use `--python` to specify
    /// a different version.
    #[arg(long, conflicts_with_all=["app", "lib", "package", "build_backend", "description"])]
    pub r#script: bool,

    /// Set the project description.
    #[arg(long, conflicts_with = "script", overrides_with = "no_description", value_hint = ValueHint::Other)]
    pub description: Option<String>,

    /// Disable the description for the project.
    #[arg(long, conflicts_with = "script", overrides_with = "description")]
    pub no_description: bool,

    /// Initialize a version control system for the project.
    ///
    /// By default, uv initializes a Git repository. Use `--vcs none` to skip version control.
    #[arg(long, value_enum, conflicts_with = "script")]
    pub vcs: Option<VersionControlSystem>,

    /// Initialize a build-backend of choice for the project.
    ///
    /// Implicitly sets `--package`.
    #[arg(long, value_enum, conflicts_with_all=["script", "no_package"], env = EnvVars::UV_INIT_BUILD_BACKEND)]
    pub build_backend: Option<ProjectBuildBackend>,

    /// Invalid option name for build backend.
    #[arg(
        long,
        required(false),
        action(clap::ArgAction::SetTrue),
        value_parser=clap::builder::UnknownArgumentValueParser::suggest_arg("--build-backend"),
        hide(true)
    )]
    backend: Option<String>,

    /// Do not create a `README.md` file.
    #[arg(long)]
    pub no_readme: bool,

    /// Fill in the `authors` field in the `pyproject.toml`.
    ///
    /// By default, uv infers author information from available sources, such as Git (`auto`). Use
    /// `--author-from git` to use only Git configuration. Use `--author-from none` to skip author
    /// information.
    #[arg(long, value_enum)]
    pub author_from: Option<AuthorFrom>,

    /// Do not create a `.python-version` file for the project.
    ///
    /// By default, uv records the discovered interpreter's minor version in `.python-version`.
    /// Later uv commands use that version.
    #[arg(long)]
    pub no_pin_python: bool,

    /// Create a `.python-version` file for the project.
    ///
    /// This is the default.
    #[arg(long, hide = true)]
    pub pin_python: bool,

    /// Avoid discovering a workspace and create a standalone project.
    ///
    /// By default, uv searches for workspaces in the current directory or any parent directory.
    #[arg(long, alias = "no-project")]
    pub no_workspace: bool,

    /// The Python interpreter to use to determine the minimum supported Python version.
    ///
    /// See `uv help python` to view supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,
}

#[derive(Args)]
pub struct RunArgs {
    /// Include optional dependencies from the specified extra name.
    ///
    /// May be provided more than once.
    ///
    /// This option is only available when running in a project.
    #[arg(
        long,
        conflicts_with = "all_extras",
        conflicts_with = "only_group",
        value_delimiter = ',',
        value_parser = extra_name_with_clap_error,
        value_hint = ValueHint::Other,
    )]
    pub extra: Option<Vec<ExtraName>>,

    /// Include all optional dependencies.
    ///
    /// This option is only available when running in a project.
    #[arg(long, conflicts_with = "extra", conflicts_with = "only_group")]
    pub all_extras: bool,

    /// Exclude the specified optional dependencies, if `--all-extras` is supplied.
    ///
    /// May be provided multiple times.
    #[arg(long, value_hint = ValueHint::Other)]
    pub no_extra: Vec<ExtraName>,

    #[arg(long, overrides_with("all_extras"), hide = true)]
    pub no_all_extras: bool,

    #[command(flatten)]
    pub dependency_groups: ProjectDependencyGroupsArgs,

    /// Run a Python module.
    ///
    /// Equivalent to `python -m <module>`.
    #[arg(short, long, conflicts_with_all = ["script", "gui_script"])]
    pub module: bool,

    /// Install any non-editable dependencies, including the project and any workspace members, as
    /// editable.
    #[arg(long, overrides_with = "no_editable", hide = true)]
    pub editable: bool,

    /// Install any editable dependencies, including the project and any workspace members, as
    /// non-editable [env: UV_NO_EDITABLE=]
    #[arg(long, overrides_with = "editable", value_parser = clap::builder::BoolishValueParser::new())]
    pub no_editable: bool,

    /// Install the specified editable packages as non-editable.
    #[arg(long, value_delimiter = ' ', value_hint = ValueHint::Other)]
    pub no_editable_package: Vec<PackageName>,

    /// Do not remove extraneous packages present in the environment.
    #[arg(long, overrides_with("exact"), alias = "no-exact", hide = true)]
    pub inexact: bool,

    /// Perform an exact sync, removing extraneous packages.
    ///
    /// This option removes extra packages from the environment. By default, `uv run` makes only
    /// the changes needed to satisfy the requirements.
    #[arg(long, overrides_with("inexact"))]
    pub exact: bool,

    /// Load environment variables from a `.env` file.
    ///
    /// Specify multiple files if needed. Values in later files override values in earlier
    /// files.
    #[arg(long, env = EnvVars::UV_ENV_FILE, value_hint = ValueHint::FilePath)]
    pub env_file: Vec<String>,

    /// Avoid reading environment variables from a `.env` file [env: UV_NO_ENV_FILE=]
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_env_file: bool,

    /// The command to run.
    ///
    /// uv runs a `.py` script with the Python interpreter.
    #[command(subcommand)]
    pub command: Option<ExternalCommand>,

    /// Run with the given packages installed.
    ///
    /// In a project, uv installs these dependencies in a separate temporary environment layered
    /// over the project environment. They may conflict with project dependencies.
    #[arg(short = 'w', long, value_hint = ValueHint::Other)]
    pub with: Vec<comma::CommaSeparatedRequirements>,

    /// Run with the given packages installed in editable mode.
    ///
    /// In a project, uv installs these dependencies in a separate temporary environment layered
    /// over the project environment. They may conflict with project dependencies.
    #[arg(long, value_hint = ValueHint::DirPath)]
    pub with_editable: Vec<comma::CommaSeparatedRequirements>,

    /// Run with the packages listed in the given files.
    ///
    /// The following formats are supported: `requirements.txt`, `.py` files with inline metadata,
    /// and `pylock.toml`.
    ///
    /// The same environment semantics as `--with` apply.
    ///
    /// Using `pyproject.toml`, `setup.py`, or `setup.cfg` files is not allowed.
    #[arg(long, value_delimiter = ',', value_parser = parse_maybe_file_path, value_hint = ValueHint::FilePath)]
    pub with_requirements: Vec<Maybe<PathBuf>>,

    /// Run the command in an isolated virtual environment [env: UV_ISOLATED=]
    ///
    /// By default, uv reuses the project environment to improve performance. This option creates a
    /// new environment that contains only the declared dependencies.
    ///
    /// An editable installation is still used for the project.
    ///
    /// With `--with` or `--with-requirements`, uv still layers additional dependencies in a second
    /// environment.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
    pub isolated: bool,

    /// Prefer the active virtual environment over the project's virtual environment.
    ///
    /// If the project virtual environment is active or no virtual environment is active, this has
    /// no effect.
    #[arg(long, overrides_with = "no_active")]
    pub active: bool,

    /// Prefer project's virtual environment over an active environment.
    ///
    /// This is the default behavior.
    #[arg(long, overrides_with = "active", hide = true)]
    pub no_active: bool,

    /// Avoid syncing the virtual environment [env: UV_NO_SYNC=]
    ///
    /// Implies `--frozen`. uv ignores project dependencies and does not update the lockfile because
    /// it does not sync the environment.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_sync: bool,

    /// Assert that the `uv.lock` will remain unchanged [env: UV_LOCKED=]
    ///
    /// Requires that the lockfile is up-to-date. If the lockfile is missing or
    /// needs to be updated, uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Run without updating the `uv.lock` file [env: UV_FROZEN=]
    ///
    /// Instead of checking if the lockfile is up-to-date, uses the versions in the lockfile as the
    /// source of truth. If the lockfile is missing, uv will exit with an error. If the
    /// `pyproject.toml` includes changes to dependencies that have not been included in the
    /// lockfile yet, they will not be present in the environment.
    #[arg(long, conflicts_with_all = ["locked", "upgrade", "no_sources"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    /// Run the given path as a Python script.
    ///
    /// Parse the path as a PEP 723 script, regardless of its file extension.
    #[arg(long, short, conflicts_with_all = ["module", "gui_script"])]
    pub script: bool,

    /// Run the given path as a Python GUI script.
    ///
    /// Parse the path as a PEP 723 script and run it with `pythonw.exe`, regardless of its file
    /// extension. This option is available only on Windows.
    #[arg(long, conflicts_with_all = ["script", "module"])]
    pub gui_script: bool,

    #[command(flatten)]
    pub installer: ResolverInstallerArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Run the command with all workspace members installed.
    ///
    /// The workspace's environment (`.venv`) is updated to include all workspace members.
    ///
    /// Any extras or groups specified via `--extra`, `--group`, or related options will be applied
    /// to all workspace members.
    #[arg(long, conflicts_with = "package")]
    pub all_packages: bool,

    /// Run the command in a specific package in the workspace.
    ///
    /// If the workspace member does not exist, uv will exit with an error.
    #[arg(long, conflicts_with = "all_packages", value_hint = ValueHint::Other)]
    pub package: Option<PackageName>,

    /// Avoid discovering the project or workspace.
    ///
    /// Do not search the current directory or parent directories for a project. Instead, use an
    /// isolated, temporary environment that contains the `--with` requirements.
    ///
    /// If a virtual environment is active or exists in the current directory or a parent
    /// directory, uv uses it as though no project or workspace exists.
    #[arg(
        long,
        alias = "no_workspace",
        env = EnvVars::UV_NO_PROJECT,
        value_parser = clap::builder::BoolishValueParser::new(),
        conflicts_with = "package"
    )]
    pub no_project: bool,

    /// The Python interpreter to use for the run environment.
    ///
    /// If a discovered environment satisfies the interpreter request, uv uses that environment.
    ///
    /// See `uv help python` to view supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Whether to show resolver and installer output from any environment modifications [env:
    /// UV_SHOW_RESOLUTION=]
    ///
    /// By default, uv hides environment changes. Use `--verbose` to show them.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), hide = true)]
    pub show_resolution: bool,

    /// Number of times that `uv run` will allow recursive invocations.
    ///
    /// uv tracks the current recursion depth in an environment variable. If you clear environment
    /// variables, uv cannot detect the recursion depth.
    ///
    /// If uv reaches the maximum recursion depth, it exits with an error.
    #[arg(long, hide = true, env = EnvVars::UV_RUN_MAX_RECURSION_DEPTH)]
    pub max_recursion_depth: Option<u32>,

    /// The platform for which requirements should be installed.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    ///
    /// For macOS (Darwin), the minimum version defaults to `13.0`. Use
    /// `MACOSX_DEPLOYMENT_TARGET` to set a different minimum, such as `14.0`.
    ///
    /// For iOS, the minimum version defaults to `13.0`. Use `IPHONEOS_DEPLOYMENT_TARGET` to set
    /// a different minimum, such as `14.0`.
    ///
    /// For Android, the minimum API level defaults to `24`. Use `ANDROID_API_LEVEL` to set a
    /// different minimum, such as `26`.
    ///
    /// WARNING: uv selects wheels for the _target_ platform, so installed distributions may not
    /// work on the _current_ platform. uv builds source distributions for the _current_ platform,
    /// so they may not work on the _target_ platform. Use `--python-platform` only for advanced
    /// use cases.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,
}

#[derive(Args)]
pub struct SyncArgs {
    /// Include optional dependencies from the specified extra name.
    ///
    /// May be provided more than once.
    ///
    /// When multiple extras or groups are specified that appear in `tool.uv.conflicts`, uv will
    /// report an error.
    ///
    /// Resolution always includes all optional dependencies. This option only selects which
    /// packages to install.
    #[arg(
        long,
        conflicts_with = "all_extras",
        conflicts_with = "only_group",
        value_delimiter = ',',
        value_parser = extra_name_with_clap_error,
        value_hint = ValueHint::Other,
    )]
    pub extra: Option<Vec<ExtraName>>,

    /// Select the output format.
    #[arg(long, value_enum, default_value_t = SyncFormat::default())]
    pub output_format: SyncFormat,

    /// Include all optional dependencies.
    ///
    /// When two or more extras are declared as conflicting in `tool.uv.conflicts`, using this flag
    /// will always result in an error.
    ///
    /// Resolution always includes all optional dependencies. This option only selects which
    /// packages to install.
    #[arg(long, conflicts_with = "extra", conflicts_with = "only_group")]
    pub all_extras: bool,

    /// Exclude the specified optional dependencies, if `--all-extras` is supplied.
    ///
    /// May be provided multiple times.
    #[arg(long, value_hint = ValueHint::Other)]
    pub no_extra: Vec<ExtraName>,

    #[arg(long, overrides_with("all_extras"), hide = true)]
    pub no_all_extras: bool,

    #[command(flatten)]
    pub dependency_groups: ConflictCheckedDependencyGroupsArgs,

    /// Install any non-editable dependencies, including the project and any workspace members, as
    /// editable.
    #[arg(long, overrides_with = "no_editable", hide = true)]
    pub editable: bool,

    /// Install any editable dependencies, including the project and any workspace members, as
    /// non-editable [env: UV_NO_EDITABLE=]
    #[arg(long, overrides_with = "editable", value_parser = clap::builder::BoolishValueParser::new())]
    pub no_editable: bool,

    /// Install the specified editable packages as non-editable.
    #[arg(long, value_delimiter = ' ', value_hint = ValueHint::Other)]
    pub no_editable_package: Vec<PackageName>,

    /// Do not remove extraneous packages present in the environment.
    ///
    /// Make only the changes needed to satisfy the requirements. By default, syncing removes extra
    /// packages from the environment.
    #[arg(long, overrides_with("exact"), alias = "no-exact")]
    pub inexact: bool,

    /// Perform an exact sync, removing extraneous packages.
    #[arg(long, overrides_with("inexact"), hide = true)]
    pub exact: bool,

    /// Sync dependencies to the active virtual environment.
    ///
    /// If `VIRTUAL_ENV` is set, prefer the active virtual environment instead of creating or
    /// updating the project or script environment.
    #[arg(long, overrides_with = "no_active")]
    pub active: bool,

    /// Prefer project's virtual environment over an active environment.
    ///
    /// This is the default behavior.
    #[arg(long, overrides_with = "active", hide = true)]
    pub no_active: bool,

    /// Do not install the current project [env: UV_NO_INSTALL_PROJECT=]
    ///
    /// By default, uv installs the current project and its dependencies. Use
    /// `--no-install-project` to install only its dependencies. This improves Docker layer caching
    /// when you install the project separately.
    ///
    /// Use `--only-install-project` to install _only_ the project and exclude all dependencies.
    #[arg(long, conflicts_with = "only_install_project")]
    pub no_install_project: bool,

    /// Only install the current project.
    #[arg(long, conflicts_with = "no_install_project", hide = true)]
    pub only_install_project: bool,

    /// Do not install any workspace members, including the root project [env: UV_NO_INSTALL_WORKSPACE=]
    ///
    /// By default, uv installs all workspace members and their dependencies. Use
    /// `--no-install-workspace` to install only their dependencies. This improves Docker layer
    /// caching when you install workspace members separately.
    ///
    /// Use `--only-install-workspace` to install _only_ workspace members and exclude all other
    /// dependencies.
    #[arg(long, conflicts_with = "only_install_workspace")]
    pub no_install_workspace: bool,

    /// Only install workspace members, including the root project.
    #[arg(long, conflicts_with = "no_install_workspace", hide = true)]
    pub only_install_workspace: bool,

    /// Do not install local path dependencies [env: UV_NO_INSTALL_LOCAL=]
    ///
    /// Skips the current project, workspace members, and any other local (path or editable)
    /// packages. Only remote/indexed dependencies are installed. Useful in Docker builds to cache
    /// heavy third-party dependencies first and layer local packages separately.
    ///
    /// Use `--only-install-local` to install _only_ local packages and exclude remote
    /// dependencies.
    #[arg(long, conflicts_with = "only_install_local")]
    pub no_install_local: bool,

    /// Only install local path dependencies
    #[arg(long, conflicts_with = "no_install_local", hide = true)]
    pub only_install_local: bool,

    /// Do not install the given package(s).
    ///
    /// By default, uv installs all project dependencies. Use `--no-install-package` to exclude
    /// specific packages. This can break the environment, so use it with caution.
    ///
    /// Use `--only-install-package` to install _only_ the specified packages and exclude all
    /// others.
    #[arg(long, conflicts_with = "only_install_package", value_hint = ValueHint::Other)]
    pub no_install_package: Vec<PackageName>,

    /// Only install the given package(s).
    #[arg(long, conflicts_with = "no_install_package", hide = true, value_hint = ValueHint::Other)]
    pub only_install_package: Vec<PackageName>,

    /// Assert that the `uv.lock` will remain unchanged [env: UV_LOCKED=]
    ///
    /// Requires that the lockfile is up-to-date. If the lockfile is missing or needs to be updated,
    /// uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Sync without updating the `uv.lock` file [env: UV_FROZEN=]
    ///
    /// Use the versions in the lockfile without checking whether it is up to date. If the lockfile
    /// is missing, uv exits with an error. Dependency changes in `pyproject.toml` that are not in
    /// the lockfile do not appear in the environment.
    #[arg(long, conflicts_with_all = ["locked", "upgrade", "no_sources"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    /// Perform a dry run, without writing the lockfile or modifying the project environment.
    ///
    /// uv resolves project dependencies and reports changes to the lockfile and environment. It
    /// does not modify either.
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub installer: ResolverInstallerArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Sync all packages in the workspace.
    ///
    /// The workspace's environment (`.venv`) is updated to include all workspace members.
    ///
    /// Any extras or groups specified via `--extra`, `--group`, or related options will be applied
    /// to all workspace members.
    #[arg(long, conflicts_with = "package")]
    pub all_packages: bool,

    /// Sync for specific packages in the workspace.
    ///
    /// The workspace's environment (`.venv`) is updated to reflect the subset of dependencies
    /// declared by the specified workspace member packages.
    ///
    /// If any workspace member does not exist, uv will exit with an error.
    #[arg(long, conflicts_with = "all_packages", value_hint = ValueHint::Other)]
    pub package: Vec<PackageName>,

    /// Sync the environment for a Python script, rather than the current project.
    ///
    /// If provided, uv will sync the dependencies based on the script's inline metadata table, in
    /// adherence with PEP 723.
    #[arg(
        long,
        conflicts_with = "all_packages",
        conflicts_with = "package",
        conflicts_with = "no_install_project",
        conflicts_with = "no_install_workspace",
        conflicts_with = "no_install_local",
        conflicts_with = "extra",
        conflicts_with = "all_extras",
        conflicts_with = "no_extra",
        conflicts_with = "no_all_extras",
        conflicts_with = "dev",
        conflicts_with = "no_dev",
        conflicts_with = "only_dev",
        conflicts_with = "group",
        conflicts_with = "no_group",
        conflicts_with = "no_default_groups",
        conflicts_with = "only_group",
        conflicts_with = "all_groups",
        value_hint = ValueHint::FilePath,
    )]
    pub script: Option<PathBuf>,

    /// The Python interpreter to use for the project environment.
    ///
    /// By default, the first interpreter that meets the project's `requires-python` constraint is
    /// used.
    ///
    /// If a Python interpreter in a virtual environment is provided, the packages will not be
    /// synced to the given environment. The interpreter will be used to create a virtual
    /// environment in the project.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// The platform for which requirements should be installed.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    ///
    /// For macOS (Darwin), the minimum version defaults to `13.0`. Use
    /// `MACOSX_DEPLOYMENT_TARGET` to set a different minimum, such as `14.0`.
    ///
    /// For iOS, the minimum version defaults to `13.0`. Use `IPHONEOS_DEPLOYMENT_TARGET` to set
    /// a different minimum, such as `14.0`.
    ///
    /// For Android, the minimum API level defaults to `24`. Use `ANDROID_API_LEVEL` to set a
    /// different minimum, such as `26`.
    ///
    /// WARNING: uv selects wheels for the _target_ platform, so installed distributions may not
    /// work on the _current_ platform. uv builds source distributions for the _current_ platform,
    /// so they may not work on the _target_ platform. Use `--python-platform` only for advanced
    /// use cases.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,

    /// Check if the Python environment is synchronized with the project.
    ///
    /// If the environment is not up to date, uv will exit with an error.
    #[arg(long, overrides_with("no_check"))]
    pub check: bool,

    #[arg(long, overrides_with("check"), hide = true)]
    pub no_check: bool,
}

#[derive(Args)]
pub struct LockArgs {
    /// Check if the lockfile is up-to-date.
    ///
    /// Asserts that the `uv.lock` would remain unchanged after a resolution. If the lockfile is
    /// missing or needs to be updated, uv will exit with an error.
    ///
    /// Equivalent to `--locked`.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), conflicts_with_all = ["check_exists", "upgrade"], overrides_with_all = ["check", "no_locked"])]
    pub check: bool,

    /// Check if the lockfile is up-to-date [env: UV_LOCKED=]
    ///
    /// Asserts that the `uv.lock` would remain unchanged after a resolution. If the lockfile is
    /// missing or needs to be updated, uv will exit with an error.
    ///
    /// Equivalent to `--check`.
    #[arg(long, conflicts_with_all = ["check_exists", "upgrade"], hide = true, overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with_all = ["locked", "check"], hide = true)]
    pub no_locked: bool,

    /// Assert that a `uv.lock` exists without checking if it is up-to-date [env: UV_FROZEN=]
    ///
    /// Equivalent to `--frozen`.
    #[arg(long, conflicts_with_all = ["check", "locked"], overrides_with = "no_frozen")]
    pub check_exists: bool,

    /// Equivalent to `--check-exists`.
    #[arg(long, hide = true, conflicts_with_all = ["check_exists", "check", "locked", "dry_run"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with_all = ["frozen", "check_exists"], hide = true)]
    pub no_frozen: bool,

    /// Perform a dry run, without writing the lockfile.
    ///
    /// In dry-run mode, uv will resolve the project's dependencies and report on the resulting
    /// changes, but will not write the lockfile to disk.
    #[arg(
        long,
        conflicts_with = "check_exists",
        conflicts_with = "check",
        conflicts_with = "locked"
    )]
    pub dry_run: bool,

    /// Lock the specified Python script, rather than the current project.
    ///
    /// If provided, uv will lock the script (based on its inline metadata table, in adherence with
    /// PEP 723) to a `.lock` file adjacent to the script itself.
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub script: Option<PathBuf>,

    #[command(flatten)]
    pub resolver: ResolverArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// The Python interpreter to use during resolution.
    ///
    /// A Python interpreter is required for building source distributions to determine package
    /// metadata when there are not wheels.
    ///
    /// The interpreter is also used as the fallback value for the minimum Python version if
    /// `requires-python` is not set.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,
}

#[derive(Args)]
pub struct UpgradeArgs {
    /// The packages to upgrade.
    #[arg(value_hint = ValueHint::Other)]
    pub packages: Vec<PackageName>,

    /// Exclude the named package from upgrades.
    #[arg(long, value_hint = ValueHint::Other)]
    pub exclude: Vec<PackageName>,
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("sources").required(true).multiple(true))]
pub struct AddArgs {
    /// The packages to add, as PEP 508 requirements (e.g., `ruff==0.5.0`).
    #[arg(group = "sources", value_hint = ValueHint::Other)]
    pub packages: Vec<String>,

    /// Add the packages listed in the given files.
    ///
    /// The following formats are supported: `requirements.txt`, `.py` files with inline metadata,
    /// `pylock.toml`, `pyproject.toml`, `setup.py`, and `setup.cfg`.
    #[arg(
        long,
        short,
        alias = "requirement",
        group = "sources",
        value_parser = parse_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub requirements: Vec<PathBuf>,

    /// Constrain versions using the given requirements files.
    ///
    /// Constraints files use the `requirements.txt` format and control only installed package
    /// _versions_. uv does _not_ add constraints to `pyproject.toml`, but _does_ apply them during
    /// dependency resolution.
    ///
    /// This is equivalent to pip's `--constraint` option.
    #[arg(
        long,
        short,
        alias = "constraint",
        env = EnvVars::UV_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub constraints: Vec<Maybe<PathBuf>>,

    /// Apply this marker to all added packages.
    #[arg(long, short, value_parser = MarkerTree::from_str, value_hint = ValueHint::Other)]
    pub marker: Option<MarkerTree>,

    /// Add the requirements to the development dependency group [env: UV_DEV=]
    ///
    /// This option is an alias for `--group dev`.
    #[arg(
        long,
        conflicts_with("optional"),
        conflicts_with("group"),
        conflicts_with("script"),
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub dev: bool,

    /// Add the requirements to the package's optional dependencies for the specified extra.
    ///
    /// Use `--extra` when installing the project to activate this group.
    ///
    /// To enable an optional extra for this requirement instead, see `--extra`.
    #[arg(long, conflicts_with("dev"), conflicts_with("group"), value_hint = ValueHint::Other)]
    pub optional: Option<ExtraName>,

    /// Add the requirements to the specified dependency group.
    ///
    /// These requirements do not appear in the project's published metadata.
    #[arg(
        long,
        conflicts_with("dev"),
        conflicts_with("optional"),
        conflicts_with("script"),
        value_hint = ValueHint::Other,
    )]
    pub group: Option<GroupName>,

    /// Add the requirements as editable.
    #[arg(long, overrides_with = "no_editable")]
    pub editable: bool,

    /// Don't add the requirements as editable [env: UV_NO_EDITABLE=]
    #[arg(long, overrides_with = "editable", hide = true, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_editable: bool,

    /// Don't add the specified requirements as editable.
    #[arg(long, value_delimiter = ' ', value_hint = ValueHint::Other, hide = true)]
    pub no_editable_package: Vec<PackageName>,

    /// Add a dependency as provided.
    ///
    /// By default, uv records Git, local, editable, and direct URL sources in `tool.uv.sources`.
    /// With `--raw`, uv adds source requirements to `project.dependencies` instead.
    ///
    /// By default, uv also adds a version bound, such as `foo>=1.0.0`. With `--raw`, it adds the
    /// dependency without a bound.
    #[arg(
        long,
        conflicts_with = "editable",
        conflicts_with = "no_editable",
        conflicts_with = "rev",
        conflicts_with = "tag",
        conflicts_with = "branch",
        alias = "raw-sources"
    )]
    pub raw: bool,

    /// The kind of version specifier to use when adding dependencies.
    ///
    /// If a dependency has no constraint or URL, uv adds a constraint based on the latest
    /// compatible version. By default, it uses a lower bound, such as `>=1.2.3`.
    ///
    /// With `--frozen`, uv skips resolution and always adds dependencies without constraints.
    ///
    /// This option is in preview and may change in any future release.
    #[arg(long, value_enum)]
    pub bounds: Option<AddBoundsKind>,

    /// Commit to use when adding a dependency from Git.
    #[arg(long, group = "git-ref", action = clap::ArgAction::Set, value_hint = ValueHint::Other)]
    pub rev: Option<String>,

    /// Tag to use when adding a dependency from Git.
    #[arg(long, group = "git-ref", action = clap::ArgAction::Set, value_hint = ValueHint::Other)]
    pub tag: Option<String>,

    /// Branch to use when adding a dependency from Git.
    #[arg(long, group = "git-ref", action = clap::ArgAction::Set, value_hint = ValueHint::Other)]
    pub branch: Option<String>,

    /// Whether to use Git LFS when adding a dependency from Git.
    #[arg(long)]
    pub lfs: bool,

    /// Extras to enable for the dependency.
    ///
    /// May be provided more than once.
    ///
    /// To add this dependency to an optional extra instead, see `--optional`.
    #[arg(long, value_hint = ValueHint::Other)]
    pub extra: Option<Vec<ExtraName>>,

    /// Avoid syncing the virtual environment [env: UV_NO_SYNC=]
    #[arg(long)]
    pub no_sync: bool,

    /// Assert that the `uv.lock` will remain unchanged [env: UV_LOCKED=]
    ///
    /// Requires that the lockfile is up-to-date. If the lockfile is missing or needs to be updated,
    /// uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Add dependencies without re-locking the project [env: UV_FROZEN=]
    ///
    /// The project environment will not be synced.
    #[arg(long, conflicts_with_all = ["locked", "upgrade", "no_sources"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    /// Prefer the active virtual environment over the project's virtual environment.
    ///
    /// If the project virtual environment is active or no virtual environment is active, this has
    /// no effect.
    #[arg(long, overrides_with = "no_active")]
    pub active: bool,

    /// Prefer project's virtual environment over an active environment.
    ///
    /// This is the default behavior.
    #[arg(long, overrides_with = "active", hide = true)]
    pub no_active: bool,

    #[command(flatten)]
    pub installer: ResolverInstallerArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Add the dependency to a specific package in the workspace.
    #[arg(long, conflicts_with = "isolated", value_hint = ValueHint::Other)]
    pub package: Option<PackageName>,

    /// Add the dependency to the specified Python script, rather than to a project.
    ///
    /// Add the dependency to the script's PEP 723 inline metadata table. If the script has no
    /// table, uv creates one. `uv run` creates a temporary environment for the script and installs
    /// all inline dependencies.
    #[arg(
        long,
        conflicts_with = "dev",
        conflicts_with = "optional",
        conflicts_with = "package",
        conflicts_with = "workspace",
        value_hint = ValueHint::FilePath,
    )]
    pub script: Option<PathBuf>,

    /// The Python interpreter to use for resolving and syncing.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Add the dependency as a workspace member.
    ///
    /// By default, uv will add path dependencies that are within the workspace directory
    /// as workspace members. When used with a path dependency, the package will be added
    /// to the workspace's `members` list in the root `pyproject.toml` file.
    #[arg(long, overrides_with = "no_workspace")]
    pub workspace: bool,

    /// Don't add the dependency as a workspace member.
    ///
    /// By default, when adding a dependency that's a local path and is within the workspace
    /// directory, uv will add it as a workspace member; pass `--no-workspace` to add the package
    /// as direct path dependency instead.
    #[arg(long, overrides_with = "workspace")]
    pub no_workspace: bool,

    /// Do not install the current project [env: UV_NO_INSTALL_PROJECT=]
    ///
    /// By default, the current project is installed into the environment with all of its
    /// dependencies. The `--no-install-project` option allows the project to be excluded, but all of
    /// its dependencies are still installed. This is particularly useful in situations like building
    /// Docker images where installing the project separately from its dependencies allows optimal
    /// layer caching.
    ///
    /// Use `--only-install-project` to install _only_ the project and exclude all dependencies.
    #[arg(
        long,
        conflicts_with = "frozen",
        conflicts_with = "no_sync",
        conflicts_with = "only_install_project"
    )]
    pub no_install_project: bool,

    /// Only install the current project.
    #[arg(
        long,
        conflicts_with = "frozen",
        conflicts_with = "no_sync",
        conflicts_with = "no_install_project",
        hide = true
    )]
    pub only_install_project: bool,

    /// Do not install any workspace members, including the current project [env: UV_NO_INSTALL_WORKSPACE=]
    ///
    /// By default, uv installs all workspace members and their dependencies. Use
    /// `--no-install-workspace` to install only their dependencies. This improves Docker layer
    /// caching when you install workspace members separately.
    ///
    /// Use `--only-install-workspace` to install _only_ workspace members and exclude all other
    /// dependencies.
    #[arg(
        long,
        conflicts_with = "frozen",
        conflicts_with = "no_sync",
        conflicts_with = "only_install_workspace"
    )]
    pub no_install_workspace: bool,

    /// Only install workspace members, including the current project.
    #[arg(
        long,
        conflicts_with = "frozen",
        conflicts_with = "no_sync",
        conflicts_with = "no_install_workspace",
        hide = true
    )]
    pub only_install_workspace: bool,

    /// Do not install local path dependencies [env: UV_NO_INSTALL_LOCAL=]
    ///
    /// Skips the current project, workspace members, and any other local (path or editable)
    /// packages. Only remote/indexed dependencies are installed. Useful in Docker builds to cache
    /// heavy third-party dependencies first and layer local packages separately.
    ///
    /// Use `--only-install-local` to install _only_ local packages and exclude remote
    /// dependencies.
    #[arg(
        long,
        conflicts_with = "frozen",
        conflicts_with = "no_sync",
        conflicts_with = "only_install_local"
    )]
    pub no_install_local: bool,

    /// Only install local path dependencies
    #[arg(
        long,
        conflicts_with = "frozen",
        conflicts_with = "no_sync",
        conflicts_with = "no_install_local",
        hide = true
    )]
    pub only_install_local: bool,

    /// Do not install the given package(s).
    ///
    /// By default, all project's dependencies are installed into the environment. The
    /// `--no-install-package` option allows exclusion of specific packages. Note this can result
    /// in a broken environment, and should be used with caution.
    ///
    /// Use `--only-install-package` to install _only_ the specified packages and exclude all
    /// others.
    #[arg(
        long,
        conflicts_with = "frozen",
        conflicts_with = "no_sync",
        conflicts_with = "only_install_package",
        value_hint = ValueHint::Other,
    )]
    pub no_install_package: Vec<PackageName>,

    /// Only install the given package(s).
    #[arg(
        long,
        conflicts_with = "frozen",
        conflicts_with = "no_sync",
        conflicts_with = "no_install_package",
        hide = true,
        value_hint = ValueHint::Other,
    )]
    pub only_install_package: Vec<PackageName>,
}

#[derive(Args)]
pub struct RemoveArgs {
    /// The names of the dependencies to remove (e.g., `ruff`).
    #[arg(required = true, value_hint = ValueHint::Other)]
    pub packages: Vec<Requirement<VerbatimParsedUrl>>,

    /// Remove the packages from the development dependency group [env: UV_DEV=]
    ///
    /// This option is an alias for `--group dev`.
    #[arg(
        long,
        conflicts_with("optional"),
        conflicts_with("group"),
        conflicts_with("script"),
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub dev: bool,

    /// Remove the packages from the project's optional dependencies for the specified extra.
    #[arg(
        long,
        conflicts_with("dev"),
        conflicts_with("group"),
        conflicts_with("script"),
        value_hint = ValueHint::Other,
    )]
    pub optional: Option<ExtraName>,

    /// Remove the packages from the specified dependency group.
    #[arg(
        long,
        conflicts_with("dev"),
        conflicts_with("optional"),
        conflicts_with("script"),
        value_hint = ValueHint::Other,
    )]
    pub group: Option<GroupName>,

    /// Avoid syncing the virtual environment after re-locking the project [env: UV_NO_SYNC=]
    #[arg(long)]
    pub no_sync: bool,

    /// Prefer the active virtual environment over the project's virtual environment.
    ///
    /// If the project virtual environment is active or no virtual environment is active, this has
    /// no effect.
    #[arg(long, overrides_with = "no_active")]
    pub active: bool,

    /// Prefer project's virtual environment over an active environment.
    ///
    /// This is the default behavior.
    #[arg(long, overrides_with = "active", hide = true)]
    pub no_active: bool,

    /// Assert that the `uv.lock` will remain unchanged [env: UV_LOCKED=]
    ///
    /// Requires that the lockfile is up-to-date. If the lockfile is missing or needs to be updated,
    /// uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Remove dependencies without re-locking the project [env: UV_FROZEN=]
    ///
    /// The project environment will not be synced.
    #[arg(long, conflicts_with_all = ["locked", "upgrade", "no_sources"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    #[command(flatten)]
    pub installer: ResolverInstallerArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Remove the dependencies from a specific package in the workspace.
    #[arg(long, conflicts_with = "isolated", value_hint = ValueHint::Other)]
    pub package: Option<PackageName>,

    /// Remove the dependency from the specified Python script, rather than from a project.
    ///
    /// If provided, uv will remove the dependency from the script's inline metadata table, in
    /// adherence with PEP 723.
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub script: Option<PathBuf>,

    /// The Python interpreter to use for resolving and syncing.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,
}

#[derive(Args)]
pub struct TreeArgs {
    /// Show a platform-independent dependency tree.
    ///
    /// Shows resolved package versions for all Python versions and platforms, rather than filtering
    /// to those that are relevant for the current environment.
    ///
    /// Multiple versions may be shown for a each package.
    #[arg(long)]
    pub universal: bool,

    /// The format in which to display the dependency graph.
    #[arg(long, value_enum, default_value_t = TreeFormat::default())]
    pub format: TreeFormat,

    #[command(flatten)]
    pub tree: DisplayTreeArgs,

    #[command(flatten)]
    pub dependency_groups: ProjectDependencyGroupsArgs,

    /// Assert that the `uv.lock` will remain unchanged [env: UV_LOCKED=]
    ///
    /// Requires that the lockfile is up-to-date. If the lockfile is missing or needs to be updated,
    /// uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Display the requirements without locking the project [env: UV_FROZEN=]
    ///
    /// If the lockfile is missing, uv will exit with an error.
    #[arg(long, conflicts_with_all = ["locked", "upgrade", "no_sources"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub resolver: ResolverArgs,

    /// Show the dependency tree the specified PEP 723 Python script, rather than the current
    /// project.
    ///
    /// If provided, uv will resolve the dependencies based on its inline metadata table, in
    /// adherence with PEP 723.
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub script: Option<PathBuf>,

    /// The Python version to use when filtering the tree.
    ///
    /// For example, pass `--python-version 3.10` to display the dependencies that would be included
    /// when installing on Python 3.10.
    ///
    /// Defaults to the version of the discovered Python interpreter.
    #[arg(long, conflicts_with = "universal")]
    pub python_version: Option<PythonVersion>,

    /// The platform to use when filtering the tree.
    ///
    /// For example, pass `--platform windows` to display the dependencies that would be included
    /// when installing on Windows.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    #[arg(long, conflicts_with = "universal")]
    pub python_platform: Option<TargetTriple>,

    /// The Python interpreter to use for locking and filtering.
    ///
    /// By default, the tree is filtered to match the platform as reported by the Python
    /// interpreter. Use `--universal` to display the tree for all platforms, or use
    /// `--python-version` or `--python-platform` to override a subset of markers.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,
}

#[derive(Args)]
pub struct ExportArgs {
    /// The format to which `uv.lock` should be exported.
    ///
    /// Supports `requirements.txt`, `pylock.toml` (PEP 751) and CycloneDX v1.5 JSON output formats.
    ///
    /// uv will infer the output format from the file extension of the output file, if
    /// provided. Otherwise, defaults to `requirements.txt`.
    #[arg(long, value_enum)]
    pub format: Option<ExportFormat>,

    /// Export the entire workspace.
    ///
    /// The dependencies for all workspace members will be included in the exported requirements
    /// file.
    ///
    /// Any extras or groups specified via `--extra`, `--group`, or related options will be applied
    /// to all workspace members.
    #[arg(long, conflicts_with = "package")]
    pub all_packages: bool,

    /// Export the dependencies for specific packages in the workspace.
    ///
    /// If any workspace member does not exist, uv will exit with an error.
    #[arg(long, conflicts_with = "all_packages", value_hint = ValueHint::Other)]
    pub package: Vec<PackageName>,

    /// Prune the given package from the dependency tree.
    ///
    /// uv excludes the pruned package and any dependencies that are no longer required from the
    /// exported requirements file.
    #[arg(long, conflicts_with = "all_packages", value_name = "PACKAGE")]
    pub prune: Vec<PackageName>,

    /// Include optional dependencies from the specified extra name.
    ///
    /// May be provided more than once.
    #[arg(long, value_delimiter = ',', conflicts_with = "all_extras", conflicts_with = "only_group", value_parser = extra_name_with_clap_error)]
    pub extra: Option<Vec<ExtraName>>,

    /// Include all optional dependencies.
    #[arg(long, conflicts_with = "extra", conflicts_with = "only_group")]
    pub all_extras: bool,

    /// Exclude the specified optional dependencies, if `--all-extras` is supplied.
    ///
    /// May be provided multiple times.
    #[arg(long)]
    pub no_extra: Vec<ExtraName>,

    #[arg(long, overrides_with("all_extras"), hide = true)]
    pub no_all_extras: bool,

    #[command(flatten)]
    pub dependency_groups: ProjectDependencyGroupsArgs,

    /// Exclude comment annotations indicating the source of each package.
    #[arg(long, overrides_with("annotate"))]
    pub no_annotate: bool,

    #[arg(long, overrides_with("no_annotate"), hide = true)]
    pub annotate: bool,

    /// Exclude the comment header at the top of the generated output file.
    #[arg(long, overrides_with("header"))]
    pub no_header: bool,

    #[arg(long, overrides_with("no_header"), hide = true)]
    pub header: bool,

    /// Include `--index-url` and `--extra-index-url` entries in the generated output file.
    #[arg(long, overrides_with("no_emit_index_url"))]
    pub emit_index_url: bool,

    #[arg(long, overrides_with("emit_index_url"), hide = true)]
    pub no_emit_index_url: bool,

    /// Include `--find-links` entries in the generated output file.
    #[arg(long, overrides_with("no_emit_find_links"))]
    pub emit_find_links: bool,

    #[arg(long, overrides_with("emit_find_links"), hide = true)]
    pub no_emit_find_links: bool,

    /// Export any non-editable dependencies, including the project and any workspace members, as
    /// editable.
    #[arg(long, overrides_with = "no_editable", hide = true)]
    pub editable: bool,

    /// Export any editable dependencies, including the project and any workspace members, as
    /// non-editable [env: UV_NO_EDITABLE=]
    #[arg(long, overrides_with = "editable", value_parser = clap::builder::BoolishValueParser::new())]
    pub no_editable: bool,

    /// Export the specified editable packages as non-editable.
    #[arg(long, value_delimiter = ' ', value_hint = ValueHint::Other)]
    pub no_editable_package: Vec<PackageName>,

    /// Include hashes for all dependencies.
    #[arg(long, overrides_with("no_hashes"), hide = true)]
    pub hashes: bool,

    /// Omit hashes in the generated output.
    #[arg(long, overrides_with("hashes"))]
    pub no_hashes: bool,

    /// Write the exported requirements to the given file.
    #[arg(long, short, value_hint = ValueHint::FilePath)]
    pub output_file: Option<PathBuf>,

    /// Do not emit the current project.
    ///
    /// By default, uv exports the current project and its dependencies. Use `--no-emit-project`
    /// to exclude the project and keep its dependencies.
    ///
    /// Use `--only-emit-project` to export _only_ the project and exclude all dependencies.
    #[arg(
        long,
        alias = "no-install-project",
        conflicts_with = "only_emit_project"
    )]
    pub no_emit_project: bool,

    /// Only emit the current project.
    #[arg(
        long,
        alias = "only-install-project",
        conflicts_with = "no_emit_project",
        hide = true
    )]
    pub only_emit_project: bool,

    /// Do not emit any workspace members, including the root project.
    ///
    /// By default, uv exports all workspace members and their dependencies. Use
    /// `--no-emit-workspace` to exclude workspace members and keep their dependencies.
    ///
    /// Use `--only-emit-workspace` to export _only_ workspace members and exclude all other
    /// dependencies.
    #[arg(
        long,
        alias = "no-install-workspace",
        conflicts_with = "only_emit_workspace"
    )]
    pub no_emit_workspace: bool,

    /// Only emit workspace members, including the root project.
    #[arg(
        long,
        alias = "only-install-workspace",
        conflicts_with = "no_emit_workspace",
        hide = true
    )]
    pub only_emit_workspace: bool,

    /// Do not include local path dependencies in the exported requirements.
    ///
    /// Exclude the current project, workspace members, and other local path or editable packages.
    /// Export only remote or indexed dependencies. This helps Docker and CI workflows cache
    /// third-party dependencies separately.
    ///
    /// Use `--only-emit-local` to export _only_ local packages and exclude remote dependencies.
    #[arg(long, alias = "no-install-local", conflicts_with = "only_emit_local")]
    pub no_emit_local: bool,

    /// Only include local path dependencies in the exported requirements.
    #[arg(
        long,
        alias = "only-install-local",
        conflicts_with = "no_emit_local",
        hide = true
    )]
    pub only_emit_local: bool,

    /// Do not emit the given package(s).
    ///
    /// By default, uv exports all project dependencies. Use `--no-emit-package` to exclude
    /// specific packages.
    ///
    /// Use `--only-emit-package` to export _only_ the specified packages and exclude all others.
    #[arg(
        long,
        alias = "no-install-package",
        conflicts_with = "only_emit_package",
        value_delimiter = ',',
        value_hint = ValueHint::Other,
    )]
    pub no_emit_package: Vec<PackageName>,

    /// Only emit the given package(s).
    #[arg(
        long,
        alias = "only-install-package",
        conflicts_with = "no_emit_package",
        hide = true,
        value_delimiter = ',',
        value_hint = ValueHint::Other,
    )]
    pub only_emit_package: Vec<PackageName>,

    /// Assert that the `uv.lock` will remain unchanged [env: UV_LOCKED=]
    ///
    /// Requires that the lockfile is up-to-date. If the lockfile is missing or needs to be updated,
    /// uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Do not update the `uv.lock` before exporting [env: UV_FROZEN=]
    ///
    /// If a `uv.lock` does not exist, uv will exit with an error.
    #[arg(long, conflicts_with_all = ["locked", "upgrade", "no_sources"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    #[command(flatten)]
    pub resolver: ResolverArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Export the dependencies for the specified PEP 723 Python script, rather than the current
    /// project.
    ///
    /// If provided, uv will resolve the dependencies based on its inline metadata table, in
    /// adherence with PEP 723.
    #[arg(
        long,
        conflicts_with_all = ["all_packages", "package", "no_emit_project", "no_emit_workspace"],
        value_hint = ValueHint::FilePath,
    )]
    pub script: Option<PathBuf>,

    /// The Python interpreter to use during resolution.
    ///
    /// A Python interpreter is required for building source distributions to determine package
    /// metadata when there are not wheels.
    ///
    /// The interpreter is also used as the fallback value for the minimum Python version if
    /// `requires-python` is not set.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,
}

#[derive(Args)]
pub struct FormatArgs {
    /// Check if files are formatted without applying changes.
    #[arg(long)]
    pub check: bool,

    /// Show a diff of formatting changes without applying them.
    ///
    /// Implies `--check`.
    #[arg(long)]
    pub diff: bool,

    /// The version of Ruff to use for formatting.
    ///
    /// Accepts an exact version, such as `0.8.2`; a version specifier, such as `>=0.8.0`; or
    /// `latest` for the latest available version.
    ///
    /// By default, uv uses a constrained Ruff version range, such as `>=0.15,<0.16`.
    #[arg(long, value_hint = ValueHint::Other)]
    pub version: Option<String>,

    /// Limit candidate Ruff versions to those released prior to the given date.
    ///
    /// Accepts a superset of [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339.html) (e.g.,
    /// `2006-12-02T02:07:43Z`) or local date in the same format (e.g. `2006-12-02`), as well as
    /// durations relative to "now" (e.g., `-1 week`).
    ///
    /// Use `false` to disable `exclude-newer`.
    #[arg(long, env = EnvVars::UV_EXCLUDE_NEWER, value_hint = ValueHint::Other)]
    pub exclude_newer: Option<ExcludeNewerOverride>,

    /// Additional arguments to pass to Ruff.
    ///
    /// For example, use `uv format -- --line-length 100` to set the line length or
    /// `uv format -- src/module/foo.py` to format a specific file.
    #[arg(last = true, value_hint = ValueHint::Other)]
    pub extra_args: Vec<String>,

    /// Avoid discovering a project or workspace.
    ///
    /// Run the formatter in the current directory instead of the current project. Use this option
    /// when the current directory is not a project.
    #[arg(
        long,
        env = EnvVars::UV_NO_PROJECT,
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub no_project: bool,

    /// Display the version of Ruff that will be used for formatting.
    ///
    /// This is useful for verifying which version was resolved when using version constraints
    /// (e.g., `--version ">=0.8.0"`) or `--version latest`.
    #[arg(long, hide = true)]
    pub show_version: bool,
}

#[derive(Args)]
pub struct CheckArgs {
    /// Apply safe fixes to resolve type-checking errors.
    #[arg(long)]
    pub fix: bool,

    /// Check all packages in the workspace.
    ///
    /// The workspace's environment is synchronized to include all workspace members, and files in
    /// every member are checked.
    #[arg(long, conflicts_with_all = ["package", "script", "no_project"])]
    pub all_packages: bool,

    /// Check specific packages in the workspace.
    ///
    /// The workspace's environment is synchronized to include the selected members and their
    /// dependencies. Only files owned by the selected members are checked.
    #[arg(
        long,
        conflicts_with_all = ["all_packages", "script", "no_project"],
        value_hint = ValueHint::Other
    )]
    pub package: Vec<PackageName>,

    /// Run checks for the specified PEP 723 Python script, rather than the current project.
    ///
    /// uv uses dependencies from the script's PEP 723 inline metadata table.
    #[arg(
        long,
        conflicts_with = "extra",
        conflicts_with = "all_extras",
        conflicts_with = "no_extra",
        conflicts_with = "no_all_extras",
        conflicts_with = "dev",
        conflicts_with = "no_dev",
        conflicts_with = "only_dev",
        conflicts_with = "group",
        conflicts_with = "no_group",
        conflicts_with = "no_default_groups",
        conflicts_with = "only_group",
        conflicts_with = "all_groups",
        conflicts_with = "no_project",
        conflicts_with = "all_packages",
        conflicts_with = "package",
        value_hint = ValueHint::FilePath,
    )]
    pub script: Option<PathBuf>,

    /// Include optional dependencies from the specified extra name.
    ///
    /// May be provided more than once.
    ///
    /// When multiple extras or groups are specified that appear in `tool.uv.conflicts`, uv will
    /// report an error.
    ///
    /// Resolution always includes all optional dependencies. This option only selects which
    /// packages to install.
    #[arg(
        long,
        conflicts_with = "all_extras",
        conflicts_with = "only_group",
        value_delimiter = ',',
        value_parser = extra_name_with_clap_error,
        value_hint = ValueHint::Other,
    )]
    pub extra: Option<Vec<ExtraName>>,

    /// Include all optional dependencies.
    ///
    /// When two or more extras are declared as conflicting in `tool.uv.conflicts`, using this flag
    /// will always result in an error.
    ///
    /// Resolution always includes all optional dependencies. This option only selects which
    /// packages to install.
    #[arg(long, conflicts_with = "extra", conflicts_with = "only_group")]
    pub all_extras: bool,

    /// Exclude the specified optional dependencies, if `--all-extras` is supplied.
    ///
    /// May be provided multiple times.
    #[arg(long, value_hint = ValueHint::Other)]
    pub no_extra: Vec<ExtraName>,

    #[arg(long, overrides_with("all_extras"), hide = true)]
    pub no_all_extras: bool,

    #[command(flatten)]
    pub dependency_groups: ConflictCheckedDependencyGroupsArgs,

    /// Assert that the `uv.lock` will remain unchanged [env: UV_LOCKED=]
    ///
    /// Requires that the lockfile is up-to-date. If the lockfile is missing or needs to be updated,
    /// uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Sync without updating the `uv.lock` file [env: UV_FROZEN=]
    ///
    /// Instead of checking if the lockfile is up-to-date, uses the versions in the lockfile as the
    /// source of truth. If the lockfile is missing, uv will exit with an error. If the
    /// `pyproject.toml` includes changes to dependencies that have not been included in the
    /// lockfile yet, they will not be present in the environment.
    #[arg(long, conflicts_with_all = ["locked", "upgrade", "no_sources"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    /// Avoid syncing the virtual environment [env: UV_NO_SYNC=]
    #[arg(long)]
    pub no_sync: bool,

    /// Do not install the current project [env: UV_NO_INSTALL_PROJECT=]
    ///
    /// By default, the current project is installed into the environment with all of its
    /// dependencies. The `--no-install-project` option excludes the project itself while still
    /// installing its dependencies, which is useful when the project can be type-checked from its
    /// source tree without building native extensions.
    #[arg(long, conflicts_with_all = ["no_sync", "script", "no_project"])]
    pub no_install_project: bool,

    /// Run checks without mutating project state [env: UV_ISOLATED=]
    ///
    /// Uses a temporary virtual environment and leaves existing environments and the project
    /// lockfile unchanged. Declared project requirements are resolved and installed into the
    /// temporary environment.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
    pub isolated: bool,

    /// The Python interpreter to use for the project environment.
    ///
    /// By default, the first interpreter that meets the project's
    /// `requires-python` constraint is used.
    ///
    /// See `uv python` for more details on Python discovery and requests.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// The version of ty to use for type checking.
    ///
    /// Accepts an exact version, such as `0.0.1`; a version specifier, such as `>=0.0.1`; or
    /// `latest` for the latest available version.
    ///
    /// If `ty` is a project dependency or is in the project's `dev` group, uv uses its exact
    /// version from `uv.lock`. Otherwise, uv uses a constrained version range, such as
    /// `>=0.0,<0.1`.
    #[arg(long, value_hint = ValueHint::Other)]
    pub ty_version: Option<String>,

    /// Display the version of ty that will be used for type checking.
    #[arg(long, hide = true)]
    pub show_version: bool,

    /// Display the ty command that will be used for type checking.
    #[arg(long, hide = true)]
    pub show_command: bool,

    /// Avoid discovering a project or workspace.
    ///
    /// Run checks in the current directory instead of the current project. Use this option when
    /// the current directory is not a project.
    #[arg(
        long,
        env = EnvVars::UV_NO_PROJECT,
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub no_project: bool,

    #[command(flatten)]
    pub installer: ResolverInstallerArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,
}

#[derive(Args)]
#[group(skip)]
pub struct AuditCommonArgs {
    /// Select the output format.
    #[arg(long, value_enum, default_value_t = AuditOutputFormat::default())]
    pub output_format: AuditOutputFormat,

    /// Ignore a vulnerability by ID.
    ///
    /// Exclude vulnerabilities that match any specified ID or alias from the audit results.
    ///
    /// May be provided multiple times.
    #[arg(long)]
    pub ignore: Vec<String>,

    /// Ignore a vulnerability by ID, but only while no fix is available.
    ///
    /// Exclude vulnerabilities that match any specified ID or alias while they have no known
    /// fixed version. Report the vulnerability again when a fixed version becomes available.
    ///
    /// May be provided multiple times.
    #[arg(long)]
    pub ignore_until_fixed: Vec<String>,

    /// The service format to use for vulnerability lookups.
    ///
    /// Each service format has a default URL. Use `--service-url` to change it. The defaults are:
    ///
    /// * OSV: <https://api.osv.dev/>
    #[arg(long, value_enum, default_value = "osv")]
    pub service_format: VulnerabilityServiceFormat,

    /// The URL to vulnerability service API endpoint.
    ///
    /// If you do not specify a URL, uv uses the default for the selected service.
    ///
    /// The service must use the OSV protocol unless `--service-format` selects a different format.
    #[arg(long, value_hint = ValueHint::Url)]
    pub service_url: Option<DisplaySafeUrl>,
}

#[derive(Args)]
pub struct AuditArgs {
    /// Don't audit the specified optional dependencies.
    ///
    /// May be provided multiple times.
    #[arg(long, value_hint = ValueHint::Other)]
    pub no_extra: Vec<ExtraName>,

    /// Don't audit the development dependency group [env: UV_NO_DEV=]
    ///
    /// This option is an alias of `--no-group dev`.
    /// See `--no-default-groups` to exclude all default groups instead.
    ///
    /// This option is only available when running in a project.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_dev: bool,

    /// Don't audit the specified dependency group [env: `UV_NO_GROUP`=]
    ///
    /// May be provided multiple times.
    #[arg(long, value_delimiter = ' ', value_hint = ValueHint::Other)]
    pub no_group: Vec<GroupName>,

    /// Don't audit the default dependency groups.
    #[arg(long, env = EnvVars::UV_NO_DEFAULT_GROUPS, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_default_groups: bool,

    /// Only audit dependencies from the specified dependency group.
    ///
    /// The project and its dependencies will be omitted.
    ///
    /// May be provided multiple times. Implies `--no-default-groups`.
    #[arg(long, value_hint = ValueHint::Other)]
    pub only_group: Vec<GroupName>,

    /// Only audit the development dependency group.
    ///
    /// The project and its dependencies will be omitted.
    ///
    /// This option is an alias for `--only-group dev`. Implies `--no-default-groups`.
    #[arg(long, conflicts_with_all = ["no_dev"])]
    pub only_dev: bool,

    /// Assert that the `uv.lock` will remain unchanged [env: UV_LOCKED=]
    ///
    /// Requires that the lockfile is up-to-date. If the lockfile is missing or needs to be updated,
    /// uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Audit the requirements without locking the project [env: UV_FROZEN=]
    ///
    /// If the lockfile is missing, uv will exit with an error.
    #[arg(long, conflicts_with_all = ["locked", "upgrade", "no_sources"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    #[command(flatten)]
    pub audit: AuditCommonArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub resolver: ResolverArgs,

    /// Audit the specified PEP 723 Python script, rather than the current
    /// project.
    ///
    /// The specified script must be locked, i.e. with `uv lock --script <script>`
    /// before it can be audited.
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub script: Option<PathBuf>,

    /// The Python version to use when auditing.
    ///
    /// For example, pass `--python-version 3.10` to audit the dependencies that would be included
    /// when installing on Python 3.10.
    ///
    /// Defaults to the version of the discovered Python interpreter.
    #[arg(long)]
    pub python_version: Option<PythonVersion>,

    /// The platform to use when auditing.
    ///
    /// For example, pass `--platform windows` to audit the dependencies that would be included
    /// when installing on Windows.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,
}

#[derive(Args)]
pub struct AuthNamespace {
    #[command(subcommand)]
    pub command: AuthCommand,
}

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Login to a service
    Login(AuthLoginArgs),
    /// Logout of a service
    Logout(AuthLogoutArgs),
    /// Show the authentication token for a service
    Token(AuthTokenArgs),
    /// Show the path to the uv credentials directory.
    ///
    /// By default, uv stores credentials in its data directory at
    /// `$XDG_DATA_HOME/uv/credentials` or `$HOME/.local/share/uv/credentials` on Unix and
    /// `%APPDATA%\uv\data\credentials` on Windows.
    ///
    /// Use `$UV_CREDENTIALS_DIR` to change the credentials directory.
    ///
    /// The plaintext backend stores credentials in this directory. The native backend stores them
    /// in the system keyring instead.
    Dir,
    /// Act as a credential helper for external tools.
    ///
    /// Use the Bazel credential helper protocol to provide credentials to external tools as JSON
    /// over stdin and stdout.
    ///
    /// External tools typically invoke this command.
    #[command(hide = true)]
    Helper(AuthHelperArgs),
}

#[derive(Args)]
pub struct ToolNamespace {
    #[command(subcommand)]
    pub command: ToolCommand,
}

#[derive(Subcommand)]
pub enum ToolCommand {
    /// Run a command provided by a Python package.
    ///
    /// By default, uv installs the package that matches the command name.
    ///
    /// Include an exact version with `<package>@<version>`, such as `uv tool run ruff@0.3.0`. Use
    /// `--from` for a more complex version requirement or a command from a different package.
    ///
    /// Run Python with `uvx python` or `uvx python@<version>`. uv starts the interpreter in an
    /// isolated virtual environment.
    ///
    /// If `uv tool install` already installed the tool, uv uses that version unless you request a
    /// version or use `--isolated`.
    ///
    /// `uvx` is an alias for `uv tool run` and behaves the same way.
    ///
    /// If you omit the command, uv lists the installed tools.
    ///
    /// uv installs packages into a temporary virtual environment in its cache directory.
    #[command(
        after_help = "Use `uvx` as a shortcut for `uv tool run`.\n\n\
        Use `uv help tool run` for more details.",
        after_long_help = ""
    )]
    Run(ToolRunArgs),
    /// Hidden alias for `uv tool run` for the `uvx` command
    #[command(
        hide = true,
        override_usage = "uvx [OPTIONS] [COMMAND]",
        about = "Run a command provided by a Python package.",
        after_help = "Use `uv help tool run` for more details.",
        after_long_help = "",
        display_name = "uvx",
        long_version = crate::version::uv_self_version()
    )]
    Uvx(UvxArgs),
    /// Install commands provided by a Python package.
    ///
    /// uv installs packages into an isolated virtual environment in its tools directory. It links
    /// executables into the tool executable directory, which follows the XDG standard. Use
    /// `uv tool dir --bin` to display that directory.
    ///
    /// If the tool is already installed, uv usually replaces it.
    Install(ToolInstallArgs),
    /// Upgrade installed tools.
    ///
    /// Upgrades respect the tool's original version constraints. To upgrade beyond those
    /// constraints, run `uv tool install` again.
    ///
    /// Upgrades also respect the tool's original installation settings. For example, if you used
    /// `--prereleases allow` during installation, upgrades keep that setting.
    #[command(alias = "update")]
    Upgrade(ToolUpgradeArgs),
    /// List installed tools.
    #[command(alias = "ls")]
    List(ToolListArgs),
    /// Audit installed tools and their dependencies.
    Audit(ToolAuditArgs),
    /// Uninstall a tool.
    Uninstall(ToolUninstallArgs),
    /// Ensure that the tool executable directory is on the `PATH`.
    ///
    /// If the tool executable directory is not on `PATH`, uv tries to add it to the relevant shell
    /// configuration files.
    ///
    /// If the shell configuration already adds the directory but it is not on `PATH`, uv exits
    /// with an error.
    ///
    /// The tool executable directory follows the XDG standard. Use `uv tool dir --bin` to display
    /// it.
    #[command(alias = "ensurepath")]
    UpdateShell,
    /// Show the path to the uv tools directory.
    ///
    /// The tools directory stores environments and metadata for installed tools.
    ///
    /// By default, uv stores tools in its data directory at `$XDG_DATA_HOME/uv/tools` or
    /// `$HOME/.local/share/uv/tools` on Unix and `%APPDATA%\uv\data\tools` on Windows.
    ///
    /// Use `$UV_TOOL_DIR` to change the tool installation directory.
    ///
    /// Use `--bin` to display the directory where uv installs executables.
    Dir(ToolDirArgs),
}

#[derive(Args)]
pub struct ToolRunArgs {
    /// The command to run.
    ///
    /// WARNING: The documentation for [`Self::command`] is not included in help output
    #[command(subcommand)]
    pub command: Option<ExternalCommand>,

    /// Use the given package to provide the command.
    ///
    /// By default, the package name is assumed to match the command name.
    #[arg(long, value_hint = ValueHint::Other)]
    pub from: Option<String>,

    /// Run with the given packages installed.
    #[arg(short = 'w', long, value_hint = ValueHint::Other)]
    pub with: Vec<comma::CommaSeparatedRequirements>,

    /// Run with the given packages installed in editable mode
    ///
    /// In a project, uv installs these dependencies in a separate temporary environment layered
    /// over the tool environment. They may conflict with the tool's dependencies.
    #[arg(long, value_hint = ValueHint::DirPath)]
    pub with_editable: Vec<comma::CommaSeparatedRequirements>,

    /// Run with the packages listed in the given files.
    ///
    /// The following formats are supported: `requirements.txt`, `.py` files with inline metadata,
    /// and `pylock.toml`.
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub with_requirements: Vec<Maybe<PathBuf>>,

    /// Constrain versions using the given requirements files.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    ///
    /// This is equivalent to pip's `--constraint` option.
    #[arg(
        long,
        short,
        alias = "constraint",
        env = EnvVars::UV_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub constraints: Vec<Maybe<PathBuf>>,

    /// Constrain build dependencies using the given requirements files when building source
    /// distributions.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    #[arg(
        long,
        short,
        alias = "build-constraint",
        env = EnvVars::UV_BUILD_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub build_constraints: Vec<Maybe<PathBuf>>,

    /// Override versions using the given requirements files.
    ///
    /// Overrides files use the `requirements.txt` format and force a specific package version.
    /// The selected version replaces package requirements, even if the result is invalid.
    ///
    /// Constraints are _additive_: uv combines them with package requirements. Overrides are
    /// _absolute_: they replace package requirements.
    #[arg(
        long,
        alias = "override",
        env = EnvVars::UV_OVERRIDE,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub overrides: Vec<Maybe<PathBuf>>,

    /// Run the tool in an isolated virtual environment, ignoring any already-installed tools [env:
    /// UV_ISOLATED=]
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
    pub isolated: bool,

    /// Load environment variables from a `.env` file.
    ///
    /// Specify multiple files if needed. Values in later files override values in earlier
    /// files.
    #[arg(long, value_delimiter = ' ', env = EnvVars::UV_ENV_FILE, value_hint = ValueHint::FilePath)]
    pub env_file: Vec<PathBuf>,

    /// Avoid reading environment variables from a `.env` file [env: UV_NO_ENV_FILE=]
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_env_file: bool,

    #[command(flatten)]
    pub installer: ResolverInstallerArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Whether to use Git LFS when adding a dependency from Git.
    #[arg(long)]
    pub lfs: bool,

    /// The Python interpreter to use to build the run environment.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// Whether to show resolver and installer output from any environment modifications [env:
    /// UV_SHOW_RESOLUTION=]
    ///
    /// By default, environment modifications are omitted, but enabled under `--verbose`.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), hide = true)]
    pub show_resolution: bool,

    /// The platform for which requirements should be installed.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    ///
    /// For macOS (Darwin), the minimum version defaults to `13.0`. Use
    /// `MACOSX_DEPLOYMENT_TARGET` to set a different minimum, such as `14.0`.
    ///
    /// For iOS, the minimum version defaults to `13.0`. Use `IPHONEOS_DEPLOYMENT_TARGET` to set
    /// a different minimum, such as `14.0`.
    ///
    /// For Android, the minimum API level defaults to `24`. Use `ANDROID_API_LEVEL` to set a
    /// different minimum, such as `26`.
    ///
    /// WARNING: uv selects wheels for the _target_ platform, so installed distributions may not
    /// work on the _current_ platform. uv builds source distributions for the _current_ platform,
    /// so they may not work on the _target_ platform. Use `--python-platform` only for advanced
    /// use cases.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,

    /// The backend to use when fetching packages in the PyTorch ecosystem (e.g., `cpu`, `cu126`, or `auto`)
    ///
    /// When set, uv will ignore the configured index URLs for packages in the PyTorch ecosystem,
    /// and will instead use the defined backend.
    ///
    /// For example, when set to `cpu`, uv will use the CPU-only PyTorch index; when set to `cu126`,
    /// uv will use the PyTorch index for CUDA 12.6.
    ///
    /// The `auto` mode will attempt to detect the appropriate PyTorch index based on the currently
    /// installed CUDA drivers.
    ///
    /// This option is in preview and may change in any future release.
    #[arg(long, value_enum, env = EnvVars::UV_TORCH_BACKEND)]
    pub torch_backend: Option<TorchMode>,

    #[arg(long, hide = true)]
    pub generate_shell_completion: Option<clap_complete_command::Shell>,
}

#[derive(Args)]
pub struct UvxArgs {
    #[command(flatten)]
    pub tool_run: ToolRunArgs,

    /// Display the uvx version.
    #[arg(short = 'V', long, action = clap::ArgAction::Version)]
    pub version: Option<bool>,
}

#[derive(Args)]
pub struct ToolInstallArgs {
    /// The package to install commands from.
    #[arg(value_hint = ValueHint::Other)]
    pub package: String,

    /// The package to install commands from.
    ///
    /// This option is provided for parity with `uv tool run`, but is redundant with `package`.
    #[arg(long, hide = true, value_hint = ValueHint::Other)]
    pub from: Option<String>,

    /// Include the following additional requirements.
    #[arg(short = 'w', long, value_hint = ValueHint::Other)]
    pub with: Vec<comma::CommaSeparatedRequirements>,

    /// Run with the packages listed in the given files.
    ///
    /// The following formats are supported: `requirements.txt`, `.py` files with inline metadata,
    /// and `pylock.toml`.
    #[arg(long, value_delimiter = ',', value_parser = parse_maybe_file_path, value_hint = ValueHint::FilePath)]
    pub with_requirements: Vec<Maybe<PathBuf>>,

    /// Install the target package in editable mode, such that changes in the package's source
    /// directory are reflected without reinstallation.
    #[arg(short, long)]
    pub editable: bool,

    /// Include the given packages in editable mode.
    #[arg(long, value_hint = ValueHint::DirPath)]
    pub with_editable: Vec<comma::CommaSeparatedRequirements>,

    /// Install executables from the following packages.
    #[arg(long, value_hint = ValueHint::Other)]
    pub with_executables_from: Vec<comma::CommaSeparatedRequirements>,

    /// Constrain versions using the given requirements files.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    ///
    /// This is equivalent to pip's `--constraint` option.
    #[arg(
        long,
        short,
        alias = "constraint",
        env = EnvVars::UV_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub constraints: Vec<Maybe<PathBuf>>,

    /// Override versions using the given requirements files.
    ///
    /// Overrides files use the `requirements.txt` format and force a specific package version.
    /// The selected version replaces package requirements, even if the result is invalid.
    ///
    /// Constraints are _additive_: uv combines them with package requirements. Overrides are
    /// _absolute_: they replace package requirements.
    #[arg(
        long,
        alias = "override",
        env = EnvVars::UV_OVERRIDE,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub overrides: Vec<Maybe<PathBuf>>,

    /// Exclude packages from resolution using the given requirements files.
    ///
    /// Excludes files use the `requirements.txt` format and identify packages to exclude from
    /// resolution. uv omits each excluded package and ignores its dependencies. Exclusions are
    /// unconditional: uv ignores requirement specifiers and markers, and omits each listed package
    /// from every resolved environment.
    #[arg(
        long,
        alias = "exclude",
        env = EnvVars::UV_EXCLUDE,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub excludes: Vec<Maybe<PathBuf>>,

    /// Constrain build dependencies using the given requirements files when building source
    /// distributions.
    ///
    /// Constraints files use the `requirements.txt` format and control only the installed
    /// _version_ of a package. Listing a package in a constraints file does _not_ install it.
    #[arg(
        long,
        short,
        alias = "build-constraint",
        env = EnvVars::UV_BUILD_CONSTRAINT,
        value_delimiter = ' ',
        value_parser = parse_maybe_file_path,
        value_hint = ValueHint::FilePath,
    )]
    pub build_constraints: Vec<Maybe<PathBuf>>,

    #[command(flatten)]
    pub installer: ResolverInstallerArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Force installation of the tool.
    ///
    /// Will recreate any existing environment for the tool and replace any existing entry points
    /// with the same name in the executable directory.
    #[arg(long)]
    pub force: bool,

    /// Whether to use Git LFS when adding a dependency from Git.
    #[arg(long)]
    pub lfs: bool,

    /// The Python interpreter to use to build the tool environment.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// The platform for which requirements should be installed.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    ///
    /// For macOS (Darwin), the minimum version defaults to `13.0`. Use
    /// `MACOSX_DEPLOYMENT_TARGET` to set a different minimum, such as `14.0`.
    ///
    /// For iOS, the minimum version defaults to `13.0`. Use `IPHONEOS_DEPLOYMENT_TARGET` to set
    /// a different minimum, such as `14.0`.
    ///
    /// For Android, the minimum API level defaults to `24`. Use `ANDROID_API_LEVEL` to set a
    /// different minimum, such as `26`.
    ///
    /// WARNING: uv selects wheels for the _target_ platform, so installed distributions may not
    /// work on the _current_ platform. uv builds source distributions for the _current_ platform,
    /// so they may not work on the _target_ platform. Use `--python-platform` only for advanced
    /// use cases.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,

    /// The backend to use when fetching packages in the PyTorch ecosystem (e.g., `cpu`, `cu126`, or `auto`)
    ///
    /// When set, uv will ignore the configured index URLs for packages in the PyTorch ecosystem,
    /// and will instead use the defined backend.
    ///
    /// For example, when set to `cpu`, uv will use the CPU-only PyTorch index; when set to `cu126`,
    /// uv will use the PyTorch index for CUDA 12.6.
    ///
    /// The `auto` mode will attempt to detect the appropriate PyTorch index based on the currently
    /// installed CUDA drivers.
    ///
    /// This option is in preview and may change in any future release.
    #[arg(long, value_enum, env = EnvVars::UV_TORCH_BACKEND)]
    pub torch_backend: Option<TorchMode>,
}

#[derive(Args)]
pub struct ToolListArgs {
    /// Whether to display the path to each tool environment and installed executable.
    #[arg(long)]
    pub show_paths: bool,

    /// Whether to display the version specifier(s) used to install each tool.
    #[arg(long)]
    pub show_version_specifiers: bool,

    /// Whether to display the additional requirements installed with each tool.
    #[arg(long)]
    pub show_with: bool,

    /// Whether to display the extra requirements installed with each tool.
    #[arg(long)]
    pub show_extras: bool,

    /// Whether to display the Python version associated with each tool.
    #[arg(long)]
    pub show_python: bool,

    /// List outdated tools.
    ///
    /// The latest version of each tool will be shown alongside the installed version. Up-to-date
    /// tools will be omitted from the output.
    #[arg(long, overrides_with("no_outdated"))]
    pub outdated: bool,

    #[arg(long, overrides_with("outdated"), hide = true)]
    pub no_outdated: bool,

    #[command(flatten)]
    pub exclude_newer: PackageExcludeNewerArgs,

    // Hide unused global Python options.
    #[arg(long, hide = true)]
    pub python_preference: Option<PythonPreference>,

    #[arg(long, hide = true)]
    pub no_python_downloads: bool,
}

#[derive(Args)]
pub struct ToolAuditArgs {
    /// The names of the installed tools to audit.
    #[arg(required = true, value_hint = ValueHint::Other)]
    pub name: Vec<PackageName>,

    /// Audit all installed tools.
    #[arg(long, conflicts_with("name"))]
    pub all: bool,

    #[command(flatten)]
    pub audit: AuditCommonArgs,
}

#[derive(Args)]
pub struct ToolDirArgs {
    /// Show the directory into which `uv tool` will install executables.
    ///
    /// By default, `uv tool dir` shows the directory into which the tool Python environments
    /// themselves are installed, rather than the directory containing the linked executables.
    ///
    /// The tool executable directory is determined according to the XDG standard and is derived
    /// from the following environment variables, in order of preference:
    ///
    /// - `$UV_TOOL_BIN_DIR`
    /// - `$XDG_BIN_HOME`
    /// - `$XDG_DATA_HOME/../bin`
    /// - `$HOME/.local/bin`
    #[arg(long, verbatim_doc_comment)]
    pub bin: bool,
}

#[derive(Args)]
pub struct ToolUninstallArgs {
    /// The name of the tool to uninstall.
    #[arg(required = true, value_hint = ValueHint::Other)]
    pub name: Vec<PackageName>,

    /// Uninstall all tools.
    #[arg(long, conflicts_with("name"))]
    pub all: bool,
}

#[derive(Args)]
pub struct ToolUpgradeArgs {
    /// The name of the tool to upgrade, along with an optional version specifier.
    #[arg(required = true, value_hint = ValueHint::Other)]
    pub name: Vec<String>,

    /// Upgrade all tools.
    #[arg(long, conflicts_with("name"))]
    pub all: bool,

    /// Upgrade a tool, and specify it to use the given Python interpreter to build its environment.
    /// Use with `--all` to apply to all tools.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,

    /// The platform for which requirements should be installed.
    ///
    /// Specify a "target triple" that describes the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    ///
    /// For macOS (Darwin), the minimum version defaults to `13.0`. Use
    /// `MACOSX_DEPLOYMENT_TARGET` to set a different minimum, such as `14.0`.
    ///
    /// For iOS, the minimum version defaults to `13.0`. Use `IPHONEOS_DEPLOYMENT_TARGET` to set
    /// a different minimum, such as `14.0`.
    ///
    /// For Android, the minimum API level defaults to `24`. Use `ANDROID_API_LEVEL` to set a
    /// different minimum, such as `26`.
    ///
    /// WARNING: uv selects wheels for the _target_ platform, so installed distributions may not
    /// work on the _current_ platform. uv builds source distributions for the _current_ platform,
    /// so they may not work on the _target_ platform. Use `--python-platform` only for advanced
    /// use cases.
    #[arg(long)]
    pub python_platform: Option<TargetTriple>,

    // The following is equivalent to flattening `ResolverInstallerArgs`, with the `--upgrade`,
    // `--upgrade-package`, and `--upgrade-group` options hidden, and the `--no-upgrade` option
    // removed.
    /// Allow package upgrades, ignoring pinned versions in any existing output file. Implies
    /// `--refresh`.
    #[arg(hide = true, long, short = 'U', help_heading = "Resolver options")]
    pub upgrade: bool,

    /// Allow upgrades for a specific package, ignoring pinned versions in any existing output
    /// file. Implies `--refresh-package`.
    #[arg(hide = true, long, short = 'P', help_heading = "Resolver options")]
    pub upgrade_package: Vec<Requirement<VerbatimParsedUrl>>,

    /// Allow upgrades for all packages in a dependency group, ignoring pinned versions in any
    /// existing output file.
    #[arg(hide = true, long, help_heading = "Resolver options")]
    pub upgrade_group: Vec<GroupName>,

    #[command(flatten)]
    pub index_args: IndexArgs,

    #[command(flatten)]
    pub reinstall: ReinstallArgs,

    #[command(flatten)]
    pub registry_client: RegistryClientArgs,

    #[command(flatten)]
    pub version_selection: VersionSelectionArgs,

    /// Settings to pass to the PEP 517 build backend, specified as `KEY=VALUE` pairs.
    #[arg(
        long,
        short = 'C',
        alias = "config-settings",
        help_heading = "Build options"
    )]
    pub config_setting: Option<Vec<ConfigSettingEntry>>,

    /// Settings to pass to the PEP 517 build backend for a specific package, specified as `PACKAGE:KEY=VALUE` pairs.
    #[arg(
        long,
        alias = "config-settings-package",
        help_heading = "Build options"
    )]
    pub config_setting_package: Option<Vec<ConfigSettingPackageEntry>>,

    #[command(flatten)]
    pub build_isolation: PackageBuildIsolationArgs,

    #[command(flatten)]
    pub exclude_newer: PackageExcludeNewerArgs,

    /// The method to use when installing packages from the global cache.
    ///
    /// Defaults to `clone` (also known as Copy-on-Write) on macOS and Linux, and `hardlink` on
    /// Windows.
    ///
    /// WARNING: Symlink mode links the target environment to the cache. Clearing the cache with
    /// `uv cache clean` removes the source files and breaks all installed packages. Avoid symlink
    /// mode unless you understand this risk.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_LINK_MODE,
        help_heading = "Installer options"
    )]
    pub link_mode: Option<uv_install_wheel::LinkMode>,

    #[command(flatten)]
    pub compile_bytecode: CompileBytecodeArgs,

    #[command(flatten)]
    pub sources: SourcesArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,
}

#[derive(Args)]
pub struct PythonNamespace {
    #[command(subcommand)]
    pub command: PythonCommand,
}

#[derive(Subcommand)]
pub enum PythonCommand {
    /// List the available Python installations.
    ///
    /// By default, uv lists installed Python versions and the latest available patch download for
    /// each supported Python major version.
    ///
    /// Use `--managed-python` to view only managed Python versions.
    ///
    /// Use `--no-managed-python` to omit managed Python versions.
    ///
    /// Use `--all-versions` to view all available patch versions.
    ///
    /// Use `--only-installed` to omit available downloads.
    #[command(alias = "ls")]
    List(PythonListArgs),

    /// Download and install Python versions.
    ///
    /// uv supports CPython and PyPy. It downloads CPython from Astral's `python-build-standalone`
    /// project and PyPy from `python.org`. Each uv release includes a list of available Python
    /// versions. You may need to upgrade uv to install a newer Python version.
    ///
    /// uv installs Python into its Python directory. Use `uv python dir` to display that directory.
    ///
    /// By default, uv adds Python executables with a minor-version suffix, such as `python3.13`,
    /// to a directory on `PATH`. Use `--default` to also install `python3` and `python`. Use
    /// `uv python dir --bin` to display the target directory.
    ///
    /// You can request multiple Python versions.
    ///
    /// See `uv help python` to view supported request formats.
    Install(PythonInstallArgs),

    /// Upgrade installed Python versions.
    ///
    /// Upgrades versions to the latest supported patch release.
    ///
    /// Specify a Python minor version, such as `3.13`, to upgrade it. You can specify multiple
    /// versions.
    ///
    /// If you do not specify a version, uv upgrades all managed CPython versions.
    ///
    /// uv does not uninstall older patch versions during an upgrade.
    ///
    /// Virtual environments created by uv automatically use the upgraded version. Environments
    /// created before uv supported upgrades continue to use the old version. Recreate those
    /// environments to enable upgrades.
    ///
    /// uv does not yet support upgrades for other implementations, such as PyPy.
    Upgrade(PythonUpgradeArgs),

    /// Search for a Python installation.
    ///
    /// Displays the path to the Python executable.
    ///
    /// See `uv help python` to view supported request formats and details on discovery behavior.
    Find(PythonFindArgs),

    /// Pin to a specific Python version.
    ///
    /// Write the Python version to `.python-version`. Other uv commands use this file to determine
    /// the required version.
    ///
    /// If you do not specify a version, uv displays the version in `.python-version`. If that file
    /// does not exist, uv exits with an error.
    ///
    /// See `uv help python` to view supported request formats.
    Pin(PythonPinArgs),

    /// Show the uv Python installation directory.
    ///
    /// By default, uv stores Python installations in its data directory at
    /// `$XDG_DATA_HOME/uv/python` or `$HOME/.local/share/uv/python` on Unix and
    /// `%APPDATA%\uv\data\python` on Windows.
    ///
    /// Use `$UV_PYTHON_INSTALL_DIR` to change the Python installation directory.
    ///
    /// Use `--bin` to display the directory where uv installs Python executables. Use
    /// `$UV_PYTHON_BIN_DIR` to change that directory.
    Dir(PythonDirArgs),

    /// Uninstall Python versions.
    Uninstall(PythonUninstallArgs),

    /// Ensure that the Python executable directory is on the `PATH`.
    ///
    /// If the Python executable directory is not on `PATH`, uv tries to add it to the relevant
    /// shell configuration files.
    ///
    /// If the shell configuration already adds the directory but it is not on `PATH`, uv exits
    /// with an error.
    ///
    /// The Python executable directory follows the XDG standard. Use `uv python dir --bin` to
    /// display it.
    #[command(alias = "ensurepath")]
    UpdateShell,
}

#[derive(Args)]
pub struct PythonListArgs {
    /// A Python request to filter by.
    ///
    /// See `uv help python` to view supported request formats.
    pub request: Option<String>,

    /// List all Python versions, including old patch versions.
    ///
    /// By default, only the latest patch version is shown for each minor version.
    #[arg(long)]
    pub all_versions: bool,

    /// List Python downloads for all platforms.
    ///
    /// By default, only downloads for the current platform are shown.
    #[arg(long)]
    pub all_platforms: bool,

    /// List Python downloads for all architectures.
    ///
    /// By default, only downloads for the current architecture are shown.
    #[arg(long, alias = "all_architectures")]
    pub all_arches: bool,

    /// Only show installed Python versions.
    ///
    /// By default, installed distributions and available downloads for the current platform are shown.
    #[arg(long, conflicts_with("only_downloads"))]
    pub only_installed: bool,

    /// Only show available Python downloads.
    ///
    /// By default, installed distributions and available downloads for the current platform are shown.
    #[arg(long, conflicts_with("only_installed"))]
    pub only_downloads: bool,

    /// Show the URLs of available Python downloads.
    ///
    /// By default, these display as `<download available>`.
    #[arg(long)]
    pub show_urls: bool,

    /// Select the output format.
    #[arg(long, value_enum, default_value_t = PythonListFormat::default())]
    pub output_format: PythonListFormat,

    /// URL pointing to JSON of custom Python installations.
    #[arg(long, value_hint = ValueHint::Other)]
    pub python_downloads_json_url: Option<String>,
}

#[derive(Args)]
pub struct PythonDirArgs {
    /// Show the directory into which `uv python` will install Python executables.
    ///
    /// The Python executable directory is determined according to the XDG standard and is derived
    /// from the following environment variables, in order of preference:
    ///
    /// - `$UV_PYTHON_BIN_DIR`
    /// - `$XDG_BIN_HOME`
    /// - `$XDG_DATA_HOME/../bin`
    /// - `$HOME/.local/bin`
    #[arg(long, verbatim_doc_comment)]
    pub bin: bool,
}

#[derive(Args)]
pub struct PythonInstallCompileBytecodeArgs {
    /// Compile Python's standard library to bytecode after installation.
    ///
    /// By default, Python compiles `.py` files to bytecode (`__pycache__/*.pyc`) when a module is
    /// first imported. Enable this option to compile during installation instead. This increases
    /// installation time and disk use, but can improve startup time for CLI applications and
    /// Docker containers.
    ///
    /// uv processes the Python version's `stdlib` directory and ignores compilation errors.
    #[arg(
        long,
        alias = "compile",
        overrides_with("no_compile_bytecode"),
        env = EnvVars::UV_COMPILE_BYTECODE,
        value_parser = clap::builder::BoolishValueParser::new(),
    )]
    pub compile_bytecode: bool,

    #[arg(
        long,
        alias = "no-compile",
        overrides_with("compile_bytecode"),
        hide = true
    )]
    pub no_compile_bytecode: bool,
}

#[derive(Args)]
pub struct PythonInstallArgs {
    /// The directory to store the Python installation in.
    ///
    /// If you set this option, set `UV_PYTHON_INSTALL_DIR` for later commands so uv can find the
    /// Python installation.
    ///
    /// See `uv python dir` to view the current Python installation directory. Defaults to
    /// `~/.local/share/uv/python`.
    #[arg(long, short, env = EnvVars::UV_PYTHON_INSTALL_DIR, value_hint = ValueHint::DirPath)]
    pub install_dir: Option<PathBuf>,

    /// Install a Python executable into the `bin` directory.
    ///
    /// This is the default behavior. If you set this flag explicitly and uv cannot install the
    /// executable, it exits with an error.
    ///
    /// This can also be set with `UV_PYTHON_INSTALL_BIN=1`.
    ///
    /// See `UV_PYTHON_BIN_DIR` to customize the target directory.
    #[arg(long, overrides_with("no_bin"), hide = true)]
    pub bin: bool,

    /// Do not install a Python executable into the `bin` directory.
    ///
    /// This can also be set with `UV_PYTHON_INSTALL_BIN=0`.
    #[arg(long, overrides_with("bin"), conflicts_with("default"))]
    pub no_bin: bool,

    /// Register the Python installation in the Windows registry.
    ///
    /// This is the default behavior on Windows. If you set this flag explicitly and uv cannot
    /// create the registry entry, it exits with an error.
    ///
    /// This can also be set with `UV_PYTHON_INSTALL_REGISTRY=1`.
    #[arg(long, overrides_with("no_registry"), hide = true)]
    pub registry: bool,

    /// Do not register the Python installation in the Windows registry.
    ///
    /// This can also be set with `UV_PYTHON_INSTALL_REGISTRY=0`.
    #[arg(long, overrides_with("registry"))]
    pub no_registry: bool,

    /// The Python version(s) to install.
    ///
    /// If you do not specify a version, uv checks `UV_PYTHON`, then `.python-versions` or
    /// `.python-version` files. If none exist, uv checks for an installed Python version. If it
    /// finds none, it installs the latest stable Python version.
    ///
    /// See `uv help python` to view supported request formats.
    #[arg(env = EnvVars::UV_PYTHON)]
    pub targets: Vec<String>,

    /// Set the URL to use as the source for downloading Python installations.
    ///
    /// The provided URL will replace
    /// `https://github.com/astral-sh/python-build-standalone/releases/download` in, e.g.,
    /// `https://github.com/astral-sh/python-build-standalone/releases/download/20240713/cpython-3.12.4%2B20240713-aarch64-apple-darwin-install_only.tar.gz`.
    ///
    /// Use a `file://` URL to read distributions from a local directory.
    #[arg(long, value_hint = ValueHint::Url)]
    pub mirror: Option<String>,

    /// Set the URL to use as the source for downloading PyPy installations.
    ///
    /// The provided URL will replace `https://downloads.python.org/pypy` in, e.g.,
    /// `https://downloads.python.org/pypy/pypy3.8-v7.3.7-osx64.tar.bz2`.
    ///
    /// Use a `file://` URL to read distributions from a local directory.
    #[arg(long, value_hint = ValueHint::Url)]
    pub pypy_mirror: Option<String>,

    /// URL pointing to JSON of custom Python installations.
    #[arg(long, value_hint = ValueHint::Other)]
    pub python_downloads_json_url: Option<String>,

    /// Reinstall the requested Python version, if it's already installed.
    ///
    /// If you request a minor version, uv reinstalls all matching installed patch versions.
    ///
    /// By default, uv exits successfully if the version is already installed.
    #[arg(long, short)]
    pub reinstall: bool,

    /// Replace existing Python executables during installation.
    ///
    /// By default, uv does not replace executables that it does not manage.
    ///
    /// Implies `--reinstall`.
    #[arg(long, short)]
    pub force: bool,

    /// Upgrade existing Python installations to the latest patch version.
    ///
    /// By default, uv does not upgrade installed Python versions to newer patch releases. With
    /// `--upgrade`, uv installs the latest patch for each specified minor version.
    ///
    /// If a requested version is not installed, uv installs it.
    ///
    /// This option accepts only minor versions, such as `3.12`. If you request a patch version,
    /// such as `3.12.2`, uv exits with an error.
    #[arg(long, short = 'U')]
    pub upgrade: bool,

    /// Use as the default Python version.
    ///
    /// By default, uv installs only `python{major}.{minor}`, such as `python3.10`. With
    /// `--default`, it also installs `python{major}`, such as `python3`, and `python`.
    ///
    /// Other Python variants retain their tag. For example, `3.13+freethreaded` with `--default`
    /// installs `python3t` and `pythont` instead of `python3` and `python`.
    ///
    /// If you request multiple Python versions, uv exits with an error.
    #[arg(long, conflicts_with("no_bin"))]
    pub default: bool,

    #[command(flatten)]
    pub compile_bytecode: PythonInstallCompileBytecodeArgs,
}

impl PythonInstallArgs {
    #[must_use]
    pub fn install_mirrors(&self) -> PythonInstallMirrors {
        PythonInstallMirrors {
            python_install_mirror: self.mirror.clone(),
            pypy_install_mirror: self.pypy_mirror.clone(),
            python_downloads_json_url: self.python_downloads_json_url.clone(),
        }
    }
}

#[derive(Args)]
pub struct PythonUpgradeArgs {
    /// The directory Python installations are stored in.
    ///
    /// If you set this option, set `UV_PYTHON_INSTALL_DIR` for later commands so uv can find the
    /// Python installation.
    ///
    /// See `uv python dir` to view the current Python installation directory. Defaults to
    /// `~/.local/share/uv/python`.
    #[arg(long, short, env = EnvVars::UV_PYTHON_INSTALL_DIR, value_hint = ValueHint::DirPath)]
    pub install_dir: Option<PathBuf>,

    /// The Python minor version(s) to upgrade.
    ///
    /// If you do not specify a version, uv upgrades all managed CPython versions.
    #[arg(env = EnvVars::UV_PYTHON)]
    pub targets: Vec<String>,

    /// Set the URL to use as the source for downloading Python installations.
    ///
    /// The provided URL will replace
    /// `https://github.com/astral-sh/python-build-standalone/releases/download` in, e.g.,
    /// `https://github.com/astral-sh/python-build-standalone/releases/download/20240713/cpython-3.12.4%2B20240713-aarch64-apple-darwin-install_only.tar.gz`.
    ///
    /// Use a `file://` URL to read distributions from a local directory.
    #[arg(long, value_hint = ValueHint::Url)]
    pub mirror: Option<String>,

    /// Set the URL to use as the source for downloading PyPy installations.
    ///
    /// The provided URL will replace `https://downloads.python.org/pypy` in, e.g.,
    /// `https://downloads.python.org/pypy/pypy3.8-v7.3.7-osx64.tar.bz2`.
    ///
    /// Use a `file://` URL to read distributions from a local directory.
    #[arg(long, value_hint = ValueHint::Url)]
    pub pypy_mirror: Option<String>,

    /// Reinstall the latest Python patch, if it's already installed.
    ///
    /// By default, uv exits successfully if the latest patch is already installed.
    #[arg(long, short)]
    pub reinstall: bool,

    /// URL pointing to JSON of custom Python installations.
    #[arg(long, value_hint = ValueHint::Other)]
    pub python_downloads_json_url: Option<String>,

    #[command(flatten)]
    pub compile_bytecode: PythonInstallCompileBytecodeArgs,
}

impl PythonUpgradeArgs {
    #[must_use]
    pub fn install_mirrors(&self) -> PythonInstallMirrors {
        PythonInstallMirrors {
            python_install_mirror: self.mirror.clone(),
            pypy_install_mirror: self.pypy_mirror.clone(),
            python_downloads_json_url: self.python_downloads_json_url.clone(),
        }
    }
}

#[derive(Args)]
pub struct PythonUninstallArgs {
    /// The directory where the Python was installed.
    #[arg(long, short, env = EnvVars::UV_PYTHON_INSTALL_DIR, value_hint = ValueHint::DirPath)]
    pub install_dir: Option<PathBuf>,

    /// The Python version(s) to uninstall.
    ///
    /// See `uv help python` to view supported request formats.
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Uninstall all managed Python versions.
    #[arg(long, conflicts_with("targets"))]
    pub all: bool,
}

#[derive(Args)]
pub struct PythonFindArgs {
    /// The Python request.
    ///
    /// See `uv help python` to view supported request formats.
    pub request: Option<String>,

    /// Avoid discovering a project or workspace.
    ///
    /// Otherwise, if you do not specify a request, uv uses the Python requirement from a project
    /// in the current directory or a parent directory.
    #[arg(
        long,
        alias = "no_workspace",
        env = EnvVars::UV_NO_PROJECT,
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub no_project: bool,

    /// Only find system Python interpreters.
    ///
    /// By default, uv reports the first Python interpreter it would use. This can include an
    /// active virtual environment or one in the current directory or a parent directory.
    ///
    /// Use `--system` to skip virtual environments and search only the system path.
    #[arg(
        long,
        env = EnvVars::UV_SYSTEM_PYTHON,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_system")
    )]
    pub system: bool,

    #[arg(long, overrides_with("system"), hide = true)]
    pub no_system: bool,

    /// Find the environment for a Python script, rather than the current project.
    #[arg(
        long,
        conflicts_with = "request",
        conflicts_with = "no_project",
        conflicts_with = "system",
        conflicts_with = "no_system",
        value_hint = ValueHint::FilePath,
    )]
    pub script: Option<PathBuf>,

    /// Show the Python version that would be used instead of the path to the interpreter.
    #[arg(long)]
    pub show_version: bool,

    /// Resolve symlinks in the output path.
    ///
    /// uv canonicalizes the output path and resolves any symlinks.
    #[arg(long)]
    pub resolve_links: bool,

    /// URL pointing to JSON of custom Python installations.
    #[arg(long, value_hint = ValueHint::Other)]
    pub python_downloads_json_url: Option<String>,
}

#[derive(Args)]
pub struct PythonPinArgs {
    /// The Python version request.
    ///
    /// uv supports more `.python-version` formats than tools such as `pyenv`. To remain compatible
    /// with those tools, use version numbers instead of requests such as `cpython@3.10`.
    ///
    /// If you omit the request, uv displays the current pinned version.
    ///
    /// See `uv help python` to view supported request formats.
    pub request: Option<String>,

    /// Write the resolved Python interpreter path instead of the request.
    ///
    /// This makes uv use the same interpreter.
    ///
    /// This option is usually not safe to use when committing the `.python-version` file to version
    /// control.
    #[arg(long, overrides_with("resolved"))]
    pub resolved: bool,

    #[arg(long, overrides_with("no_resolved"), hide = true)]
    pub no_resolved: bool,

    /// Avoid validating the Python pin is compatible with the project or workspace.
    ///
    /// By default, uv searches the current directory and parent directories for a project or
    /// workspace. If it finds a workspace, it checks the Python pin against the workspace's
    /// `requires-python` constraint.
    #[arg(
        long,
        alias = "no-workspace",
        env = EnvVars::UV_NO_PROJECT,
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub no_project: bool,

    /// Update the global Python version pin.
    ///
    /// Write the pinned version to `.python-version` in the uv user configuration directory:
    /// `XDG_CONFIG_HOME/uv` on Linux or macOS, and `%APPDATA%/uv` on Windows.
    ///
    /// If the working directory and its parent directories contain no local Python pin, uv uses
    /// this version.
    #[arg(long)]
    pub global: bool,

    /// Remove the Python version pin.
    #[arg(long, conflicts_with = "request", conflicts_with = "resolved")]
    pub rm: bool,

    /// URL pointing to JSON of custom Python installations.
    #[arg(long, value_hint = ValueHint::Other)]
    pub python_downloads_json_url: Option<String>,
}

#[derive(Args)]
pub struct AuthLogoutArgs {
    /// The domain or URL of the service to logout from.
    pub service: Service,

    /// The username to logout.
    #[arg(long, short, value_hint = ValueHint::Other)]
    pub username: Option<String>,

    /// The keyring provider to use for storage of credentials.
    ///
    /// `logout` supports only `--keyring-provider native`, which uses uv's built-in system keyring
    /// integration.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_KEYRING_PROVIDER,
    )]
    pub keyring_provider: Option<KeyringProviderType>,
}

#[derive(Args)]
pub struct AuthLoginArgs {
    /// The domain or URL of the service to log into.
    #[arg(value_hint = ValueHint::Url)]
    pub service: Service,

    /// The username to use for the service.
    #[arg(long, short, conflicts_with = "token", value_hint = ValueHint::Other)]
    pub username: Option<String>,

    /// The password to use for the service.
    ///
    /// Use `-` to read the password from stdin.
    #[arg(long, conflicts_with = "token", value_hint = ValueHint::Other)]
    pub password: Option<String>,

    /// The token to use for the service.
    ///
    /// uv sets the username to `__token__`.
    ///
    /// Use `-` to read the token from stdin.
    #[arg(long, short, conflicts_with = "username", conflicts_with = "password", value_hint = ValueHint::Other)]
    pub token: Option<String>,

    /// The keyring provider to use for storage of credentials.
    ///
    /// `login` supports only `--keyring-provider native`, which uses uv's built-in system keyring
    /// integration.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_KEYRING_PROVIDER,
    )]
    pub keyring_provider: Option<KeyringProviderType>,
}

#[derive(Args)]
pub struct AuthTokenArgs {
    /// The domain or URL of the service to lookup.
    #[arg(value_hint = ValueHint::Url)]
    pub service: Service,

    /// The username to lookup.
    #[arg(long, short, value_hint = ValueHint::Other)]
    pub username: Option<String>,

    /// The keyring provider to use for reading credentials.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_KEYRING_PROVIDER,
    )]
    pub keyring_provider: Option<KeyringProviderType>,
}

#[derive(Args)]
pub struct AuthHelperArgs {
    #[command(subcommand)]
    pub command: AuthHelperCommand,

    /// The credential helper protocol to use
    #[arg(long, value_enum, required = true)]
    pub protocol: AuthHelperProtocol,
}

/// Credential helper protocols supported by uv
#[derive(Debug, Copy, Clone, PartialEq, Eq, clap::ValueEnum)]
pub enum AuthHelperProtocol {
    /// Bazel credential helper protocol as described in [the
    /// spec](https://github.com/bazelbuild/proposals/blob/main/designs/2022-06-07-bazel-credential-helpers.md)
    Bazel,
}

#[derive(Subcommand)]
pub enum AuthHelperCommand {
    /// Retrieve credentials for a URI
    Get,
}

#[derive(Args)]
pub struct GenerateShellCompletionArgs {
    /// The shell to generate the completion script for
    pub shell: clap_complete_command::Shell,

    // Hide unused global options.
    #[arg(long, short, hide = true)]
    pub no_cache: bool,
    #[arg(long, hide = true)]
    pub cache_dir: Option<PathBuf>,

    #[arg(long, hide = true)]
    pub python_preference: Option<PythonPreference>,
    #[arg(long, hide = true)]
    pub no_python_downloads: bool,

    #[arg(long, short, action = clap::ArgAction::Count, conflicts_with = "verbose", hide = true)]
    pub quiet: u8,
    #[arg(long, short, action = clap::ArgAction::Count, conflicts_with = "quiet", hide = true)]
    pub verbose: u8,
    #[arg(long, conflicts_with = "no_color", hide = true)]
    pub color: Option<ColorChoice>,
    #[arg(long, hide = true)]
    pub native_tls: bool,
    #[arg(long, hide = true)]
    pub offline: bool,
    #[arg(long, hide = true)]
    pub no_progress: bool,
    #[arg(long, hide = true)]
    pub config_file: Option<PathBuf>,
    #[arg(long, hide = true)]
    pub no_config: bool,
    #[arg(long, short, action = clap::ArgAction::HelpShort, hide = true)]
    pub help: Option<bool>,
    #[arg(short = 'V', long, hide = true)]
    pub version: bool,
}

#[derive(Args)]
pub struct IndexArgs {
    /// The indexes to use when resolving dependencies, in addition to the default index.
    ///
    /// Accepts a PEP 503-compliant repository (the simple repository API) or a local directory in
    /// the same format.
    ///
    /// All indexes provided via this flag take priority over the index specified by
    /// `--default-index` (which defaults to PyPI). When multiple `--index` flags are provided,
    /// earlier values take priority.
    ///
    /// Indexes configured in `uv.toml` or `pyproject.toml` may be selected by name. Enable the
    /// `index-by-name` preview feature to prefer index names over relative paths.
    ///
    /// Relative paths can be disambiguated from index names with `./` or `../` on Unix or `.\\`,
    /// `..\\`, `./` or `../` on Windows.
    //
    // The nested Vec structure (`Vec<Vec<Maybe<IndexArg>>>`) is required for clap's
    // value parsing mechanism, which processes one value at a time, in order to handle
    // `UV_INDEX` the same way pip handles `PIP_EXTRA_INDEX_URL`.
    #[arg(
        long,
        env = EnvVars::UV_INDEX,
        hide_env_values = true,
        value_parser = parse_indices,
        help_heading = "Index options"
    )]
    pub index: Option<Vec<Vec<Maybe<IndexArg>>>>,

    /// The default package index (by default: <https://pypi.org/simple>).
    ///
    /// Accepts a PEP 503-compliant repository (the simple repository API) or a local directory in
    /// the same format.
    ///
    /// The index given by this flag is given lower priority than all other indexes specified via
    /// the `--index` flag.
    ///
    /// Indexes configured in `uv.toml` or `pyproject.toml` may be selected by name. Enable the
    /// `index-by-name` preview feature to prefer index names over relative paths.
    #[arg(
        long,
        env = EnvVars::UV_DEFAULT_INDEX,
        hide_env_values = true,
        value_parser = parse_default_index,
        help_heading = "Index options"
    )]
    pub default_index: Option<Maybe<IndexArg>>,

    /// (Deprecated: use `--default-index` instead) The URL of the Python package index (by default:
    /// <https://pypi.org/simple>).
    ///
    /// Accepts a PEP 503-compliant repository (the simple repository API) or a local directory in
    /// the same format.
    ///
    /// The index given by this flag is given lower priority than all other indexes specified via
    /// the `--extra-index-url` flag.
    #[arg(
        long,
        short,
        env = EnvVars::UV_INDEX_URL,
        hide_env_values = true,
        value_parser = parse_index_url,
        help_heading = "Index options"
    )]
    pub index_url: Option<Maybe<PipIndex>>,

    /// (Deprecated: use `--index` instead) Extra URLs of package indexes to use, in addition to
    /// `--index-url`.
    ///
    /// Accepts a PEP 503-compliant repository (the simple repository API) or a local directory in
    /// the same format.
    ///
    /// All indexes provided via this flag take priority over the index specified by `--index-url`
    /// (which defaults to PyPI). When multiple `--extra-index-url` flags are provided, earlier
    /// values take priority.
    #[arg(
        long,
        env = EnvVars::UV_EXTRA_INDEX_URL,
        hide_env_values = true,
        value_delimiter = ' ',
        value_parser = parse_extra_index_url,
        help_heading = "Index options"
    )]
    pub extra_index_url: Option<Vec<Maybe<PipExtraIndex>>>,

    /// Locations to search for candidate distributions, in addition to those found in the registry
    /// indexes.
    ///
    /// If a path, the target must be a directory that contains packages as wheel files (`.whl`) or
    /// source distributions (e.g., `.tar.gz` or `.zip`) at the top level.
    ///
    /// If a URL, the page must contain a flat list of links to package files adhering to the
    /// formats described above.
    #[arg(
        long,
        short,
        env = EnvVars::UV_FIND_LINKS,
        hide_env_values = true,
        value_delimiter = ',',
        value_parser = parse_find_links,
        help_heading = "Index options"
    )]
    pub find_links: Option<Vec<Maybe<PipFindLinks>>>,

    /// Ignore the registry index (e.g., PyPI), instead relying on direct URL dependencies and those
    /// provided via `--find-links`.
    #[arg(long, help_heading = "Index options")]
    pub no_index: bool,
}

/// Arguments that configure the package registry client.
#[derive(Args)]
#[group(skip)]
pub struct RegistryClientArgs {
    /// The strategy to use when resolving against multiple index URLs.
    ///
    /// By default, uv will stop at the first index on which a given package is available, and limit
    /// resolutions to those present on that first index (`first-index`). This prevents "dependency
    /// confusion" attacks, whereby an attacker can upload a malicious package under the same name
    /// to an alternate index.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_INDEX_STRATEGY,
        help_heading = "Index options"
    )]
    pub index_strategy: Option<IndexStrategy>,

    /// Attempt to use `keyring` for authentication for index URLs.
    ///
    /// At present, only `--keyring-provider subprocess` is supported, which configures uv to use
    /// the `keyring` CLI to handle authentication.
    ///
    /// Defaults to `disabled`.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_KEYRING_PROVIDER,
        help_heading = "Index options"
    )]
    pub keyring_provider: Option<KeyringProviderType>,
}

/// Arguments that control dependency sources.
#[derive(Args)]
#[group(skip)]
pub struct SourcesArgs {
    /// Ignore the `tool.uv.sources` table when resolving dependencies. Used to lock against the
    /// standards-compliant, publishable package metadata, as opposed to using any workspace, Git,
    /// URL, or local path sources.
    #[arg(
        long,
        env = EnvVars::UV_NO_SOURCES,
        value_parser = clap::builder::BoolishValueParser::new(),
        help_heading = "Resolver options",
    )]
    no_sources: bool,

    /// Don't use sources from the `tool.uv.sources` table for the specified packages [env: `UV_NO_SOURCES_PACKAGE`=]
    #[arg(long, help_heading = "Resolver options", value_delimiter = ' ')]
    no_sources_package: Vec<PackageName>,
}

/// Arguments that configure package version selection.
#[derive(Args)]
#[group(skip)]
pub struct VersionSelectionArgs {
    /// The strategy to use when selecting between the different compatible versions for a given
    /// package requirement.
    ///
    /// By default, uv will use the latest compatible version of each package (`highest`).
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_RESOLUTION,
        help_heading = "Resolver options"
    )]
    resolution: Option<ResolutionMode>,

    /// The strategy to use when considering pre-release versions.
    ///
    /// By default, uv will prefer stable candidates, falling back to pre-releases only after every
    /// stable candidate that satisfies the active constraints is rejected
    /// (`if-necessary`).
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_PRERELEASE,
        help_heading = "Resolver options"
    )]
    prerelease: Option<PrereleaseMode>,

    /// The strategy to use when considering pre-release versions for a specific package.
    ///
    /// Accepts package-mode pairs in the format `PACKAGE=MODE`, where `MODE` is any value
    /// accepted by `--prerelease`.
    ///
    /// May be provided multiple times for different packages.
    #[arg(long, help_heading = "Resolver options", value_hint = ValueHint::Other)]
    prerelease_package: Option<Vec<PrereleasePackageEntry>>,

    #[arg(long, hide = true, help_heading = "Resolver options")]
    pre: bool,

    /// The strategy to use when selecting multiple versions of a given package across Python
    /// versions and platforms.
    ///
    /// By default, uv will optimize for selecting the latest version of each package for each
    /// supported Python version (`requires-python`), while minimizing the number of selected
    /// versions across platforms.
    ///
    /// Under `fewest`, uv will minimize the number of selected versions for each package,
    /// preferring older versions that are compatible with a wider range of supported Python
    /// versions or platforms.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_FORK_STRATEGY,
        help_heading = "Resolver options"
    )]
    fork_strategy: Option<ForkStrategy>,
}

/// Arguments that select dependency groups in a project or workspace.
#[derive(Args)]
#[group(skip)]
pub struct ProjectDependencyGroupsArgs<const CHECKS_CONFLICTS: bool = false> {
    /// Include the development dependency group [env: UV_DEV=]
    ///
    /// Development dependencies are defined via `dependency-groups.dev` or
    /// `tool.uv.dev-dependencies` in a `pyproject.toml`.
    ///
    /// This option is an alias for `--group dev`.
    ///
    /// This option is only available when running in a project.
    #[arg(long, overrides_with("no_dev"), hide = true, value_parser = clap::builder::BoolishValueParser::new())]
    pub dev: bool,

    /// Disable the development dependency group [env: UV_NO_DEV=]
    ///
    /// This option is an alias of `--no-group dev`.
    /// See `--no-default-groups` to disable all default groups instead.
    ///
    /// This option is only available when running in a project.
    #[arg(long, overrides_with("dev"), value_parser = clap::builder::BoolishValueParser::new())]
    pub no_dev: bool,

    /// Only include the development dependency group.
    ///
    /// The project and its dependencies will be omitted.
    ///
    /// This option is an alias for `--only-group dev`. Implies `--no-default-groups`.
    #[arg(long, conflicts_with_all = ["group", "all_groups", "no_dev"])]
    pub only_dev: bool,

    /// Include dependencies from the specified dependency group.
    ///
    /// May be provided multiple times.
    #[arg(
        long,
        conflicts_with_all = ["only_group", "only_dev"],
        value_hint = ValueHint::Other,
        long_help = if CHECKS_CONFLICTS {
            concat!(
                "Include dependencies from the specified dependency group.\n\n",
                "When multiple extras or groups are specified that appear in ",
                "`tool.uv.conflicts`, uv will report an error.\n\n",
                "May be provided multiple times."
            )
        } else {
            concat!(
                "Include dependencies from the specified dependency group.\n\n",
                "May be provided multiple times."
            )
        }
    )]
    pub group: Vec<GroupName>,

    /// Disable the specified dependency group [env: `UV_NO_GROUP`=]
    ///
    /// This option always takes precedence over default groups,
    /// `--all-groups`, and `--group`.
    ///
    /// May be provided multiple times.
    #[arg(long, value_delimiter = ' ', value_hint = ValueHint::Other)]
    pub no_group: Vec<GroupName>,

    /// Ignore the default dependency groups.
    ///
    /// uv includes the groups defined in `tool.uv.default-groups` by default.
    /// This disables that option, however, specific groups can still be included with `--group`.
    #[arg(long, env = EnvVars::UV_NO_DEFAULT_GROUPS, value_parser = clap::builder::BoolishValueParser::new())]
    pub no_default_groups: bool,

    /// Only include dependencies from the specified dependency group.
    ///
    /// The project and its dependencies will be omitted.
    ///
    /// May be provided multiple times. Implies `--no-default-groups`.
    #[arg(long, conflicts_with_all = ["group", "dev", "all_groups"], value_hint = ValueHint::Other)]
    pub only_group: Vec<GroupName>,

    /// Include dependencies from all dependency groups.
    ///
    /// `--no-group` can be used to exclude specific groups.
    #[arg(long, conflicts_with_all = ["only_group", "only_dev"])]
    pub all_groups: bool,
}

/// Dependency-group arguments for commands that reject conflicting extras or groups.
pub type ConflictCheckedDependencyGroupsArgs = ProjectDependencyGroupsArgs<true>;

/// Arguments that configure requirement hash checking.
#[derive(Args)]
#[group(skip)]
pub struct HashCheckingArgs {
    /// Require a matching hash for each requirement.
    ///
    /// By default, uv will verify any available hashes in the requirements file, but will not
    /// require that all requirements have an associated hash.
    ///
    /// When `--require-hashes` is enabled, _all_ requirements must include a hash or set of hashes,
    /// and _all_ requirements must either be pinned to exact versions (e.g., `==1.0.0`), or be
    /// specified via direct URL.
    ///
    /// Hash-checking mode introduces a number of additional constraints:
    ///
    /// - Git dependencies are not supported.
    /// - Editable installations are not supported.
    /// - Local dependencies are not supported, unless they point to a specific wheel (`.whl`) or
    ///   source archive (`.zip`, `.tar.gz`), as opposed to a directory.
    #[arg(
        long,
        env = EnvVars::UV_REQUIRE_HASHES,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("no_require_hashes"),
    )]
    pub require_hashes: bool,

    #[arg(long, overrides_with("require_hashes"), hide = true)]
    pub no_require_hashes: bool,

    #[arg(long, overrides_with("no_verify_hashes"), hide = true)]
    pub verify_hashes: bool,

    /// Disable validation of hashes in the requirements file.
    ///
    /// By default, uv will verify any available hashes in the requirements file, but will not
    /// require that all requirements have an associated hash. To enforce hash validation, use
    /// `--require-hashes`.
    #[arg(
        long,
        env = EnvVars::UV_NO_VERIFY_HASHES,
        value_parser = clap::builder::BoolishValueParser::new(),
        overrides_with("verify_hashes"),
    )]
    pub no_verify_hashes: bool,
}

/// Arguments that filter packages by upload date.
#[derive(Args)]
#[group(skip)]
pub struct ExcludeNewerArgs {
    /// Limit candidate packages to those that were uploaded prior to the given date.
    ///
    /// The date is compared against the upload time of each individual distribution artifact
    /// (i.e., when each file was uploaded to the package index), not the release date of the
    /// package version.
    ///
    /// Accepts RFC 3339 timestamps (e.g., `2006-12-02T02:07:43Z`), local dates in the same format
    /// (e.g., `2006-12-02`) resolved based on your system's configured time zone, a "friendly"
    /// duration (e.g., `24 hours`, `1 week`, `30 days`), or an ISO 8601 duration (e.g., `PT24H`,
    /// `P7D`, `P30D`).
    ///
    /// Durations use a fixed number of seconds and treat each day as 24 hours. They ignore local
    /// time zones and DST transitions. Calendar units such as months and years are not allowed.
    ///
    /// Use `false` to disable `exclude-newer`.
    #[arg(
        long,
        env = EnvVars::UV_EXCLUDE_NEWER,
        help_heading = "Resolver options",
        value_hint = ValueHint::Other,
    )]
    pub exclude_newer: Option<ExcludeNewerOverride>,
}

/// Arguments that filter packages by global and package-specific upload dates.
#[derive(Args)]
#[group(skip)]
pub struct PackageExcludeNewerArgs {
    #[command(flatten)]
    pub exclude_newer: ExcludeNewerArgs,

    /// Limit candidate packages for specific packages to those that were uploaded prior to the
    /// given date.
    ///
    /// Accepts package-date pairs in the format `PACKAGE=DATE`, where `DATE` is an RFC 3339
    /// timestamp (e.g., `2006-12-02T02:07:43Z`), a local date in the same format (e.g.,
    /// `2006-12-02`) resolved based on your system's configured time zone, a "friendly" duration
    /// (e.g., `24 hours`, `1 week`, `30 days`), or an ISO 8601 duration (e.g., `PT24H`, `P7D`,
    /// `P30D`).
    ///
    /// Durations use a fixed number of seconds and treat each day as 24 hours. They ignore local
    /// time zones and DST transitions. Calendar units such as months and years are not allowed.
    ///
    /// Can be provided multiple times for different packages.
    #[arg(long, help_heading = "Resolver options", value_hint = ValueHint::Other)]
    pub exclude_newer_package: Option<Vec<ExcludeNewerPackageEntry>>,
}

#[derive(Args)]
pub struct RefreshArgs {
    /// Refresh all cached data.
    #[arg(long, overrides_with("no_refresh"), help_heading = "Cache options")]
    refresh: bool,

    #[arg(
        long,
        overrides_with("refresh"),
        hide = true,
        help_heading = "Cache options"
    )]
    no_refresh: bool,

    /// Refresh cached data for a specific package.
    #[arg(long, help_heading = "Cache options", value_hint = ValueHint::Other)]
    refresh_package: Vec<PackageName>,
}

#[derive(Args)]
pub struct BuildOptionsArgs {
    /// Don't build source distributions.
    ///
    /// uv reuses cached wheels from previously built source distributions. If an operation
    /// requires a new source build, uv exits with an error. uv still builds first-party packages,
    /// such as projects in the workspace. uv may also build editable requirements, and their build
    /// backends may run arbitrary Python code.
    #[arg(
        long,
        env = EnvVars::UV_NO_BUILD,
        overrides_with("build"),
        value_parser = clap::builder::BoolishValueParser::new(),
        help_heading = "Build options",
    )]
    no_build: bool,

    #[arg(
        long,
        overrides_with("no_build"),
        hide = true,
        help_heading = "Build options"
    )]
    build: bool,

    /// Don't build source distributions for a specific package [env: `UV_NO_BUILD_PACKAGE`=]
    ///
    /// First-party packages, such as projects in the workspace, will still be built.
    #[arg(
        long,
        help_heading = "Build options",
        value_delimiter = ' ',
        value_hint = ValueHint::Other,
    )]
    no_build_package: Vec<PackageName>,

    /// Don't install pre-built wheels.
    ///
    /// uv builds and installs the specified packages from source. If a pre-built wheel is
    /// available, the resolver still uses it to read package metadata.
    #[arg(
        long,
        env = EnvVars::UV_NO_BINARY,
        overrides_with("binary"),
        value_parser = clap::builder::BoolishValueParser::new(),
        help_heading = "Build options"
    )]
    no_binary: bool,

    #[arg(
        long,
        overrides_with("no_binary"),
        hide = true,
        help_heading = "Build options"
    )]
    binary: bool,

    /// Don't install pre-built wheels for a specific package [env: `UV_NO_BINARY_PACKAGE`=]
    #[arg(
        long,
        help_heading = "Build options",
        value_delimiter = ' ',
        value_hint = ValueHint::Other,
    )]
    no_binary_package: Vec<PackageName>,
}

/// Arguments that configure build isolation for source distributions.
#[derive(Args)]
#[group(skip)]
pub struct BuildIsolationArgs {
    /// Disable isolation when building source distributions.
    ///
    /// Assumes that build dependencies specified by PEP 518 are already installed.
    #[arg(
        long,
        overrides_with("build_isolation"),
        help_heading = "Build options",
        env = EnvVars::UV_NO_BUILD_ISOLATION,
        value_parser = clap::builder::BoolishValueParser::new(),
    )]
    no_build_isolation: bool,

    #[arg(
        long,
        overrides_with("no_build_isolation"),
        hide = true,
        help_heading = "Build options"
    )]
    build_isolation: bool,
}

/// Arguments that configure global and package-specific build isolation.
#[derive(Args)]
#[group(skip)]
pub struct PackageBuildIsolationArgs {
    #[command(flatten)]
    build_isolation: BuildIsolationArgs,

    /// Disable isolation when building source distributions for a specific package.
    ///
    /// Assumes that the packages' build dependencies specified by PEP 518 are already installed.
    #[arg(long, help_heading = "Build options", value_hint = ValueHint::Other)]
    no_build_isolation_package: Vec<PackageName>,
}

#[derive(Args)]
#[group(skip)]
pub struct ReinstallArgs {
    /// Reinstall all packages, regardless of whether they're already installed. Implies
    /// `--refresh`.
    #[arg(
        long,
        alias = "force-reinstall",
        overrides_with("no_reinstall"),
        help_heading = "Installer options"
    )]
    pub reinstall: bool,

    #[arg(
        long,
        overrides_with("reinstall"),
        hide = true,
        help_heading = "Installer options"
    )]
    pub no_reinstall: bool,

    /// Reinstall a specific package, regardless of whether it's already installed. Implies
    /// `--refresh-package`.
    #[arg(long, help_heading = "Installer options", value_hint = ValueHint::Other)]
    pub reinstall_package: Vec<PackageName>,
}

#[derive(Args)]
#[group(skip)]
pub struct CompileBytecodeArgs {
    /// Compile Python files to bytecode after installation.
    ///
    /// By default, uv does not compile Python (`.py`) files to bytecode (`__pycache__/*.pyc`);
    /// instead, compilation is performed lazily the first time a module is imported. For use-cases
    /// in which start time is critical, such as CLI applications and Docker containers, this option
    /// can be enabled to trade longer installation times for faster start times.
    ///
    /// When enabled, install operations (e.g., `uv pip install`) will compile installed or
    /// reinstalled Python files. Commands that perform a sync operation (e.g., `uv sync` or `uv
    /// run`) will process the entire site-packages directory including packages that are not being
    /// modified.
    #[arg(
        long,
        alias = "compile",
        overrides_with("no_compile_bytecode"),
        help_heading = "Installer options",
        env = EnvVars::UV_COMPILE_BYTECODE,
        value_parser = clap::builder::BoolishValueParser::new(),
    )]
    compile_bytecode: bool,

    #[arg(
        long,
        alias = "no-compile",
        overrides_with("compile_bytecode"),
        hide = true,
        help_heading = "Installer options"
    )]
    no_compile_bytecode: bool,
}

/// Arguments that are used by commands that need to install (but not resolve) packages.
#[derive(Args)]
pub struct InstallerArgs {
    #[command(flatten)]
    index_args: IndexArgs,

    #[command(flatten)]
    reinstall: ReinstallArgs,

    #[command(flatten)]
    registry_client: RegistryClientArgs,

    /// Settings to pass to the PEP 517 build backend, specified as `KEY=VALUE` pairs.
    #[arg(
        long,
        short = 'C',
        alias = "config-settings",
        help_heading = "Build options"
    )]
    config_setting: Option<Vec<ConfigSettingEntry>>,

    /// Settings to pass to the PEP 517 build backend for a specific package, specified as `PACKAGE:KEY=VALUE` pairs.
    #[arg(
        long,
        alias = "config-settings-package",
        help_heading = "Build options"
    )]
    config_settings_package: Option<Vec<ConfigSettingPackageEntry>>,

    #[command(flatten)]
    build_isolation: BuildIsolationArgs,

    #[command(flatten)]
    exclude_newer: PackageExcludeNewerArgs,

    /// The method to use when installing packages from the global cache.
    ///
    /// Defaults to `clone` (also known as Copy-on-Write) on macOS and Linux, and `hardlink` on
    /// Windows.
    ///
    /// WARNING: Symlink mode links the target environment to the cache. Clearing the cache with
    /// `uv cache clean` removes the source files and breaks all installed packages. Avoid symlink
    /// mode unless you understand this risk.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_LINK_MODE,
        help_heading = "Installer options"
    )]
    link_mode: Option<uv_install_wheel::LinkMode>,

    #[command(flatten)]
    compile_bytecode: CompileBytecodeArgs,

    #[command(flatten)]
    sources: SourcesArgs,
}

/// Arguments that are used by commands that need to resolve (but not install) packages.
#[derive(Args)]
pub struct ResolverArgs {
    #[command(flatten)]
    index_args: IndexArgs,

    /// Allow package upgrades, ignoring pinned versions in any existing output file. Implies
    /// `--refresh`.
    #[arg(
        long,
        short = 'U',
        overrides_with("no_upgrade"),
        help_heading = "Resolver options"
    )]
    upgrade: bool,

    #[arg(
        long,
        overrides_with("upgrade"),
        hide = true,
        help_heading = "Resolver options"
    )]
    no_upgrade: bool,

    /// Allow upgrades for a specific package, ignoring pinned versions in any existing output
    /// file. Implies `--refresh-package`.
    #[arg(long, short = 'P', help_heading = "Resolver options")]
    upgrade_package: Vec<Requirement<VerbatimParsedUrl>>,

    /// Allow upgrades for all packages in a dependency group, ignoring pinned versions in any
    /// existing output file.
    #[arg(long, help_heading = "Resolver options")]
    upgrade_group: Vec<GroupName>,

    #[command(flatten)]
    registry_client: RegistryClientArgs,

    #[command(flatten)]
    version_selection: VersionSelectionArgs,

    /// Settings to pass to the PEP 517 build backend, specified as `KEY=VALUE` pairs.
    #[arg(
        long,
        short = 'C',
        alias = "config-settings",
        help_heading = "Build options"
    )]
    config_setting: Option<Vec<ConfigSettingEntry>>,

    /// Settings to pass to the PEP 517 build backend for a specific package, specified as `PACKAGE:KEY=VALUE` pairs.
    #[arg(
        long,
        alias = "config-settings-package",
        help_heading = "Build options"
    )]
    config_settings_package: Option<Vec<ConfigSettingPackageEntry>>,

    #[command(flatten)]
    build_isolation: PackageBuildIsolationArgs,

    #[command(flatten)]
    exclude_newer: PackageExcludeNewerArgs,

    /// The method to use when installing packages from the global cache.
    ///
    /// This option is only used when building source distributions.
    ///
    /// Defaults to `clone` (also known as Copy-on-Write) on macOS and Linux, and `hardlink` on
    /// Windows.
    ///
    /// WARNING: Symlink mode links the target environment to the cache. Clearing the cache with
    /// `uv cache clean` removes the source files and breaks all installed packages. Avoid symlink
    /// mode unless you understand this risk.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_LINK_MODE,
        help_heading = "Installer options"
    )]
    link_mode: Option<uv_install_wheel::LinkMode>,

    #[command(flatten)]
    sources: SourcesArgs,
}

/// Arguments that are used by commands that need to resolve and install packages.
#[derive(Args)]
pub struct ResolverInstallerArgs {
    #[command(flatten)]
    pub index_args: IndexArgs,

    /// Allow package upgrades, ignoring pinned versions in any existing output file. Implies
    /// `--refresh`.
    #[arg(
        long,
        short = 'U',
        overrides_with("no_upgrade"),
        help_heading = "Resolver options"
    )]
    pub upgrade: bool,

    #[arg(
        long,
        overrides_with("upgrade"),
        hide = true,
        help_heading = "Resolver options"
    )]
    pub no_upgrade: bool,

    /// Allow upgrades for a specific package, ignoring pinned versions in any existing output file.
    /// Implies `--refresh-package`.
    #[arg(long, short = 'P', help_heading = "Resolver options", value_hint = ValueHint::Other)]
    pub upgrade_package: Vec<Requirement<VerbatimParsedUrl>>,

    /// Allow upgrades for all packages in a dependency group, ignoring pinned versions in any
    /// existing output file.
    #[arg(long, help_heading = "Resolver options")]
    pub upgrade_group: Vec<GroupName>,

    #[command(flatten)]
    pub reinstall: ReinstallArgs,

    #[command(flatten)]
    pub registry_client: RegistryClientArgs,

    #[command(flatten)]
    pub version_selection: VersionSelectionArgs,

    /// Settings to pass to the PEP 517 build backend, specified as `KEY=VALUE` pairs.
    #[arg(
        long,
        short = 'C',
        alias = "config-settings",
        help_heading = "Build options",
        value_hint = ValueHint::Other,
    )]
    pub config_setting: Option<Vec<ConfigSettingEntry>>,

    /// Settings to pass to the PEP 517 build backend for a specific package, specified as `PACKAGE:KEY=VALUE` pairs.
    #[arg(
        long,
        alias = "config-settings-package",
        help_heading = "Build options",
        value_hint = ValueHint::Other,
    )]
    pub config_settings_package: Option<Vec<ConfigSettingPackageEntry>>,

    #[command(flatten)]
    pub build_isolation: PackageBuildIsolationArgs,

    #[command(flatten)]
    pub exclude_newer: PackageExcludeNewerArgs,

    /// The method to use when installing packages from the global cache.
    ///
    /// Defaults to `clone` (also known as Copy-on-Write) on macOS and Linux, and `hardlink` on
    /// Windows.
    ///
    /// WARNING: Symlink mode links the target environment to the cache. Clearing the cache with
    /// `uv cache clean` removes the source files and breaks all installed packages. Avoid symlink
    /// mode unless you understand this risk.
    #[arg(
        long,
        value_enum,
        env = EnvVars::UV_LINK_MODE,
        help_heading = "Installer options"
    )]
    pub link_mode: Option<uv_install_wheel::LinkMode>,

    #[command(flatten)]
    pub compile_bytecode: CompileBytecodeArgs,

    #[command(flatten)]
    pub sources: SourcesArgs,
}

/// Arguments that are used by commands that need to fetch from the Simple API.
#[derive(Args)]
pub struct FetchArgs {
    #[command(flatten)]
    index_args: IndexArgs,

    #[command(flatten)]
    registry_client: RegistryClientArgs,

    #[command(flatten)]
    exclude_newer: PackageExcludeNewerArgs,
}

#[derive(Args)]
pub struct DisplayTreeArgs {
    /// Maximum display depth of the dependency tree
    #[arg(long, short, default_value_t = 255)]
    pub depth: u8,

    /// Prune the given package from the display of the dependency tree.
    #[arg(long, value_hint = ValueHint::Other)]
    pub prune: Vec<PackageName>,

    /// Display only the specified packages.
    #[arg(long, value_hint = ValueHint::Other)]
    pub package: Vec<PackageName>,

    /// Do not de-duplicate repeated dependencies. Usually, when a package has already displayed its
    /// dependencies, further occurrences will not re-display its dependencies, and will include a
    /// (*) to indicate it has already been shown. This flag will cause those duplicates to be
    /// repeated.
    #[arg(long)]
    pub no_dedupe: bool,

    /// Show the reverse dependencies for the given package. This flag will invert the tree and
    /// display the packages that depend on the given package.
    #[arg(long, alias = "reverse")]
    pub invert: bool,

    /// Show the latest available version of each package in the tree.
    #[arg(long)]
    pub outdated: bool,

    /// Show compressed wheel sizes for packages in the tree.
    #[arg(long)]
    pub show_sizes: bool,
}

#[derive(Args, Debug)]
pub struct PublishArgs {
    /// Paths to the files to upload. Accepts glob expressions.
    ///
    /// Defaults to the `dist` directory. Selects only wheels and source distributions
    /// and their attestations, while ignoring other files.
    #[arg(default_value = "dist/*", value_hint = ValueHint::FilePath)]
    pub files: Vec<String>,

    /// The name of an index in the configuration to use for publishing.
    ///
    /// The index must have a `publish-url` setting, for example:
    ///
    /// ```toml
    /// [[tool.uv.index]]
    /// name = "pypi"
    /// url = "https://pypi.org/simple"
    /// publish-url = "https://upload.pypi.org/legacy/"
    /// ```
    ///
    /// The index `url` will be used to check for existing files to skip duplicate uploads.
    ///
    /// With these settings, the following two calls are equivalent:
    ///
    /// ```shell
    /// uv publish --index pypi
    /// uv publish --publish-url https://upload.pypi.org/legacy/ --check-url https://pypi.org/simple
    /// ```
    #[arg(
        long,
        verbatim_doc_comment,
        env = EnvVars::UV_PUBLISH_INDEX,
        conflicts_with = "publish_url",
        conflicts_with = "check_url",
        value_hint = ValueHint::Other,
    )]
    pub index: Option<String>,

    /// The username for the upload.
    #[arg(
        short,
        long,
        env = EnvVars::UV_PUBLISH_USERNAME,
        hide_env_values = true,
        value_hint = ValueHint::Other
    )]
    pub username: Option<String>,

    /// The password for the upload.
    #[arg(
        short,
        long,
        env = EnvVars::UV_PUBLISH_PASSWORD,
        hide_env_values = true,
        value_hint = ValueHint::Other
    )]
    pub password: Option<String>,

    /// The token for the upload.
    ///
    /// Using a token is equivalent to passing `__token__` as `--username` and the token as
    /// `--password` password.
    #[arg(
        short,
        long,
        env = EnvVars::UV_PUBLISH_TOKEN,
        hide_env_values = true,
        conflicts_with = "username",
        conflicts_with = "password",
        value_hint = ValueHint::Other,
    )]
    pub token: Option<String>,

    /// Configure trusted publishing.
    ///
    /// By default, uv checks for trusted publishing when running in a supported environment, but
    /// ignores it if it isn't configured.
    ///
    /// uv's supported environments for trusted publishing include GitHub Actions and GitLab CI/CD.
    #[arg(long)]
    pub trusted_publishing: Option<TrustedPublishing>,

    /// Attempt to use `keyring` for authentication for remote requirements files.
    ///
    /// At present, only `--keyring-provider subprocess` is supported, which configures uv to use
    /// the `keyring` CLI to handle authentication.
    ///
    /// Defaults to `disabled`.
    #[arg(long, value_enum, env = EnvVars::UV_KEYRING_PROVIDER)]
    pub keyring_provider: Option<KeyringProviderType>,

    /// The URL of the upload endpoint (not the index URL).
    ///
    /// Note that there are typically different URLs for index access (e.g., `https:://.../simple`)
    /// and index upload.
    ///
    /// Defaults to PyPI's publish URL (<https://upload.pypi.org/legacy/>).
    #[arg(long, env = EnvVars::UV_PUBLISH_URL, hide_env_values = true)]
    pub publish_url: Option<DisplaySafeUrl>,

    /// Check an index URL for existing files to skip duplicate uploads.
    ///
    /// This option allows retrying publishing that failed after only some, but not all files have
    /// been uploaded, and handles errors due to parallel uploads of the same file.
    ///
    /// Before uploading, the index is checked. If the exact same file already exists in the index,
    /// the file will not be uploaded. If an error occurred during the upload, the index is checked
    /// again, to handle cases where the identical file was uploaded twice in parallel.
    ///
    /// The exact behavior will vary based on the index. When uploading to PyPI, uploading the same
    /// file succeeds even without `--check-url`, while most other indexes error.
    ///
    /// The index must provide one of the supported hashes (SHA-256, SHA-384, or SHA-512).
    #[arg(long, env = EnvVars::UV_PUBLISH_CHECK_URL, hide_env_values = true)]
    pub check_url: Option<IndexUrl>,

    #[arg(long, hide = true)]
    pub skip_existing: bool,

    /// Perform a dry run without uploading files.
    ///
    /// The command checks the distribution metadata locally, and checks for existing files if
    /// `--check-url` or `--index` is provided, but will not upload any files.
    #[arg(long)]
    pub dry_run: bool,

    /// Do not upload attestations for the published files.
    ///
    /// By default, uv attempts to upload matching PEP 740 attestations with each distribution
    /// that is published.
    #[arg(long, env = EnvVars::UV_PUBLISH_NO_ATTESTATIONS)]
    pub no_attestations: bool,
}

#[derive(Args)]
pub struct WorkspaceNamespace {
    #[command(subcommand)]
    pub command: WorkspaceCommand,
}

#[derive(Subcommand)]
pub enum WorkspaceCommand {
    /// View metadata about the current workspace.
    ///
    /// The output of this command is not yet stable.
    Metadata(Box<MetadataArgs>),
    /// Display the path of a workspace member.
    ///
    /// By default, the path to the workspace root directory is displayed.
    /// The `--package` option can be used to display the path to a workspace member instead.
    ///
    /// If used outside of a workspace, i.e., if a `pyproject.toml` cannot be found, uv will exit with an error.
    Dir(WorkspaceDirArgs),
    /// List the members of a workspace.
    ///
    /// Displays newline separated names of workspace members.
    List(WorkspaceListArgs),
}
#[derive(Args)]
pub struct MetadataArgs {
    /// View metadata for the specified PEP 723 Python script, rather than the current workspace.
    ///
    /// If provided, uv will resolve the dependencies based on the script's inline metadata table,
    /// in adherence with PEP 723.
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub script: Option<PathBuf>,

    /// Check if the lockfile is up-to-date [env: UV_LOCKED=]
    ///
    /// Asserts that the `uv.lock` would remain unchanged after a resolution. If the lockfile is
    /// missing or needs to be updated, uv will exit with an error.
    #[arg(long, conflicts_with_all = ["frozen", "upgrade"], overrides_with = "no_locked")]
    pub locked: bool,

    /// Disable locked mode, overriding `UV_LOCKED`.
    #[arg(long, overrides_with = "locked", hide = true)]
    pub no_locked: bool,

    /// Assert that a `uv.lock` exists without checking if it is up-to-date [env: UV_FROZEN=]
    #[arg(long, conflicts_with_all = ["locked"], overrides_with = "no_frozen")]
    pub frozen: bool,

    /// Disable frozen mode, overriding `UV_FROZEN`.
    #[arg(long, overrides_with = "frozen", hide = true)]
    pub no_frozen: bool,

    /// Perform a dry run, without writing the lockfile.
    ///
    /// In dry-run mode, uv will resolve the project's dependencies and report on the resulting
    /// changes, but will not write the lockfile to disk.
    #[arg(
        long,
        conflicts_with = "frozen",
        conflicts_with = "locked",
        conflicts_with = "sync"
    )]
    pub dry_run: bool,

    #[command(flatten)]
    pub resolver: ResolverArgs,

    #[command(flatten)]
    pub build: BuildOptionsArgs,

    #[command(flatten)]
    pub refresh: RefreshArgs,

    /// Sync the environment to include module ownership metadata in the output.
    ///
    /// This adds a mapping from importable module names to references to the package nodes
    /// that provide them. By default, the environment is synced in inexact mode.
    #[arg(long)]
    pub sync: bool,

    /// Perform an exact sync, removing extraneous packages.
    ///
    /// By default, synchronization preserves packages that are not part of the selected
    /// resolution. When enabled, uv removes those packages from the environment.
    #[arg(long, requires = "sync")]
    pub exact: bool,

    /// Sync dependencies to the active virtual environment.
    ///
    /// Instead of creating or updating the virtual environment for the project or script, the
    /// active virtual environment will be preferred, if the `VIRTUAL_ENV` environment variable is
    /// set.
    #[arg(long)]
    pub active: bool,

    /// The Python interpreter to use during resolution.
    ///
    /// A Python interpreter is required for building source distributions to determine package
    /// metadata when there are not wheels.
    ///
    /// The interpreter is also used as the fallback value for the minimum Python version if
    /// `requires-python` is not set.
    ///
    /// See `uv help python` for details on Python discovery and supported request formats.
    #[arg(
        long,
        short,
        env = EnvVars::UV_PYTHON,
        verbatim_doc_comment,
        help_heading = "Python options",
        value_parser = parse_maybe_string,
        value_hint = ValueHint::Other,
    )]
    pub python: Option<Maybe<String>>,
}

#[derive(Args, Debug)]
pub struct WorkspaceDirArgs {
    /// Display the path to a specific package in the workspace.
    #[arg(long, value_hint = ValueHint::Other)]
    pub package: Option<PackageName>,
}

#[derive(Args, Debug)]
pub struct WorkspaceListArgs {
    /// Show paths instead of names.
    #[arg(long)]
    pub paths: bool,

    /// List all standalone scripts with inline metadata in the workspace.
    #[arg(long)]
    pub scripts: bool,
}

/// See [PEP 517](https://peps.python.org/pep-0517/) and
/// [PEP 660](https://peps.python.org/pep-0660/) for specifications of the parameters.
#[derive(Subcommand)]
pub enum BuildBackendCommand {
    /// PEP 517 hook `build_sdist`.
    BuildSdist { sdist_directory: PathBuf },
    /// PEP 517 hook `build_wheel`.
    BuildWheel {
        wheel_directory: PathBuf,
        #[arg(long)]
        metadata_directory: Option<PathBuf>,
    },
    /// PEP 660 hook `build_editable`.
    BuildEditable {
        wheel_directory: PathBuf,
        #[arg(long)]
        metadata_directory: Option<PathBuf>,
    },
    /// PEP 517 hook `get_requires_for_build_sdist`.
    GetRequiresForBuildSdist,
    /// PEP 517 hook `get_requires_for_build_wheel`.
    GetRequiresForBuildWheel,
    /// PEP 517 hook `prepare_metadata_for_build_wheel`.
    PrepareMetadataForBuildWheel { wheel_directory: PathBuf },
    /// PEP 660 hook `get_requires_for_build_editable`.
    GetRequiresForBuildEditable,
    /// PEP 660 hook `prepare_metadata_for_build_editable`.
    PrepareMetadataForBuildEditable { wheel_directory: PathBuf },
}
