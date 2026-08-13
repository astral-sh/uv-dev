# Storage

## Storage directories

uv stores data in the following directories.

For each location, uv checks environment variables in the listed order. It uses the first available
path.

Storage directory paths depend on the platform. uv follows the
[XDG](https://specifications.freedesktop.org/basedir-spec/latest/) conventions on Linux and macOS
and the [Known Folder](https://learn.microsoft.com/en-us/windows/win32/shell/known-folders) scheme
on Windows.

### Temporary directory

The temporary directory stores short-lived data.

=== "Unix"

    1. `$TMPDIR`
    1. `/tmp`

=== "Windows"

    1. `%TMP%`
    1. `%TEMP%`
    1. `%USERPROFILE%`

### Cache directory

The cache directory stores reusable data that uv can discard.

=== "Unix"

    1. `$XDG_CACHE_HOME/uv`
    1. `$HOME/.cache/uv`

=== "Windows"

    1. `%LOCALAPPDATA%\uv\cache`
    1. `uv\cache` within [`FOLDERID_LocalAppData`](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid#FOLDERID_LocalAppData)

### Persistent data directory

The persistent data directory stores data that uv must keep.

=== "Unix"

    1. `$XDG_DATA_HOME/uv`
    1. `$HOME/.local/share/uv`
    1. `$CWD/.uv`

=== "Windows"

    1. `%APPDATA%\uv\data`
    1. `.\.uv`

### Configuration directories

The configuration directories store changes to uv's settings.

User-level configuration

=== "Unix"

    1. `$XDG_CONFIG_HOME/uv`
    1. `$HOME/.config/uv`

=== "Windows"

    1. `%APPDATA%\uv`
    1. `uv` within [`FOLDERID_RoamingAppData`](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid#FOLDERID_RoamingAppData)

System-level configuration

=== "Unix"

    1. `$XDG_CONFIG_DIRS/uv`
    1. `/etc/uv`

=== "Windows"

    1. `%PROGRAMDATA%\uv`
    1. `uv` within [`FOLDERID_AppDataProgramData`](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid#FOLDERID_AppDataProgramData)

### Executable directory

The executable directory stores files that users can run. This directory should be on the `PATH`.

=== "Unix"

    1. `$XDG_BIN_HOME`
    1. `$XDG_DATA_HOME/../bin`
    1. `$HOME/.local/bin`

=== "Windows"

    1. `%XDG_BIN_HOME%`
    1. `%XDG_DATA_HOME%\..\bin`
    1. `%USERPROFILE%\.local\bin`

## Types of data

### Dependency cache

uv uses a local cache to avoid downloading and building dependencies again.

By default, uv stores the cache in the [cache directory](#cache-directory). Command-line arguments,
environment variables, or settings can change this location, as described in
[the cache documentation](../concepts/cache.md#cache-directory). When caching is disabled, uv uses a
[temporary directory](#temporary-directory).

The `uv cache dir` command shows the current cache directory path.

!!! important

    For best performance, the cache directory and virtual environments must be on the same
    filesystem.

### Python versions

uv installs managed [Python versions](../concepts/python-versions.md) with commands such as
`uv python install`.

By default, uv stores managed Python versions in a `python/` subdirectory of the
[persistent data directory](#persistent-data-directory), e.g., `~/.local/share/uv/python`.

The `uv python dir` command shows the Python installation directory.

The `UV_PYTHON_INSTALL_DIR` environment variable changes the installation directory.

!!! note

    Changing the Python installation directory does not update existing virtual environments. They
    still refer to the previous location and must be updated manually, for example, by recreating
    them.

### Python executables

uv installs executables for [Python versions](#python-versions), e.g., `python3.13`.

By default, uv stores Python executables in the [executable directory](#executable-directory).

The `uv python dir --bin` command shows the Python executable directory.

The `UV_PYTHON_BIN_DIR` environment variable changes the Python executable directory.

### Tools

uv installs Python packages as [command-line tools](../concepts/tools.md) with `uv tool install`.

By default, uv installs tools in a `tools/` subdirectory of the
[persistent data directory](#persistent-data-directory), e.g., `~/.local/share/uv/tools`.

The `uv tool dir` command shows the tool installation directory.

The `UV_TOOL_DIR` environment variable configures the installation directory.

### Tool executables

uv installs executables for installed [tools](#tools), e.g., `ruff`.

By default, uv stores tool executables in the [executable directory](#executable-directory).

The `uv tool dir --bin` command shows the tool executable directory.

The `UV_TOOL_BIN_DIR` environment variable configures the tool executable directory.

### The uv executable

The uv [standalone installer](./installer.md) installs the `uv` and `uvx` executables in the
[executable directory](#executable-directory).

The `UV_INSTALL_DIR` environment variable configures uv's installation directory.

### Configuration files

TOML files configure uv's behavior.

uv discovers these files in the [configuration directories](#configuration-directories).

The [configuration files documentation](../concepts/configuration-files.md) provides more details.

### Project virtual environments

uv creates a separate virtual environment for each [project](../concepts/projects/index.md).

By default, uv creates project virtual environments in `.venv` at the project or workspace root,
next to `pyproject.toml`.

The `UV_PROJECT_ENVIRONMENT` environment variable changes this location. The
[projects environment documentation](../concepts/projects/config.md#project-environment-path)
provides more details.

With the [`centralized-project-envs` preview feature](../concepts/preview.md), uv stores default
project environments in the [cache directory](#cache-directory). The `uv cache clean` and
`uv cache prune` commands can remove these environments. uv recreates them when needed. The
[centralized project environments](../concepts/projects/layout.md#centralized-project-environments)
documentation provides more details.

### Script virtual environments

uv creates a separate virtual environment for each
[script with inline metadata](../guides/scripts.md). It stores these environments in the
[cache directory](#cache-directory).
