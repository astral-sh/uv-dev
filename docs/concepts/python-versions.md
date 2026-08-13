# Python versions

A Python version includes a Python interpreter, the standard library, and other supporting files.
The interpreter is the `python` executable.

## Managed and system Python installations

uv can [discover](#discovery-of-python-versions) existing Python versions and
[install Python versions](#installing-a-python-version). Python versions that uv installs are
_managed_ Python installations. All other Python versions are _system_ Python installations.

!!! note

    uv treats Python versions from the operating system and other tools the same. For example, uv
    considers a Python installation that `pyenv` manages a _system_ Python version.

## Requesting a version

Most uv commands accept the `--python` flag to request a specific Python version. For example,
create a virtual environment:

```console
$ uv venv --python 3.11.6
```

uv finds Python 3.11.6 or downloads and installs it. It then creates the virtual environment with
that version.

uv supports these Python version request formats:

- `<version>` (e.g., `3`, `3.12`, `3.12.3`)
- `<version-specifier>` (e.g., `>=3.12,<3.13`)
- `<version><short-variant>` (e.g., `3.13t`, `3.12.0d`)
- `<version>+<variant>` (e.g., `3.13+freethreaded`, `3.12.0+debug`, `3.14+gil`)
- `<implementation>` (e.g., `cpython` or `cp`)
- `<implementation>@<version>` (e.g., `cpython@3.12`)
- `<implementation><version>` (e.g., `cpython3.12` or `cp312`)
- `<implementation><version-specifier>` (e.g., `cpython>=3.12,<3.13`)
- `<implementation>-<version>-<os>-<arch>-<libc>` (e.g., `cpython-3.12.3-macos-aarch64-none`)

To request a specific system Python interpreter, use one of these formats:

- `<executable-path>` (e.g., `/opt/homebrew/bin/python3`)
- `<executable-name>` (e.g., `mypython3`)
- `<install-dir>` (e.g., `/some/environment/`)

By default, uv downloads a Python version if it cannot find that version on the system. The
[`python-downloads` option can disable this behavior](#disabling-automatic-python-downloads).

### Python version files

A `.python-version` file defines a default Python version request. uv searches the current directory
and each parent directory for this file. If uv does not find one, it checks the user-level
configuration directory. The file supports any request format described above. A version number is
recommended for compatibility with other tools.

The [`uv python pin`](../reference/cli.md#uv-python-pin) command creates a `.python-version` file in
the current directory.

The [`uv python pin --global`](../reference/cli.md#uv-python-pin) command creates a global
`.python-version` file in the user configuration directory.

The `--no-config` option disables discovery of `.python-version` files.

uv does not search beyond project or workspace boundaries, except in the user configuration
directory.

## Installing a Python version

uv bundles a list of downloadable CPython and PyPy distributions for macOS, Linux, and Windows.

!!! tip

    By default, uv automatically downloads Python versions as needed. An explicit `uv python install`
    command is not necessary.

To install a specific Python version:

```console
$ uv python install 3.12.3
```

To install the latest patch version:

```console
$ uv python install 3.12
```

To install a version that satisfies constraints:

```console
$ uv python install '>=3.8,<3.10'
```

To install multiple versions:

```console
$ uv python install 3.9 3.10 3.11
```

To install a specific implementation:

```console
$ uv python install pypy
```

The command supports all [Python version request](#requesting-a-version) formats except requests for
local interpreters, such as file paths.

By default, `uv python install` confirms that a managed Python version exists or installs the latest
version. If a `.python-version` file exists, uv installs the version from that file. A project that
requires multiple versions can define a `.python-versions` file. If that file exists, uv installs
every version in it.

!!! important

    Each uv release has a fixed list of available Python versions. New Python versions might require
    an upgrade to uv.

See the [storage documentation](../reference/storage.md#python-versions) for Python installation
locations.

### Installing Python executables

By default, uv installs Python executables into `PATH`. For example, on Unix,
`uv python install 3.12` installs `python3.12` into `~/.local/bin`. See the
[storage documentation](../reference/storage.md#python-executables) for details about the target
directory.

!!! tip

    If `~/.local/bin` is not in `PATH`, add it with `uv python update-shell`.

To install `python` and `python3` executables, include the experimental `--default` option:

```console
$ uv python install 3.12 --default
```

uv overwrites an existing Python executable only if uv manages it. For example, uv does not
overwrite an unmanaged `~/.local/bin/python3.12` without `--force`.

uv updates executables that it manages. By default, it prefers the latest patch version of each
Python minor version. For example:

```console
$ uv python install 3.12.7  # Adds `python3.12` to `~/.local/bin`
$ uv python install 3.12.6  # Does not update `python3.12`
$ uv python install 3.12.8  # Updates `python3.12` to point to 3.12.8
```

## Upgrading Python versions

!!! important

    uv supports upgrades only for managed Python versions.

    uv does not support upgrades for PyPy, GraalPy, or Pyodide.

uv can upgrade Python to the latest patch release, such as from 3.13.4 to 3.13.5. It does not
automatically upgrade between minor versions, such as from 3.12 to 3.13. A different minor version
can change dependency resolution.

The `python upgrade` command upgrades managed Python versions to the latest supported patch release.

To upgrade a Python version to the latest supported patch release:

```console
$ uv python upgrade 3.12
```

To upgrade all installed Python versions:

```console
$ uv python upgrade
```

After an upgrade, uv prefers the new version. It retains the old version because virtual
environments might still use it.

uv automatically upgrades virtual environments that use the Python version to the new patch version.

If a virtual environment explicitly requests a patch version, uv does not automatically upgrade it.
For example, `uv venv -p 3.10.8` remains on Python 3.10.8.

### Minor version directories

Automatic virtual environment upgrades use a directory named for the Python minor version. For
example:

```
~/.local/share/uv/python/cpython-3.12-macos-aarch64-none
```

On Unix, this directory is a symbolic link. On Windows, it is a junction. It points to a specific
patch version:

```console
$ readlink ~/.local/share/uv/python/cpython-3.12-macos-aarch64-none
~/.local/share/uv/python/cpython-3.12.11-macos-aarch64-none
```

If another tool resolves this link before it creates a virtual environment, uv cannot automatically
upgrade that environment. For example, a tool might resolve the link when it canonicalizes the
interpreter path.

## Project Python versions

For project commands, uv follows the `requires-python` setting in `pyproject.toml`. It uses the
first compatible Python version unless another source requests a version. For example,
`.python-version` and `--python` can request a specific version.

## Viewing available Python versions

To list installed and available Python versions:

```console
$ uv python list
```

To show all Python 3.13 interpreters, specify the version:

```console
$ uv python list 3.13
```

To show all PyPy interpreters:

```console
$ uv python list pypy
```

By default, uv hides downloads for other platforms and older patch versions.

To view all versions:

```console
$ uv python list --all-versions
```

To view Python versions for other platforms:

```console
$ uv python list --all-platforms
```

To exclude downloads and show only installed Python versions:

```console
$ uv python list --only-installed
```

See the [`uv python list`](../reference/cli.md#uv-python-list) reference for details.

## Finding a Python executable

To find a Python executable, use the `uv python find` command:

```console
$ uv python find
```

By default, the command displays the path to the first available Python executable. See the
[discovery rules](#discovery-of-python-versions) for details.

This command supports multiple [request formats](#requesting-a-version). For example, find a Python
3.11 or newer executable:

```console
$ uv python find '>=3.11'
```

By default, `uv python find` includes Python versions from virtual environments. A `.venv` directory
in the current directory or a parent directory takes precedence over executables on `PATH`. The
`VIRTUAL_ENV` environment variable also takes precedence over executables on `PATH`.

To ignore virtual environments, use the `--system` flag:

```console
$ uv python find --system
```

## Discovery of Python versions

uv searches these locations for Python versions:

- Managed Python installations in the `UV_PYTHON_INSTALL_DIR`.
- A Python interpreter on the `PATH` as `python`, `python3`, or `python3.x` on macOS and Linux, or
  `python.exe` on Windows.
- On Windows, the Python interpreters in the Windows registry and Microsoft Store Python
  interpreters (see `py --list-paths`) that match the requested version.

In some cases, uv can use a Python version from a virtual environment. It checks whether that
interpreter matches the request before it searches the other locations. See the
[pip-compatible virtual environment discovery](../pip/environments.md#discovery-of-python-environments)
documentation for details.

uv ignores files that are not executable. For each executable, it checks metadata against the
[requested Python version](#requesting-a-version). If the metadata query fails, uv skips that
executable. If the executable matches the request, uv uses it and stops the search.

For managed Python installations, uv prefers newer versions. For system Python installations, uv
uses the first compatible version, not the newest version.

If uv cannot find a compatible Python version on the system, it checks for a compatible managed
Python download.

## Python pre-releases

By default, uv prefers stable Python releases. It uses a pre-release only if no other available
installation matches the request. If both a stable release and a pre-release match, uv uses the
stable release. If a request specifies the path to a pre-release executable, only that executable
matches.

If an available pre-release matches the request, uv does not download a stable release instead.

## Free-threaded Python

uv can discover and install
[free-threaded](https://docs.python.org/3.14/glossary.html#term-free-threading) Python variants in
CPython 3.13+.

For Python 3.13, uv selects free-threaded versions only when explicitly requested. For example, use
`3.13t` or `3.13+freethreaded`.

For Python 3.14+, uv can use free-threaded interpreters without an explicit request. It still
prefers GIL-enabled builds when it installs Python, such as with `uv python install 3.14`. However,
if a free-threaded interpreter appears first on `PATH`, uv uses that interpreter.

If both variants exist on the system, use the `+gil` specifier to require a GIL-enabled interpreter
for a project.

## Debug Python variants

uv can discover and install
[debug builds](https://docs.python.org/3.14/using/configure.html#debug-build) of Python. These
builds enable debug assertions.

!!! important

    Debug builds are slower and are not appropriate for general use.

uv uses a debug build only if no other available installation matches the request. If a regular
build also matches, uv uses the regular build. If a request specifies the path to a debug
executable, only that executable matches.

To explicitly request a debug build, use a specifier such as `3.13d` or `3.13+debug`.

!!! note

    Standard CPython installations omit debug symbols to reduce their size. Debug builds retain
    these symbols. A C-level debugger can use them to inspect Python processes.

## Disabling automatic Python downloads

By default, uv automatically downloads Python versions when necessary.

The [`python-downloads`](../reference/settings.md#python-downloads) option controls this behavior.
Its default value is `automatic`. Set it to `manual` to allow downloads only during
`uv python install`.

!!! tip

    Set `python-downloads` in a [persistent configuration file](./configuration-files.md) to change
    the default. Alternatively, pass `--no-python-downloads` to any uv command.

## Requiring or disabling managed Python versions

By default, uv uses available Python installations and downloads managed versions only when
necessary. To ignore system Python versions and use only managed versions, pass `--managed-python`:

```console
$ uv python list --managed-python
```

To ignore managed Python versions and use only system versions, pass `--no-managed-python`:

```console
$ uv python list --no-managed-python
```

To change the default in a configuration file, use the
[`python-preference` setting](#adjusting-python-version-preferences).

## Adjusting Python version preferences

The [`python-preference`](../reference/settings.md#python-preference) setting selects whether uv
prefers system Python installations or managed Python installations.

By default, `python-preference` is `managed`, so uv prefers existing managed installations over
system installations. However, existing system installations take precedence over a new managed
download.

These alternative values are available:

- `only-managed`: Only use managed Python installations; never use system Python installations.
  Equivalent to `--managed-python`.
- `system`: Prefer system Python installations over managed Python installations.
- `only-system`: Only use system Python installations; never use managed Python installations.
  Equivalent to `--no-managed-python`.

!!! note

    [Disable automatic Python downloads](#disabling-automatic-python-downloads) without a change to
    this preference.

## Python implementation support

uv supports the CPython, PyPy, Pyodide, and GraalPy implementations. It cannot discover interpreters
from unsupported implementations.

Request an implementation with its long or short name:

- CPython: `cpython`, `cp`
- PyPy: `pypy`, `pp`
- GraalPy: `graalpy`, `gp`
- Pyodide: `pyodide`

Implementation names are not case-sensitive.

See the [Python version request](#requesting-a-version) documentation for the supported formats.

## Managed Python distributions

uv can download and install CPython, PyPy, and Pyodide distributions.

### CPython distributions

Python does not publish official CPython binaries for redistribution. uv uses prebuilt distributions
from the Astral [`python-build-standalone`](https://github.com/astral-sh/python-build-standalone)
project. Other projects also use `python-build-standalone`, including
[Mise](https://mise.jdx.dev/lang/python.html) and
[bazelbuild/rules_python](https://github.com/bazelbuild/rules_python).

The uv Python distributions are self-contained, portable, and fast. Tools such as `pyenv` can build
Python from source, but these builds require system dependencies. Optimized builds with features
such as PGO and LTO also take a long time.

These distributions have some behavior differences because they are portable. See the
[`python-build-standalone` quirks](https://gregoryszorc.com/docs/python-build-standalone/main/quirks.html)
documentation for details.

### PyPy distributions

!!! note

    PyPy releases follow CPython releases and currently support Python versions only through 3.11.

The [PyPy project](https://pypy.org) provides PyPy distributions.

### Pyodide distributions

The [Pyodide project](https://github.com/pyodide/pyodide) provides Pyodide distributions.

Pyodide runs CPython on the WebAssembly / Emscripten platform.

## Transparent x86_64 emulation on aarch64

macOS and Windows can run x86_64 binaries on aarch64 through emulation. macOS uses
[Rosetta 2](https://support.apple.com/en-gb/102527), and Windows uses
[Windows on ARM (WoA) emulation](https://learn.microsoft.com/en-us/windows/arm/apps-on-arm-x86-emulation).
An x86_64 uv binary or Python interpreter can run on aarch64. Either uv architecture can use either
Python architecture. However, Python packages must match the interpreter architecture: all x86_64 or
all aarch64.

## Registration in the Windows registry

On Windows, uv registers managed Python installations as
[PEP 514](https://peps.python.org/pep-0514/) defines.

After installation, the `py` launcher can select these Python versions:

```console
$ uv python install 3.13.1
$ py -V:Astral/CPython3.13.1
```

When uv uninstalls a Python version, it removes the matching registry entry and any broken entries.
