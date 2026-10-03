use uv_static::EnvVars;

use uv_test::uv_snapshot;

#[test]
fn cert_is_limited_to_pip() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.command()
        .arg("sync")
        .arg("--cert")
        .arg("ca-bundle.pem"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: unexpected argument '--cert' found

      tip: a similar argument exists: '--script'

    Usage: uv sync --script <SCRIPT>

    For more information, try '--help'.
    ");
}

#[test]
fn help() {
    let context = uv_test::test_context_with_versions!(&[]);

    // The `uv help` command should show the long help message
    uv_snapshot!(context.filters(), context.help(), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    An extremely fast Python package manager.

    Usage: uv [OPTIONS] <COMMAND>

    Commands:
      auth                       Manage authentication
      run                        Run a command or script
      init                       Create a new project
      add                        Add dependencies to the project
      remove                     Remove dependencies from the project
      version                    Read or update the project's version
      sync                       Update the project's environment
      lock                       Update the project's lockfile
      export                     Export the project's lockfile to an alternate format
      tree                       Display the project's dependency tree
      format                     Format Python code in the project
      check                      Run checks on the project
      audit                      Audit the project's dependencies
      tool                       Run and install commands provided by Python packages
      python                     Manage Python versions and installations
      pip                        Manage Python packages with a pip-compatible interface
      venv                       Create a virtual environment
      build                      Build Python packages into source distributions and wheels
      publish                    Upload distributions to an index
      workspace                  Inspect uv workspaces
      cache                      Manage uv's cache
      self                       Manage the uv executable
      generate-shell-completion  Generate shell completion
      help                       Display documentation for a command

    Cache options:
      -n, --no-cache               Avoid reading from or writing to the cache, instead using a temporary
                                   directory for the duration of the operation [env: UV_NO_CACHE=]
          --cache-dir [CACHE_DIR]  Path to the cache directory [env: UV_CACHE_DIR=]

    Python options:
          --managed-python       Require use of uv-managed Python versions [env: UV_MANAGED_PYTHON=]
          --no-managed-python    Disable use of uv-managed Python versions [env: UV_NO_MANAGED_PYTHON=]
          --no-python-downloads  Disable automatic downloads of Python. [env:
                                 "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output
      -v, --verbose...
              Use verbose output
          --color <COLOR_CHOICE>
              Control the use of color in output [possible values: auto, always, never]
          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]
          --offline
              Disable network access [env: UV_OFFLINE=]
          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host [env: UV_INSECURE_HOST=]
          --no-progress
              Hide all progress outputs [env: UV_NO_PROGRESS=]
          --directory <DIRECTORY>
              Change to the given directory prior to running the command [env: UV_WORKING_DIR=]
          --project <PROJECT>
              Discover a project in the given directory [env: UV_PROJECT=]
          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration [env: UV_CONFIG_FILE=]
          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`) [env: UV_NO_CONFIG=]
      -h, --help
              Display the concise help for this command
      -V, --version
              Display the uv version

    Use `uv help <command>` for more information on a specific command.
    "#);
}

#[test]
fn help_flag() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.command().arg("--help"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    An extremely fast Python package manager.

    Usage: uv [OPTIONS] <COMMAND>

    Commands:
      auth       Manage authentication
      run        Run a command or script
      init       Create a new project
      add        Add dependencies to the project
      remove     Remove dependencies from the project
      version    Read or update the project's version
      sync       Update the project's environment
      lock       Update the project's lockfile
      export     Export the project's lockfile to an alternate format
      tree       Display the project's dependency tree
      format     Format Python code in the project
      check      Run checks on the project
      audit      Audit the project's dependencies
      tool       Run and install commands provided by Python packages
      python     Manage Python versions and installations
      pip        Manage Python packages with a pip-compatible interface
      venv       Create a virtual environment
      build      Build Python packages into source distributions and wheels
      publish    Upload distributions to an index
      workspace  Inspect uv workspaces
      cache      Manage uv's cache
      self       Manage the uv executable
      help       Display documentation for a command

    Cache options:
      -n, --no-cache               Avoid reading from or writing to the cache, instead using a temporary
                                   directory for the duration of the operation [env: UV_NO_CACHE=]
          --cache-dir [CACHE_DIR]  Path to the cache directory [env: UV_CACHE_DIR=]

    Python options:
          --managed-python       Require use of uv-managed Python versions [env: UV_MANAGED_PYTHON=]
          --no-managed-python    Disable use of uv-managed Python versions [env: UV_NO_MANAGED_PYTHON=]
          --no-python-downloads  Disable automatic downloads of Python. [env:
                                 "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output
      -v, --verbose...
              Use verbose output
          --color <COLOR_CHOICE>
              Control the use of color in output [possible values: auto, always, never]
          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]
          --offline
              Disable network access [env: UV_OFFLINE=]
          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host [env: UV_INSECURE_HOST=]
          --no-progress
              Hide all progress outputs [env: UV_NO_PROGRESS=]
          --directory <DIRECTORY>
              Change to the given directory prior to running the command [env: UV_WORKING_DIR=]
          --project <PROJECT>
              Discover a project in the given directory [env: UV_PROJECT=]
          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration [env: UV_CONFIG_FILE=]
          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`) [env: UV_NO_CONFIG=]
      -h, --help
              Display the concise help for this command
      -V, --version
              Display the uv version

    Use `uv help` for more details.
    "#);
}

#[test]
fn help_short_flag() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.command().arg("-h"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    An extremely fast Python package manager.

    Usage: uv [OPTIONS] <COMMAND>

    Commands:
      auth       Manage authentication
      run        Run a command or script
      init       Create a new project
      add        Add dependencies to the project
      remove     Remove dependencies from the project
      version    Read or update the project's version
      sync       Update the project's environment
      lock       Update the project's lockfile
      export     Export the project's lockfile to an alternate format
      tree       Display the project's dependency tree
      format     Format Python code in the project
      check      Run checks on the project
      audit      Audit the project's dependencies
      tool       Run and install commands provided by Python packages
      python     Manage Python versions and installations
      pip        Manage Python packages with a pip-compatible interface
      venv       Create a virtual environment
      build      Build Python packages into source distributions and wheels
      publish    Upload distributions to an index
      workspace  Inspect uv workspaces
      cache      Manage uv's cache
      self       Manage the uv executable
      help       Display documentation for a command

    Cache options:
      -n, --no-cache               Avoid reading from or writing to the cache, instead using a temporary
                                   directory for the duration of the operation [env: UV_NO_CACHE=]
          --cache-dir [CACHE_DIR]  Path to the cache directory [env: UV_CACHE_DIR=]

    Python options:
          --managed-python       Require use of uv-managed Python versions [env: UV_MANAGED_PYTHON=]
          --no-managed-python    Disable use of uv-managed Python versions [env: UV_NO_MANAGED_PYTHON=]
          --no-python-downloads  Disable automatic downloads of Python. [env:
                                 "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output
      -v, --verbose...
              Use verbose output
          --color <COLOR_CHOICE>
              Control the use of color in output [possible values: auto, always, never]
          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]
          --offline
              Disable network access [env: UV_OFFLINE=]
          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host [env: UV_INSECURE_HOST=]
          --no-progress
              Hide all progress outputs [env: UV_NO_PROGRESS=]
          --directory <DIRECTORY>
              Change to the given directory prior to running the command [env: UV_WORKING_DIR=]
          --project <PROJECT>
              Discover a project in the given directory [env: UV_PROJECT=]
          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration [env: UV_CONFIG_FILE=]
          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`) [env: UV_NO_CONFIG=]
      -h, --help
              Display the concise help for this command
      -V, --version
              Display the uv version

    Use `uv help` for more details.
    "#);
}

#[test]
fn help_flag_workspace() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.command().arg("workspace").arg("--help"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    Inspect uv workspaces

    Usage: uv workspace [OPTIONS] <COMMAND>

    Commands:
      metadata  View metadata about the current workspace
      dir       Display the path of a workspace member
      list      List the members of a workspace

    Cache options:
      -n, --no-cache               Avoid reading from or writing to the cache, instead using a temporary
                                   directory for the duration of the operation [env: UV_NO_CACHE=]
          --cache-dir [CACHE_DIR]  Path to the cache directory [env: UV_CACHE_DIR=]

    Python options:
          --managed-python       Require use of uv-managed Python versions [env: UV_MANAGED_PYTHON=]
          --no-managed-python    Disable use of uv-managed Python versions [env: UV_NO_MANAGED_PYTHON=]
          --no-python-downloads  Disable automatic downloads of Python. [env:
                                 "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output
      -v, --verbose...
              Use verbose output
          --color <COLOR_CHOICE>
              Control the use of color in output [possible values: auto, always, never]
          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]
          --offline
              Disable network access [env: UV_OFFLINE=]
          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host [env: UV_INSECURE_HOST=]
          --no-progress
              Hide all progress outputs [env: UV_NO_PROGRESS=]
          --directory <DIRECTORY>
              Change to the given directory prior to running the command [env: UV_WORKING_DIR=]
          --project <PROJECT>
              Discover a project in the given directory [env: UV_PROJECT=]
          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration [env: UV_CONFIG_FILE=]
          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`) [env: UV_NO_CONFIG=]
      -h, --help
              Display the concise help for this command

    Use `uv help workspace` for more details.
    "#);
}

#[test]
fn help_subcommand() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.help().arg("python"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    Manage Python versions and installations

    uv first searches for Python in an active virtual environment or a `.venv` directory. The
    `.venv` directory can be in the current working directory or a parent directory. If a virtual
    environment is not required, uv then searches `PATH` for a Python executable.

    On Windows, uv also searches the registry for Python executables.

    By default, uv downloads Python if it cannot find the requested version. Use
    `--no-python-downloads` or the `python-downloads` setting to disable downloads.

    Use `--python` to request a different interpreter.

    The following Python version request formats are supported:

    - `<version>` e.g. `3`, `3.12`, `3.12.3`
    - `<version-specifier>` e.g. `>=3.12,<3.13`
    - `<version><short-variant>` (e.g., `3.13t`, `3.12.0d`)
    - `<version>+<variant>` (e.g., `3.13+freethreaded`, `3.12.0+debug`)
    - `<implementation>` e.g. `cpython` or `cp`
    - `<implementation>@<version>` e.g. `cpython@3.12`
    - `<implementation><version>` e.g. `cpython3.12` or `cp312`
    - `<implementation><version-specifier>` e.g. `cpython>=3.12,<3.13`
    - `<implementation>-<version>-<os>-<arch>-<libc>` e.g. `cpython-3.12.3-macos-aarch64-none`

    You can also request a specific system Python interpreter with:

    - `<executable-path>` e.g. `/opt/homebrew/bin/python3`
    - `<executable-name>` e.g. `mypython3`
    - `<install-dir>` e.g. `/some/environment/`

    When you use `--python`, uv follows the normal discovery rules and checks each interpreter
    against the request. For example, if you request `pypy`, uv first checks the virtual
    environment for a PyPy interpreter. It then checks each executable in `PATH`.

    uv finds CPython, PyPy, and GraalPy interpreters and skips unsupported interpreters. If you
    request an unsupported interpreter implementation, uv exits with an error.

    Usage: uv python [OPTIONS] <COMMAND>

    Commands:
      list          List the available Python installations
      install       Download and install Python versions
      upgrade       Upgrade installed Python versions
      find          Search for a Python installation
      pin           Pin to a specific Python version
      dir           Show the uv Python installation directory
      uninstall     Uninstall Python versions
      update-shell  Ensure that the Python executable directory is on the `PATH`

    Cache options:
      -n, --no-cache
              Avoid reading from or writing to the cache, instead using a temporary directory for the
              duration of the operation

              [env: UV_NO_CACHE=]

          --cache-dir [CACHE_DIR]
              Path to the cache directory.

              Defaults to `$XDG_CACHE_HOME/uv` or `$HOME/.cache/uv` on macOS and Linux, and
              `%LOCALAPPDATA%/uv/cache` on Windows.

              To view the location of the cache directory, run `uv cache dir`.

              [env: UV_CACHE_DIR=]

    Python options:
          --managed-python
              Require use of uv-managed Python versions.

              By default, uv prefers Python versions that it manages. If no managed version is
              installed, uv uses a system Python version. This option prevents uv from using system
              Python versions.

              [env: UV_MANAGED_PYTHON=]

          --no-managed-python
              Disable use of uv-managed Python versions.

              Instead, uv searches the system for a suitable Python version.

              [env: UV_NO_MANAGED_PYTHON=]

          --no-python-downloads
              Disable automatic downloads of Python. [env: "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output.

              Repeat this option, such as `-qq`, to prevent uv from writing output to stdout.

      -v, --verbose...
              Use verbose output.

              Use the `RUST_LOG` environment variable to configure detailed logging.
              (<https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives>)

          --color <COLOR_CHOICE>
              Control the use of color in output.

              By default, uv detects whether the terminal supports color.

              Possible values:
              - auto:   Enables colored output only when the output is going to a terminal or TTY with
                support
              - always: Enables colored output regardless of the detected environment
              - never:  Disables colored output

          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]

              By default, uv uses bundled Mozilla root certificates. This improves portability and
              performance, especially on macOS.

              Use the platform's native certificate store if you need a certificate that is in the
              system store. For example, a corporate proxy may require a corporate trust root.

          --offline
              Disable network access.

              When network access is disabled, uv uses only cached data and local files.

              [env: UV_OFFLINE=]

          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host.

              Use this option multiple times to add multiple hosts.

              Accepts a hostname, such as `localhost`; a host-port pair, such as `localhost:8080`; or a
              URL, such as `https://localhost`.

              WARNING: uv does not verify these hosts against the system's certificate store. This
              option bypasses SSL verification and can expose you to MITM attacks. Use
              `--allow-insecure-host` only on a secure network with verified sources.

              [env: UV_INSECURE_HOST=]

          --no-progress
              Hide all progress outputs.

              For example, spinners or progress bars.

              [env: UV_NO_PROGRESS=]

          --directory <DIRECTORY>
              Change to the given directory prior to running the command.

              uv resolves relative paths from the specified directory.

              See `--project` to only change the project root directory.

              [env: UV_WORKING_DIR=]

          --project <PROJECT>
              Discover a project in the given directory.

              uv searches the project root and its parent directories for `pyproject.toml`, `uv.toml`,
              and `.python-version` files. It also searches for the project's virtual environment
              (`.venv`).

              uv resolves other command-line arguments, such as relative paths, from the current working
              directory.

              See `--directory` to change the working directory entirely.

              This setting has no effect when used in the `uv pip` interface.

              [env: UV_PROJECT=]

          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration.

              A `pyproject.toml` file can contain uv configuration, but this option does not accept one.

              [env: UV_CONFIG_FILE=]

          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`).

              By default, uv searches the current directory, parent directories, and user configuration
              directories for configuration files.

              [env: UV_NO_CONFIG=]

      -h, --help
              Display the concise help for this command

    Use `uv help python <command>` for more information on a specific command.
    "#);
}

#[test]
fn help_subsubcommand() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.help().env_remove(EnvVars::UV_PYTHON_INSTALL_DIR).arg("python").arg("install"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    Download and install Python versions.

    uv supports CPython and PyPy. It downloads CPython from Astral's `python-build-standalone` project
    and PyPy from `python.org`. Each uv release includes a list of available Python versions. You may
    need to upgrade uv to install a newer Python version.

    uv installs Python into its Python directory. Use `uv python dir` to display that directory.

    By default, uv adds Python executables with a minor-version suffix, such as `python3.13`, to a
    directory on `PATH`. Use `--default` to also install `python3` and `python`. Use `uv python dir
    --bin` to display the target directory.

    You can request multiple Python versions.

    See `uv help python` to view supported request formats.

    Usage: uv python install [OPTIONS] [TARGETS]...

    Arguments:
      [TARGETS]...
              The Python version(s) to install.

              If you do not specify a version, uv checks `UV_PYTHON`, then `.python-versions` or
              `.python-version` files. If none exist, uv checks for an installed Python version. If it
              finds none, it installs the latest stable Python version.

              See `uv help python` to view supported request formats.

              [env: UV_PYTHON=]

    Options:
      -i, --install-dir <INSTALL_DIR>
              The directory to store the Python installation in.

              If you set this option, set `UV_PYTHON_INSTALL_DIR` for later commands so uv can find the
              Python installation.

              See `uv python dir` to view the current Python installation directory. Defaults to
              `~/.local/share/uv/python`.

              [env: UV_PYTHON_INSTALL_DIR=]

          --no-bin
              Do not install a Python executable into the `bin` directory.

              This can also be set with `UV_PYTHON_INSTALL_BIN=0`.

          --no-registry
              Do not register the Python installation in the Windows registry.

              This can also be set with `UV_PYTHON_INSTALL_REGISTRY=0`.

          --mirror <MIRROR>
              Set the URL to use as the source for downloading Python installations.

              The provided URL will replace
              `https://github.com/astral-sh/python-build-standalone/releases/download` in, e.g.,
              `https://github.com/astral-sh/python-build-standalone/releases/download/20240713/cpython-3.12.4%2B20240713-aarch64-apple-darwin-install_only.tar.gz`.

              Use a `file://` URL to read distributions from a local directory.

          --pypy-mirror <PYPY_MIRROR>
              Set the URL to use as the source for downloading PyPy installations.

              The provided URL will replace `https://downloads.python.org/pypy` in, e.g.,
              `https://downloads.python.org/pypy/pypy3.8-v7.3.7-osx64.tar.bz2`.

              Use a `file://` URL to read distributions from a local directory.

          --python-downloads-json-url <PYTHON_DOWNLOADS_JSON_URL>
              URL pointing to JSON of custom Python installations

      -r, --reinstall
              Reinstall the requested Python version, if it's already installed.

              If you request a minor version, uv reinstalls all matching installed patch versions.

              By default, uv exits successfully if the version is already installed.

      -f, --force
              Replace existing Python executables during installation.

              By default, uv does not replace executables that it does not manage.

              Implies `--reinstall`.

      -U, --upgrade
              Upgrade existing Python installations to the latest patch version.

              By default, uv does not upgrade installed Python versions to newer patch releases. With
              `--upgrade`, uv installs the latest patch for each specified minor version.

              If a requested version is not installed, uv installs it.

              This option accepts only minor versions, such as `3.12`. If you request a patch version,
              such as `3.12.2`, uv exits with an error.

          --default
              Use as the default Python version.

              By default, uv installs only `python{major}.{minor}`, such as `python3.10`. With
              `--default`, it also installs `python{major}`, such as `python3`, and `python`.

              Other Python variants retain their tag. For example, `3.13+freethreaded` with `--default`
              installs `python3t` and `pythont` instead of `python3` and `python`.

              If you request multiple Python versions, uv exits with an error.

          --compile-bytecode
              Compile Python's standard library to bytecode after installation.

              By default, Python compiles `.py` files to bytecode (`__pycache__/*.pyc`) when a module is
              first imported. Enable this option to compile during installation instead. This increases
              installation time and disk use, but can improve startup time for CLI applications and
              Docker containers.

              uv processes the Python version's `stdlib` directory and ignores compilation errors.

              [env: UV_COMPILE_BYTECODE=]

    Cache options:
      -n, --no-cache
              Avoid reading from or writing to the cache, instead using a temporary directory for the
              duration of the operation

              [env: UV_NO_CACHE=]

          --cache-dir [CACHE_DIR]
              Path to the cache directory.

              Defaults to `$XDG_CACHE_HOME/uv` or `$HOME/.cache/uv` on macOS and Linux, and
              `%LOCALAPPDATA%/uv/cache` on Windows.

              To view the location of the cache directory, run `uv cache dir`.

              [env: UV_CACHE_DIR=]

    Python options:
          --managed-python
              Require use of uv-managed Python versions.

              By default, uv prefers Python versions that it manages. If no managed version is
              installed, uv uses a system Python version. This option prevents uv from using system
              Python versions.

              [env: UV_MANAGED_PYTHON=]

          --no-managed-python
              Disable use of uv-managed Python versions.

              Instead, uv searches the system for a suitable Python version.

              [env: UV_NO_MANAGED_PYTHON=]

          --no-python-downloads
              Disable automatic downloads of Python. [env: "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output.

              Repeat this option, such as `-qq`, to prevent uv from writing output to stdout.

      -v, --verbose...
              Use verbose output.

              Use the `RUST_LOG` environment variable to configure detailed logging.
              (<https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives>)

          --color <COLOR_CHOICE>
              Control the use of color in output.

              By default, uv detects whether the terminal supports color.

              Possible values:
              - auto:   Enables colored output only when the output is going to a terminal or TTY with
                support
              - always: Enables colored output regardless of the detected environment
              - never:  Disables colored output

          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]

              By default, uv uses bundled Mozilla root certificates. This improves portability and
              performance, especially on macOS.

              Use the platform's native certificate store if you need a certificate that is in the
              system store. For example, a corporate proxy may require a corporate trust root.

          --offline
              Disable network access.

              When network access is disabled, uv uses only cached data and local files.

              [env: UV_OFFLINE=]

          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host.

              Use this option multiple times to add multiple hosts.

              Accepts a hostname, such as `localhost`; a host-port pair, such as `localhost:8080`; or a
              URL, such as `https://localhost`.

              WARNING: uv does not verify these hosts against the system's certificate store. This
              option bypasses SSL verification and can expose you to MITM attacks. Use
              `--allow-insecure-host` only on a secure network with verified sources.

              [env: UV_INSECURE_HOST=]

          --no-progress
              Hide all progress outputs.

              For example, spinners or progress bars.

              [env: UV_NO_PROGRESS=]

          --directory <DIRECTORY>
              Change to the given directory prior to running the command.

              uv resolves relative paths from the specified directory.

              See `--project` to only change the project root directory.

              [env: UV_WORKING_DIR=]

          --project <PROJECT>
              Discover a project in the given directory.

              uv searches the project root and its parent directories for `pyproject.toml`, `uv.toml`,
              and `.python-version` files. It also searches for the project's virtual environment
              (`.venv`).

              uv resolves other command-line arguments, such as relative paths, from the current working
              directory.

              See `--directory` to change the working directory entirely.

              This setting has no effect when used in the `uv pip` interface.

              [env: UV_PROJECT=]

          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration.

              A `pyproject.toml` file can contain uv configuration, but this option does not accept one.

              [env: UV_CONFIG_FILE=]

          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`).

              By default, uv searches the current directory, parent directories, and user configuration
              directories for configuration files.

              [env: UV_NO_CONFIG=]

      -h, --help
              Display the concise help for this command
    "#);
}

#[test]
fn help_flag_subcommand() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.command().arg("python").arg("--help"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    Manage Python versions and installations

    Usage: uv python [OPTIONS] <COMMAND>

    Commands:
      list          List the available Python installations
      install       Download and install Python versions
      upgrade       Upgrade installed Python versions
      find          Search for a Python installation
      pin           Pin to a specific Python version
      dir           Show the uv Python installation directory
      uninstall     Uninstall Python versions
      update-shell  Ensure that the Python executable directory is on the `PATH`

    Cache options:
      -n, --no-cache               Avoid reading from or writing to the cache, instead using a temporary
                                   directory for the duration of the operation [env: UV_NO_CACHE=]
          --cache-dir [CACHE_DIR]  Path to the cache directory [env: UV_CACHE_DIR=]

    Python options:
          --managed-python       Require use of uv-managed Python versions [env: UV_MANAGED_PYTHON=]
          --no-managed-python    Disable use of uv-managed Python versions [env: UV_NO_MANAGED_PYTHON=]
          --no-python-downloads  Disable automatic downloads of Python. [env:
                                 "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output
      -v, --verbose...
              Use verbose output
          --color <COLOR_CHOICE>
              Control the use of color in output [possible values: auto, always, never]
          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]
          --offline
              Disable network access [env: UV_OFFLINE=]
          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host [env: UV_INSECURE_HOST=]
          --no-progress
              Hide all progress outputs [env: UV_NO_PROGRESS=]
          --directory <DIRECTORY>
              Change to the given directory prior to running the command [env: UV_WORKING_DIR=]
          --project <PROJECT>
              Discover a project in the given directory [env: UV_PROJECT=]
          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration [env: UV_CONFIG_FILE=]
          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`) [env: UV_NO_CONFIG=]
      -h, --help
              Display the concise help for this command

    Use `uv help python` for more details.
    "#);
}

#[test]
fn help_flag_subsubcommand() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.command().arg("python").arg("install").arg("--help"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    Download and install Python versions

    Usage: uv python install [OPTIONS] [TARGETS]...

    Arguments:
      [TARGETS]...  The Python version(s) to install [env: UV_PYTHON=]

    Options:
      -i, --install-dir <INSTALL_DIR>
              The directory to store the Python installation in [env: UV_PYTHON_INSTALL_DIR=]
          --no-bin
              Do not install a Python executable into the `bin` directory
          --no-registry
              Do not register the Python installation in the Windows registry
          --mirror <MIRROR>
              Set the URL to use as the source for downloading Python installations
          --pypy-mirror <PYPY_MIRROR>
              Set the URL to use as the source for downloading PyPy installations
          --python-downloads-json-url <PYTHON_DOWNLOADS_JSON_URL>
              URL pointing to JSON of custom Python installations
      -r, --reinstall
              Reinstall the requested Python version, if it's already installed
      -f, --force
              Replace existing Python executables during installation
      -U, --upgrade
              Upgrade existing Python installations to the latest patch version
          --default
              Use as the default Python version
          --compile-bytecode
              Compile Python's standard library to bytecode after installation [env:
              UV_COMPILE_BYTECODE=]

    Cache options:
      -n, --no-cache               Avoid reading from or writing to the cache, instead using a temporary
                                   directory for the duration of the operation [env: UV_NO_CACHE=]
          --cache-dir [CACHE_DIR]  Path to the cache directory [env: UV_CACHE_DIR=]

    Python options:
          --managed-python       Require use of uv-managed Python versions [env: UV_MANAGED_PYTHON=]
          --no-managed-python    Disable use of uv-managed Python versions [env: UV_NO_MANAGED_PYTHON=]
          --no-python-downloads  Disable automatic downloads of Python. [env:
                                 "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output
      -v, --verbose...
              Use verbose output
          --color <COLOR_CHOICE>
              Control the use of color in output [possible values: auto, always, never]
          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]
          --offline
              Disable network access [env: UV_OFFLINE=]
          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host [env: UV_INSECURE_HOST=]
          --no-progress
              Hide all progress outputs [env: UV_NO_PROGRESS=]
          --directory <DIRECTORY>
              Change to the given directory prior to running the command [env: UV_WORKING_DIR=]
          --project <PROJECT>
              Discover a project in the given directory [env: UV_PROJECT=]
          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration [env: UV_CONFIG_FILE=]
          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`) [env: UV_NO_CONFIG=]
      -h, --help
              Display the concise help for this command
    "#);
}

#[test]
fn help_unknown_subcommand() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.help().arg("foobar"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: There is no command `foobar` for `uv`. Did you mean one of:
        auth
        run
        init
        add
        remove
        version
        sync
        lock
        export
        tree
        format
        check
        audit
        tool
        python
        pip
        venv
        build
        publish
        workspace
        cache
        self
        generate-shell-completion
    ");

    uv_snapshot!(context.filters(), context.help().arg("foo").arg("bar"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: There is no command `foo bar` for `uv`. Did you mean one of:
        auth
        run
        init
        add
        remove
        version
        sync
        lock
        export
        tree
        format
        check
        audit
        tool
        python
        pip
        venv
        build
        publish
        workspace
        cache
        self
        generate-shell-completion
    ");
}

#[test]
fn help_unknown_subsubcommand() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.help().arg("python").arg("foobar"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: There is no command `foobar` for `uv python`. Did you mean one of:
        list
        install
        upgrade
        find
        pin
        dir
        uninstall
        update-shell
    ");
}

#[test]
fn help_with_global_option() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.help().arg("--no-cache"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    An extremely fast Python package manager.

    Usage: uv [OPTIONS] <COMMAND>

    Commands:
      auth                       Manage authentication
      run                        Run a command or script
      init                       Create a new project
      add                        Add dependencies to the project
      remove                     Remove dependencies from the project
      version                    Read or update the project's version
      sync                       Update the project's environment
      lock                       Update the project's lockfile
      export                     Export the project's lockfile to an alternate format
      tree                       Display the project's dependency tree
      format                     Format Python code in the project
      check                      Run checks on the project
      audit                      Audit the project's dependencies
      tool                       Run and install commands provided by Python packages
      python                     Manage Python versions and installations
      pip                        Manage Python packages with a pip-compatible interface
      venv                       Create a virtual environment
      build                      Build Python packages into source distributions and wheels
      publish                    Upload distributions to an index
      workspace                  Inspect uv workspaces
      cache                      Manage uv's cache
      self                       Manage the uv executable
      generate-shell-completion  Generate shell completion
      help                       Display documentation for a command

    Cache options:
      -n, --no-cache               Avoid reading from or writing to the cache, instead using a temporary
                                   directory for the duration of the operation [env: UV_NO_CACHE=]
          --cache-dir [CACHE_DIR]  Path to the cache directory [env: UV_CACHE_DIR=]

    Python options:
          --managed-python       Require use of uv-managed Python versions [env: UV_MANAGED_PYTHON=]
          --no-managed-python    Disable use of uv-managed Python versions [env: UV_NO_MANAGED_PYTHON=]
          --no-python-downloads  Disable automatic downloads of Python. [env:
                                 "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output
      -v, --verbose...
              Use verbose output
          --color <COLOR_CHOICE>
              Control the use of color in output [possible values: auto, always, never]
          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]
          --offline
              Disable network access [env: UV_OFFLINE=]
          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host [env: UV_INSECURE_HOST=]
          --no-progress
              Hide all progress outputs [env: UV_NO_PROGRESS=]
          --directory <DIRECTORY>
              Change to the given directory prior to running the command [env: UV_WORKING_DIR=]
          --project <PROJECT>
              Discover a project in the given directory [env: UV_PROJECT=]
          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration [env: UV_CONFIG_FILE=]
          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`) [env: UV_NO_CONFIG=]
      -h, --help
              Display the concise help for this command
      -V, --version
              Display the uv version

    Use `uv help <command>` for more information on a specific command.
    "#);
}

#[test]
fn help_with_help() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.help().arg("--help"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Display documentation for a command

    Usage: uv help [OPTIONS] [COMMAND]...

    Options:
      --no-pager Disable pager when printing help
    ");
}

#[test]
fn help_with_version() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.help().arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: unexpected argument '--version' found

      tip: a similar argument exists: '--verbose'

    Usage: uv help --verbose... [COMMAND]...

    For more information, try '--help'.
    ");
}

#[test]
fn help_with_no_pager() {
    let context = uv_test::test_context_with_versions!(&[]);

    // We can't really test whether the --no-pager option works with a snapshot test.
    // It's still nice to have a test for the option to confirm the option exists.
    uv_snapshot!(context.filters(), context.help().arg("--no-pager"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    An extremely fast Python package manager.

    Usage: uv [OPTIONS] <COMMAND>

    Commands:
      auth                       Manage authentication
      run                        Run a command or script
      init                       Create a new project
      add                        Add dependencies to the project
      remove                     Remove dependencies from the project
      version                    Read or update the project's version
      sync                       Update the project's environment
      lock                       Update the project's lockfile
      export                     Export the project's lockfile to an alternate format
      tree                       Display the project's dependency tree
      format                     Format Python code in the project
      check                      Run checks on the project
      audit                      Audit the project's dependencies
      tool                       Run and install commands provided by Python packages
      python                     Manage Python versions and installations
      pip                        Manage Python packages with a pip-compatible interface
      venv                       Create a virtual environment
      build                      Build Python packages into source distributions and wheels
      publish                    Upload distributions to an index
      workspace                  Inspect uv workspaces
      cache                      Manage uv's cache
      self                       Manage the uv executable
      generate-shell-completion  Generate shell completion
      help                       Display documentation for a command

    Cache options:
      -n, --no-cache               Avoid reading from or writing to the cache, instead using a temporary
                                   directory for the duration of the operation [env: UV_NO_CACHE=]
          --cache-dir [CACHE_DIR]  Path to the cache directory [env: UV_CACHE_DIR=]

    Python options:
          --managed-python       Require use of uv-managed Python versions [env: UV_MANAGED_PYTHON=]
          --no-managed-python    Disable use of uv-managed Python versions [env: UV_NO_MANAGED_PYTHON=]
          --no-python-downloads  Disable automatic downloads of Python. [env:
                                 "UV_PYTHON_DOWNLOADS=never"]

    Global options:
      -q, --quiet...
              Use quiet output
      -v, --verbose...
              Use verbose output
          --color <COLOR_CHOICE>
              Control the use of color in output [possible values: auto, always, never]
          --system-certs
              Whether to load TLS certificates from the platform's native certificate store [env:
              UV_SYSTEM_CERTS=]
          --offline
              Disable network access [env: UV_OFFLINE=]
          --allow-insecure-host <ALLOW_INSECURE_HOST>
              Allow insecure connections to a host [env: UV_INSECURE_HOST=]
          --no-progress
              Hide all progress outputs [env: UV_NO_PROGRESS=]
          --directory <DIRECTORY>
              Change to the given directory prior to running the command [env: UV_WORKING_DIR=]
          --project <PROJECT>
              Discover a project in the given directory [env: UV_PROJECT=]
          --config-file <CONFIG_FILE>
              The path to a `uv.toml` file to use for configuration [env: UV_CONFIG_FILE=]
          --no-config
              Avoid discovering configuration files (`pyproject.toml`, `uv.toml`) [env: UV_NO_CONFIG=]
      -h, --help
              Display the concise help for this command
      -V, --version
              Display the uv version

    Use `uv help <command>` for more information on a specific command.
    "#);
}
