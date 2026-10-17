---
title: Using tools
description:
  Use uv to run Python tools, request specific versions, install tools, and keep them up to date.
---

# Using tools

Many Python packages provide applications that you can use as tools. uv runs and installs these
tools.

## Running tools

The `uvx` command runs a tool without permanently installing it.

For example, to run `ruff`, use:

```console
$ uvx ruff
```

!!! note

    This command is equivalent to:

    ```console
    $ uv tool run ruff
    ```

    The `uvx` command is a convenient alias for `uv tool run`.

Add arguments after the tool name:

```console
$ uvx pycowsay hello from uv

  -------------
< hello from uv >
  -------------
   \   ^__^
    \  (oo)\_______
       (__)\       )\/\
           ||----w |
           ||     ||

```

The `uvx` command installs tools in temporary, isolated environments.

!!! note

    Some tools, such as `pytest` and `mypy`, can require your [_project_](../concepts/projects/index.md)
    to be installed. To run these tools, use [`uv run`](./projects.md#running-commands) instead of
    `uvx`. Otherwise, the tool runs in a virtual environment that is separate from your project.

    Projects with a flat structure do not use a `src` directory for modules. These projects do not
    need to be installed, so you can use `uvx`. Use `uv run` if you want to pin the tool version in
    your project dependencies.

## Commands with different package names

When you run `uvx ruff`, uv installs the `ruff` package and runs the `ruff` command. Some packages
provide commands with different names.

Use `--from` to run a command from a specific package. For example, the `httpie` package provides
the `http` command:

```console
$ uvx --from httpie http
```

## Requesting specific versions

To run a specific tool version, use `command@<version>`:

```console
$ uvx ruff@0.3.0 check
```

To run the latest tool version, use `command@latest`:

```console
$ uvx ruff@latest check
```

You can also use `--from` to specify a package version:

```console
$ uvx --from 'ruff==0.3.0' ruff check
```

To specify a range of versions, run:

```console
$ uvx --from 'ruff>0.2.0,<0.3.0' ruff check
```

The `@` syntax supports only an exact version.

## Requesting extras

Use `--from` to run a tool with extras:

```console
$ uvx --from 'mypy[faster-cache,reports]' mypy --xml-report mypy_report
```

You can also request a specific version:

```console
$ uvx --from 'mypy[faster-cache,reports]==1.13.0' mypy --xml-report mypy_report
```

## Requesting different sources

Use `--from` to install a tool from an alternative source.

For example, to install from Git, run:

```console
$ uvx --from git+https://github.com/httpie/cli httpie
```

To install the latest commit from a specific branch, run:

```console
$ uvx --from git+https://github.com/httpie/cli@master httpie
```

To install a specific tag, run:

```console
$ uvx --from git+https://github.com/httpie/cli@3.2.4 httpie
```

To install a specific commit, run:

```console
$ uvx --from git+https://github.com/httpie/cli@2843b87 httpie
```

To enable [Git LFS](https://git-lfs.com) support, run:

```console
$ uvx --lfs --from git+https://github.com/astral-sh/lfs-cowsay lfs-cowsay
```

## Commands with plugins

Use `--with` to include additional dependencies. For example, include `mkdocs-material` when you run
`mkdocs`:

```console
$ uvx --with mkdocs-material mkdocs --help
```

## Installing tools

If you use a tool frequently, install it in a persistent environment. Add the tool to your `PATH` so
that you do not need to run `uvx` each time.

!!! tip

    The `uvx` command is an alias for `uv tool run`. All other tool commands require the full
    `uv tool` prefix.

To install `ruff`, run:

```console
$ uv tool install ruff
```

When uv installs a tool, it puts the executables in a `bin` directory. If that directory is in your
`PATH`, you can run the tool without uv. If the directory is not in your `PATH`, uv displays a
warning. Run `uv tool update-shell` to add the directory to your `PATH`.

After you install `ruff`, run:

```console
$ ruff --version
```

Unlike `uv pip install`, `uv tool install` does not add tool modules to the current environment. For
example, this command fails:

```console
$ python -c "import ruff"
```

This isolation reduces conflicts between the dependencies of tools, scripts, and projects.

Unlike `uvx`, `uv tool install` installs a _package_ and all executables that the package provides.

For example, this command installs the `http`, `https`, and `httpie` executables:

```console
$ uv tool install httpie
```

You can specify a package version without `--from`:

```console
$ uv tool install 'httpie>0.1.0'
```

You can also specify a package source:

```console
$ uv tool install git+https://github.com/httpie/cli
```

To use [Git LFS](https://git-lfs.com), run:

```console
$ uv tool install --lfs git+https://github.com/astral-sh/lfs-cowsay
```

As with `uvx`, you can include additional packages:

```console
$ uv tool install mkdocs --with mkdocs-material
```

Use `--with-executables-from` to install related executables in the same tool environment. For
example, this command installs executables from `ansible`, `ansible-core`, and `ansible-lint`:

```console
$ uv tool install --with-executables-from ansible-core,ansible-lint ansible
```

## Upgrading tools

To upgrade a tool, use `uv tool upgrade`:

```console
$ uv tool upgrade ruff
```

Tool upgrades use the version constraints from the original installation. For example,
`uv tool install ruff >=0.3,<0.4` limits upgrades to versions in the range `>=0.3,<0.4`. A later
`uv tool upgrade ruff` command installs the latest version in that range.

To replace the version constraints, reinstall the tool with `uv tool install`:

```console
$ uv tool install ruff>=0.4
```

To upgrade all tools, run:

```console
$ uv tool upgrade --all
```

## Requesting Python versions

By default, uv runs, installs, and upgrades tools with the first Python interpreter it finds. Use
`--python` to select a different interpreter.

For example, to run a tool with a specific Python version, use:

```console
$ uvx --python 3.10 ruff
```

To install a tool with a specific Python version, run:

```console
$ uv tool install --python 3.10 ruff
```

To upgrade a tool with a specific Python version, run:

```console
$ uv tool upgrade --python 3.10 ruff
```

For details, see the [Python version](../concepts/python-versions.md#requesting-a-version) concept
page.

## Legacy Windows Scripts

Tools can also run
[legacy setuptools scripts](https://packaging.python.org/en/latest/guides/distributing-packages-using-setuptools/#scripts).
After installation, these scripts are in `$(uv tool dir)\<tool-name>\Scripts`.

uv supports legacy scripts with the `.ps1`, `.cmd`, or `.bat` extension.

For example, this command runs a Command Prompt script:

```console
$ uv tool run --from nuitka==2.6.7 nuitka.cmd --version
```

You do not need to specify the file extension. The `uvx` command checks for `.ps1`, `.cmd`, and
`.bat` files in that order.

```console
$ uv tool run --from nuitka==2.6.7 nuitka --version
```

## Next steps

For details about tools, see the [Tools concept](../concepts/tools.md) page and the
[command reference](../reference/cli.md#uv-tool).

Next, learn how to [work on projects](./projects.md).
