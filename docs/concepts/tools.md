# Tools

Tools are Python packages that provide command-line interfaces.

!!! note

    The [tools guide](../guides/tools.md) introduces the tools interface. This document describes
    tool management.

## The `uv tool` interface

uv provides a dedicated interface for tools. The `uv tool run` command runs a tool without a
persistent installation. It installs dependencies in a temporary virtual environment separate from
the current project.

The `uvx` alias is equivalent to `uv tool run`. The documentation primarily uses `uvx` because the
alias is shorter.

The `uv tool install` command installs tools with executables
[available on the `PATH`](#tool-executables). It uses an isolated virtual environment that remains
after the command completes.

## Execution vs installation

In most cases, use `uvx` instead of installing a tool. Install a tool when other programs must
access it. For example, an external script might require the tool, or users might need it in a
Docker image.

## Tool environments

The `uvx` command stores a disposable virtual environment in the uv cache directory. The
`uv cache clean` command deletes this environment. uv caches the environment to reduce the cost of
repeated commands. If the environment does not exist, uv automatically creates a new one.

The `uv tool install` command creates a virtual environment in the
[uv tools directory](../reference/storage.md#tools). The environment remains until the tool is
uninstalled. If the environment is manually deleted, the tool cannot run.

!!! important

    Do _not_ change tool environments directly. For example, do not run a `pip` operation inside a
    tool environment.

## Tool versions

Unless a specific version is requested, `uv tool install` installs the latest available tool
version. The `uvx` command uses the latest available version _on the first invocation_. Later
commands use the cached version unless the request specifies another version or the cache is cleaned
or refreshed.

For example, to run a specific version of Ruff:

```console
$ uvx ruff@0.6.0 --version
ruff 0.6.0
```

A later `uvx` command without a version uses the latest version instead of the cached version:

```console
$ uvx ruff --version
ruff 0.6.2
```

If another Ruff version is released later, uv does not use it until the cache is refreshed.

To request the latest version of Ruff and refresh the cache, use the `@latest` suffix:

```console
$ uvx ruff@latest --version
0.6.2
```

If `uv tool install` installs a tool, `uvx` uses the installed version by default.

For example, install an older version of Ruff:

```console
$ uv tool install ruff==0.5.0
```

The `ruff` and `uvx ruff` commands then use the same version:

```console
$ ruff --version
ruff 0.5.0
$ uvx ruff --version
ruff 0.5.0
```

To ignore the installed version, explicitly request the latest version:

```console
$ uvx ruff@latest --version
0.6.2
```

Alternatively, use `--isolated` to ignore the installed version without refreshing the cache:

```console
$ uvx --isolated ruff --version
0.6.2
```

The `uv tool install` command also accepts the `{package}@{version}` and `{package}@latest` forms:

```console
$ uv tool install ruff@latest
$ uv tool install ruff@0.6.0
```

## Upgrading tools

The `uv tool upgrade` command upgrades tool environments. The `uv tool install` command can also
re-create them.

To upgrade all packages in a tool environment:

```console
$ uv tool upgrade black
```

To upgrade a single package in a tool environment:

```console
$ uv tool upgrade black --upgrade-package click
```

Tool upgrades preserve the version constraints from installation. For example,
`uv tool install black >=23,<24` followed by `uv tool upgrade black` upgrades Black to the latest
version in the `>=23,<24` range.

To replace the version constraints, reinstall the tool with `uv tool install`:

```console
$ uv tool install black>=24
```

Tool upgrades also preserve settings from installation. For example,
`uv tool install black --prerelease allow` followed by `uv tool upgrade black` preserves
`--prerelease allow`.

!!! note

    Tool upgrades reinstall tool executables, even if the executables have not changed.

To reinstall packages during upgrade, use the `--reinstall` and `--reinstall-package` options.

To reinstall all packages in a tool environment:

```console
$ uv tool upgrade black --reinstall
```

To reinstall a single package in a tool environment:

```console
$ uv tool upgrade black --reinstall-package click
```

## Including additional dependencies

To include additional packages when a tool runs:

```console
$ uvx --with <extra-package> <tool>
```

To include additional packages when a tool is installed:

```console
$ uv tool install --with <extra-package> <tool-package>
```

Repeat `--with` to include multiple additional packages.

The `--with` option accepts package specifications. To request a specific version:

```console
$ uvx --with <extra-package>==<version> <tool-package>
```

The `-w` option is a shorter form of `--with`:

```console
$ uvx -w <extra-package> <tool-package>
```

If the requested version conflicts with the tool requirements, package resolution and the command
fail.

## Installing executables from additional packages

Tool environments can include executables from additional packages. This supports related tools or
multiple executables that share dependencies.

Use `--with-executables-from` to install executables from additional packages with the main tool:

```console
$ uv tool install --with-executables-from <package1>,<package2> <tool-package>
```

For example, to install Ansible along with executables from `ansible-core` and `ansible-lint`:

```console
$ uv tool install --with-executables-from ansible-core,ansible-lint ansible
```

This command installs executables from `ansible`, `ansible-core`, and `ansible-lint` in the same
tool environment. All executables are available on `PATH`.

The `--with-executables-from` option also works with other installation options:

```console
$ uv tool install --with-executables-from ansible-core --with mkdocs-material ansible
```

The `--with-executables-from` and `--with` options differ:

- `--with` includes additional packages as dependencies but does not install their executables.
- `--with-executables-from` includes the packages as dependencies and installs their executables.

## Python versions

Each tool environment uses a specific Python version. It uses the same
[discovery logic](./python-versions.md#discovery-of-python-versions) as other uv virtual
environments. However, it ignores local version requests, such as `.python-version` files and the
`requires-python` value in `pyproject.toml`.

The `--python` option requests a specific version. See the [Python version](./python-versions.md)
documentation for details.

If the Python version for a tool is _uninstalled_, the tool environment breaks and the tool might
not run.

## Tool executables

Tool executables include console entry points, script entry points, and binary scripts from a Python
package. On Unix, uv links these executables into the
[executable directory](../reference/storage.md#tool-executables). On Windows, uv copies them.

!!! note

    uv does not install executables from tool dependencies.

The [executable directory](../reference/storage.md#executable-directory) must appear in `PATH` for
the shell to find tool executables. If the directory is absent, uv displays a warning. The
`uv tool update-shell` command adds the directory to `PATH` in common shell configuration files.

### Overwriting executables

When uv installs a tool, it does not overwrite executables that another program installed. For
example, if `pipx` installed a tool, `uv tool install` fails. The `--force` flag overrides this
behavior.

## Relationship to `uv run`

The `uv tool run <name>` and `uvx <name>` commands are almost equivalent to:

```console
$ uv run --no-project --with <name> -- <name>
```

The tool interface has these differences:

- The `--with` option is not necessary because uv identifies the package from the command name.
- uv caches the temporary environment in a dedicated location.
- The `--no-project` flag is not necessary because tools always run separately from the project.
- If a tool is already installed, `uv tool run` uses the installed version, but `uv run` does not.

If a tool requires the project environment, use `uv run` instead of `uv tool run`. For example,
`pytest` and `mypy` often require the project environment.
