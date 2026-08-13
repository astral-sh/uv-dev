//! Environment variables that uv supports.
//! Generates `docs/reference/environment.md`.
use uv_macros::{attr_added_in, attr_env_var_pattern, attr_hidden, attribute_env_vars_metadata};

/// Declares the environment variables that `uv` and its crates use.
pub struct EnvVars;

#[attribute_env_vars_metadata]
impl EnvVars {
    /// The path to the binary that started uv.
    ///
    /// uv passes this path to each subprocess.
    ///
    /// If the executable is a symbolic link, the returned path depends on the platform.
    /// Some platforms return the symbolic link. Others return its target.
    ///
    /// See <https://doc.rust-lang.org/std/env/fn.current_exe.html#security> for security
    /// considerations.
    #[attr_added_in("0.6.0")]
    pub const UV: &'static str = "UV";

    /// The path to the Ruff binary that `uv format` uses.
    #[attr_added_in("0.11.22")]
    pub const RUFF: &'static str = "RUFF";

    /// The path to the ty binary that `uv check` uses.
    #[attr_added_in("0.11.22")]
    pub const TY: &'static str = "TY";

    /// Matches the `--offline` command-line option. When set, uv disables network access.
    #[attr_added_in("0.5.9")]
    pub const UV_OFFLINE: &'static str = "UV_OFFLINE";

    /// Matches the `--default-index` command-line option.
    /// When set, uv searches this index as the default package index.
    #[attr_added_in("0.4.23")]
    pub const UV_DEFAULT_INDEX: &'static str = "UV_DEFAULT_INDEX";

    /// Matches the `--index` command-line option.
    /// When set, uv searches these space-separated indexes as additional package indexes.
    #[attr_added_in("0.4.23")]
    pub const UV_INDEX: &'static str = "UV_INDEX";

    /// Matches the `--index-url` command-line option.
    /// When set, uv searches this URL as the default package index.
    /// Deprecated: use `UV_DEFAULT_INDEX` instead.
    #[attr_added_in("0.0.5")]
    pub const UV_INDEX_URL: &'static str = "UV_INDEX_URL";

    /// Matches the `--extra-index-url` command-line option.
    /// When set, uv searches these space-separated URLs as additional package indexes.
    /// Deprecated: use `UV_INDEX` instead.
    #[attr_added_in("0.1.3")]
    pub const UV_EXTRA_INDEX_URL: &'static str = "UV_EXTRA_INDEX_URL";

    /// Matches the `--find-links` command-line option.
    /// When set, uv searches these additional comma-separated locations for packages.
    #[attr_added_in("0.4.19")]
    pub const UV_FIND_LINKS: &'static str = "UV_FIND_LINKS";

    /// Matches the `--no-sources` command-line option.
    /// When set, uv ignores `[tool.uv.sources]` annotations during dependency resolution.
    #[attr_added_in("0.9.8")]
    pub const UV_NO_SOURCES: &'static str = "UV_NO_SOURCES";

    /// Matches the `--cache-dir` command-line option.
    /// When set, uv uses this directory instead of the default cache directory.
    #[attr_added_in("0.0.5")]
    pub const UV_CACHE_DIR: &'static str = "UV_CACHE_DIR";

    /// The directory that stores credentials for a plain-text backend.
    #[attr_added_in("0.8.15")]
    pub const UV_CREDENTIALS_DIR: &'static str = "UV_CREDENTIALS_DIR";

    /// Matches the `--no-cache` command-line option.
    /// When set, uv does not use the cache.
    #[attr_added_in("0.1.2")]
    pub const UV_NO_CACHE: &'static str = "UV_NO_CACHE";

    /// Matches the `--resolution` command-line option.
    /// When set to `lowest-direct`, uv installs the lowest compatible version of each direct
    /// dependency.
    #[attr_added_in("0.1.27")]
    pub const UV_RESOLUTION: &'static str = "UV_RESOLUTION";

    /// Matches the `--prerelease` command-line option.
    /// When set to `allow`, uv permits pre-release versions for all dependencies.
    #[attr_added_in("0.1.16")]
    pub const UV_PRERELEASE: &'static str = "UV_PRERELEASE";

    /// Matches the `--fork-strategy` option.
    /// Controls version selection during universal resolution.
    #[attr_added_in("0.5.9")]
    pub const UV_FORK_STRATEGY: &'static str = "UV_FORK_STRATEGY";

    /// Matches the `--system` command-line option.
    /// When set to `true`, uv uses the first Python interpreter in the system `PATH`.
    ///
    /// WARNING: Use `UV_SYSTEM_PYTHON=true` carefully.
    /// It is intended for continuous integration (CI) or container environments.
    /// Changes to the system Python can cause unexpected behavior.
    #[attr_added_in("0.1.18")]
    pub const UV_SYSTEM_PYTHON: &'static str = "UV_SYSTEM_PYTHON";

    /// Matches the `--python` command-line option.
    /// When set to a path, uv uses this Python interpreter for all operations.
    #[attr_added_in("0.1.40")]
    pub const UV_PYTHON: &'static str = "UV_PYTHON";

    /// Matches the `--break-system-packages` command-line option.
    /// When set to `true`, uv can install packages that conflict with system packages.
    ///
    /// WARNING: Use `UV_BREAK_SYSTEM_PACKAGES=true` carefully.
    /// It is intended for continuous integration (CI) or container environments.
    /// Changes to the system Python can cause unexpected behavior.
    #[attr_added_in("0.1.32")]
    pub const UV_BREAK_SYSTEM_PACKAGES: &'static str = "UV_BREAK_SYSTEM_PACKAGES";

    /// Matches the `--native-tls` command-line option.
    /// When set to `true`, uv loads TLS certificates from the platform certificate store.
    /// It does not use the bundled Mozilla root certificates.
    /// Deprecated: use `UV_SYSTEM_CERTS` instead.
    #[attr_added_in("0.1.19")]
    pub const UV_NATIVE_TLS: &'static str = "UV_NATIVE_TLS";

    /// Matches the `--system-certs` command-line option.
    /// When set to `true`, uv loads TLS certificates from the platform certificate store.
    /// It does not use the bundled Mozilla root certificates.
    #[attr_added_in("0.11.0")]
    pub const UV_SYSTEM_CERTS: &'static str = "UV_SYSTEM_CERTS";

    /// Matches the `--index-strategy` command-line option.
    ///
    /// When set to `unsafe-best-match`, uv considers package versions from all index URLs.
    /// It does not limit the search to the first index that contains the package.
    #[attr_added_in("0.1.29")]
    pub const UV_INDEX_STRATEGY: &'static str = "UV_INDEX_STRATEGY";

    /// Matches the `--require-hashes` command-line option.
    /// When set to `true`, uv requires a hash for each dependency in the requirements file.
    #[attr_added_in("0.1.34")]
    pub const UV_REQUIRE_HASHES: &'static str = "UV_REQUIRE_HASHES";

    /// Requires HTTP range requests to fetch wheel metadata when separate metadata is unavailable.
    /// When set to `true`, uv fails instead of downloading the entire wheel.
    #[attr_hidden]
    #[attr_added_in("0.12.8")]
    pub const UV_REQUIRE_METADATA_RANGE_REQUESTS: &'static str =
        "UV_REQUIRE_METADATA_RANGE_REQUESTS";

    /// Matches the `--constraints` command-line option.
    /// When set, uv uses these space-separated files as constraints files.
    #[attr_added_in("0.1.36")]
    pub const UV_CONSTRAINT: &'static str = "UV_CONSTRAINT";

    /// Matches the `--build-constraints` command-line option.
    /// When set, uv uses these space-separated files as constraints for source distribution builds.
    #[attr_added_in("0.2.34")]
    pub const UV_BUILD_CONSTRAINT: &'static str = "UV_BUILD_CONSTRAINT";

    /// Matches the `--overrides` command-line option.
    /// When set, uv uses these space-separated files as overrides files.
    #[attr_added_in("0.2.22")]
    pub const UV_OVERRIDE: &'static str = "UV_OVERRIDE";

    /// Matches the `--excludes` command-line option.
    /// When set, uv uses these space-separated files as excludes files.
    #[attr_added_in("0.9.8")]
    pub const UV_EXCLUDE: &'static str = "UV_EXCLUDE";

    /// Matches the `--link-mode` command-line option.
    /// When set, uv uses this link mode.
    #[attr_added_in("0.1.40")]
    pub const UV_LINK_MODE: &'static str = "UV_LINK_MODE";

    /// Matches the `--no-build-isolation` command-line option.
    /// When set, uv skips isolation for source distribution builds.
    #[attr_added_in("0.1.40")]
    pub const UV_NO_BUILD_ISOLATION: &'static str = "UV_NO_BUILD_ISOLATION";

    /// Matches the `--custom-compile-command` command-line option.
    ///
    /// Replaces uv in the header of `requirements.txt` files that `uv pip compile` generates.
    /// Use this option when a wrapper script runs `uv pip compile`.
    /// The output file then includes the wrapper script name.
    #[attr_added_in("0.1.23")]
    pub const UV_CUSTOM_COMPILE_COMMAND: &'static str = "UV_CUSTOM_COMPILE_COMMAND";

    /// Matches the `--keyring-provider` command-line option.
    /// When set, uv uses this keyring provider.
    #[attr_added_in("0.1.19")]
    pub const UV_KEYRING_PROVIDER: &'static str = "UV_KEYRING_PROVIDER";

    /// Matches the `--config-file` command-line option.
    /// Requires the path to a local `uv.toml` configuration file.
    #[attr_added_in("0.1.34")]
    pub const UV_CONFIG_FILE: &'static str = "UV_CONFIG_FILE";

    /// Matches the `--no-config` command-line option.
    /// When set, uv does not read configuration files from the current directory or its parents.
    /// It also ignores user configuration directories.
    #[attr_added_in("0.2.30")]
    pub const UV_NO_CONFIG: &'static str = "UV_NO_CONFIG";

    /// When set, uv does not read system-level configuration files.
    #[attr_added_in("0.11.16")]
    pub const UV_NO_SYSTEM_CONFIG: &'static str = "UV_NO_SYSTEM_CONFIG";

    /// Matches the `--isolated` command-line option.
    /// When set, uv does not discover `pyproject.toml` or `uv.toml` files.
    #[attr_added_in("0.8.14")]
    pub const UV_ISOLATED: &'static str = "UV_ISOLATED";

    /// Matches the `--exclude-newer` command-line option.
    /// When set, uv excludes distributions published after the specified date.
    /// Set the value to `false` to disable `exclude-newer`.
    #[attr_added_in("0.2.12")]
    pub const UV_EXCLUDE_NEWER: &'static str = "UV_EXCLUDE_NEWER";

    /// Controls whether uv prefers system or managed Python versions.
    #[attr_added_in("0.3.2")]
    pub const UV_PYTHON_PREFERENCE: &'static str = "UV_PYTHON_PREFERENCE";

    /// Requires uv-managed Python versions.
    #[attr_added_in("0.6.8")]
    pub const UV_MANAGED_PYTHON: &'static str = "UV_MANAGED_PYTHON";

    /// Disables uv-managed Python versions.
    #[attr_added_in("0.6.8")]
    pub const UV_NO_MANAGED_PYTHON: &'static str = "UV_NO_MANAGED_PYTHON";

    /// Matches the [`python-downloads`](../reference/settings.md#python-downloads) setting.
    /// When disabled, it matches the `--no-python-downloads` option.
    /// Controls whether uv allows Python downloads.
    #[attr_added_in("0.3.2")]
    pub const UV_PYTHON_DOWNLOADS: &'static str = "UV_PYTHON_DOWNLOADS";

    /// Overrides the libc that uv detects on Linux for Python version requests.
    /// Valid values are `gnu`, `gnueabi`, `gnueabihf`, `musl`, `musleabi`, `musleabihf`, and
    /// `none`.
    #[attr_added_in("0.7.22")]
    pub const UV_LIBC: &'static str = "UV_LIBC";

    /// Matches the `--compile-bytecode` command-line option.
    /// When set, uv compiles Python source files to bytecode after installation.
    #[attr_added_in("0.3.3")]
    pub const UV_COMPILE_BYTECODE: &'static str = "UV_COMPILE_BYTECODE";

    /// The bytecode compilation timeout, in seconds.
    #[attr_added_in("0.7.22")]
    pub const UV_COMPILE_BYTECODE_TIMEOUT: &'static str = "UV_COMPILE_BYTECODE_TIMEOUT";

    /// Matches the `--no-editable` command-line option.
    /// When set, uv installs or exports editable dependencies as non-editable.
    /// This includes the project and workspace members.
    #[attr_added_in("0.6.15")]
    pub const UV_NO_EDITABLE: &'static str = "UV_NO_EDITABLE";

    /// Matches the `--dev` command-line option.
    /// When set, uv includes development dependencies.
    #[attr_added_in("0.8.7")]
    pub const UV_DEV: &'static str = "UV_DEV";

    /// Matches the `--no-dev` command-line option.
    /// When set, uv excludes development dependencies.
    #[attr_added_in("0.8.7")]
    pub const UV_NO_DEV: &'static str = "UV_NO_DEV";

    /// Matches the `--no-group` command-line option.
    /// When set, uv disables the specified dependency groups for these space-separated packages.
    #[attr_added_in("0.9.8")]
    pub const UV_NO_GROUP: &'static str = "UV_NO_GROUP";

    /// Matches the `--no-default-groups` command-line option.
    /// When set, uv does not select the default groups in `tool.uv.default-groups`.
    #[attr_added_in("0.9.9")]
    pub const UV_NO_DEFAULT_GROUPS: &'static str = "UV_NO_DEFAULT_GROUPS";

    /// Matches the `--no-install-project` command-line option.
    /// When set, uv installs the project dependencies but not the project.
    #[attr_added_in("0.11.20")]
    pub const UV_NO_INSTALL_PROJECT: &'static str = "UV_NO_INSTALL_PROJECT";

    /// Matches the `--no-install-workspace` command-line option.
    /// When set, uv installs workspace dependencies but not workspace members.
    /// This includes the current project.
    #[attr_added_in("0.11.20")]
    pub const UV_NO_INSTALL_WORKSPACE: &'static str = "UV_NO_INSTALL_WORKSPACE";

    /// Matches the `--no-install-local` command-line option.
    /// When set, uv installs only remote dependencies.
    /// It skips the current project, workspace members, and other local path or editable packages.
    #[attr_added_in("0.11.20")]
    pub const UV_NO_INSTALL_LOCAL: &'static str = "UV_NO_INSTALL_LOCAL";

    /// Matches the hidden `--only-install-project` command-line option.
    #[attr_hidden]
    #[attr_added_in("0.11.20")]
    pub const UV_ONLY_INSTALL_PROJECT: &'static str = "UV_ONLY_INSTALL_PROJECT";

    /// Matches the hidden `--only-install-workspace` command-line option.
    #[attr_hidden]
    #[attr_added_in("0.11.20")]
    pub const UV_ONLY_INSTALL_WORKSPACE: &'static str = "UV_ONLY_INSTALL_WORKSPACE";

    /// Matches the hidden `--only-install-local` command-line option.
    #[attr_hidden]
    #[attr_added_in("0.11.20")]
    pub const UV_ONLY_INSTALL_LOCAL: &'static str = "UV_ONLY_INSTALL_LOCAL";

    /// Matches the `--no-binary` command-line option.
    /// When set, uv installs all packages from source.
    /// The resolver can still extract metadata from available pre-built wheels.
    #[attr_added_in("0.5.30")]
    pub const UV_NO_BINARY: &'static str = "UV_NO_BINARY";

    /// Matches the `--no-binary-package` command-line option.
    /// When set, uv does not use pre-built wheels for these space-separated packages.
    #[attr_added_in("0.5.30")]
    pub const UV_NO_BINARY_PACKAGE: &'static str = "UV_NO_BINARY_PACKAGE";

    /// Matches the `--no-build` command-line option.
    /// When set, uv does not build source distributions. uv still builds first-party packages,
    /// such as projects in the workspace.
    #[attr_added_in("0.1.40")]
    pub const UV_NO_BUILD: &'static str = "UV_NO_BUILD";

    /// Matches the `--no-build-package` command-line option.
    /// When set, uv does not build source distributions for these space-separated packages. uv
    /// still builds first-party packages, such as projects in the workspace.
    #[attr_added_in("0.6.5")]
    pub const UV_NO_BUILD_PACKAGE: &'static str = "UV_NO_BUILD_PACKAGE";

    /// Matches the `--no-sources-package` command-line option.
    /// When set, uv ignores `tool.uv.sources` for these space-separated packages.
    #[attr_added_in("0.9.26")]
    pub const UV_NO_SOURCES_PACKAGE: &'static str = "UV_NO_SOURCES_PACKAGE";

    /// Matches the `--publish-url` command-line option.
    /// Sets the index upload URL for `uv publish`.
    #[attr_added_in("0.4.16")]
    pub const UV_PUBLISH_URL: &'static str = "UV_PUBLISH_URL";

    /// Matches the `--token` option for `uv publish`.
    /// When set, uv publishes with this token and the username `__token__`.
    #[attr_added_in("0.4.16")]
    pub const UV_PUBLISH_TOKEN: &'static str = "UV_PUBLISH_TOKEN";

    /// Matches the `--index` option for `uv publish`.
    /// When set, uv publishes to the configured index with this name.
    #[attr_added_in("0.5.8")]
    pub const UV_PUBLISH_INDEX: &'static str = "UV_PUBLISH_INDEX";

    /// Matches the `--username` option for `uv publish`.
    /// When set, uv publishes with this username.
    #[attr_added_in("0.4.16")]
    pub const UV_PUBLISH_USERNAME: &'static str = "UV_PUBLISH_USERNAME";

    /// Matches the `--password` option for `uv publish`.
    /// When set, uv publishes with this password.
    #[attr_added_in("0.4.16")]
    pub const UV_PUBLISH_PASSWORD: &'static str = "UV_PUBLISH_PASSWORD";

    /// Matches the `--check-url` option for `uv publish`.
    /// Do not upload a file that already exists on the index.
    /// The value is the index URL.
    #[attr_added_in("0.4.30")]
    pub const UV_PUBLISH_CHECK_URL: &'static str = "UV_PUBLISH_CHECK_URL";

    /// Matches the `--no-attestations` option for `uv publish`.
    /// When set, uv does not upload attestations for published distributions.
    #[attr_added_in("0.9.12")]
    pub const UV_PUBLISH_NO_ATTESTATIONS: &'static str = "UV_PUBLISH_NO_ATTESTATIONS";

    /// Matches the `--no-sync` command-line option.
    /// When set, uv does not update the environment.
    #[attr_added_in("0.4.18")]
    pub const UV_NO_SYNC: &'static str = "UV_NO_SYNC";

    /// Matches the `--locked` command-line option.
    /// When set, uv requires `uv.lock` to remain unchanged.
    #[attr_added_in("0.4.25")]
    pub const UV_LOCKED: &'static str = "UV_LOCKED";

    /// Matches the `--frozen` command-line option.
    /// When set, uv does not update `uv.lock`.
    #[attr_added_in("0.4.25")]
    pub const UV_FROZEN: &'static str = "UV_FROZEN";

    /// Matches the `--preview` option. Enables preview mode.
    #[attr_added_in("0.1.37")]
    pub const UV_PREVIEW: &'static str = "UV_PREVIEW";

    /// Matches the `--preview-features` option. Enables specific preview features.
    #[attr_added_in("0.8.4")]
    pub const UV_PREVIEW_FEATURES: &'static str = "UV_PREVIEW_FEATURES";

    /// Matches the `--token` option for self update.
    /// Sets the GitHub authentication token.
    #[attr_added_in("0.4.10")]
    pub const UV_GITHUB_TOKEN: &'static str = "UV_GITHUB_TOKEN";

    /// Matches the `--no-verify-hashes` option.
    /// Disables hash checks for `requirements.txt` files.
    #[attr_added_in("0.5.3")]
    pub const UV_NO_VERIFY_HASHES: &'static str = "UV_NO_VERIFY_HASHES";

    /// Matches the `--allow-insecure-host` option.
    #[attr_added_in("0.3.5")]
    pub const UV_INSECURE_HOST: &'static str = "UV_INSECURE_HOST";

    /// Disables ZIP validation for streamed wheels and ZIP-based source distributions.
    ///
    /// WARNING: Do not disable ZIP validation unless necessary.
    /// This skips integrity checks and can let uv install malicious ZIP files.
    /// A ZIP file that fails validation is likely malformed.
    /// Report the issue to the package maintainer.
    #[attr_added_in("0.8.6")]
    pub const UV_INSECURE_NO_ZIP_VALIDATION: &'static str = "UV_INSECURE_NO_ZIP_VALIDATION";

    /// Sets the maximum number of concurrent downloads.
    #[attr_added_in("0.1.43")]
    pub const UV_CONCURRENT_DOWNLOADS: &'static str = "UV_CONCURRENT_DOWNLOADS";

    /// Sets the maximum number of concurrent source distribution builds.
    #[attr_added_in("0.1.43")]
    pub const UV_CONCURRENT_BUILDS: &'static str = "UV_CONCURRENT_BUILDS";

    /// Sets the number of threads that install and extract packages.
    #[attr_added_in("0.1.45")]
    pub const UV_CONCURRENT_INSTALLS: &'static str = "UV_CONCURRENT_INSTALLS";

    /// Sets the number of threads that read cached HTTP responses.
    #[attr_added_in("0.11.29")]
    pub const UV_CONCURRENT_CACHE_READS: &'static str = "UV_CONCURRENT_CACHE_READS";

    /// Matches the `--no-progress` command-line option.
    /// Disables all progress output, including spinners and progress bars.
    #[attr_added_in("0.2.28")]
    pub const UV_NO_PROGRESS: &'static str = "UV_NO_PROGRESS";

    /// Sets the directory where uv stores managed tools.
    #[attr_added_in("0.2.16")]
    pub const UV_TOOL_DIR: &'static str = "UV_TOOL_DIR";

    /// Sets the `bin` directory for installed tool executables.
    #[attr_added_in("0.3.0")]
    pub const UV_TOOL_BIN_DIR: &'static str = "UV_TOOL_BIN_DIR";

    /// Matches the `--bare` option for `uv init`.
    /// When set, uv creates only `pyproject.toml`.
    #[attr_added_in("0.10.7")]
    pub const UV_INIT_BARE: &'static str = "UV_INIT_BARE";

    /// Matches the `--build-backend` option for `uv init`.
    /// Sets the default build backend for new projects.
    #[attr_added_in("0.8.2")]
    pub const UV_INIT_BUILD_BACKEND: &'static str = "UV_INIT_BUILD_BACKEND";

    /// Sets the directory for the project virtual environment.
    ///
    /// See the [project documentation](../concepts/projects/config.md#project-environment-path)
    /// for details.
    #[attr_added_in("0.4.4")]
    pub const UV_PROJECT_ENVIRONMENT: &'static str = "UV_PROJECT_ENVIRONMENT";

    /// Sets the directory for links to installed managed Python executables.
    #[attr_added_in("0.4.29")]
    pub const UV_PYTHON_BIN_DIR: &'static str = "UV_PYTHON_BIN_DIR";

    /// Sets the directory for managed Python installations.
    #[attr_added_in("0.2.22")]
    pub const UV_PYTHON_INSTALL_DIR: &'static str = "UV_PYTHON_INSTALL_DIR";

    /// Controls whether uv installs the Python executable into `UV_PYTHON_BIN_DIR`.
    #[attr_added_in("0.8.0")]
    pub const UV_PYTHON_INSTALL_BIN: &'static str = "UV_PYTHON_INSTALL_BIN";

    /// Controls whether uv adds the Python executable to the Windows registry.
    #[attr_added_in("0.8.0")]
    pub const UV_PYTHON_INSTALL_REGISTRY: &'static str = "UV_PYTHON_INSTALL_REGISTRY";

    /// Disables the Windows registry for Python discovery and registration.
    ///
    /// When set, uv does not discover Python interpreters from the registry or Microsoft Store.
    /// It does not add managed Python installations to the Windows registry.
    #[attr_added_in("0.11.8")]
    pub const UV_PYTHON_NO_REGISTRY: &'static str = "UV_PYTHON_NO_REGISTRY";

    /// The `uv` binary contains a fixed list of managed Python installations.
    ///
    /// Set this variable to the local path or URL of a JSON installation list.
    /// This list replaces the built-in list.
    ///
    /// Use it to change download URLs or select Python versions outside the built-in list.
    #[attr_added_in("0.6.13")]
    pub const UV_PYTHON_DOWNLOADS_JSON_URL: &'static str = "UV_PYTHON_DOWNLOADS_JSON_URL";

    /// Sets the directory that caches managed Python archives before installation.
    #[attr_added_in("0.7.0")]
    pub const UV_PYTHON_CACHE_DIR: &'static str = "UV_PYTHON_CACHE_DIR";

    /// uv downloads managed Python installations from the Astral
    /// [`python-build-standalone`](https://github.com/astral-sh/python-build-standalone) project.
    ///
    /// Set this variable to a mirror URL to use a different Python source.
    /// The URL replaces
    /// `https://github.com/astral-sh/python-build-standalone/releases/download` in URLs such as
    /// `https://github.com/astral-sh/python-build-standalone/releases/download/20240713/cpython-3.12.4%2B20240713-aarch64-apple-darwin-install_only.tar.gz`.
    /// Use a `file://` URL to read distributions from a local directory.
    ///
    /// This specific mirror takes precedence over
    /// [`UV_ASTRAL_MIRROR_URL`](Self::UV_ASTRAL_MIRROR_URL) for CPython downloads.
    #[attr_added_in("0.2.35")]
    pub const UV_PYTHON_INSTALL_MIRROR: &'static str = "UV_PYTHON_INSTALL_MIRROR";

    /// uv downloads managed PyPy installations from [python.org](https://downloads.python.org/).
    ///
    /// Set this variable to a mirror URL to use a different PyPy source.
    /// The URL replaces `https://downloads.python.org/pypy` in URLs such as
    /// `https://downloads.python.org/pypy/pypy3.8-v7.3.7-osx64.tar.bz2`.
    /// Use a `file://` URL to read distributions from a local directory.
    #[attr_added_in("0.2.35")]
    pub const UV_PYPY_INSTALL_MIRROR: &'static str = "UV_PYPY_INSTALL_MIRROR";

    /// Replaces `https://releases.astral.sh` for Astral-mirrored metadata and artifact downloads.
    ///
    /// When set, uv uses only the configured mirror URL.
    /// It does not fall back to GitHub or raw GitHub.
    /// uv preserves URL path components and removes only trailing slashes.
    /// It then adds the normal path suffix, such as `/github/versions/main/v1/uv.ndjson`.
    ///
    /// Use this option with proxy repositories, such as Artifactory or Nexus.
    /// These repositories mirror `releases.astral.sh`.
    ///
    /// More specific sources take precedence.
    /// [`UV_PYTHON_INSTALL_MIRROR`](Self::UV_PYTHON_INSTALL_MIRROR) and
    /// `python-install-mirror` override this variable for CPython downloads.
    /// [`UV_INSTALLER_GITHUB_BASE_URL`](Self::UV_INSTALLER_GITHUB_BASE_URL) and
    /// [`UV_INSTALLER_GHE_BASE_URL`](Self::UV_INSTALLER_GHE_BASE_URL) override this
    /// variable for `uv self update`.
    #[attr_added_in("0.11.14")]
    pub(crate) const UV_ASTRAL_MIRROR_URL: &'static str = "UV_ASTRAL_MIRROR_URL";

    /// Pins managed CPython versions to a specific build.
    ///
    /// For CPython, use the build date, such as `20250814`.
    #[attr_added_in("0.8.14")]
    pub const UV_PYTHON_CPYTHON_BUILD: &'static str = "UV_PYTHON_CPYTHON_BUILD";

    /// Pins managed PyPy versions to a specific build.
    ///
    /// For PyPy, use the PyPy version, such as `7.3.20`.
    #[attr_added_in("0.8.14")]
    pub const UV_PYTHON_PYPY_BUILD: &'static str = "UV_PYTHON_PYPY_BUILD";

    /// Pins managed GraalPy versions to a specific build.
    ///
    /// For GraalPy, use the GraalPy version, such as `24.2.2`.
    #[attr_added_in("0.8.14")]
    pub const UV_PYTHON_GRAALPY_BUILD: &'static str = "UV_PYTHON_GRAALPY_BUILD";

    /// Pins managed Pyodide versions to a specific build.
    ///
    /// For Pyodide, use the Pyodide version, such as `0.28.1`.
    #[attr_added_in("0.8.14")]
    pub const UV_PYTHON_PYODIDE_BUILD: &'static str = "UV_PYTHON_PYODIDE_BUILD";

    /// Matches the `--clear` command-line option.
    /// When set, uv removes existing files or directories at the target path.
    #[attr_added_in("0.8.0")]
    pub const UV_VENV_CLEAR: &'static str = "UV_VENV_CLEAR";

    /// Matches the `--relocatable` command-line option.
    /// When set, uv creates a relocatable virtual environment.
    #[attr_added_in("0.10.8")]
    pub const UV_VENV_RELOCATABLE: &'static str = "UV_VENV_RELOCATABLE";

    /// Installs seed packages into the virtual environment that `uv venv` creates.
    /// Seed packages include `pip`, `setuptools`, and `wheel`.
    ///
    /// Python 3.12 and later environments do not include `setuptools` or `wheel`.
    #[attr_added_in("0.5.21")]
    pub const UV_VENV_SEED: &'static str = "UV_VENV_SEED";

    /// Overrides `PATH` for Python executable discovery.
    ///
    /// When set, uv searches these directories for Python interpreters instead of `PATH`.
    #[attr_added_in("0.11.8")]
    pub const UV_PYTHON_SEARCH_PATH: &'static str = "UV_PYTHON_SEARCH_PATH";

    /// Includes resolver and installer output about environment changes.
    #[attr_hidden]
    #[attr_added_in("0.2.32")]
    pub const UV_SHOW_RESOLUTION: &'static str = "UV_SHOW_RESOLUTION";

    /// Updates JSON schema files.
    #[attr_hidden]
    #[attr_added_in("0.1.34")]
    pub const UV_UPDATE_SCHEMA: &'static str = "UV_UPDATE_SCHEMA";

    /// Disables line wrapping for diagnostics.
    #[attr_added_in("0.0.5")]
    pub const UV_NO_WRAP: &'static str = "UV_NO_WRAP";

    /// Set to `1` to enable the automatic malware check that runs after `uv sync`.
    ///
    /// When enabled, uv checks the OSV database for known malware advisories after each lockfile
    /// sync.
    /// Set this variable to `0` to disable the check.
    #[attr_added_in("0.11.16")]
    pub const UV_MALWARE_CHECK: &'static str = "UV_MALWARE_CHECK";

    /// Overrides the vulnerability service URL for the automatic malware check.
    ///
    /// The default is the OSV API endpoint, `https://api.osv.dev/`.
    #[attr_added_in("0.11.16")]
    pub const UV_MALWARE_CHECK_URL: &'static str = "UV_MALWARE_CHECK_URL";

    /// Sets the HTTP Basic authentication username for a named index.
    ///
    /// The `name` parameter identifies the index.
    /// For an index named `foo`, the variable is `UV_INDEX_FOO_USERNAME`.
    #[attr_added_in("0.4.23")]
    #[attr_env_var_pattern("UV_INDEX_{name}_USERNAME")]
    pub fn index_username(name: &str) -> String {
        format!("UV_INDEX_{name}_USERNAME")
    }

    /// Sets the HTTP Basic authentication password for a named index.
    ///
    /// The `name` parameter identifies the index.
    /// For an index named `foo`, the variable is `UV_INDEX_FOO_PASSWORD`.
    #[attr_added_in("0.4.23")]
    #[attr_env_var_pattern("UV_INDEX_{name}_PASSWORD")]
    pub fn index_password(name: &str) -> String {
        format!("UV_INDEX_{name}_PASSWORD")
    }

    /// Sets the uv commit hash through `build.rs` at build time.
    #[attr_hidden]
    #[attr_added_in("0.1.11")]
    pub const UV_COMMIT_HASH: &'static str = "UV_COMMIT_HASH";

    /// Sets the short uv commit hash through `build.rs` at build time.
    #[attr_hidden]
    #[attr_added_in("0.1.11")]
    pub const UV_COMMIT_SHORT_HASH: &'static str = "UV_COMMIT_SHORT_HASH";

    /// Sets the uv commit date through `build.rs` at build time.
    #[attr_hidden]
    #[attr_added_in("0.1.11")]
    pub const UV_COMMIT_DATE: &'static str = "UV_COMMIT_DATE";

    /// Sets the uv tag through `build.rs` at build time.
    #[attr_hidden]
    #[attr_added_in("0.1.11")]
    pub const UV_LAST_TAG: &'static str = "UV_LAST_TAG";

    /// Sets the distance from the uv tag to the head through `build.rs` at build time.
    #[attr_hidden]
    #[attr_added_in("0.1.11")]
    pub const UV_LAST_TAG_DISTANCE: &'static str = "UV_LAST_TAG_DISTANCE";

    /// Sets the parent interpreter for `--system` in the test suite.
    #[attr_hidden]
    #[attr_added_in("0.2.0")]
    pub const UV_INTERNAL__PARENT_INTERPRETER: &'static str = "UV_INTERNAL__PARENT_INTERPRETER";

    /// Identifies the source tree for PEP 517 build hooks.
    #[attr_hidden]
    #[attr_added_in("0.11.22")]
    pub const UV_INTERNAL__BUILD_DIR: &'static str = "UV_INTERNAL__BUILD_DIR";

    /// Shows the derivation tree in resolver errors.
    #[attr_hidden]
    #[attr_added_in("0.3.0")]
    pub const UV_INTERNAL__SHOW_DERIVATION_TREE: &'static str = "UV_INTERNAL__SHOW_DERIVATION_TREE";

    /// Sets a temporary directory for some tests.
    #[attr_hidden]
    #[attr_added_in("0.3.4")]
    pub const UV_INTERNAL__TEST_DIR: &'static str = "UV_INTERNAL__TEST_DIR";

    /// The path to a directory on a filesystem that supports copy-on-write, such as btrfs or APFS.
    ///
    /// When set, uv runs additional tests that require copy-on-write.
    #[attr_hidden]
    #[attr_added_in("0.10.5")]
    pub const UV_INTERNAL__TEST_COW_FS: &'static str = "UV_INTERNAL__TEST_COW_FS";

    /// The path to a directory on a filesystem that does **not** support copy-on-write.
    ///
    /// When set, uv runs additional tests that check behavior without copy-on-write.
    #[attr_hidden]
    #[attr_added_in("0.10.5")]
    pub const UV_INTERNAL__TEST_NOCOW_FS: &'static str = "UV_INTERNAL__TEST_NOCOW_FS";

    /// The path to a test directory on a different filesystem.
    ///
    /// This filesystem must use a different device than the default test filesystem.
    ///
    /// When set, uv runs additional tests for links between filesystems.
    #[attr_hidden]
    #[attr_added_in("0.10.5")]
    pub const UV_INTERNAL__TEST_ALT_FS: &'static str = "UV_INTERNAL__TEST_ALT_FS";

    /// The network path to a test directory on an SMB filesystem.
    ///
    /// When set, uv runs additional tests for SMB-specific filesystem behavior.
    #[attr_hidden]
    #[attr_added_in("0.11.16")]
    pub const UV_INTERNAL__TEST_SMB_FS: &'static str = "UV_INTERNAL__TEST_SMB_FS";

    /// The path to a filesystem with a low hardlink limit, such as minix with approximately 250.
    ///
    /// When set, uv runs additional tests for EMLINK recovery.
    #[attr_hidden]
    #[attr_added_in("0.10.9")]
    pub const UV_INTERNAL__TEST_LOWLINKS_FS: &'static str = "UV_INTERNAL__TEST_LOWLINKS_FS";

    /// Forces tests to treat an interpreter as managed.
    #[attr_hidden]
    #[attr_added_in("0.8.0")]
    pub const UV_INTERNAL__TEST_PYTHON_MANAGED: &'static str = "UV_INTERNAL__TEST_PYTHON_MANAGED";

    /// Forces tests to ignore Git LFS commands.
    /// `PATH` cannot override `git-lfs` detection.
    #[attr_hidden]
    #[attr_added_in("0.9.15")]
    pub const UV_INTERNAL__TEST_LFS_DISABLED: &'static str = "UV_INTERNAL__TEST_LFS_DISABLED";

    /// The path to the system configuration directory on Unix.
    #[attr_added_in("0.4.26")]
    pub const XDG_CONFIG_DIRS: &'static str = "XDG_CONFIG_DIRS";

    /// The path to the system configuration directory on Windows.
    #[attr_added_in("0.4.26")]
    pub const SYSTEMDRIVE: &'static str = "SYSTEMDRIVE";

    /// The path to the user configuration directory on Windows.
    #[attr_added_in("0.1.42")]
    pub const APPDATA: &'static str = "APPDATA";

    /// The path to the user profile root directory on Windows.
    #[attr_added_in("0.0.5")]
    pub const USERPROFILE: &'static str = "USERPROFILE";

    /// The path to the user configuration directory on Unix.
    #[attr_added_in("0.1.34")]
    pub const XDG_CONFIG_HOME: &'static str = "XDG_CONFIG_HOME";

    /// The path to the cache directory on Unix.
    #[attr_added_in("0.1.17")]
    pub const XDG_CACHE_HOME: &'static str = "XDG_CACHE_HOME";

    /// The path to the directory for managed Python installations and tools.
    #[attr_added_in("0.2.16")]
    pub const XDG_DATA_HOME: &'static str = "XDG_DATA_HOME";

    /// The path to the directory for installed executables.
    #[attr_added_in("0.2.16")]
    pub const XDG_BIN_HOME: &'static str = "XDG_BIN_HOME";

    /// The path to a CA certificate bundle for TLS connections.
    ///
    /// Use a PEM-encoded certificate file, such as `certs.pem` or `ca-bundle.crt`.
    /// uv does not support DER-encoded files.
    ///
    /// When set, this replaces the bundled Mozilla roots or system certificates.
    /// uv trusts only the certificates in this file.
    #[attr_added_in("0.1.14")]
    pub const SSL_CERT_FILE: &'static str = "SSL_CERT_FILE";

    /// The path to a directory of PEM-encoded CA certificates for TLS connections.
    ///
    /// To add multiple directories, separate entries with `:` on Unix or `;` on Windows.
    ///
    /// Certificates usually use `.pem`, `.crt`, or `.cer` extensions.
    /// uv tries to read a certificate from each regular file in `SSL_CERT_DIR`.
    ///
    /// uv ignores files that are not valid PEM certificates.
    /// It resolves symbolic links and ignores links without targets.
    ///
    /// uv supports only PEM-encoded files. It does not support DER-encoded files.
    ///
    /// When set, this replaces the bundled Mozilla roots or system certificates.
    /// uv trusts only the certificates in this directory.
    #[attr_added_in("0.9.10")]
    pub const SSL_CERT_DIR: &'static str = "SSL_CERT_DIR";

    /// When set, uv uses this file for mTLS authentication.
    /// The PEM-encoded file must contain the certificate and the private key.
    #[attr_added_in("0.2.11")]
    pub const SSL_CLIENT_CERT: &'static str = "SSL_CLIENT_CERT";

    /// Sets the proxy for HTTP requests.
    #[attr_added_in("0.1.38")]
    pub const HTTP_PROXY: &'static str = "HTTP_PROXY";

    /// Sets the proxy for HTTPS requests.
    #[attr_added_in("0.1.38")]
    pub const HTTPS_PROXY: &'static str = "HTTPS_PROXY";

    /// Sets the proxy for all network requests.
    #[attr_added_in("0.1.38")]
    pub const ALL_PROXY: &'static str = "ALL_PROXY";

    /// Lists comma-separated hostnames or patterns that bypass the proxy.
    /// Examples include `example.com` and `192.168.1.0/24`.
    #[attr_added_in("0.1.38")]
    pub const NO_PROXY: &'static str = "NO_PROXY";

    /// The HTTP upload timeout, in seconds. The default is 900 seconds.
    #[attr_added_in("0.9.1")]
    pub const UV_UPLOAD_HTTP_TIMEOUT: &'static str = "UV_UPLOAD_HTTP_TIMEOUT";

    /// The HTTP read timeout, in seconds. The default is 30 seconds.
    #[attr_added_in("0.1.7")]
    pub const UV_HTTP_TIMEOUT: &'static str = "UV_HTTP_TIMEOUT";

    /// The server connection timeout, in seconds. The default is 10 seconds.
    ///
    /// If `UV_HTTP_TIMEOUT` is lower, uv uses that value instead.
    #[attr_added_in("0.10.0")]
    pub const UV_HTTP_CONNECT_TIMEOUT: &'static str = "UV_HTTP_CONNECT_TIMEOUT";

    /// The number of retries for HTTP requests. The default is 3.
    #[attr_added_in("0.7.21")]
    pub const UV_HTTP_RETRIES: &'static str = "UV_HTTP_RETRIES";

    /// The HTTP request timeout, in seconds. Matches `UV_HTTP_TIMEOUT`.
    #[attr_added_in("0.1.6")]
    pub const UV_REQUEST_TIMEOUT: &'static str = "UV_REQUEST_TIMEOUT";

    /// The HTTP request timeout, in seconds. Matches `UV_HTTP_TIMEOUT`.
    #[attr_added_in("0.1.7")]
    pub const HTTP_TIMEOUT: &'static str = "HTTP_TIMEOUT";

    /// Sets the validation modes for `--compile`.
    ///
    /// See [`PycInvalidationMode`](https://docs.python.org/3/library/py_compile.html#py_compile.PycInvalidationMode).
    #[attr_added_in("0.1.7")]
    pub const PYC_INVALIDATION_MODE: &'static str = "PYC_INVALIDATION_MODE";

    /// Detects an active virtual environment.
    #[attr_added_in("0.0.5")]
    pub const VIRTUAL_ENV: &'static str = "VIRTUAL_ENV";

    /// Detects the path to the active Conda environment.
    #[attr_added_in("0.0.5")]
    pub const CONDA_PREFIX: &'static str = "CONDA_PREFIX";

    /// Identifies the active Conda environment.
    #[attr_added_in("0.5.0")]
    pub const CONDA_DEFAULT_ENV: &'static str = "CONDA_DEFAULT_ENV";

    /// Identifies the Conda installation root.
    #[attr_added_in("0.8.18")]
    pub const CONDA_ROOT: &'static str = "_CONDA_ROOT";

    /// Detects whether uv runs in Dependabot.
    #[attr_added_in("0.9.11")]
    pub const DEPENDABOT: &'static str = "DEPENDABOT";

    /// Set to `1` before virtual environment activation to hide its name from the terminal prompt.
    #[attr_added_in("0.0.5")]
    pub const VIRTUAL_ENV_DISABLE_PROMPT: &'static str = "VIRTUAL_ENV_DISABLE_PROMPT";

    /// Detects Windows Command Prompt instead of PowerShell.
    #[attr_added_in("0.1.16")]
    pub const PROMPT: &'static str = "PROMPT";

    /// Detects `NuShell`.
    #[attr_added_in("0.1.16")]
    pub const NU_VERSION: &'static str = "NU_VERSION";

    /// Detects the Fish shell.
    #[attr_added_in("0.1.28")]
    pub const FISH_VERSION: &'static str = "FISH_VERSION";

    /// Detects the Bash shell.
    #[attr_added_in("0.1.28")]
    pub const BASH_VERSION: &'static str = "BASH_VERSION";

    /// Detects the Zsh shell.
    #[attr_added_in("0.1.28")]
    pub const ZSH_VERSION: &'static str = "ZSH_VERSION";

    /// Identifies the `.zshenv` file for Zsh.
    #[attr_added_in("0.2.25")]
    pub const ZDOTDIR: &'static str = "ZDOTDIR";

    /// Detects the Ksh shell.
    #[attr_added_in("0.2.33")]
    pub const KSH_VERSION: &'static str = "KSH_VERSION";

    /// Detects PowerShell. PowerShell sets this variable on all platforms.
    #[attr_added_in("0.10.0")]
    pub const PS_MODULE_PATH: &'static str = "PSModulePath";

    /// Sets the minimum supported macOS version for `--python-platform macos` and related variants.
    ///
    /// The default is `13.0`.
    /// This was the oldest macOS version that had not reached end of life.
    #[attr_added_in("0.1.42")]
    pub const MACOSX_DEPLOYMENT_TARGET: &'static str = "MACOSX_DEPLOYMENT_TARGET";

    /// Sets the minimum supported iOS version for `--python-platform arm64-apple-ios` and related
    /// variants.
    ///
    /// The default is `13.0`.
    #[attr_added_in("0.8.16")]
    pub const IPHONEOS_DEPLOYMENT_TARGET: &'static str = "IPHONEOS_DEPLOYMENT_TARGET";

    /// Sets the minimum Android API level for `--python-platform aarch64-linux-android` and related
    /// variants.
    ///
    /// The default is `24`.
    #[attr_added_in("0.8.16")]
    pub const ANDROID_API_LEVEL: &'static str = "ANDROID_API_LEVEL";

    /// Disables colored output. Takes precedence over `FORCE_COLOR`.
    ///
    /// See [no-color.org](https://no-color.org).
    #[attr_added_in("0.2.7")]
    pub const NO_COLOR: &'static str = "NO_COLOR";

    /// Forces colored output even when the terminal does not support it.
    ///
    /// See [force-color.org](https://force-color.org).
    #[attr_added_in("0.2.7")]
    pub const FORCE_COLOR: &'static str = "FORCE_COLOR";

    /// Controls color through `anstyle`.
    #[attr_added_in("0.1.32")]
    pub const CLICOLOR_FORCE: &'static str = "CLICOLOR_FORCE";

    /// The standard `PATH` environment variable.
    #[attr_added_in("0.0.5")]
    pub const PATH: &'static str = "PATH";

    /// The standard `HOME` environment variable.
    #[attr_added_in("0.0.5")]
    pub const HOME: &'static str = "HOME";

    /// The standard POSIX `SHELL` environment variable.
    #[attr_added_in("0.1.16")]
    pub const SHELL: &'static str = "SHELL";

    /// The standard POSIX `PWD` environment variable.
    #[attr_added_in("0.0.5")]
    pub const PWD: &'static str = "PWD";

    /// Locates Python installations from the Microsoft Store.
    #[attr_added_in("0.3.3")]
    pub const LOCALAPPDATA: &'static str = "LOCALAPPDATA";

    /// The path to the `.git` directory. uv ignores this variable during fetches.
    #[attr_hidden]
    #[attr_added_in("0.0.5")]
    pub const GIT_DIR: &'static str = "GIT_DIR";

    /// The path to the Git working tree. uv ignores this variable during fetches.
    #[attr_hidden]
    #[attr_added_in("0.0.5")]
    pub const GIT_WORK_TREE: &'static str = "GIT_WORK_TREE";

    /// The path to the index of staged changes. uv ignores this variable during fetches.
    #[attr_hidden]
    #[attr_added_in("0.0.5")]
    pub const GIT_INDEX_FILE: &'static str = "GIT_INDEX_FILE";

    /// The path to Git objects. uv ignores this variable during fetches.
    #[attr_hidden]
    #[attr_added_in("0.0.5")]
    pub const GIT_OBJECT_DIRECTORY: &'static str = "GIT_OBJECT_DIRECTORY";

    /// Alternate locations for Git objects. uv ignores this variable during fetches.
    #[attr_hidden]
    #[attr_added_in("0.0.5")]
    pub const GIT_ALTERNATE_OBJECT_DIRECTORIES: &'static str = "GIT_ALTERNATE_OBJECT_DIRECTORIES";

    /// Disables SSL verification for Git operations.
    #[attr_hidden]
    #[attr_added_in("0.5.28")]
    pub const GIT_SSL_NO_VERIFY: &'static str = "GIT_SSL_NO_VERIFY";

    /// Sets the allowed protocols for Git operations.
    ///
    /// In offline mode, uv allows only the `file` protocol.
    #[attr_hidden]
    #[attr_added_in("0.6.13")]
    pub const GIT_ALLOW_PROTOCOL: &'static str = "GIT_ALLOW_PROTOCOL";

    /// Sets the SSH command that Git uses for SSH connections.
    #[attr_hidden]
    #[attr_added_in("0.7.11")]
    pub const GIT_SSH_COMMAND: &'static str = "GIT_SSH_COMMAND";

    /// Disables interactive Git terminal prompts, including credential prompts.
    /// Does not disable graphical prompts.
    #[attr_hidden]
    #[attr_added_in("0.6.4")]
    pub const GIT_TERMINAL_PROMPT: &'static str = "GIT_TERMINAL_PROMPT";

    /// Skips the Git LFS smudge filter.
    #[attr_hidden]
    #[attr_added_in("0.9.15")]
    pub const GIT_LFS_SKIP_SMUDGE: &'static str = "GIT_LFS_SKIP_SMUDGE";

    /// Sets the global user Git configuration path for tests.
    #[attr_hidden]
    #[attr_added_in("0.9.15")]
    pub const GIT_CONFIG_GLOBAL: &'static str = "GIT_CONFIG_GLOBAL";

    /// Isolates Git repositories during tests.
    ///
    /// Some tests run in `~/.local/share/uv/tests`.
    /// If `$HOME` is a Git repository, it can change those tests.
    /// Set `GIT_CEILING_DIRECTORIES=/home/andrew/.local/share/uv/tests`
    /// to stop Git from searching parent directories for repositories.
    #[attr_hidden]
    #[attr_added_in("0.4.29")]
    pub const GIT_CEILING_DIRECTORIES: &'static str = "GIT_CEILING_DIRECTORIES";

    /// uv clears this variable for Git commands to prevent unexpected behavior.
    #[attr_hidden]
    #[attr_added_in("0.11.8")]
    pub const GIT_COMMON_DIR: &'static str = "GIT_COMMON_DIR";

    /// Identifies a process that runs in GitHub Actions.
    ///
    /// When set to `true`, `uv publish` can attempt trusted publishing.
    #[attr_added_in("0.4.16")]
    pub const GITHUB_ACTIONS: &'static str = "GITHUB_ACTIONS";

    /// Identifies a process that runs in GitLab CI.
    ///
    /// When set to `true`, `uv publish` can attempt trusted publishing.
    #[attr_added_in("0.8.18")]
    pub const GITLAB_CI: &'static str = "GITLAB_CI";

    /// Tests trusted publishing through GitLab CI.
    #[attr_hidden]
    #[attr_added_in("0.8.18")]
    pub const PYPI_ID_TOKEN: &'static str = "PYPI_ID_TOKEN";

    /// Tests trusted publishing through GitLab CI.
    #[attr_hidden]
    #[attr_added_in("0.8.18")]
    pub const TESTPYPI_ID_TOKEN: &'static str = "TESTPYPI_ID_TOKEN";

    /// Sets the encoding for standard I/O streams, such as `PYTHONIOENCODING=utf-8`.
    #[attr_hidden]
    #[attr_added_in("0.4.18")]
    pub const PYTHONIOENCODING: &'static str = "PYTHONIOENCODING";

    /// Forces unbuffered I/O streams. Matches the Python `-u` option.
    #[attr_hidden]
    #[attr_added_in("0.1.15")]
    pub const PYTHONUNBUFFERED: &'static str = "PYTHONUNBUFFERED";

    /// Enables UTF-8 mode for Python. Matches the `-X utf8` option.
    #[attr_hidden]
    #[attr_added_in("0.4.19")]
    pub const PYTHONUTF8: &'static str = "PYTHONUTF8";

    /// Adds directories to the Python module search path.
    /// For example, set `PYTHONPATH=/path/to/modules`.
    #[attr_added_in("0.1.22")]
    pub const PYTHONPATH: &'static str = "PYTHONPATH";

    /// Sets the Python standard library location for trampolines.
    #[attr_hidden]
    #[attr_added_in("0.7.13")]
    pub const PYTHONHOME: &'static str = "PYTHONHOME";

    /// Overrides the executable Python uses to determine its environment.
    #[attr_hidden]
    #[attr_added_in("0.12.4")]
    pub const PYTHONEXECUTABLE: &'static str = "PYTHONEXECUTABLE";

    /// Detects virtual environments when uv uses trampolines.
    #[attr_hidden]
    #[attr_added_in("0.7.13")]
    pub const PYVENV_LAUNCHER: &'static str = "__PYVENV_LAUNCHER__";

    /// Sets a consistent locale for tests.
    #[attr_hidden]
    #[attr_added_in("0.4.28")]
    pub const LC_ALL: &'static str = "LC_ALL";

    /// Detects a CI runner. CI runners usually set this variable.
    #[attr_hidden]
    #[attr_added_in("0.0.5")]
    pub const CI: &'static str = "CI";

    /// The Azure DevOps build identifier. Detects a CI environment.
    #[attr_hidden]
    #[attr_added_in("0.1.22")]
    pub const BUILD_BUILDID: &'static str = "BUILD_BUILDID";

    /// A generic build identifier. Detects a CI environment.
    #[attr_hidden]
    #[attr_added_in("0.1.22")]
    pub const BUILD_ID: &'static str = "BUILD_ID";

    /// The pip environment variable that identifies a CI environment.
    #[attr_hidden]
    #[attr_added_in("0.1.22")]
    pub const PIP_IS_CI: &'static str = "PIP_IS_CI";

    /// Sets the `.netrc` file location.
    #[attr_added_in("0.1.16")]
    pub const NETRC: &'static str = "NETRC";

    /// The standard POSIX `PAGER` environment variable. uv uses it to select a pager.
    #[attr_added_in("0.4.18")]
    pub const PAGER: &'static str = "PAGER";

    /// Detects whether uv runs in a Jupyter notebook.
    #[attr_added_in("0.2.6")]
    pub const JPY_SESSION_NAME: &'static str = "JPY_SESSION_NAME";

    /// Creates the tracing root directory for the `tracing-durations-export` feature.
    #[attr_hidden]
    #[attr_added_in("0.1.32")]
    pub const TRACING_DURATIONS_TEST_ROOT: &'static str = "TRACING_DURATIONS_TEST_ROOT";

    /// Creates the tracing durations file for the `tracing-durations-export` feature.
    #[attr_added_in("0.0.5")]
    pub const TRACING_DURATIONS_FILE: &'static str = "TRACING_DURATIONS_FILE";

    /// Sets `RUST_HOST_TARGET` through `build.rs` at build time.
    #[attr_hidden]
    #[attr_added_in("0.1.11")]
    pub const TARGET: &'static str = "TARGET";

    /// When set, uv uses this value as the log level for `--verbose` output.
    /// Accepts any filter that the `tracing_subscriber` crate supports.
    ///
    /// For example:
    ///
    /// * `RUST_LOG=uv=debug` matches the `--verbose` command-line option.
    /// * `RUST_LOG=trace` enables trace-level logging.
    ///
    /// See the [tracing documentation](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#example-syntax)
    /// for details.
    #[attr_added_in("0.0.5")]
    pub const RUST_LOG: &'static str = "RUST_LOG";

    /// When set, displays more stack trace details after a panic.
    /// On Windows, uv uses this variable to show details about platform exceptions.
    ///
    /// For example:
    ///
    /// * `RUST_BACKTRACE=1` prints a short backtrace.
    /// * `RUST_BACKTRACE=full` prints a full backtrace.
    ///
    /// See the [Rust backtrace documentation](https://doc.rust-lang.org/std/backtrace/index.html)
    /// for details.
    #[attr_added_in("0.7.22")]
    pub const RUST_BACKTRACE: &'static str = "RUST_BACKTRACE";

    /// Adds context and structure to log messages.
    ///
    /// This has no effect unless `RUST_LOG`, `-v`, or another option enables logging.
    #[attr_added_in("0.6.4")]
    pub const UV_LOG_CONTEXT: &'static str = "UV_LOG_CONTEXT";

    /// Sets the stack size that uv uses, in bytes.
    ///
    /// If `UV_STACK_SIZE` and `RUST_MIN_STACK` are unset, uv uses a 4 MB (4194304 byte) stack.
    /// `UV_STACK_SIZE` takes precedence over `RUST_MIN_STACK`.
    ///
    /// This variable can also change the main thread stack size.
    /// uv starts a separate `main2` thread because the Windows main thread has only 1 MB.
    /// The `main2` stack size is `max(UV_STACK_SIZE, 1MB)`.
    #[attr_added_in("0.0.5")]
    pub const UV_STACK_SIZE: &'static str = "UV_STACK_SIZE";

    /// Sets the stack size that uv uses, in bytes.
    ///
    /// If `UV_STACK_SIZE` and `RUST_MIN_STACK` are unset, uv uses a 4 MB (4194304 byte) stack.
    /// `UV_STACK_SIZE` takes precedence over `RUST_MIN_STACK`.
    ///
    /// Prefer `UV_STACK_SIZE` because `RUST_MIN_STACK` also affects subprocesses.
    /// This includes build backends that use Rust.
    ///
    /// This variable can also change the main thread stack size.
    /// uv starts a separate `main2` thread because the Windows main thread has only 1 MB.
    /// The `main2` stack size is `max(RUST_MIN_STACK, 1MB)`.
    #[attr_added_in("0.5.19")]
    pub const RUST_MIN_STACK: &'static str = "RUST_MIN_STACK";

    /// The directory that contains a package `Cargo.toml` manifest.
    #[attr_hidden]
    #[attr_added_in("0.1.11")]
    pub const CARGO_MANIFEST_DIR: &'static str = "CARGO_MANIFEST_DIR";

    /// Sets the target directory where Cargo stores build artifacts.
    #[attr_hidden]
    #[attr_added_in("0.0.5")]
    pub const CARGO_TARGET_DIR: &'static str = "CARGO_TARGET_DIR";

    /// Cargo sets this variable for Windows-like build targets.
    #[attr_hidden]
    #[attr_added_in("0.0.5")]
    pub const CARGO_CFG_WINDOWS: &'static str = "CARGO_CFG_WINDOWS";

    /// Sets the directory where Cargo stores intermediate build artifacts.
    #[attr_hidden]
    #[attr_added_in("0.8.25")]
    pub const OUT_DIR: &'static str = "OUT_DIR";

    /// Tests environment variable substitution in `requirements.in`.
    #[attr_hidden]
    #[attr_added_in("0.1.18")]
    pub const URL: &'static str = "URL";

    /// Tests environment variable substitution in `requirements.in`.
    #[attr_hidden]
    #[attr_added_in("0.1.18")]
    pub const FILE_PATH: &'static str = "FILE_PATH";

    /// Tests environment variable substitution in `requirements.in`.
    #[attr_hidden]
    #[attr_added_in("0.1.25")]
    pub const HATCH_PATH: &'static str = "HATCH_PATH";

    /// Tests environment variable substitution in `requirements.in`.
    #[attr_hidden]
    #[attr_added_in("0.1.25")]
    pub const BLACK_PATH: &'static str = "BLACK_PATH";

    /// Tests the Hatch `root.uri` feature.
    ///
    /// See <https://hatch.pypa.io/dev/config/dependency/#local>.
    #[attr_hidden]
    #[attr_added_in("0.1.22")]
    pub const ROOT_PATH: &'static str = "ROOT_PATH";

    /// Tests extra build dependencies.
    #[attr_hidden]
    #[attr_added_in("0.8.5")]
    pub const EXPECTED_ANYIO_VERSION: &'static str = "EXPECTED_ANYIO_VERSION";

    /// Sets credentials for keyring tests.
    #[attr_hidden]
    #[attr_added_in("0.1.34")]
    pub const KEYRING_TEST_CREDENTIALS: &'static str = "KEYRING_TEST_CREDENTIALS";

    /// Disables HTTP retry delays in tests.
    #[attr_added_in("0.7.21")]
    pub const UV_TEST_NO_HTTP_RETRY_DELAY: &'static str = "UV_TEST_NO_HTTP_RETRY_DELAY";

    /// Tests authentication for named indexes.
    #[attr_hidden]
    #[attr_added_in("0.5.21")]
    pub const UV_INDEX_MY_INDEX_USERNAME: &'static str = "UV_INDEX_MY_INDEX_USERNAME";

    /// Tests authentication for named indexes.
    #[attr_hidden]
    #[attr_added_in("0.5.21")]
    pub const UV_INDEX_MY_INDEX_PASSWORD: &'static str = "UV_INDEX_MY_INDEX_PASSWORD";

    /// Sets the GitHub fast-path URL for tests.
    #[attr_hidden]
    #[attr_added_in("0.7.15")]
    pub const UV_GITHUB_FAST_PATH_URL: &'static str = "UV_GITHUB_FAST_PATH_URL";

    /// Hides progress messages with non-deterministic order in tests.
    #[attr_hidden]
    #[attr_added_in("0.5.29")]
    pub const UV_TEST_NO_CLI_PROGRESS: &'static str = "UV_TEST_NO_CLI_PROGRESS";

    /// Mocks the current time for relative `--exclude-newer` values in tests.
    /// Set an RFC 3339 timestamp, such as `2025-11-21T12:00:00Z`.
    #[attr_hidden]
    #[attr_added_in("0.9.8")]
    pub const UV_TEST_CURRENT_TIMESTAMP: &'static str = "UV_TEST_CURRENT_TIMESTAMP";

    /// Applies an `exclude-newer` timestamp to versions that indexes make available.
    ///
    /// Use this variable to make resolver errors reproducible.
    /// `exclude-newer` retains available version information for better errors.
    /// Versions published after this cutoff do not exist to the resolver.
    ///
    /// Set an RFC 3339 timestamp, such as `2024-03-25T00:00:00Z`.
    #[attr_hidden]
    #[attr_added_in("0.11.7")]
    pub const UV_TEST_AVAILABLE_VERSION_CUTOFF: &'static str = "UV_TEST_AVAILABLE_VERSION_CUTOFF";

    /// Sets the `.env` files that provide environment variables for `uv run`.
    #[attr_added_in("0.4.30")]
    pub const UV_ENV_FILE: &'static str = "UV_ENV_FILE";

    /// Ignores `.env` files for `uv run`.
    #[attr_added_in("0.4.30")]
    pub const UV_NO_ENV_FILE: &'static str = "UV_NO_ENV_FILE";

    /// Sets the download URL for the standalone installer and `self update`.
    /// Replaces the default GitHub URL.
    ///
    /// This specific installer source takes precedence over
    /// [`UV_ASTRAL_MIRROR_URL`](Self::UV_ASTRAL_MIRROR_URL) for `uv self update`.
    #[attr_added_in("0.5.0")]
    pub const UV_INSTALLER_GITHUB_BASE_URL: &'static str = "UV_INSTALLER_GITHUB_BASE_URL";

    /// Sets the download URL for the standalone installer and `self update`.
    /// Replaces the default GitHub Enterprise URL.
    ///
    /// This specific installer source takes precedence over
    /// [`UV_ASTRAL_MIRROR_URL`](Self::UV_ASTRAL_MIRROR_URL) for `uv self update`.
    #[attr_added_in("0.5.0")]
    pub const UV_INSTALLER_GHE_BASE_URL: &'static str = "UV_INSTALLER_GHE_BASE_URL";

    /// Sets the installation directory for the standalone installer and `self update`.
    /// The default is `~/.local/bin`.
    #[attr_added_in("0.5.0")]
    pub const UV_INSTALL_DIR: &'static str = "UV_INSTALL_DIR";

    /// Installs uv to a specific path in temporary environments, such as CI.
    /// Prevents the installer from changing shell profiles or environment variables.
    #[attr_added_in("0.5.0")]
    pub const UV_UNMANAGED_INSTALL: &'static str = "UV_UNMANAGED_INSTALL";

    /// Sets the download URL for the standalone installer.
    /// By default, the installer downloads uv from GitHub Releases.
    /// `INSTALLER_DOWNLOAD_URL` remains available as a compatibility alias.
    #[attr_added_in("0.8.4")]
    pub const UV_DOWNLOAD_URL: &'static str = "UV_DOWNLOAD_URL";

    /// Prevents the standalone installer and `self update` from changing `PATH`.
    /// `INSTALLER_NO_MODIFY_PATH` remains available as a compatibility alias.
    #[attr_added_in("0.8.4")]
    pub const UV_NO_MODIFY_PATH: &'static str = "UV_NO_MODIFY_PATH";

    /// Skips installer metadata files in site-packages `.dist-info` directories.
    /// These files include `INSTALLER`, `REQUESTED`, and `direct_url.json`.
    #[attr_added_in("0.5.7")]
    pub const UV_NO_INSTALLER_METADATA: &'static str = "UV_NO_INSTALLER_METADATA";

    /// Fetches Git LFS files when uv installs a package from a Git repository.
    #[attr_added_in("0.5.19")]
    pub const UV_GIT_LFS: &'static str = "UV_GIT_LFS";

    /// Sets the soft open-file descriptor limit for commands that `uv run` executes.
    ///
    /// uv applies the limit after it prepares the environment and before it starts the command.
    /// The hard open-file descriptor limit does not change.
    /// If uv cannot apply the limit, it exits with an error and does not start the command.
    /// Only Unix supports this option.
    #[attr_added_in("0.12.3")]
    pub const UV_RUN_RLIMIT_NOFILE: &'static str = "UV_RUN_RLIMIT_NOFILE";

    /// Counts recursive `uv run` calls.
    /// Prevents infinite recursion when a script shebang uses `uv run`.
    #[attr_hidden]
    #[attr_added_in("0.5.31")]
    pub const UV_RUN_RECURSION_DEPTH: &'static str = "UV_RUN_RECURSION_DEPTH";

    /// Sets the maximum number of recursive `uv run` calls before uv exits with an error.
    #[attr_hidden]
    #[attr_added_in("0.5.31")]
    pub const UV_RUN_MAX_RECURSION_DEPTH: &'static str = "UV_RUN_MAX_RECURSION_DEPTH";

    /// Overrides the terminal width for line wrapping.
    /// uv does not read this variable directly.
    ///
    /// `ncurses(3x)` describes this common variable.
    #[attr_added_in("0.6.2")]
    pub const COLUMNS: &'static str = "COLUMNS";

    /// Sets the CUDA driver version for PyTorch backend detection, such as `550.144.03`.
    #[attr_hidden]
    #[attr_added_in("0.6.9")]
    pub const UV_CUDA_DRIVER_VERSION: &'static str = "UV_CUDA_DRIVER_VERSION";

    /// Sets the AMD GPU architecture for PyTorch backend detection, such as `gfx1100`.
    #[attr_hidden]
    #[attr_added_in("0.7.14")]
    pub const UV_AMD_GPU_ARCHITECTURE: &'static str = "UV_AMD_GPU_ARCHITECTURE";

    /// Matches the `--torch-backend` command-line option.
    /// Examples include `cpu`, `cu126`, and `auto`.
    #[attr_added_in("0.6.9")]
    pub const UV_TORCH_BACKEND: &'static str = "UV_TORCH_BACKEND";

    /// Matches the `--project` command-line option.
    #[attr_added_in("0.4.4")]
    pub const UV_PROJECT: &'static str = "UV_PROJECT";

    /// Matches the `--no-project` command-line option.
    #[attr_added_in("0.11.8")]
    pub const UV_NO_PROJECT: &'static str = "UV_NO_PROJECT";

    /// Matches the `--directory` command-line option.
    /// `UV_WORKING_DIRECTORY`, added in v0.9.1, remains available for compatibility.
    #[attr_added_in("0.9.14")]
    pub const UV_WORKING_DIR: &'static str = "UV_WORKING_DIR";

    /// Matches the `--directory` command-line option.
    #[attr_hidden]
    #[attr_added_in("0.9.1")]
    pub const UV_WORKING_DIRECTORY: &'static str = "UV_WORKING_DIRECTORY";

    /// Disables GitHub-specific requests that can let uv skip `git fetch`.
    #[attr_added_in("0.7.13")]
    pub const UV_NO_GITHUB_FAST_PATH: &'static str = "UV_NO_GITHUB_FAST_PATH";

    /// Sets the authentication token for Hugging Face requests.
    /// uv uses this token for `https://huggingface.co/` and its subdomains.
    #[attr_added_in("0.8.1")]
    pub const HF_TOKEN: &'static str = "HF_TOKEN";

    /// Disables Hugging Face authentication, even when `HF_TOKEN` is set.
    #[attr_added_in("0.8.1")]
    pub const UV_NO_HF_TOKEN: &'static str = "UV_NO_HF_TOKEN";

    /// Sets the URL of an S3-compatible storage endpoint.
    /// uv signs requests with AWS Signature Version 4.
    /// It uses `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_PROFILE`, and `AWS_CONFIG_FILE`.
    #[attr_added_in("0.8.21")]
    pub const UV_S3_ENDPOINT_URL: &'static str = "UV_S3_ENDPOINT_URL";

    /// Sets the URL of a GCS-compatible storage endpoint.
    /// uv signs requests with Google Cloud authentication.
    /// It uses `GOOGLE_APPLICATION_CREDENTIALS` or Application Default Credentials.
    #[attr_added_in("0.9.26")]
    pub const UV_GCS_ENDPOINT_URL: &'static str = "UV_GCS_ENDPOINT_URL";

    /// Sets the URL of an Azure Blob Storage endpoint.
    /// uv signs requests with the default Azure credential chain.
    /// This chain includes Azure CLI credentials and workload identity.
    #[attr_added_in("0.11.14")]
    pub const UV_AZURE_ENDPOINT_URL: &'static str = "UV_AZURE_ENDPOINT_URL";

    /// Sets the pyx API key, such as `sk-pyx-...`.
    #[attr_added_in("0.8.15")]
    pub const PYX_API_KEY: &'static str = "PYX_API_KEY";

    /// Sets the pyx API key for compatibility.
    #[attr_hidden]
    #[attr_added_in("0.8.15")]
    pub const UV_API_KEY: &'static str = "UV_API_KEY";

    /// Sets the pyx authentication token from `uv auth token`.
    /// For example, `eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...`.
    #[attr_added_in("0.8.15")]
    pub const PYX_AUTH_TOKEN: &'static str = "PYX_AUTH_TOKEN";

    /// Sets the pyx authentication token for compatibility.
    #[attr_hidden]
    #[attr_added_in("0.8.15")]
    pub const UV_AUTH_TOKEN: &'static str = "UV_AUTH_TOKEN";

    /// Sets the AWS region for S3 request signatures.
    #[attr_added_in("0.8.21")]
    pub const AWS_REGION: &'static str = "AWS_REGION";

    /// Sets the default AWS region for S3 request signatures when `AWS_REGION` is unset.
    #[attr_added_in("0.8.21")]
    pub const AWS_DEFAULT_REGION: &'static str = "AWS_DEFAULT_REGION";

    /// Sets the AWS access key ID for S3 request signatures.
    #[attr_added_in("0.8.21")]
    pub const AWS_ACCESS_KEY_ID: &'static str = "AWS_ACCESS_KEY_ID";

    /// Sets the AWS secret access key for S3 request signatures.
    #[attr_added_in("0.8.21")]
    pub const AWS_SECRET_ACCESS_KEY: &'static str = "AWS_SECRET_ACCESS_KEY";

    /// Sets the AWS session token for S3 request signatures.
    #[attr_added_in("0.8.21")]
    pub const AWS_SESSION_TOKEN: &'static str = "AWS_SESSION_TOKEN";

    /// Sets the AWS profile for S3 request signatures.
    #[attr_added_in("0.8.21")]
    pub const AWS_PROFILE: &'static str = "AWS_PROFILE";

    /// Sets the AWS configuration file for S3 request signatures.
    #[attr_added_in("0.8.21")]
    pub const AWS_CONFIG_FILE: &'static str = "AWS_CONFIG_FILE";

    /// Sets the AWS shared credentials file for S3 request signatures.
    #[attr_added_in("0.8.21")]
    pub const AWS_SHARED_CREDENTIALS_FILE: &'static str = "AWS_SHARED_CREDENTIALS_FILE";

    /// Skips the check that wheel filenames match their contents.
    /// Do not use this option unless necessary.
    /// Wheels with inconsistent filenames are invalid and their maintainers must correct them.
    /// Use this option only to work around an invalid wheel.
    #[attr_added_in("0.8.23")]
    pub const UV_SKIP_WHEEL_FILENAME_CHECK: &'static str = "UV_SKIP_WHEEL_FILENAME_CHECK";

    /// Hides build backend output for source distribution builds, even when a build fails.
    #[attr_added_in("0.9.15")]
    pub const UV_HIDE_BUILD_OUTPUT: &'static str = "UV_HIDE_BUILD_OUTPUT";

    /// The time uv waits for a file lock, in seconds.
    ///
    /// The default is 300 seconds, or 5 minutes.
    #[attr_added_in("0.9.4")]
    pub const UV_LOCK_TIMEOUT: &'static str = "UV_LOCK_TIMEOUT";
}
