---
title: Installing and managing Python
description:
  Use uv to install Python, request specific versions, download Python automatically, and view
  installed versions.
---

# Installing Python

If your system already has Python, uv [detects and uses](#using-existing-python-versions) it without
configuration. uv can also install and manage Python versions. It
[automatically installs](#automatic-python-downloads) missing versions when necessary. You do not
need to install Python before you start.

## Getting started

To install the latest Python version, run:

```console
$ uv python install
```

!!! note

    Python does not publish official distributable binaries. uv uses distributions from the Astral
    [`python-build-standalone`](https://github.com/astral-sh/python-build-standalone) project. For
    details, see the [Python distributions](../concepts/python-versions.md#managed-python-distributions)
    documentation.

After you install Python, `uv` commands use it automatically. uv also adds the installed version to
your `PATH`:

```console
$ python3.13
```

By default, uv installs only a _versioned_ executable. To install `python` and `python3`, use the
experimental `--default` option:

```console
$ uv python install --default
```

!!! tip

    For details, see [installing Python executables](../concepts/python-versions.md#installing-python-executables).

## Installing a specific version

To install a specific Python version, run:

```console
$ uv python install 3.12
```

To install multiple Python versions, run:

```console
$ uv python install 3.11 3.12
```

To install another Python implementation, such as PyPy, run:

```console
$ uv python install pypy@3.10
```

For details, see the [`python install`](../concepts/python-versions.md#installing-a-python-version)
documentation.

## Reinstalling Python

To reinstall Python versions that uv manages, use `--reinstall`:

```console
$ uv python install --reinstall
```

This command reinstalls all Python versions that uv previously installed. Python distributions
receive frequent improvements. A reinstall can resolve bugs even if the Python version does not
change.

## Viewing Python installations

To view available and installed Python versions, run:

```console
$ uv python list
```

For details, see the
[`python list`](../concepts/python-versions.md#viewing-available-python-versions) documentation.

## Automatic Python downloads

You do not need to install Python before you use uv. By default, uv automatically downloads Python
versions when necessary. For example, this command downloads Python 3.12 if it is not installed:

```console
$ uvx python@3.12 -c "print('hello world')"
```

If you do not request a specific Python version, uv downloads the latest version when necessary. For
example, this command installs Python if your system does not have it. It then creates a virtual
environment:

```console
$ uv venv
```

!!! tip

    To control when uv downloads Python, [disable automatic Python downloads](../concepts/python-versions.md#disabling-automatic-python-downloads).

<!-- TODO(zanieb): Restore when Python shim management is added
Note that when an automatic Python installation occurs, the `python` command will not be added to the shell. Use `uv python install-shim` to ensure the `python` shim is installed.
-->

## Using existing Python versions

uv uses existing Python installations without additional configuration. It uses the system Python if
that version meets the requirements of the command. For details, see the
[Python discovery](../concepts/python-versions.md#discovery-of-python-versions) documentation.

To require the system Python, use the `--no-managed-python` flag. For details, see the
[Python version preference](../concepts/python-versions.md#requiring-or-disabling-managed-python-versions)
documentation.

## Upgrading Python versions

!!! important

    Upgrades to Python patch versions are in _preview_. This experimental behavior can change.

To upgrade a Python version to the latest supported patch release, run:

```console
$ uv python upgrade 3.12
```

To upgrade all Python versions that uv manages, run:

```console
$ uv python upgrade
```

For details, see the [`python upgrade`](../concepts/python-versions.md#upgrading-python-versions)
documentation.

## Next steps

For details about `uv python`, see the [Python version concept](../concepts/python-versions.md) page
and the [command reference](../reference/cli.md#uv-python).

Next, learn how to [run scripts](./scripts.md) and invoke Python with uv.
